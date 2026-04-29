package to.iris.chat.nearby

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Base64
import android.util.Log
import androidx.core.content.ContextCompat
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import java.util.zip.Deflater
import java.util.zip.Inflater
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import to.iris.chat.core.AppManager
import to.iris.chat.core.NearbyPublishedEvent

class IrisNearbyService(
    context: Context,
    private val applicationScope: CoroutineScope,
    private val appManager: AppManager,
) {
    data class Snapshot(
        val visible: Boolean,
        val status: String,
        val peerCount: Int,
        val peers: List<Peer>,
    )

    data class Peer(
        val id: String,
        val name: String,
        val ownerPubkeyHex: String?,
        val pictureUrl: String?,
        val profileEventId: String?,
        val lastSeenMillis: Long,
    )

    private val appContext = context.applicationContext
    private val bluetoothManager = appContext.getSystemService(BluetoothManager::class.java)
    private val adapter: BluetoothAdapter? = bluetoothManager?.adapter
    private val peerId = UUID.randomUUID().toString().lowercase()
    private val ownOutbound = linkedMapOf<String, StoredNearbyEvent>()
    private val forwarded = linkedMapOf<String, StoredNearbyEvent>()
    private val gatts = linkedMapOf<String, BluetoothGatt>()
    private val writableCharacteristics = linkedMapOf<String, BluetoothGattCharacteristic>()
    private val centralAssemblers = linkedMapOf<String, FrameAssembler>()
    private val serverAssemblers = linkedMapOf<String, FrameAssembler>()
    private val peerIdsByAddress = linkedMapOf<String, String>()
    private val peers = linkedMapOf<String, Peer>()
    private val knownProfiles = linkedMapOf<String, NearbyProfileEvent>()
    private val mtuPayloadBytes = linkedMapOf<String, Int>()
    private val incomingFragments = linkedMapOf<String, IncomingFragment>()
    private val sendMutex = Mutex()

    private var gattServer: BluetoothGattServer? = null
    private var visible = false
    private var status = "Off"
    private var ownProfileEventId: String? = null

    val snapshot: Snapshot
        get() =
            Snapshot(
                visible = visible,
                status = status,
                peerCount = peers.size,
                peers =
                    peers.values
                        .sortedWith(compareBy<Peer> { it.name.lowercase() }.thenBy { it.id }),
            )

    fun setVisible(nextVisible: Boolean) {
        if (visible == nextVisible) {
            if (visible) {
                announceToConnectedPeers()
            }
            return
        }
        visible = nextVisible
        if (nextVisible) {
            Log.d(TAG, "visible on")
            start()
        } else {
            Log.d(TAG, "visible off")
            stop()
        }
    }

    fun toggleVisible() {
        setVisible(!visible)
    }

    fun publish(event: NearbyPublishedEvent) {
        val record =
            StoredNearbyEvent(
                id = event.eventId,
                kind = event.kind.toLong(),
                createdAtSecs = event.createdAtSecs.toLong(),
                eventJson = event.eventJson,
                storedAtMillis = System.currentTimeMillis(),
            )
        ownOutbound[event.eventId] = record
        forwarded.remove(event.eventId)
        if (record.kind == 0L) {
            NearbyProfileEvent.fromEventJson(event.eventJson)?.let { profile ->
                ownProfileEventId = record.id
                knownProfiles[record.id] = profile
            }
        }
        pruneMailbags()
        Log.d(TAG, "published event kind=${record.kind} id=${record.id} visible=$visible")
        if (visible) {
            if (record.kind == 0L) {
                sendHello(excludingPeerId = null)
            }
            sendEvent(record, excludingPeerId = null)
        }
    }

    private fun start() {
        if (!hasBluetoothPermissions()) {
            status = "No Bluetooth access"
            Log.w(TAG, "missing Bluetooth permissions")
            return
        }
        if (adapter?.isEnabled != true) {
            status = "Bluetooth off"
            return
        }
        status = "Starting"
        startGattServer()
        startAdvertising()
        startScanning()
    }

    @SuppressLint("MissingPermission")
    private fun stop() {
        status = "Off"
        runCatching { adapter?.bluetoothLeScanner?.stopScan(scanCallback) }
        runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback) }
        gatts.values.forEach { gatt -> runCatching { gatt.close() } }
        gatts.clear()
        writableCharacteristics.clear()
        centralAssemblers.clear()
        serverAssemblers.clear()
        peerIdsByAddress.clear()
        mtuPayloadBytes.clear()
        incomingFragments.clear()
        peers.clear()
        runCatching { gattServer?.close() }
        gattServer = null
    }

    @SuppressLint("MissingPermission")
    private fun startGattServer() {
        val manager = bluetoothManager ?: return
        val server = manager.openGattServer(appContext, gattServerCallback) ?: return
        val service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        val characteristic =
            BluetoothGattCharacteristic(
                CHARACTERISTIC_UUID,
                BluetoothGattCharacteristic.PROPERTY_WRITE or
                    BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE or
                    BluetoothGattCharacteristic.PROPERTY_NOTIFY,
                BluetoothGattCharacteristic.PERMISSION_WRITE,
            )
        characteristic.addDescriptor(
            BluetoothGattDescriptor(
                CLIENT_CONFIG_UUID,
                BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE,
            ),
        )
        service.addCharacteristic(characteristic)
        server.addService(service)
        gattServer = server
    }

    @SuppressLint("MissingPermission")
    private fun startAdvertising() {
        val advertiser = adapter?.bluetoothLeAdvertiser
        if (advertiser == null) {
            status = "Advertise unavailable"
            return
        }
        val settings =
            AdvertiseSettings.Builder()
                .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
                .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
                .setConnectable(true)
                .build()
        val data =
            AdvertiseData.Builder()
                .addServiceUuid(ParcelUuid(SERVICE_UUID))
                .setIncludeDeviceName(false)
                .build()
        advertiser.startAdvertising(settings, data, advertiseCallback)
    }

    @SuppressLint("MissingPermission")
    private fun startScanning() {
        val scanner = adapter?.bluetoothLeScanner ?: return
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE_UUID)).build()
        val settings =
            ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build()
        status = "Scanning"
        scanner.startScan(listOf(filter), settings, scanCallback)
    }

    @SuppressLint("MissingPermission")
    private fun connect(device: BluetoothDevice) {
        val address = device.address
        if (gatts.containsKey(address)) {
            return
        }
        status = "Connecting"
        gatts[address] = device.connectGatt(appContext, false, gattCallback, BluetoothDevice.TRANSPORT_LE)
    }

    private fun announceToConnectedPeers() {
        sendHello(excludingPeerId = null)
        sendInventory(excludingPeerId = null)
    }

    private fun sendHello(excludingPeerId: String?) {
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "hello")
                .put("peer_id", peerId)
                .put("name", "Iris")
        ownProfileEventId?.let { envelope.put("profile_event_id", it) }
        sendEnvelope(envelope, excludingPeerId)
    }

    private fun sendInventory(excludingPeerId: String?) {
        val records = mailbagEvents().take(200)
        if (records.isEmpty()) {
            return
        }
        val events = JSONArray()
        records.forEach { record ->
            events.put(
                JSONObject()
                    .put("id", record.id)
                    .put("kind", record.kind)
                    .put("created_at", record.createdAtSecs)
                    .put("size", record.eventJson.toByteArray(Charsets.UTF_8).size),
            )
        }
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "inv")
                .put("peer_id", peerId)
                .put("events", events)
        sendEnvelope(envelope, excludingPeerId)
    }

    private fun sendWant(ids: List<String>, excludingPeerId: String?) {
        if (ids.isEmpty()) {
            return
        }
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "want")
                .put("peer_id", peerId)
                .put("ids", JSONArray(ids))
        sendEnvelope(envelope, excludingPeerId)
    }

    private fun sendEvent(record: StoredNearbyEvent, excludingPeerId: String?) {
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "event")
                .put("peer_id", peerId)
                .put("event_json", record.eventJson)
        val frame = FrameCodec.encode(envelope)
        if (frame != null && frame.size <= SINGLE_FRAME_BYTES) {
            sendFrame("event", frame, excludingPeerId)
        } else {
            sendEventFragments(record, excludingPeerId)
        }
    }

    private fun sendEventFragments(record: StoredNearbyEvent, excludingPeerId: String?) {
        val bytes = record.eventJson.toByteArray(Charsets.UTF_8)
        val total = (bytes.size + FRAGMENT_PAYLOAD_BYTES - 1) / FRAGMENT_PAYLOAD_BYTES
        if (total <= 1 || total > MAX_EVENT_FRAGMENTS) {
            return
        }
        val fragmentId = UUID.randomUUID().toString().lowercase()
        val frames = mutableListOf<ByteArray>()
        for (index in 0 until total) {
            val start = index * FRAGMENT_PAYLOAD_BYTES
            val end = minOf(start + FRAGMENT_PAYLOAD_BYTES, bytes.size)
            val chunk = bytes.copyOfRange(start, end)
            val envelope =
                JSONObject()
                    .put("v", 1)
                    .put("type", "event_frag")
                    .put("peer_id", peerId)
                    .put("frag_id", fragmentId)
                    .put("event_id", record.id)
                    .put("index", index)
                    .put("total", total)
                    .put("data", Base64.encodeToString(chunk, Base64.NO_WRAP))
            FrameCodec.encode(envelope)?.let { frames += it }
        }
        if (frames.size != total) {
            return
        }
        Log.d(TAG, "send event_frag count=${frames.size} event=${record.id} peers=${peers.size}")
        applicationScope.launch {
            sendMutex.withLock {
                frames.forEach { frame ->
                    sendFrame(frame, excludingPeerId)
                    delay(FRAGMENT_SEND_DELAY_MS)
                }
            }
        }
    }

    private fun sendEnvelope(envelope: JSONObject, excludingPeerId: String?) {
        if (!visible) {
            return
        }
        val type = envelope.optString("type", "unknown")
        val frame = FrameCodec.encode(envelope) ?: return
        sendFrame(type, frame, excludingPeerId)
    }

    private fun sendFrame(type: String, frame: ByteArray, excludingPeerId: String?) {
        if (!visible) {
            return
        }
        Log.d(TAG, "send $type bytes=${frame.size} peers=${peers.size}")
        applicationScope.launch {
            sendMutex.withLock {
                sendFrame(frame, excludingPeerId)
            }
        }
    }

    @SuppressLint("MissingPermission")
    private suspend fun sendFrame(frame: ByteArray, excludingPeerId: String?) {
        writableCharacteristics.toList().forEach { (address, characteristic) ->
            if (peerIdsByAddress[address] == excludingPeerId) {
                return@forEach
            }
            val gatt = gatts[address] ?: return@forEach
            writeToGatt(gatt, characteristic, frame)
        }
        val server = gattServer ?: return
        val characteristic = server.getService(SERVICE_UUID)?.getCharacteristic(CHARACTERISTIC_UUID) ?: return
        bluetoothManager
            ?.getConnectedDevices(BluetoothProfile.GATT)
            ?.toList()
            ?.forEach { device ->
                if (peerIdsByAddress[device.address] == excludingPeerId) {
                    return@forEach
                }
                notifyDevice(server, device, characteristic, frame)
            }
    }

    @SuppressLint("MissingPermission")
    private suspend fun writeToGatt(
        gatt: BluetoothGatt,
        characteristic: BluetoothGattCharacteristic,
        data: ByteArray,
    ) {
        var offset = 0
        val chunkSize = mtuPayloadBytes[gatt.device.address] ?: BLE_CHUNK_BYTES
        while (offset < data.size) {
            val chunk = data.copyOfRange(offset, minOf(offset + chunkSize, data.size))
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                gatt.writeCharacteristic(characteristic, chunk, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT)
            } else {
                @Suppress("DEPRECATION")
                characteristic.value = chunk
                characteristic.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
                @Suppress("DEPRECATION")
                gatt.writeCharacteristic(characteristic)
            }
            offset += chunk.size
            delay(BLE_CHUNK_DELAY_MS)
        }
    }

    @SuppressLint("MissingPermission")
    private suspend fun notifyDevice(
        server: BluetoothGattServer,
        device: BluetoothDevice,
        characteristic: BluetoothGattCharacteristic,
        data: ByteArray,
    ) {
        var offset = 0
        val chunkSize = mtuPayloadBytes[device.address] ?: BLE_CHUNK_BYTES
        while (offset < data.size) {
            val chunk = data.copyOfRange(offset, minOf(offset + chunkSize, data.size))
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                server.notifyCharacteristicChanged(device, characteristic, false, chunk)
            } else {
                @Suppress("DEPRECATION")
                characteristic.value = chunk
                @Suppress("DEPRECATION")
                server.notifyCharacteristicChanged(device, characteristic, false)
            }
            offset += chunk.size
            delay(BLE_CHUNK_DELAY_MS)
        }
    }

    private fun ingestFrame(frame: ByteArray, address: String?) {
        val envelope = FrameCodec.decode(frame) ?: return
        val type = envelope.optString("type")
        val remotePeerId = envelope.optString("peer_id").trim()
        if (remotePeerId == peerId) {
            return
        }
        when (type) {
            "hello" -> {
                if (remotePeerId.isNotEmpty()) {
                    if (address != null) {
                        peerIdsByAddress[address] = remotePeerId
                    }
                    val wasNew = rememberPeer(
                        peerId = remotePeerId,
                        name = envelope.optString("name"),
                        profileEventId = envelope.optString("profile_event_id"),
                    )
                    if (wasNew) {
                        Log.d(TAG, "peer nearby id=$remotePeerId")
                    }
                }
                status = if (peers.size == 1) "1 nearby" else "${peers.size} nearby"
                sendInventory(excludingPeerId = null)
            }
            "inv" -> handleInventory(envelope)
            "want" -> handleWant(envelope)
            "event" -> handleEventEnvelope(envelope, remotePeerId.takeIf(String::isNotEmpty))
            "event_frag" -> handleEventFragment(envelope, remotePeerId.takeIf(String::isNotEmpty))
        }
    }

    private fun handleInventory(envelope: JSONObject) {
        val events = envelope.optJSONArray("events") ?: return
        val wanted = mutableListOf<String>()
        for (index in 0 until minOf(events.length(), 200)) {
            val item = events.optJSONObject(index) ?: continue
            val id = item.optString("id")
            val size = item.optInt("size", 0)
            if (
                id.length == 64 &&
                size in 1..MAX_EVENT_BYTES &&
                !ownOutbound.containsKey(id) &&
                !forwarded.containsKey(id)
            ) {
                wanted += id
            }
        }
        sendWant(wanted.take(64), excludingPeerId = null)
    }

    private fun handleWant(envelope: JSONObject) {
        val ids = envelope.optJSONArray("ids") ?: return
        for (index in 0 until minOf(ids.length(), 64)) {
            val id = ids.optString(index)
            val record = ownOutbound[id] ?: forwarded[id] ?: continue
            sendEvent(record, excludingPeerId = null)
        }
    }

    private fun handleEventEnvelope(envelope: JSONObject, remotePeerId: String?) {
        val eventJson = envelope.optString("event_json")
        handleEventJson(eventJson, remotePeerId)
    }

    private fun handleEventFragment(envelope: JSONObject, remotePeerId: String?) {
        pruneIncomingFragments()
        val fragmentId = envelope.optString("frag_id")
        val index = envelope.optInt("index", -1)
        val total = envelope.optInt("total", -1)
        val data = runCatching {
            Base64.decode(envelope.optString("data"), Base64.NO_WRAP)
        }.getOrNull() ?: return
        if (
            fragmentId.isBlank() ||
            index !in 0 until total ||
            total !in 1..MAX_EVENT_FRAGMENTS ||
            data.isEmpty()
        ) {
            return
        }
        val fragment =
            incomingFragments.getOrPut(fragmentId) {
                IncomingFragment(
                    total = total,
                    parts = linkedMapOf(),
                    storedAtMillis = System.currentTimeMillis(),
                    remotePeerId = remotePeerId,
                )
            }
        if (fragment.total != total) {
            incomingFragments.remove(fragmentId)
            return
        }
        fragment.parts[index] = data
        if (fragment.parts.size != total) {
            return
        }
        val output = ByteArrayOutputStream()
        for (partIndex in 0 until total) {
            output.write(fragment.parts[partIndex] ?: return)
            if (output.size() > MAX_EVENT_BYTES) {
                incomingFragments.remove(fragmentId)
                return
            }
        }
        incomingFragments.remove(fragmentId)
        val eventJson = output.toString(Charsets.UTF_8.name())
        handleEventJson(eventJson, remotePeerId ?: fragment.remotePeerId)
    }

    private fun handleEventJson(eventJson: String, remotePeerId: String?) {
        if (eventJson.toByteArray(Charsets.UTF_8).size > MAX_EVENT_BYTES) {
            return
        }
        val record = StoredNearbyEvent.fromEventJson(eventJson) ?: return
        val existing = ownOutbound[record.id] ?: forwarded[record.id]
        if (existing != null) {
            rememberProfile(existing.eventJson, remotePeerId)
            return
        }
        if (!appManager.ingestNearbyEventJson(eventJson)) {
            return
        }
        rememberProfile(eventJson, remotePeerId)
        forwarded[record.id] = record
        pruneMailbags()
        sendInventory(excludingPeerId = remotePeerId)
        Log.d(TAG, "accepted event kind=${record.kind} id=${record.id}")
    }

    private fun pruneIncomingFragments() {
        val cutoff = System.currentTimeMillis() - FRAGMENT_TTL_MS
        incomingFragments.entries.removeAll { it.value.storedAtMillis < cutoff }
        while (incomingFragments.size > MAX_INCOMING_FRAGMENT_SETS) {
            val oldest = incomingFragments.entries.minByOrNull { it.value.storedAtMillis }?.key ?: return
            incomingFragments.remove(oldest)
        }
    }

    private fun rememberPeer(
        peerId: String,
        name: String?,
        profileEventId: String?,
    ): Boolean {
        val existing = peers[peerId]
        val sanitizedProfileEventId = profileEventId.sanitizedEventId()
        peers[peerId] =
            Peer(
                id = peerId,
                name = name.sanitizedName() ?: existing?.name ?: "Iris",
                ownerPubkeyHex = existing?.ownerPubkeyHex,
                pictureUrl = existing?.pictureUrl,
                profileEventId = sanitizedProfileEventId ?: existing?.profileEventId,
                lastSeenMillis = System.currentTimeMillis(),
            )
        val profile = peers[peerId]?.profileEventId?.let { knownProfiles[it] }
        if (profile != null) {
            applyAdvertisedProfile(peerId, profile)
        }
        return existing == null
    }

    private fun rememberProfile(eventJson: String, remotePeerId: String?) {
        val profile = NearbyProfileEvent.fromEventJson(eventJson) ?: return
        knownProfiles[profile.id] = profile
        if (remotePeerId != null) {
            applyAdvertisedProfile(remotePeerId, profile)
        }
    }

    private fun applyAdvertisedProfile(peerId: String, profile: NearbyProfileEvent) {
        val peer = peers[peerId] ?: return
        if (peer.profileEventId != profile.id) {
            return
        }
        peers[peerId] =
            peer.copy(
                name = profile.displayName ?: peer.name,
                ownerPubkeyHex = profile.ownerPubkeyHex,
                pictureUrl = profile.pictureUrl ?: peer.pictureUrl,
                lastSeenMillis = System.currentTimeMillis(),
            )
    }

    private fun mailbagEvents(): List<StoredNearbyEvent> {
        val records =
            (ownOutbound.values + forwarded.values)
                .sortedByDescending { it.createdAtSecs }
                .toMutableList()
        val profile = ownProfileEventId?.let { ownOutbound[it] }
        if (profile != null) {
            records.removeAll { it.id == profile.id }
            records.add(0, profile)
        }
        return records
    }

    private fun pruneMailbags() {
        prune(ownOutbound, preservingId = ownProfileEventId)
        prune(forwarded, preservingId = null)
        pruneKnownProfiles()
    }

    private fun prune(
        bag: LinkedHashMap<String, StoredNearbyEvent>,
        preservingId: String?,
    ) {
        if (bag.size <= MAX_MAILBAG_EVENTS) {
            return
        }
        val keep =
            bag.values
                .sortedByDescending { it.createdAtSecs }
                .take(MAX_MAILBAG_EVENTS)
                .map { it.id }
                .toSet()
        bag.keys.toList().forEach { id ->
            if (!keep.contains(id) && id != preservingId) {
                bag.remove(id)
            }
        }
    }

    private fun pruneKnownProfiles() {
        val keep = linkedSetOf<String>()
        keep += ownOutbound.keys
        keep += forwarded.keys
        keep += peers.values.mapNotNull { it.profileEventId }
        ownProfileEventId?.let { keep += it }
        knownProfiles.keys.toList().forEach { id ->
            if (!keep.contains(id)) {
                knownProfiles.remove(id)
            }
        }
    }

    private fun String?.sanitizedName(): String? =
        this?.trim()?.takeIf(String::isNotEmpty)

    private fun String?.sanitizedEventId(): String? =
        this?.trim()?.takeIf { it.length == 64 }

    private fun hasBluetoothPermissions(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            return ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH) ==
                PackageManager.PERMISSION_GRANTED &&
                ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_ADMIN) ==
                PackageManager.PERMISSION_GRANTED &&
                ContextCompat.checkSelfPermission(appContext, Manifest.permission.ACCESS_FINE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED
        }
        return ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_SCAN) ==
            PackageManager.PERMISSION_GRANTED &&
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_CONNECT) ==
            PackageManager.PERMISSION_GRANTED &&
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_ADVERTISE) ==
            PackageManager.PERMISSION_GRANTED
    }

    private val scanCallback =
        object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                if (hasBluetoothPermissions()) {
                    connect(result.device)
                }
            }
        }

    private val advertiseCallback =
        object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                status = if (peers.isEmpty()) "Visible" else "${peers.size} nearby"
                Log.d(TAG, "advertising")
            }

            override fun onStartFailure(errorCode: Int) {
                status = "Advertise failed"
                Log.w(TAG, "advertise failed code=$errorCode")
            }
        }

    private val gattCallback =
        object : BluetoothGattCallback() {
            @SuppressLint("MissingPermission")
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    val address = gatt.device.address
                    applicationScope.launch {
                        delay(200)
                        val requested = runCatching { gatt.requestMtu(517) }.getOrDefault(false)
                        if (!requested) {
                            Log.w(TAG, "MTU request failed to start for $address")
                            gatt.discoverServices()
                        }
                    }
                } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    val address = gatt.device.address
                    gatts.remove(address)
                    writableCharacteristics.remove(address)
                    centralAssemblers.remove(address)
                    mtuPayloadBytes.remove(address)
                    peerIdsByAddress.remove(address)?.let { peers.remove(it) }
                    gatt.close()
                }
            }

            @SuppressLint("MissingPermission")
            override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
                val address = gatt.device.address
                if (status == BluetoothGatt.GATT_SUCCESS) {
                    mtuPayloadBytes[address] = (mtu - 3).coerceAtLeast(20)
                    Log.d(TAG, "MTU $mtu for $address")
                } else {
                    Log.w(TAG, "MTU request failed for $address status=$status")
                }
                gatt.discoverServices()
            }

            @SuppressLint("MissingPermission")
            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                val characteristic =
                    gatt.getService(SERVICE_UUID)?.getCharacteristic(CHARACTERISTIC_UUID) ?: return
                val address = gatt.device.address
                writableCharacteristics[address] = characteristic
                centralAssemblers[address] = FrameAssembler()
                gatt.setCharacteristicNotification(characteristic, true)
                characteristic.getDescriptor(CLIENT_CONFIG_UUID)?.let { descriptor ->
                    val value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        gatt.writeDescriptor(descriptor, value)
                    } else {
                        @Suppress("DEPRECATION")
                        descriptor.value = value
                        @Suppress("DEPRECATION")
                        gatt.writeDescriptor(descriptor)
                    }
                }
                sendHello(excludingPeerId = null)
                sendInventory(excludingPeerId = null)
            }

            override fun onCharacteristicChanged(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                value: ByteArray,
            ) {
                ingestChunk(gatt.device.address, value, centralAssemblers)
            }

            @Deprecated("Deprecated by Android")
            override fun onCharacteristicChanged(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
            ) {
                @Suppress("DEPRECATION")
                ingestChunk(gatt.device.address, characteristic.value ?: return, centralAssemblers)
            }
        }

    private val gattServerCallback =
        object : BluetoothGattServerCallback() {
            override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
                mtuPayloadBytes[device.address] = (mtu - 3).coerceAtLeast(20)
                Log.d(TAG, "server MTU $mtu for ${device.address}")
            }

            override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    serverAssemblers.remove(device.address)
                    mtuPayloadBytes.remove(device.address)
                    peerIdsByAddress.remove(device.address)?.let { peers.remove(it) }
                }
            }

            override fun onCharacteristicWriteRequest(
                device: BluetoothDevice,
                requestId: Int,
                characteristic: BluetoothGattCharacteristic,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray,
            ) {
                ingestChunk(device.address, value, serverAssemblers)
                if (responseNeeded && hasBluetoothPermissions()) {
                    @SuppressLint("MissingPermission")
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
                }
            }

            override fun onDescriptorWriteRequest(
                device: BluetoothDevice,
                requestId: Int,
                descriptor: BluetoothGattDescriptor,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray,
            ) {
                if (responseNeeded && hasBluetoothPermissions()) {
                    @SuppressLint("MissingPermission")
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
                }
                sendHello(excludingPeerId = null)
                sendInventory(excludingPeerId = null)
            }
        }

    private fun ingestChunk(
        address: String,
        chunk: ByteArray,
        assemblers: MutableMap<String, FrameAssembler>,
    ) {
        val assembler = assemblers.getOrPut(address) { FrameAssembler() }
        assembler.append(chunk).forEach { frame ->
            ingestFrame(frame, address)
        }
    }

    private data class StoredNearbyEvent(
        val id: String,
        val kind: Long,
        val createdAtSecs: Long,
        val eventJson: String,
        val storedAtMillis: Long,
    ) {
        companion object {
            fun fromEventJson(eventJson: String): StoredNearbyEvent? {
                val json = runCatching { JSONObject(eventJson) }.getOrNull() ?: return null
                val id = json.optString("id")
                if (id.isBlank()) {
                    return null
                }
                return StoredNearbyEvent(
                    id = id,
                    kind = json.optLong("kind", 0L),
                    createdAtSecs = json.optLong("created_at", 0L),
                    eventJson = eventJson,
                    storedAtMillis = System.currentTimeMillis(),
                )
            }
        }
    }

    private data class NearbyProfileEvent(
        val id: String,
        val ownerPubkeyHex: String,
        val displayName: String?,
        val pictureUrl: String?,
    ) {
        companion object {
            fun fromEventJson(eventJson: String): NearbyProfileEvent? {
                val json = runCatching { JSONObject(eventJson) }.getOrNull() ?: return null
                val id = json.optString("id").trim()
                if (id.length != 64 || json.optLong("kind", -1L) != 0L) {
                    return null
                }
                val ownerPubkeyHex = json.optString("pubkey").trim()
                if (ownerPubkeyHex.length != 64) {
                    return null
                }
                val metadata =
                    runCatching { JSONObject(json.optString("content")) }.getOrNull() ?: return null
                return NearbyProfileEvent(
                    id = id,
                    ownerPubkeyHex = ownerPubkeyHex,
                    displayName =
                        metadata.optString("display_name").trim().takeIf(String::isNotEmpty)
                            ?: metadata.optString("name").trim().takeIf(String::isNotEmpty),
                    pictureUrl = metadata.optString("picture").trim().takeIf(String::isNotEmpty),
                )
            }
        }
    }

    private data class IncomingFragment(
        val total: Int,
        val parts: LinkedHashMap<Int, ByteArray>,
        val storedAtMillis: Long,
        val remotePeerId: String?,
    )

    private object FrameCodec {
        private val magic = byteArrayOf(0x49, 0x52, 0x49, 0x53)
        private const val COMPRESSED_FLAG = 0x01
        private const val HEADER_SIZE = 13
        private const val COMPRESSION_THRESHOLD = 100

        fun encode(envelope: JSONObject): ByteArray? {
            val payload = envelope.toString().toByteArray(Charsets.UTF_8)
            if (payload.size > MAX_FRAME_BYTES) {
                return null
            }
            val compressed = compressIfBeneficial(payload)
            val body = compressed ?: payload
            if (body.size > MAX_FRAME_BYTES) {
                return null
            }
            val buffer = ByteBuffer.allocate(HEADER_SIZE + body.size).order(ByteOrder.BIG_ENDIAN)
            buffer.put(magic)
            buffer.put(if (compressed == null) 0 else COMPRESSED_FLAG.toByte())
            buffer.putInt(body.size)
            buffer.putInt(payload.size)
            buffer.put(body)
            return buffer.array()
        }

        fun decode(frame: ByteArray): JSONObject? {
            if (frame.size < HEADER_SIZE || !frame.take(4).toByteArray().contentEquals(magic)) {
                return null
            }
            val flags = frame[4].toInt()
            val originalSize = ByteBuffer.wrap(frame, 9, 4).order(ByteOrder.BIG_ENDIAN).int
            val payload = frame.copyOfRange(HEADER_SIZE, frame.size)
            val body =
                if ((flags and COMPRESSED_FLAG) != 0) {
                    decompress(payload, originalSize) ?: return null
                } else {
                    payload
                }
            return runCatching {
                JSONObject(String(body, Charsets.UTF_8))
            }.getOrNull()
        }

        private fun compressIfBeneficial(data: ByteArray): ByteArray? {
            if (data.size < COMPRESSION_THRESHOLD) {
                return null
            }
            return runCatching {
                val deflater = Deflater(Deflater.DEFAULT_COMPRESSION, true)
                deflater.setInput(data)
                deflater.finish()
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(1024)
                while (!deflater.finished()) {
                    val count = deflater.deflate(buffer)
                    if (count <= 0) {
                        break
                    }
                    output.write(buffer, 0, count)
                }
                deflater.end()
                output.toByteArray().takeIf { it.isNotEmpty() && it.size < data.size }
            }.getOrNull()
        }

        private fun decompress(data: ByteArray, originalSize: Int): ByteArray? {
            if (originalSize <= 0 || originalSize > MAX_FRAME_BYTES) {
                return null
            }
            return runCatching {
                val inflater = Inflater(true)
                inflater.setInput(data)
                val output = ByteArray(originalSize)
                val size = inflater.inflate(output)
                inflater.end()
                output.takeIf { size == originalSize }
            }.getOrNull()
        }
    }

    private class FrameAssembler {
        private val buffer = ByteArrayOutputStream()

        fun append(chunk: ByteArray): List<ByteArray> {
            buffer.write(chunk)
            val frames = mutableListOf<ByteArray>()
            while (buffer.size() >= 13) {
                val data = buffer.toByteArray()
                if (!data.take(4).toByteArray().contentEquals(MAGIC)) {
                    buffer.reset()
                    buffer.write(data.copyOfRange(1, data.size))
                    continue
                }
                val length = ByteBuffer.wrap(data, 5, 4).order(ByteOrder.BIG_ENDIAN).int
                if (length <= 0 || length > MAX_FRAME_BYTES) {
                    buffer.reset()
                    buffer.write(data.copyOfRange(1, data.size))
                    continue
                }
                val frameLength = 13 + length
                if (data.size < frameLength) {
                    break
                }
                frames += data.copyOfRange(0, frameLength)
                buffer.reset()
                if (data.size > frameLength) {
                    buffer.write(data.copyOfRange(frameLength, data.size))
                }
            }
            return frames
        }
    }

    private companion object {
        const val TAG = "IrisNearby"
        val SERVICE_UUID: UUID = UUID.fromString("8A0DAE01-D8E5-4F27-9F20-A616F1FBA6D0")
        val CHARACTERISTIC_UUID: UUID = UUID.fromString("8A0DAE02-D8E5-4F27-9F20-A616F1FBA6D0")
        val CLIENT_CONFIG_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        val MAGIC = byteArrayOf(0x49, 0x52, 0x49, 0x53)
        const val SINGLE_FRAME_BYTES = 480
        const val FRAGMENT_PAYLOAD_BYTES = 180
        const val MAX_EVENT_FRAGMENTS = 1024
        const val MAX_INCOMING_FRAGMENT_SETS = 64
        const val FRAGMENT_TTL_MS = 30_000L
        const val BLE_CHUNK_BYTES = 180
        const val BLE_CHUNK_DELAY_MS = 30L
        const val FRAGMENT_SEND_DELAY_MS = 30L
        const val MAX_EVENT_BYTES = 128 * 1024
        const val MAX_MAILBAG_EVENTS = 500
        const val MAX_FRAME_BYTES = 256 * 1024
    }
}
