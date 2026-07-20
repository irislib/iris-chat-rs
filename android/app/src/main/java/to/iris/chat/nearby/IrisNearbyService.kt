package to.iris.chat.nearby

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import android.os.Build
import androidx.core.content.ContextCompat
import to.iris.chat.rust.DesktopNearbySnapshot

/** UI and permission state for FIPS-owned Nearby transports. */
class IrisNearbyService(context: Context) {
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
        val mailbagSummary: String?,
    )

    data class Peer(
        val id: String,
        val name: String,
        val ownerPubkeyHex: String?,
        val pictureUrl: String?,
        val profileEventId: String?,
        val bluetoothRssi: Int?,
        val lastSeenMillis: Long,
    )

    private val appContext = context.applicationContext
    @Volatile private var fipsBluetoothVisible = false
    @Volatile private var fipsLanVisible = false
    @Volatile private var localNetworkPermissionGranted = readLocalNetworkPermission()
    @Volatile private var fipsPeers: List<Peer> = emptyList()
    @Volatile private var fipsBluetoothPeerIds: Set<String> = emptySet()
    @Volatile private var fipsLanPeerIds: Set<String> = emptySet()
    private var multicastLock: WifiManager.MulticastLock? = null

    val snapshot: Snapshot
        get() =
            Snapshot(
                visible = fipsBluetoothVisible,
                status = if (fipsBluetoothVisible) "Visible" else "Off",
                bluetoothOn = fipsBluetoothVisible,
                bluetoothPermissionGranted = hasBluetoothPermission(),
                localNetworkVisible = fipsLanVisible,
                localNetworkPermissionGranted = localNetworkPermissionGranted,
                localNetworkStatus = if (fipsLanVisible) "Visible" else "Off",
                peerCount = fipsPeers.size,
                peers = fipsPeers,
                bluetoothPeers = fipsPeers.filter { it.id in fipsBluetoothPeerIds },
                localNetworkPeers = fipsPeers.filter { it.id in fipsLanPeerIds },
                mailbagSummary = null,
            )

    fun refreshPermissionState() {
        localNetworkPermissionGranted = readLocalNetworkPermission()
    }

    fun hasBluetoothPermission(): Boolean =
        bluetoothPermissions().all {
            ContextCompat.checkSelfPermission(appContext, it) == PackageManager.PERMISSION_GRANTED
        }

    fun hasLocalNetworkPermission(): Boolean = localNetworkPermissionGranted

    fun setFipsBluetoothVisible(visible: Boolean) {
        fipsBluetoothVisible = visible
    }

    fun setLocalNetworkVisible(visible: Boolean) {
        fipsLanVisible = visible
        if (visible) {
            acquireMulticastLock()
        } else {
            releaseMulticastLock()
        }
    }

    fun applyFipsPeerSnapshot(
        snapshot: DesktopNearbySnapshot,
        bluetoothPeerIds: List<String>,
        lanPeerIds: List<String>,
    ) {
        fipsPeers =
            snapshot.peers.map { peer ->
                Peer(
                    id = peer.id,
                    name = peer.name,
                    ownerPubkeyHex = peer.ownerPubkeyHex,
                    pictureUrl = peer.pictureUrl,
                    profileEventId = peer.profileEventId,
                    bluetoothRssi = null,
                    lastSeenMillis = peer.lastSeenSecs.toLong() * 1_000L,
                )
            }
        fipsBluetoothPeerIds = bluetoothPeerIds.toSet()
        fipsLanPeerIds = lanPeerIds.toSet()
    }

    private fun readLocalNetworkPermission(): Boolean =
        localNetworkPermissions().all {
            ContextCompat.checkSelfPermission(appContext, it) == PackageManager.PERMISSION_GRANTED
        }

    /** Android filters multicast unless the app explicitly permits FIPS's mDNS scan window. */
    @Synchronized
    private fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) return
        val lock =
            appContext
                .getSystemService(WifiManager::class.java)
                ?.createMulticastLock("iris-fips-lan")
                ?: return
        lock.setReferenceCounted(false)
        if (runCatching { lock.acquire() }.isSuccess) {
            multicastLock = lock
        }
    }

    @Synchronized
    private fun releaseMulticastLock() {
        val lock = multicastLock ?: return
        multicastLock = null
        runCatching {
            if (lock.isHeld) lock.release()
        }
    }

    private fun bluetoothPermissions(): Array<String> =
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
}
