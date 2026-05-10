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
import android.bluetooth.BluetoothStatusCodes
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
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import androidx.core.content.ContextCompat
import java.io.ByteArrayOutputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
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
        val bluetoothOn: Boolean,
        val bluetoothPermissionGranted: Boolean,
        val localNetworkVisible: Boolean,
        val localNetworkPermissionGranted: Boolean,
        val localNetworkStatus: String,
        val peerCount: Int,
        val peers: List<Peer>,
        val bluetoothPeers: List<Peer>,
        val localNetworkPeers: List<Peer>,
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
    private val lanService =
        IrisNearbyLanService(
            context = appContext,
            applicationScope = applicationScope,
            peerId = peerId,
            frameBodyLength = { header -> appManager.nearbyFrameBodyLenFromHeader(header) },
            onFrame = { connectionId, frame -> ingestFrame(frame, NearbySource.Lan(connectionId)) },
            onConnected = { announceToConnectedPeers() },
        )
    private val ownOutbound = linkedMapOf<String, StoredNearbyEvent>()
    private val forwarded = linkedMapOf<String, StoredNearbyEvent>()
    private val gatts = linkedMapOf<String, BluetoothGatt>()
    private val writableCharacteristics = linkedMapOf<String, BluetoothGattCharacteristic>()
    private val centralAssemblers = linkedMapOf<String, FrameAssembler>()
    private val serverAssemblers = linkedMapOf<String, FrameAssembler>()
    private val peerIdsByAddress = linkedMapOf<String, String>()
    private val bluetoothPeerLastSeenMillis = linkedMapOf<String, Long>()
    private val peerInventorySentMillis = linkedMapOf<String, Long>()
    private val centralReconnectSuppressedUntilMillis = linkedMapOf<String, Long>()
    private val peerNonces = linkedMapOf<String, String>()
    private val connectionNonces = linkedMapOf<String, String>()
    private val peers = linkedMapOf<String, Peer>()
    private val knownProfiles = linkedMapOf<String, NearbyProfileEvent>()
    private val mtuPayloadBytes = linkedMapOf<String, Int>()
    private val subscribedServerAddresses = ConcurrentHashMap.newKeySet<String>()
    private val pendingGattWrites = ConcurrentHashMap<String, CompletableDeferred<Int>>()
    private val pendingNotifications = ConcurrentHashMap<String, CompletableDeferred<Int>>()
    private val ignoredAddresses = linkedMapOf<String, Long>()
    private val incomingFragments = linkedMapOf<String, IncomingFragment>()
    private val sendMutex = Mutex()

    private sealed interface NearbySource {
        data class BluetoothAddress(val address: String?) : NearbySource
        data class Lan(val connectionId: String) : NearbySource
    }

    private var gattServer: BluetoothGattServer? = null
    private var visible = false
    private var localNetworkVisible = false
    private var status = "Off"
    private var localNonce = newNonce()
    private var ownProfileEventId: String? = null
    private var maintenanceJob: Job? = null

    val snapshot: Snapshot
        get() {
            val bluetoothPermissionGranted = hasBluetoothPermission()
            val localNetworkPermissionGranted = hasLocalNetworkPermission()
            val sortedPeers =
                peers.values
                    .sortedWith(compareBy<Peer> { it.name.lowercase() }.thenBy { it.id })
            val bluetoothPeerIds = recentBluetoothPeerIds()
            val localNetworkPeerIds = lanService.peerIds()
            return Snapshot(
                visible = visible,
                status = if (!visible && !bluetoothPermissionGranted) "No Bluetooth access" else status,
                bluetoothOn = isBluetoothOn(),
                bluetoothPermissionGranted = bluetoothPermissionGranted,
                localNetworkVisible = localNetworkVisible,
                localNetworkPermissionGranted = localNetworkPermissionGranted,
                localNetworkStatus =
                    when {
                        localNetworkVisible -> lanService.status
                        !localNetworkPermissionGranted -> "No local network access"
                        else -> "Off"
                    },
                peerCount = sortedPeers.size,
                peers = sortedPeers,
                bluetoothPeers = sortedPeers.filter { it.id in bluetoothPeerIds },
                localNetworkPeers = sortedPeers.filter { it.id in localNetworkPeerIds },
            )
        }

    fun hasBluetoothPermission(): Boolean =
        nearbyPermissions().all {
            ContextCompat.checkSelfPermission(appContext, it) == PackageManager.PERMISSION_GRANTED
        }

    fun hasLocalNetworkPermission(): Boolean =
        localNetworkPermissions().all {
            ContextCompat.checkSelfPermission(appContext, it) == PackageManager.PERMISSION_GRANTED
        }

    private fun isBluetoothOn(): Boolean =
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
                ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_CONNECT) !=
                PackageManager.PERMISSION_GRANTED
            ) {
                false
            } else {
                adapter?.isEnabled == true
            }
        } catch (_: SecurityException) {
            false
        }

    private fun nearbyPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_CONNECT,
                Manifest.permission.BLUETOOTH_ADVERTISE,
            )
        } else {
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    private fun localNetworkPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(Manifest.permission.NEARBY_WIFI_DEVICES)
        } else {
            emptyArray()
        }

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
            localNonce = newNonce()
            start()
        } else {
            Log.d(TAG, "visible off")
            stop()
        }
    }

    fun setLocalNetworkVisible(nextVisible: Boolean) {
        if (localNetworkVisible == nextVisible) {
            if (localNetworkVisible) {
                lanService.start()
                announceToConnectedPeers()
            }
            return
        }
        if (nextVisible && !hasLocalNetworkPermission()) {
            localNetworkVisible = false
            lanService.stop()
            return
        }
        localNetworkVisible = nextVisible
        if (nextVisible) {
            Log.d(TAG, "local network on")
            localNonce = newNonce()
            lanService.start()
            startMaintenance()
        } else {
            Log.d(TAG, "local network off")
            val lanPeerIds = lanService.peerIds()
            lanService.stop()
            removeLanOnlyPeers(lanPeerIds)
            if (!nearbyActive()) {
                stopMaintenance()
            }
        }
    }

    fun toggleVisible() {
        setVisible(!visible)
    }

    private fun nearbyActive(): Boolean = visible || localNetworkVisible

    private inline fun <T> guardBluetooth(
        operation: String,
        fallback: T,
        statusOnFailure: String? = "Bluetooth unavailable",
        block: () -> T,
    ): T =
        try {
            block()
        } catch (error: Throwable) {
            if (shouldRethrow(error)) {
                throw error
            }
            statusOnFailure?.let { status = it }
            Log.w(TAG, "$operation failed", error)
            fallback
        }

    private suspend fun <T> guardBluetoothSuspend(
        operation: String,
        fallback: T,
        statusOnFailure: String? = "Bluetooth unavailable",
        block: suspend () -> T,
    ): T =
        try {
            block()
        } catch (error: Throwable) {
            if (shouldRethrow(error)) {
                throw error
            }
            statusOnFailure?.let { status = it }
            Log.w(TAG, "$operation failed", error)
            fallback
        }

    private fun launchBluetooth(
        operation: String,
        block: suspend () -> Unit,
    ) {
        applicationScope.launch {
            guardBluetoothSuspend(operation, Unit, block = block)
        }
    }

    fun publish(event: NearbyPublishedEvent) {
        val record =
            StoredNearbyEvent(
                id = event.eventId,
                kind = event.kind.toLong(),
                createdAtSecs = event.createdAtSecs.toLong(),
                eventJson = event.eventJson,
                authorPubkeyHex = eventAuthorHex(event.eventJson),
                storedAtMillis = System.currentTimeMillis(),
            )
        ownOutbound[event.eventId] = record
        forwarded.remove(event.eventId)
        peerInventorySentMillis.clear()
        if (record.kind == 0L) {
            NearbyProfileEvent.fromEventJson(event.eventJson)?.let { profile ->
                ownProfileEventId = record.id
                knownProfiles[record.id] = profile
            }
        }
        pruneMailbags()
        Log.d(TAG, "published event kind=${record.kind} id=${record.id} visible=${nearbyActive()}")
        if (nearbyActive()) {
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
        val enabled =
            guardBluetooth("read Bluetooth state", false, "Bluetooth off") {
                adapter?.isEnabled == true
            }
        if (!enabled) {
            status = "Bluetooth off"
            return
        }
        status = "Starting"
        startGattServer()
        startAdvertising()
        startScanning()
        startMaintenance()
    }

    @SuppressLint("MissingPermission")
    private fun stop() {
        status = "Off"
        if (!localNetworkVisible) {
            stopMaintenance()
        }
        guardBluetooth("stop scan", Unit, statusOnFailure = null) {
            adapter?.bluetoothLeScanner?.stopScan(scanCallback)
        }
        guardBluetooth("stop advertising", Unit, statusOnFailure = null) {
            adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback)
        }
        gatts.values.forEach { gatt ->
            guardBluetooth("close GATT", Unit, statusOnFailure = null) {
                gatt.close()
            }
        }
        gatts.clear()
        writableCharacteristics.clear()
        centralAssemblers.clear()
        serverAssemblers.clear()
        val bluetoothPeerIds = recentBluetoothPeerIds()
        peerIdsByAddress.clear()
        bluetoothPeerLastSeenMillis.clear()
        peerInventorySentMillis.clear()
        centralReconnectSuppressedUntilMillis.clear()
        connectionNonces.clear()
        if (localNetworkVisible) {
            val lanPeerIds = lanService.peerIds()
            bluetoothPeerIds
                .filterNot { lanPeerIds.contains(it) }
                .forEach { peerId ->
                    peers.remove(peerId)
                    peerNonces.remove(peerId)
                }
        } else {
            peerNonces.clear()
            peers.clear()
        }
        mtuPayloadBytes.clear()
        subscribedServerAddresses.clear()
        pendingGattWrites.values.forEach { it.complete(BluetoothGatt.GATT_FAILURE) }
        pendingGattWrites.clear()
        pendingNotifications.values.forEach { it.complete(BluetoothGatt.GATT_FAILURE) }
        pendingNotifications.clear()
        ignoredAddresses.clear()
        if (!localNetworkVisible) {
            incomingFragments.clear()
        }
        guardBluetooth("close GATT server", Unit, statusOnFailure = null) {
            gattServer?.close()
        }
        gattServer = null
    }

    @SuppressLint("MissingPermission")
    private fun startGattServer() {
        val manager = bluetoothManager ?: return
        val server =
            guardBluetooth("open GATT server", null as BluetoothGattServer?, "Bluetooth unavailable") {
                manager.openGattServer(appContext, gattServerCallback)
            } ?: return
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
        val added =
            guardBluetooth("add GATT service", false, "Bluetooth unavailable") {
                server.addService(service)
            }
        if (added) {
            gattServer = server
        } else {
            runCatching { server.close() }
        }
    }

    @SuppressLint("MissingPermission")
    private fun startAdvertising() {
        val advertiser =
            guardBluetooth("get advertiser", null as android.bluetooth.le.BluetoothLeAdvertiser?, "Advertise unavailable") {
                adapter?.bluetoothLeAdvertiser
            }
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
        guardBluetooth("start advertising", Unit, "Advertise failed") {
            advertiser.startAdvertising(settings, data, advertiseCallback)
        }
    }

    @SuppressLint("MissingPermission")
    private fun startScanning() {
        val scanner =
            guardBluetooth("get scanner", null as android.bluetooth.le.BluetoothLeScanner?, "Scan failed") {
                adapter?.bluetoothLeScanner
            } ?: return
        val settings =
            ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build()
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE_UUID)).build()
        status = "Scanning"
        guardBluetooth("start scan", Unit, "Scan failed") {
            scanner.startScan(listOf(filter), settings, scanCallback)
        }
    }

    @SuppressLint("MissingPermission")
    private fun connect(device: BluetoothDevice) {
        val address =
            guardBluetooth("read device address", null as String?, "Connect failed") {
                device.address
            } ?: return
        val nowMillis = SystemClock.elapsedRealtime()
        val ignoredUntil = ignoredAddresses[address]
        if (ignoredUntil != null && ignoredUntil > nowMillis) {
            return
        }
        val suppressedUntil = centralReconnectSuppressedUntilMillis[address]
        if (suppressedUntil != null && suppressedUntil > nowMillis) {
            return
        }
        if (suppressedUntil != null) {
            centralReconnectSuppressedUntilMillis.remove(address)
        }
        if (gatts.containsKey(address)) {
            return
        }
        if (subscribedServerAddresses.contains(address)) {
            return
        }
        if (gatts.size >= MAX_SIMULTANEOUS_GATTS) {
            return
        }
        status = "Connecting"
        val gatt =
            guardBluetooth("connect GATT", null as BluetoothGatt?, "Connect failed") {
                device.connectGatt(appContext, false, gattCallback, BluetoothDevice.TRANSPORT_LE)
            } ?: return
        gatts[address] = gatt
    }

    @SuppressLint("MissingPermission")
    private fun closeNonIrisGatt(gatt: BluetoothGatt, reason: String) {
        val address =
            guardBluetooth("read non-Iris GATT address", null as String?, statusOnFailure = null) {
                gatt.device.address
            } ?: return
        Log.d(TAG, "$reason for $address")
        ignoreAddress(address)
        gatts.remove(address)
        writableCharacteristics.remove(address)
        centralAssemblers.remove(address)
        mtuPayloadBytes.remove(address)
        peerIdsByAddress.remove(address)
        guardBluetooth("close non-Iris GATT", Unit, statusOnFailure = null) {
            gatt.close()
        }
        status = if (peers.isEmpty()) "Scanning" else "${peers.size} nearby"
    }

    private fun ignoreAddress(address: String) {
        ignoredAddresses[address] = SystemClock.elapsedRealtime() + NON_IRIS_BACKOFF_MS
        while (ignoredAddresses.size > MAX_IGNORED_ADDRESSES) {
            ignoredAddresses.remove(ignoredAddresses.keys.first())
        }
    }

    private fun announceToConnectedPeers() {
        sendHello(excludingPeerId = null)
        sendInventory(excludingPeerId = null)
    }

    private fun announceIdentityToConnectedPeers() {
        sendHello(excludingPeerId = null)
        peerNonces.values.forEach(::sendPresence)
    }

    private fun startMaintenance() {
        maintenanceJob?.cancel()
        maintenanceJob =
            applicationScope.launch {
                var lastHelloMillis = 0L
                while (nearbyActive()) {
                    val nowMillis = System.currentTimeMillis()
                    pruneStalePeers(nowMillis)
                    if (nowMillis - lastHelloMillis >= HELLO_INTERVAL_MS) {
                        sendHello(excludingPeerId = null)
                        lastHelloMillis = nowMillis
                    }
                    delay(PEER_SWEEP_INTERVAL_MS)
                }
            }
    }

    private fun stopMaintenance() {
        maintenanceJob?.cancel()
        maintenanceJob = null
    }

    private fun sendHello(excludingPeerId: String?) {
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "hello")
                .put("nonce", localNonce)
                .put("name", "Iris")
        sendEnvelope(envelope, excludingPeerId)
    }

    private fun sendInventory(excludingPeerId: String?) {
        val records = mailbagEvents().take(200)
        if (records.isEmpty()) {
            return
        }
        records.forEach { record ->
            val envelope =
                JSONObject()
                    .put("v", 1)
                    .put("type", "inv")
                    .put("id", record.id)
                    .put("kind", record.kind)
                    .put("created_at", record.createdAtSecs)
                    .put("size", record.eventJson.toByteArray(Charsets.UTF_8).size)
            record.authorPubkeyHex?.let { envelope.put("author", it) }
            sendEnvelope(envelope, excludingPeerId)
        }
    }

    private fun sendInventoryAfterHelloIfNeeded(
        remotePeerId: String,
        force: Boolean,
    ) {
        val nowMillis = System.currentTimeMillis()
        val lastSentMillis = peerInventorySentMillis[remotePeerId]
        if (!force && lastSentMillis != null && nowMillis - lastSentMillis < INVENTORY_RESEND_INTERVAL_MS) {
            return
        }
        peerInventorySentMillis[remotePeerId] = nowMillis
        sendInventory(excludingPeerId = null)
    }

    private fun sendWant(ids: List<String>, excludingPeerId: String?) {
        if (ids.isEmpty()) {
            return
        }
        ids.take(64).forEach { id ->
            val envelope =
                JSONObject()
                    .put("v", 1)
                    .put("type", "want")
                    .put("id", id)
            sendEnvelope(envelope, excludingPeerId)
        }
    }

    private fun sendEvent(record: StoredNearbyEvent, excludingPeerId: String?) {
        sendEventJson(record.eventJson, excludingPeerId)
    }

    private fun sendEventJson(eventJson: String, excludingPeerId: String?) {
        val envelope =
            JSONObject()
                .put("v", 1)
                .put("type", "event")
                .put("event_json", eventJson)
        val frame = appManager.encodeNearbyFrame(envelope)
        if (frame != null && frame.size <= SINGLE_FRAME_BYTES) {
            sendFrame("event", frame, excludingPeerId)
        } else {
            StoredNearbyEvent.fromEventJson(eventJson)?.let { sendEventFragments(it, excludingPeerId) }
        }
    }

    private fun sendPresence(remoteNonce: String) {
        val eventJson =
            appManager.buildNearbyPresenceEventJson(
                peerId = peerId,
                myNonce = localNonce,
                theirNonce = remoteNonce,
                profileEventId = ownProfileEventId,
            )
        if (eventJson.isNotBlank()) {
            sendEventJson(eventJson, excludingPeerId = null)
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
                    .put("frag_id", fragmentId)
                    .put("event_id", record.id)
                    .put("index", index)
                    .put("total", total)
                    .put("data", Base64.encodeToString(chunk, Base64.NO_WRAP))
            appManager.encodeNearbyFrame(envelope)?.let { frames += it }
        }
        if (frames.size != total) {
            return
        }
        Log.d(TAG, "send event_frag count=${frames.size} event=${record.id} peers=${peers.size}")
        frames.forEach { frame ->
            sendFrame("event_frag", frame, excludingPeerId)
        }
    }

    private fun sendEnvelope(envelope: JSONObject, excludingPeerId: String?) {
        if (!nearbyActive()) {
            return
        }
        val type = envelope.optString("type", "unknown")
        val frame = appManager.encodeNearbyFrame(envelope) ?: return
        sendFrame(type, frame, excludingPeerId)
    }

    private fun sendFrame(type: String, frame: ByteArray, excludingPeerId: String?) {
        if (!nearbyActive()) {
            return
        }
        Log.d(TAG, "send $type bytes=${frame.size} peers=${peers.size}")
        if (localNetworkVisible) {
            lanService.send(frame, excludingPeerId)
        }
        if (!visible) {
            return
        }
        launchBluetooth("send frame") {
            sendMutex.withLock {
                sendBluetoothFrame(frame, excludingPeerId)
            }
        }
    }

    @SuppressLint("MissingPermission")
    private suspend fun sendBluetoothFrame(frame: ByteArray, excludingPeerId: String?) {
        gattServer?.let { server ->
            val characteristic =
                server.getService(SERVICE_UUID)?.getCharacteristic(CHARACTERISTIC_UUID) ?: return@let
            val connectedDevices = guardBluetooth("connected GATT devices", emptyList(), statusOnFailure = null) {
                bluetoothManager
                    ?.getConnectedDevices(BluetoothProfile.GATT)
                    ?.toList()
                    ?: emptyList()
            }
            connectedDevices.forEach { device ->
                val address =
                    guardBluetooth("read connected device address", null as String?, statusOnFailure = null) {
                        device.address
                    } ?: return@forEach
                if (!subscribedServerAddresses.contains(address)) {
                    return@forEach
                }
                if (!shouldSendViaServerBluetoothRoute(address, excludingPeerId)) {
                    return@forEach
                }
                notifyDevice(server, device, characteristic, frame)
            }
        }

        writableCharacteristics.toList().forEach { (address, characteristic) ->
            if (!shouldSendViaOutgoingBluetoothRoute(address, excludingPeerId)) {
                return@forEach
            }
            val gatt = gatts[address] ?: return@forEach
            if (!writeToGatt(gatt, characteristic, frame)) {
                forgetCentralConnection(address, gatt, "write failed")
                return@forEach
            }
        }
    }

    @SuppressLint("MissingPermission")
    private suspend fun writeToGatt(
        gatt: BluetoothGatt,
        characteristic: BluetoothGattCharacteristic,
        data: ByteArray,
    ): Boolean {
        var offset = 0
        val address =
            guardBluetooth("read GATT device address", null as String?, statusOnFailure = null) {
                gatt.device.address
            } ?: return false
        val chunkSize = mtuPayloadBytes[address] ?: BLE_CHUNK_BYTES
        while (offset < data.size) {
            val chunk = data.copyOfRange(offset, minOf(offset + chunkSize, data.size))
            val completed = CompletableDeferred<Int>()
            pendingGattWrites.put(address, completed)?.complete(BluetoothGatt.GATT_FAILURE)
            val started = guardBluetooth("write GATT characteristic", false, statusOnFailure = null) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    gatt.writeCharacteristic(
                        characteristic,
                        chunk,
                        BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT,
                    ) == BluetoothStatusCodes.SUCCESS
                } else {
                    @Suppress("DEPRECATION")
                    characteristic.value = chunk
                    characteristic.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
                    @Suppress("DEPRECATION")
                    gatt.writeCharacteristic(characteristic)
                }
            }
            if (!started) {
                pendingGattWrites.remove(address, completed)
                Log.w(TAG, "write GATT characteristic failed to start for $address")
                return false
            }
            val status = withTimeoutOrNull(BLE_WRITE_TIMEOUT_MS) { completed.await() }
            pendingGattWrites.remove(address, completed)
            if (status != BluetoothGatt.GATT_SUCCESS) {
                Log.w(TAG, "write GATT characteristic failed for $address status=$status")
                return false
            }
            offset += chunk.size
        }
        return true
    }

    private fun blePayloadBytesForMtu(mtu: Int): Int =
        (mtu - 3).coerceIn(20, BLE_CHUNK_BYTES)

    @SuppressLint("MissingPermission")
    private suspend fun notifyDevice(
        server: BluetoothGattServer,
        device: BluetoothDevice,
        characteristic: BluetoothGattCharacteristic,
        data: ByteArray,
    ) {
        var offset = 0
        val address =
            guardBluetooth("read notify device address", null as String?, statusOnFailure = null) {
                device.address
            } ?: return
        val chunkSize = mtuPayloadBytes[address] ?: BLE_CHUNK_BYTES
        while (offset < data.size) {
            val chunk = data.copyOfRange(offset, minOf(offset + chunkSize, data.size))
            val completed = CompletableDeferred<Int>()
            pendingNotifications.put(address, completed)?.complete(BluetoothGatt.GATT_FAILURE)
            val started = guardBluetooth("notify GATT device", false, statusOnFailure = null) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    server.notifyCharacteristicChanged(device, characteristic, false, chunk) == BluetoothStatusCodes.SUCCESS
                } else {
                    @Suppress("DEPRECATION")
                    characteristic.value = chunk
                    @Suppress("DEPRECATION")
                    server.notifyCharacteristicChanged(device, characteristic, false)
                }
            }
            if (!started) {
                pendingNotifications.remove(address, completed)
                Log.w(TAG, "notify GATT device failed to start for $address")
                forgetServerConnection(address, "notify failed to start")
                return
            }
            val status = withTimeoutOrNull(BLE_NOTIFY_TIMEOUT_MS) { completed.await() }
            pendingNotifications.remove(address, completed)
            if (status != BluetoothGatt.GATT_SUCCESS) {
                Log.w(TAG, "notify GATT device failed for $address status=$status")
                forgetServerConnection(address, "notify failed")
                return
            }
            offset += chunk.size
        }
    }

    private fun forgetServerConnection(
        address: String,
        reason: String,
    ) {
        Log.d(TAG, "forget server GATT $address: $reason")
        subscribedServerAddresses.remove(address)
        pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
        serverAssemblers.remove(address)
        mtuPayloadBytes.remove(address)
        removePeerAddressMappingIfUnused(address)
        status = if (peers.isEmpty()) "Visible" else "${peers.size} nearby"
    }

    @SuppressLint("MissingPermission")
    private fun forgetCentralConnection(
        address: String,
        gatt: BluetoothGatt?,
        reason: String,
        suppressReconnect: Boolean = false,
    ) {
        Log.d(TAG, "forget central GATT $address: $reason")
        pendingGattWrites.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
        pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
        gatts.remove(address)
        writableCharacteristics.remove(address)
        centralAssemblers.remove(address)
        mtuPayloadBytes.remove(address)
        if (suppressReconnect) {
            suppressCentralReconnect(address)
        }
        removePeerAddressMappingIfUnused(address)
        guardBluetooth("close stale GATT", Unit, statusOnFailure = null) {
            gatt?.close()
        }
    }

    private fun ingestFrame(frame: ByteArray, source: NearbySource) {
        val envelope = appManager.decodeNearbyFrame(frame) ?: return
        val type = envelope.optString("type")
        if (envelope.has("peer_id")) {
            rejectLegacyNearbySource(source, type)
            return
        }
        val remotePeerId = peerIdForSource(source)
        val sourceKey = sourceKey(source)
        if (remotePeerId.isNotEmpty()) {
            touchPeer(remotePeerId)
            markTransportPeer(remotePeerId, source)
        }
        when (type) {
            "hello" -> {
                val remoteNonce = envelope.optString("nonce").sanitizedNonce()
                if (remoteNonce != null && sourceKey != null) {
                    connectionNonces[sourceKey] = remoteNonce
                }
                if (remotePeerId.isNotEmpty()) {
                    val previousNonce = peerNonces[remotePeerId]
                    val wasNew = rememberPeer(
                        peerId = remotePeerId,
                        name = envelope.optString("name"),
                        profileEventId = null,
                    )
                    val nonceChanged = remoteNonce != null && remoteNonce != previousNonce
                    if (wasNew || nonceChanged) {
                        sendHello(excludingPeerId = null)
                    }
                    if (remoteNonce != null) {
                        peerNonces[remotePeerId] = remoteNonce
                        if (wasNew || nonceChanged) {
                            sendPresence(remoteNonce)
                        }
                    }
                    if (wasNew) {
                        Log.d(TAG, "peer nearby id=$remotePeerId")
                    }
                    sendInventoryAfterHelloIfNeeded(
                        remotePeerId = remotePeerId,
                        force = wasNew || nonceChanged,
                    )
                } else if (remoteNonce != null) {
                    sendPresence(remoteNonce)
                    sendInventory(excludingPeerId = null)
                }
                status = if (peers.size == 1) "1 nearby" else "${peers.size} nearby"
            }
            "inv" -> handleInventory(envelope)
            "want" -> handleWant(envelope)
            "event" -> handleEventEnvelope(envelope, remotePeerId.takeIf(String::isNotEmpty), sourceKey)
            "event_frag" -> handleEventFragment(envelope, remotePeerId.takeIf(String::isNotEmpty), sourceKey)
        }
    }

    private fun handleInventory(envelope: JSONObject) {
        val id = envelope.optString("id")
        val size = envelope.optInt("size", 0)
        if (
            id.length == 64 &&
            size in 1..MAX_EVENT_BYTES &&
            !ownOutbound.containsKey(id) &&
            !forwarded.containsKey(id)
        ) {
            sendWant(listOf(id), excludingPeerId = null)
        }
    }

    private fun handleWant(envelope: JSONObject) {
        val id = envelope.optString("id")
        val record = ownOutbound[id] ?: forwarded[id] ?: return
        sendEvent(record, excludingPeerId = null)
    }

    private fun handleEventEnvelope(
        envelope: JSONObject,
        remotePeerId: String?,
        sourceKey: String?,
    ) {
        val eventJson = envelope.optString("event_json")
        handleEventJson(eventJson, remotePeerId, sourceKey)
    }

    private fun handleEventFragment(
        envelope: JSONObject,
        remotePeerId: String?,
        sourceKey: String?,
    ) {
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
                    sourceKey = sourceKey,
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
        handleEventJson(eventJson, remotePeerId ?: fragment.remotePeerId, sourceKey ?: fragment.sourceKey)
    }

    private fun handleEventJson(
        eventJson: String,
        remotePeerId: String?,
        sourceKey: String?,
    ) {
        if (eventJson.toByteArray(Charsets.UTF_8).size > MAX_EVENT_BYTES) {
            return
        }
        val record = StoredNearbyEvent.fromEventJson(eventJson) ?: return
        if (record.kind == NEARBY_PRESENCE_KIND.toLong()) {
            if (handlePresenceEvent(eventJson, remotePeerId, sourceKey)) {
                Log.d(TAG, "accepted nearby presence")
            }
            return
        }
        val existing = ownOutbound[record.id] ?: forwarded[record.id]
        if (existing != null) {
            rememberProfile(existing.eventJson, remotePeerId)
            return
        }
        val transport = transportLabel(remotePeerId)
        if (!appManager.ingestNearbyEventJsonWithTransport(eventJson, transport)) {
            return
        }
        rememberProfile(eventJson, remotePeerId)
        forwarded[record.id] = record
        pruneMailbags()
        sendEventJson(eventJson, excludingPeerId = remotePeerId)
        Log.d(TAG, "accepted event kind=${record.kind} id=${record.id}")
    }

    private fun handlePresenceEvent(
        eventJson: String,
        remotePeerId: String?,
        sourceKey: String?,
    ): Boolean {
        val peer =
            remotePeerId?.takeIf(String::isNotBlank)
                ?: nearbyPresencePeerId(eventJson)
                ?: return false
        val nonceCandidates =
            presenceNonceCandidates(remotePeerId, sourceKey)
        nonceCandidates.forEach { (nonceKey, remoteNonce) ->
            val result =
                appManager.verifyNearbyPresenceEventJson(
                    eventJson = eventJson,
                    peerId = peer,
                    myNonce = localNonce,
                    theirNonce = remoteNonce,
                )
            val json = runCatching { JSONObject(result) }.getOrNull() ?: return@forEach
            val ownerPubkeyHex =
                json.optString("owner_pubkey_hex").takeIf { it.length == 64 } ?: return@forEach
            val profileEventId = json.optString("profile_event_id").sanitizedEventId()
            sourceKey?.let { markTransportPeer(peer, it) }
            sourceKey?.let { connectionNonces.remove(it) }
            nonceKey?.let { connectionNonces.remove(it) }
            rememberPresence(peer, ownerPubkeyHex, profileEventId)
            return true
        }
        return false
    }

    private fun presenceNonceCandidates(
        remotePeerId: String?,
        sourceKey: String?,
    ): List<Pair<String?, String>> {
        remotePeerId?.let { peerNonces[it] }?.let { return listOf(null to it) }
        val candidates = linkedMapOf<String?, String>()
        sourceKey?.let { key ->
            connectionNonces[key]?.let { candidates[key] = it }
        }
        connectionNonces.forEach { (key, nonce) ->
            candidates.putIfAbsent(key, nonce)
        }
        return candidates.entries.map { it.key to it.value }
    }

    private fun pruneIncomingFragments() {
        val cutoff = System.currentTimeMillis() - FRAGMENT_TTL_MS
        incomingFragments.entries.removeAll { it.value.storedAtMillis < cutoff }
        while (incomingFragments.size > MAX_INCOMING_FRAGMENT_SETS) {
            val oldest = incomingFragments.entries.minByOrNull { it.value.storedAtMillis }?.key ?: return
            incomingFragments.remove(oldest)
        }
    }

    @SuppressLint("MissingPermission")
    private fun pruneStalePeers(nowMillis: Long) {
        bluetoothPeerLastSeenMillis.entries.removeAll { nowMillis - it.value > PEER_TTL_MS }
        val stalePeerIds =
            peers.values
                .filter { nowMillis - it.lastSeenMillis > PEER_TTL_MS }
                .map { it.id }
                .toSet()
        if (stalePeerIds.isEmpty()) {
            return
        }

        stalePeerIds.forEach { peerId ->
            peers.remove(peerId)
            bluetoothPeerLastSeenMillis.remove(peerId)
            peerInventorySentMillis.remove(peerId)
            peerNonces.remove(peerId)
        }

        val staleAddresses =
            peerIdsByAddress
                .filterValues { it in stalePeerIds }
                .keys
                .toList()
        staleAddresses.forEach { address ->
            peerIdsByAddress.remove(address)
            pendingGattWrites.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
            pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
            writableCharacteristics.remove(address)
            centralAssemblers.remove(address)
            serverAssemblers.remove(address)
            mtuPayloadBytes.remove(address)
            subscribedServerAddresses.remove(address)
            gatts.remove(address)?.let { gatt ->
                guardBluetooth("close stale peer GATT", Unit, statusOnFailure = null) {
                    gatt.close()
                }
            }
        }

        pruneKnownProfiles()
        status = nearbyStatusWhenVisible()
        Log.d(TAG, "expired stale peers count=${stalePeerIds.size}")
    }

    private fun removeLanOnlyPeers(lanPeerIds: Set<String>) {
        if (lanPeerIds.isEmpty()) {
            return
        }
        val bluetoothPeerIds = recentBluetoothPeerIds()
        lanPeerIds
            .filterNot { bluetoothPeerIds.contains(it) }
            .forEach { peerId ->
                peers.remove(peerId)
                peerNonces.remove(peerId)
            }
        pruneKnownProfiles()
        status = nearbyStatusWhenVisible()
    }

    private fun markTransportPeer(
        peerId: String,
        source: NearbySource,
    ) {
        when (source) {
            is NearbySource.BluetoothAddress -> {
                bluetoothPeerLastSeenMillis[peerId] = System.currentTimeMillis()
                source.address?.let { peerIdsByAddress[it] = peerId }
            }
            is NearbySource.Lan -> lanService.markPeer(source.connectionId, peerId)
        }
        pruneDuplicateBluetoothRoutes(peerId)
    }

    private fun markTransportPeer(
        peerId: String,
        sourceKey: String,
    ) {
        when {
            sourceKey.startsWith(BLUETOOTH_SOURCE_PREFIX) -> {
                val address = sourceKey.removePrefix(BLUETOOTH_SOURCE_PREFIX)
                bluetoothPeerLastSeenMillis[peerId] = System.currentTimeMillis()
                peerIdsByAddress[address] = peerId
            }
            sourceKey.startsWith(LAN_SOURCE_PREFIX) -> {
                lanService.markPeer(sourceKey.removePrefix(LAN_SOURCE_PREFIX), peerId)
            }
        }
        pruneDuplicateBluetoothRoutes(peerId)
    }

    private fun peerIdForSource(source: NearbySource): String =
        when (source) {
            is NearbySource.BluetoothAddress -> source.address?.let(peerIdsByAddress::get).orEmpty()
            is NearbySource.Lan -> lanService.peerIdForConnection(source.connectionId).orEmpty()
        }

    private fun sourceKey(source: NearbySource): String? =
        when (source) {
            is NearbySource.BluetoothAddress -> source.address?.let(::bluetoothSourceKey)
            is NearbySource.Lan -> lanSourceKey(source.connectionId)
        }

    private fun rejectLegacyNearbySource(
        source: NearbySource,
        type: String,
    ) {
        when (source) {
            is NearbySource.BluetoothAddress -> {
                val address = source.address ?: return
                Log.d(TAG, "legacy nearby $type frame from $address")
                ignoreAddress(address)
                gatts[address]?.let { gatt ->
                    forgetCentralConnection(
                        address = address,
                        gatt = gatt,
                        reason = "legacy nearby frame",
                        suppressReconnect = true,
                    )
                }
                forgetServerConnection(address, "legacy nearby frame")
            }
            is NearbySource.Lan -> Unit
        }
    }

    private fun bluetoothSourceKey(address: String): String = "$BLUETOOTH_SOURCE_PREFIX$address"

    private fun lanSourceKey(connectionId: String): String = "$LAN_SOURCE_PREFIX$connectionId"

    private fun transportLabel(remotePeerId: String?): String {
        if (remotePeerId.isNullOrBlank()) return "nearby"
        return if (recentBluetoothPeerIds().contains(remotePeerId)) "bluetooth" else "wifi"
    }

    private fun recentBluetoothPeerIds(nowMillis: Long = System.currentTimeMillis()): Set<String> =
        bluetoothPeerLastSeenMillis
            .filterValues { nowMillis - it <= PEER_TTL_MS }
            .keys
            .toSet()

    private fun shouldSendViaOutgoingBluetoothRoute(
        address: String,
        excludingPeerId: String?,
    ): Boolean {
        val remotePeerId = peerIdsByAddress[address]
        if (remotePeerId == null) {
            return true
        }
        if (remotePeerId == excludingPeerId) {
            return false
        }
        if (lanService.hasPeer(remotePeerId)) {
            return false
        }
        if (hasOutgoingBluetoothRoute(remotePeerId) && hasServerBluetoothRoute(remotePeerId)) {
            return shouldUseOutgoingBluetoothRoute(remotePeerId)
        }
        return true
    }

    private fun shouldSendViaServerBluetoothRoute(
        address: String,
        excludingPeerId: String?,
    ): Boolean {
        val remotePeerId = peerIdsByAddress[address]
        if (remotePeerId == null) {
            return true
        }
        if (remotePeerId == excludingPeerId) {
            return false
        }
        if (lanService.hasPeer(remotePeerId)) {
            return false
        }
        if (hasOutgoingBluetoothRoute(remotePeerId) && hasServerBluetoothRoute(remotePeerId)) {
            return !shouldUseOutgoingBluetoothRoute(remotePeerId)
        }
        return true
    }

    private fun shouldUseOutgoingBluetoothRoute(remotePeerId: String): Boolean =
        peerId < remotePeerId

    private fun hasOutgoingBluetoothRoute(remotePeerId: String): Boolean =
        writableCharacteristics.keys.any { peerIdsByAddress[it] == remotePeerId }

    private fun hasServerBluetoothRoute(remotePeerId: String): Boolean =
        subscribedServerAddresses.any { peerIdsByAddress[it] == remotePeerId }

    private fun pruneDuplicateBluetoothRoutes(remotePeerId: String) {
        if (!hasOutgoingBluetoothRoute(remotePeerId) || !hasServerBluetoothRoute(remotePeerId)) {
            return
        }
        if (shouldUseOutgoingBluetoothRoute(remotePeerId)) {
            peerIdsByAddress
                .filterValues { it == remotePeerId }
                .keys
                .filter { subscribedServerAddresses.contains(it) }
                .toList()
                .forEach { closeDuplicateServerRoute(it, remotePeerId) }
        } else {
            peerIdsByAddress
                .filterValues { it == remotePeerId }
                .keys
                .filter { writableCharacteristics.containsKey(it) || gatts.containsKey(it) }
                .toList()
                .forEach { address ->
                    forgetCentralConnection(
                        address = address,
                        gatt = gatts[address],
                        reason = "duplicate peer route",
                        suppressReconnect = true,
                    )
                }
        }
    }

    @SuppressLint("MissingPermission")
    private fun closeDuplicateServerRoute(
        address: String,
        remotePeerId: String,
    ) {
        Log.d(TAG, "close duplicate server GATT $address peer=$remotePeerId")
        subscribedServerAddresses.remove(address)
        pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
        serverAssemblers.remove(address)
        guardBluetooth("cancel duplicate server GATT", Unit, statusOnFailure = null) {
            adapter?.getRemoteDevice(address)?.let { gattServer?.cancelConnection(it) }
        }
        removePeerAddressMappingIfUnused(address)
    }

    private fun removePeerAddressMappingIfUnused(address: String) {
        if (!writableCharacteristics.containsKey(address) &&
            !gatts.containsKey(address) &&
            !subscribedServerAddresses.contains(address)
        ) {
            peerIdsByAddress.remove(address)
            connectionNonces.remove(bluetoothSourceKey(address))
        }
    }

    private fun suppressCentralReconnect(address: String) {
        centralReconnectSuppressedUntilMillis[address] =
            SystemClock.elapsedRealtime() + DEDUP_RECONNECT_BACKOFF_MS
        while (centralReconnectSuppressedUntilMillis.size > MAX_SUPPRESSED_RECONNECT_ADDRESSES) {
            centralReconnectSuppressedUntilMillis.remove(centralReconnectSuppressedUntilMillis.keys.first())
        }
    }

    private fun touchPeer(peerId: String) {
        val existing = peers[peerId] ?: return
        peers[peerId] = existing.copy(lastSeenMillis = System.currentTimeMillis())
    }

    private fun nearbyStatusWhenVisible(): String =
        when {
            peers.size == 1 -> "1 nearby"
            peers.size > 1 -> "${peers.size} nearby"
            localNetworkVisible && !visible -> "Visible"
            !hasBluetoothPermissions() -> "No Bluetooth access"
            isBluetoothOn() -> "Visible"
            else -> "Bluetooth off"
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
                name =
                    nearbyPeerName(
                        advertisedName = name,
                        ownerPubkeyHex = existing?.ownerPubkeyHex,
                        profileDisplayName = null,
                        existingName = existing?.name,
                    ),
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
            if (!peers.containsKey(remotePeerId)) {
                peers[remotePeerId] =
                    Peer(
                        id = remotePeerId,
                        name =
                            nearbyPeerName(
                                advertisedName = null,
                                ownerPubkeyHex = profile.ownerPubkeyHex,
                                profileDisplayName = profile.displayName,
                                existingName = null,
                            ),
                        ownerPubkeyHex = profile.ownerPubkeyHex,
                        pictureUrl = profile.pictureUrl,
                        profileEventId = profile.id,
                        lastSeenMillis = System.currentTimeMillis(),
                    )
                status = if (peers.size == 1) "1 nearby" else "${peers.size} nearby"
            }
            applyAdvertisedProfile(remotePeerId, profile)
        }
    }

    private fun rememberPresence(
        peerId: String,
        ownerPubkeyHex: String,
        profileEventId: String?,
    ) {
        val existing =
            peers[peerId]
                ?: Peer(
                    id = peerId,
                    name =
                        nearbyPeerName(
                            advertisedName = null,
                            ownerPubkeyHex = ownerPubkeyHex,
                            profileDisplayName = null,
                            existingName = null,
                        ),
                    ownerPubkeyHex = null,
                    pictureUrl = null,
                    profileEventId = null,
                    lastSeenMillis = System.currentTimeMillis(),
                )
        val nextProfileEventId = profileEventId ?: existing.profileEventId
        peers[peerId] =
            existing.copy(
                name =
                    nearbyPeerName(
                        advertisedName = null,
                        ownerPubkeyHex = ownerPubkeyHex,
                        profileDisplayName = null,
                        existingName = existing.name,
                    ),
                ownerPubkeyHex = ownerPubkeyHex,
                profileEventId = nextProfileEventId,
                lastSeenMillis = System.currentTimeMillis(),
            )
        val profile = nextProfileEventId?.let { knownProfiles[it] }
        if (profile != null) {
            applyAdvertisedProfile(peerId, profile)
        } else if (nextProfileEventId != null &&
            !ownOutbound.containsKey(nextProfileEventId) &&
            !forwarded.containsKey(nextProfileEventId)
        ) {
            sendWant(listOf(nextProfileEventId), excludingPeerId = null)
        }
    }

    private fun applyAdvertisedProfile(peerId: String, profile: NearbyProfileEvent) {
        val peer = peers[peerId] ?: return
        if (peer.ownerPubkeyHex != null && !peer.ownerPubkeyHex.equals(profile.ownerPubkeyHex, ignoreCase = true)) {
            return
        }
        if (peer.profileEventId != null && peer.profileEventId != profile.id) {
            return
        }
        peers[peerId] =
            peer.copy(
                name =
                    nearbyPeerName(
                        advertisedName = null,
                        ownerPubkeyHex = profile.ownerPubkeyHex,
                        profileDisplayName = profile.displayName,
                        existingName = peer.name,
                    ),
                ownerPubkeyHex = profile.ownerPubkeyHex,
                pictureUrl = profile.pictureUrl ?: peer.pictureUrl,
                profileEventId = profile.id,
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

    private fun nearbyPeerName(
        advertisedName: String?,
        ownerPubkeyHex: String?,
        profileDisplayName: String?,
        existingName: String?,
    ): String {
        profileDisplayName.sanitizedPeerLabel()?.let { return it }
        ownerPubkeyHex.sanitizedPeerLabel()?.let { return fallbackProfileNameForIdentity(it) }
        return advertisedName.sanitizedPeerLabel()
            ?: existingName.sanitizedPeerLabel()
            ?: "Iris"
    }

    private fun String?.sanitizedPeerLabel(): String? =
        this?.trim()?.takeIf { it.isNotEmpty() && it != "Iris" }

    private fun fallbackProfileNameForIdentity(identity: String): String {
        val adjectives =
            listOf(
                "Amber",
                "Bright",
                "Calm",
                "Clear",
                "Golden",
                "Lunar",
                "Nova",
                "Quiet",
                "Silver",
                "Solar",
                "Velvet",
                "Wild",
            )
        val nouns =
            listOf(
                "Aurora",
                "Comet",
                "Echo",
                "Falcon",
                "Harbor",
                "Listener",
                "Otter",
                "Raven",
                "Signal",
                "Sparrow",
                "Tide",
                "Voyager",
            )
        val trimmed = identity.trim()
        if (trimmed.isEmpty()) {
            return "Quiet Listener"
        }
        val hash =
            trimmed
                .encodeToByteArray()
                .fold(0L) { partial, byte ->
                    (partial * 31L + (byte.toInt() and 0xff).toLong()) and 0xffff_ffffL
                }
        val adjective = adjectives[(hash % adjectives.size).toInt()]
        val noun = nouns[((hash / adjectives.size) % nouns.size).toInt()]
        return "$adjective $noun"
    }

    private fun String?.sanitizedEventId(): String? =
        this?.trim()?.takeIf { it.length == 64 }

    private fun String?.sanitizedNonce(): String? =
        this?.trim()?.takeIf { it.length in 16..128 }

    private fun eventAuthorHex(eventJson: String): String? =
        runCatching { JSONObject(eventJson) }
            .getOrNull()
            ?.optString("pubkey")
            ?.trim()
            ?.takeIf { it.length == 64 }

    private fun nearbyPresencePeerId(eventJson: String): String? {
        val event = runCatching { JSONObject(eventJson) }.getOrNull() ?: return null
        if (event.optLong("kind", -1L) != NEARBY_PRESENCE_KIND) {
            return null
        }
        val content = runCatching { JSONObject(event.optString("content")) }.getOrNull() ?: return null
        return content.optString("peer_id").trim().takeIf(String::isNotEmpty)
    }

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
                guardBluetooth("scan result", Unit, statusOnFailure = null) {
                    if (hasBluetoothPermissions()) {
                        val advertisedServices = result.scanRecord?.serviceUuids.orEmpty()
                        if (advertisedServices.isNotEmpty() && advertisedServices.none { it.uuid == SERVICE_UUID }) {
                            return@guardBluetooth
                        }
                        connect(result.device)
                    }
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
                guardBluetooth("GATT connection state", Unit, statusOnFailure = null) {
                    if (newState == BluetoothProfile.STATE_CONNECTED) {
                        val address =
                            guardBluetooth("read connected GATT address", "unknown", statusOnFailure = null) {
                                gatt.device.address
                            }
                        launchBluetooth("request GATT MTU") {
                            delay(200)
                            val requested =
                                guardBluetooth("request MTU", false, statusOnFailure = null) {
                                    gatt.requestMtu(517)
                                }
                            if (!requested) {
                                Log.w(TAG, "MTU request failed to start for $address")
                                guardBluetooth("discover services", Unit, statusOnFailure = null) {
                                    gatt.discoverServices()
                                }
                            }
                        }
                    } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                        val address =
                            guardBluetooth("read disconnected GATT address", null as String?, statusOnFailure = null) {
                                gatt.device.address
                            } ?: return
                        val remotePeerId = peerIdsByAddress[address]
                        pendingGattWrites.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
                        pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
                        if (status != BluetoothGatt.GATT_SUCCESS && !peerIdsByAddress.containsKey(address)) {
                            ignoreAddress(address)
                        }
                        gatts.remove(address)
                        writableCharacteristics.remove(address)
                        centralAssemblers.remove(address)
                        mtuPayloadBytes.remove(address)
                        if (remotePeerId != null && !shouldUseOutgoingBluetoothRoute(remotePeerId)) {
                            suppressCentralReconnect(address)
                        }
                        removePeerAddressMappingIfUnused(address)
                        guardBluetooth("close GATT", Unit, statusOnFailure = null) {
                            gatt.close()
                        }
                    }
                }
            }

            override fun onCharacteristicWrite(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                status: Int,
            ) {
                guardBluetooth("GATT characteristic write", Unit, statusOnFailure = null) {
                    if (characteristic.uuid != CHARACTERISTIC_UUID) {
                        return@guardBluetooth
                    }
                    val address =
                        guardBluetooth("read write GATT address", "unknown", statusOnFailure = null) {
                            gatt.device.address
                        }
                    pendingGattWrites.remove(address)?.complete(status)
                }
            }

            @SuppressLint("MissingPermission")
            override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
                guardBluetooth("GATT MTU changed", Unit, statusOnFailure = null) {
                    val address = gatt.device.address
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        mtuPayloadBytes[address] = blePayloadBytesForMtu(mtu)
                        Log.d(TAG, "MTU $mtu for $address")
                    } else {
                        Log.w(TAG, "MTU request failed for $address status=$status")
                    }
                    guardBluetooth("discover services after MTU", Unit, statusOnFailure = null) {
                        gatt.discoverServices()
                    }
                }
            }

            @SuppressLint("MissingPermission")
            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                guardBluetooth("GATT services discovered", Unit, statusOnFailure = null) {
                    if (status != BluetoothGatt.GATT_SUCCESS) {
                        closeNonIrisGatt(gatt, "service discovery failed status=$status")
                        return@guardBluetooth
                    }
                    val characteristic =
                        gatt.getService(SERVICE_UUID)?.getCharacteristic(CHARACTERISTIC_UUID)
                            ?: run {
                                closeNonIrisGatt(gatt, "missing Iris service")
                                return@guardBluetooth
                            }
                    val address = gatt.device.address
                    writableCharacteristics[address] = characteristic
                    centralAssemblers[address] = newFrameAssembler()
                    guardBluetooth("enable characteristic notifications", Unit, statusOnFailure = null) {
                        gatt.setCharacteristicNotification(characteristic, true)
                    }
                    val descriptorWriteStarted = characteristic.getDescriptor(CLIENT_CONFIG_UUID)?.let { descriptor ->
                        val value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                        guardBluetooth("write notification descriptor", false, statusOnFailure = null) {
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                                gatt.writeDescriptor(descriptor, value) == BluetoothStatusCodes.SUCCESS
                            } else {
                                @Suppress("DEPRECATION")
                                descriptor.value = value
                                @Suppress("DEPRECATION")
                                gatt.writeDescriptor(descriptor)
                            }
                        }
                    } ?: false
                    if (!descriptorWriteStarted) {
                        Log.d(TAG, "notification descriptor unavailable; sending hello without subscribe")
                        announceIdentityToConnectedPeers()
                    }
                }
            }

            override fun onDescriptorWrite(
                gatt: BluetoothGatt,
                descriptor: BluetoothGattDescriptor,
                status: Int,
            ) {
                guardBluetooth("GATT descriptor write", Unit, statusOnFailure = null) {
                    if (descriptor.uuid != CLIENT_CONFIG_UUID) {
                        return@guardBluetooth
                    }
                    val address =
                        guardBluetooth("read descriptor GATT address", "unknown", statusOnFailure = null) {
                            gatt.device.address
                        }
                    Log.d(TAG, "notifications ready for $address status=$status")
                    announceIdentityToConnectedPeers()
                }
            }

            override fun onCharacteristicChanged(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                value: ByteArray,
            ) {
                guardBluetooth("GATT characteristic changed", Unit, statusOnFailure = null) {
                    ingestChunk(gatt.device.address, value, centralAssemblers)
                }
            }

            @Deprecated("Deprecated by Android")
            override fun onCharacteristicChanged(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
            ) {
                guardBluetooth("GATT characteristic changed", Unit, statusOnFailure = null) {
                    @Suppress("DEPRECATION")
                    ingestChunk(gatt.device.address, characteristic.value ?: return, centralAssemblers)
                }
            }
        }

    private val gattServerCallback =
        object : BluetoothGattServerCallback() {
            override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
                guardBluetooth("server MTU changed", Unit, statusOnFailure = null) {
                    val address = device.address
                    mtuPayloadBytes[address] = blePayloadBytesForMtu(mtu)
                    Log.d(TAG, "server MTU $mtu for $address")
                }
            }

            override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
                guardBluetooth("server connection state", Unit, statusOnFailure = null) {
                    if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                        val address = device.address
                        serverAssemblers.remove(address)
                        mtuPayloadBytes.remove(address)
                        subscribedServerAddresses.remove(address)
                        pendingGattWrites.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
                        pendingNotifications.remove(address)?.complete(BluetoothGatt.GATT_FAILURE)
                        removePeerAddressMappingIfUnused(address)
                    }
                }
            }

            override fun onNotificationSent(device: BluetoothDevice, status: Int) {
                guardBluetooth("server notification sent", Unit, statusOnFailure = null) {
                    pendingNotifications.remove(device.address)?.complete(status)
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
                guardBluetooth("server characteristic write", Unit, statusOnFailure = null) {
                    ingestChunk(device.address, value, serverAssemblers)
                    if (responseNeeded && hasBluetoothPermissions()) {
                        @SuppressLint("MissingPermission")
                        guardBluetooth("send write response", Unit, statusOnFailure = null) {
                            gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
                        }
                    }
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
                guardBluetooth("server descriptor write", Unit, statusOnFailure = null) {
                    if (descriptor.uuid == CLIENT_CONFIG_UUID) {
                        if (value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)) {
                            subscribedServerAddresses.add(device.address)
                            gatts[device.address]?.let { existing ->
                                forgetCentralConnection(device.address, existing, "peer subscribed as central")
                            }
                        } else {
                            subscribedServerAddresses.remove(device.address)
                        }
                    }
                    if (responseNeeded && hasBluetoothPermissions()) {
                        @SuppressLint("MissingPermission")
                        guardBluetooth("send descriptor response", Unit, statusOnFailure = null) {
                            gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
                        }
                    }
                    announceIdentityToConnectedPeers()
                }
            }
        }

    private fun ingestChunk(
        address: String,
        chunk: ByteArray,
        assemblers: MutableMap<String, FrameAssembler>,
    ) {
        val assembler = assemblers.getOrPut(address) { newFrameAssembler() }
        assembler.append(chunk).forEach { frame ->
            ingestFrame(frame, NearbySource.BluetoothAddress(address))
        }
    }

    private fun newFrameAssembler(): FrameAssembler =
        FrameAssembler { header -> appManager.nearbyFrameBodyLenFromHeader(header) }

    private data class StoredNearbyEvent(
        val id: String,
        val kind: Long,
        val createdAtSecs: Long,
        val eventJson: String,
        val authorPubkeyHex: String?,
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
                    authorPubkeyHex = json.optString("pubkey").trim().takeIf { it.length == 64 },
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
        val sourceKey: String?,
    )

    private class FrameAssembler(
        private val bodyLenFromHeader: (ByteArray) -> Int,
    ) {
        private val buffer = ByteArrayOutputStream()

        fun append(chunk: ByteArray): List<ByteArray> {
            buffer.write(chunk)
            val frames = mutableListOf<ByteArray>()
            while (buffer.size() >= NEARBY_FRAME_HEADER_BYTES) {
                val data = buffer.toByteArray()
                val length = bodyLenFromHeader(data.copyOfRange(0, NEARBY_FRAME_HEADER_BYTES))
                if (length <= 0) {
                    buffer.reset()
                    buffer.write(data.copyOfRange(1, data.size))
                    continue
                }
                val frameLength = NEARBY_FRAME_HEADER_BYTES + length
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

    private fun shouldRethrow(error: Throwable): Boolean =
        error is CancellationException || error is VirtualMachineError || error is ThreadDeath

    private companion object {
        const val TAG = "IrisNearby"
        val SERVICE_UUID: UUID = UUID.fromString("8A0DAE01-D8E5-4F27-9F20-A616F1FBA6D0")
        val CHARACTERISTIC_UUID: UUID = UUID.fromString("8A0DAE02-D8E5-4F27-9F20-A616F1FBA6D0")
        val CLIENT_CONFIG_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        const val NEARBY_FRAME_HEADER_BYTES = 13
        const val SINGLE_FRAME_BYTES = 16 * 1024
        const val FRAGMENT_PAYLOAD_BYTES = 4 * 1024
        const val MAX_EVENT_FRAGMENTS = 1024
        const val MAX_INCOMING_FRAGMENT_SETS = 64
        const val FRAGMENT_TTL_MS = 30_000L
        const val HELLO_INTERVAL_MS = 5_000L
        const val INVENTORY_RESEND_INTERVAL_MS = 60_000L
        const val PEER_SWEEP_INTERVAL_MS = 1_000L
        const val PEER_TTL_MS = 15_000L
        const val BLE_CHUNK_BYTES = 180
        const val BLE_WRITE_TIMEOUT_MS = 1_500L
        const val BLE_NOTIFY_TIMEOUT_MS = 1_500L
        const val NON_IRIS_BACKOFF_MS = 60_000L
        const val DEDUP_RECONNECT_BACKOFF_MS = 30_000L
        const val MAX_IGNORED_ADDRESSES = 100
        const val MAX_SUPPRESSED_RECONNECT_ADDRESSES = 100
        const val MAX_SIMULTANEOUS_GATTS = 4
        const val NEARBY_PRESENCE_KIND = 22242L
        const val MAX_EVENT_BYTES = 128 * 1024
        const val MAX_MAILBAG_EVENTS = 500
        const val BLUETOOTH_SOURCE_PREFIX = "bt:"
        const val LAN_SOURCE_PREFIX = "lan:"

        fun newNonce(): String = UUID.randomUUID().toString().lowercase()
    }
}
