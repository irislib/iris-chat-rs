package to.iris.chat.nearby

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import java.io.EOFException
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.net.Socket
import java.util.ArrayDeque
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executor
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

class IrisNearbyLanService(
    context: Context,
    private val applicationScope: CoroutineScope,
    private val peerId: String,
    private val frameBodyLength: (ByteArray) -> Int,
    private val onFrame: (connectionId: String, frame: ByteArray) -> Unit,
    private val onConnected: () -> Unit,
) {
    private data class Connection(
        val id: String,
        val socket: Socket,
        val writerLock: Any = Any(),
        @Volatile var peerId: String? = null,
    )

    private val appContext = context.applicationContext
    private val nsdManager = appContext.getSystemService(NsdManager::class.java)
    private val connectivityManager = appContext.getSystemService(ConnectivityManager::class.java)
    private val wifiManager = appContext.getSystemService(WifiManager::class.java)
    private val connections = ConcurrentHashMap<String, Connection>()
    private val endpointKeys = ConcurrentHashMap.newKeySet<String>()
    private val resolveQueue = ArrayDeque<NsdServiceInfo>()
    private val resolveLock = Any()

    @Volatile
    private var enabled = false

    @Volatile
    var status: String = "Off"
        private set

    private var serverSocket: ServerSocket? = null
    private var acceptJob: Job? = null
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var registrationListener: NsdManager.RegistrationListener? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private var resolving = false
    @Volatile private var localNetwork: Network? = null
    private val directExecutor = Executor { command -> command.run() }

    fun start() {
        if (enabled) {
            return
        }
        enabled = true
        status = "Starting"
        if (!startServer()) {
            enabled = false
            return
        }
        acquireMulticastLock()
        startDiscovery()
    }

    fun stop() {
        enabled = false
        status = "Off"
        discoveryListener?.let { listener ->
            runCatching { nsdManager?.stopServiceDiscovery(listener) }
        }
        registrationListener?.let { listener ->
            runCatching { nsdManager?.unregisterService(listener) }
        }
        discoveryListener = null
        registrationListener = null
        acceptJob?.cancel()
        acceptJob = null
        releaseMulticastLock()
        runCatching { serverSocket?.close() }
        serverSocket = null
        localNetwork = null
        connections.values.forEach { connection ->
            runCatching { connection.socket.close() }
        }
        connections.clear()
        endpointKeys.clear()
        synchronized(resolveLock) {
            resolveQueue.clear()
            resolving = false
        }
    }

    fun send(
        frame: ByteArray,
        excludingPeerId: String?,
    ) {
        if (!enabled) {
            return
        }
        connections.values.forEach { connection ->
            if (excludingPeerId != null && connection.peerId == excludingPeerId) {
                return@forEach
            }
            applicationScope.launch {
                runCatching {
                    synchronized(connection.writerLock) {
                        connection.socket.getOutputStream().write(frame)
                        connection.socket.getOutputStream().flush()
                    }
                }.onFailure {
                    close(connection.id)
                }
            }
        }
    }

    fun markPeer(
        connectionId: String,
        peerId: String,
    ) {
        connections[connectionId]?.peerId = peerId
        status = if (connections.isEmpty()) "Visible" else "Connected"
    }

    fun hasPeer(peerId: String): Boolean = connections.values.any { it.peerId == peerId }

    fun peerIdForConnection(connectionId: String): String? = connections[connectionId]?.peerId

    fun peerIds(): Set<String> = connections.values.mapNotNullTo(mutableSetOf()) { it.peerId }

    private fun startServer(): Boolean {
        val binding = localNearbyBinding(connectivityManager) ?: run {
            status = "Local network unavailable"
            return false
        }
        localNetwork = binding.network
        val server =
            runCatching {
                ServerSocket().apply {
                    reuseAddress = true
                    bind(InetSocketAddress(binding.address, 0))
                }
            }.getOrElse { error ->
                Log.w(TAG, "LAN server failed", error)
                status = "Local network unavailable"
                return false
            }
        serverSocket = server
        registerService(server.localPort)
        acceptJob =
            applicationScope.launch {
                while (enabled) {
                    val socket =
                        try {
                            server.accept()
                        } catch (error: Throwable) {
                            if (error is CancellationException) throw error
                            if (enabled) {
                                Log.w(TAG, "LAN accept failed", error)
                            }
                            break
                        }
                    if (!isPrivateAddress(socket.inetAddress)) {
                        runCatching { socket.close() }
                        continue
                    }
                    add(socket)
                }
            }
        return true
    }

    private fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) {
            return
        }
        multicastLock =
            runCatching {
                wifiManager
                    ?.createMulticastLock("iris-nearby-lan")
                    ?.apply {
                        setReferenceCounted(false)
                        acquire()
                    }
            }.onFailure { error ->
                Log.w(TAG, "LAN multicast lock failed", error)
            }.getOrNull()
    }

    private fun releaseMulticastLock() {
        multicastLock?.let { lock ->
            runCatching {
                if (lock.isHeld) {
                    lock.release()
                }
            }
        }
        multicastLock = null
    }

    private fun registerService(port: Int) {
        val listener =
            object : NsdManager.RegistrationListener {
                override fun onServiceRegistered(serviceInfo: NsdServiceInfo) {
                    status = if (connections.isEmpty()) "Visible" else "Connected"
                }

                override fun onRegistrationFailed(
                    serviceInfo: NsdServiceInfo,
                    errorCode: Int,
                ) {
                    Log.w(TAG, "LAN registration failed $errorCode")
                    status = "Local network failed"
                }

                override fun onServiceUnregistered(serviceInfo: NsdServiceInfo) = Unit

                override fun onUnregistrationFailed(
                    serviceInfo: NsdServiceInfo,
                    errorCode: Int,
                ) = Unit
            }
        val serviceInfo =
            NsdServiceInfo().apply {
                serviceName = "Iris-$peerId"
                serviceType = SERVICE_TYPE
                setPort(port)
                runCatching { setAttribute("v", "1") }
            }
        registrationListener = listener
        runCatching {
            nsdManager?.registerService(serviceInfo, NsdManager.PROTOCOL_DNS_SD, listener)
        }.onFailure { error ->
            Log.w(TAG, "LAN service registration failed", error)
            status = "Local network failed"
        }
    }

    private fun startDiscovery() {
        val listener =
            object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(serviceType: String) = Unit

                override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                    if (normalizeServiceType(serviceInfo.serviceType) != SERVICE_TYPE ||
                        serviceInfo.serviceName.contains(peerId)
                    ) {
                        return
                    }
                    enqueueResolve(serviceInfo)
                }

                override fun onServiceLost(serviceInfo: NsdServiceInfo) = Unit

                override fun onDiscoveryStopped(serviceType: String) = Unit

                override fun onStartDiscoveryFailed(
                    serviceType: String,
                    errorCode: Int,
                ) {
                    Log.w(TAG, "LAN discovery failed $errorCode")
                    status = "Local network failed"
                }

                override fun onStopDiscoveryFailed(
                    serviceType: String,
                    errorCode: Int,
                ) = Unit
            }
        discoveryListener = listener
        runCatching {
            nsdManager?.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        }.onFailure { error ->
            Log.w(TAG, "LAN discovery start failed", error)
            status = "Local network failed"
        }
    }

    private fun enqueueResolve(serviceInfo: NsdServiceInfo) {
        synchronized(resolveLock) {
            resolveQueue.add(serviceInfo)
            if (!resolving) {
                resolveNextLocked()
            }
        }
    }

    private fun resolveNextLocked() {
        val next = resolveQueue.poll() ?: run {
            resolving = false
            return
        }
        resolving = true
        val listener =
            object : NsdManager.ResolveListener {
                override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                    connect(serviceInfo)
                    synchronized(resolveLock) {
                        resolveNextLocked()
                    }
                }

                override fun onResolveFailed(
                    serviceInfo: NsdServiceInfo,
                    errorCode: Int,
                ) {
                    synchronized(resolveLock) {
                        resolveNextLocked()
                    }
                }
            }
        runCatching {
            resolveService(next, listener)
        }.onFailure {
            synchronized(resolveLock) {
                resolveNextLocked()
            }
        }
    }

    private fun connect(serviceInfo: NsdServiceInfo) {
        val host = resolvedHost(serviceInfo) ?: return
        val port = serviceInfo.port
        if (!isPrivateAddress(host) || port <= 0) {
            return
        }
        val key = "${host.hostAddress}:$port"
        if (!endpointKeys.add(key)) {
            return
        }
        val network = localNetwork
        applicationScope.launch {
            val socket =
                openConnectedSocket(network, host, port)
                    ?: openConnectedSocket(null, host, port)
                    ?: run {
                        endpointKeys.remove(key)
                        return@launch
                    }
            add(socket)
        }
    }

    private fun openConnectedSocket(
        network: Network?,
        host: InetAddress,
        port: Int,
    ): Socket? =
        runCatching {
            val factory = network?.socketFactory
            val s = factory?.createSocket() ?: Socket()
            s.tcpNoDelay = true
            s.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MILLIS)
            s
        }.getOrElse { error ->
            if (enabled) {
                Log.d(TAG, "LAN connect failed ${host.hostAddress}:$port via=${network ?: "default"}", error)
            }
            null
        }

    private fun add(socket: Socket) {
        if (!enabled) {
            runCatching { socket.close() }
            return
        }
        val id = UUID.randomUUID().toString().lowercase()
        socket.tcpNoDelay = true
        val connection = Connection(id = id, socket = socket)
        connections[id] = connection
        status = "Connected"
        onConnected()
        applicationScope.launch {
            readLoop(connection)
        }
    }

    private fun readLoop(connection: Connection) {
        try {
            val input = connection.socket.getInputStream()
            while (enabled && !connection.socket.isClosed) {
                val header = readExact(input, NEARBY_FRAME_HEADER_BYTES) ?: break
                val bodyLength = frameBodyLength(header)
                if (bodyLength <= 0 || bodyLength > MAX_FRAME_BODY_BYTES) {
                    break
                }
                val body = readExact(input, bodyLength) ?: break
                onFrame(connection.id, header + body)
            }
        } catch (error: Throwable) {
            if (error is CancellationException) throw error
            if (enabled && error !is EOFException) {
                Log.d(TAG, "LAN read failed", error)
            }
        } finally {
            close(connection.id)
        }
    }

    private fun close(connectionId: String) {
        val connection = connections.remove(connectionId) ?: return
        endpointKeys.remove("${connection.socket.inetAddress.hostAddress}:${connection.socket.port}")
        runCatching { connection.socket.close() }
        status = if (!enabled) "Off" else if (connections.isEmpty()) "Visible" else "Connected"
    }

    private companion object {
        const val TAG = "IrisNearbyLan"
        const val SERVICE_TYPE = "_iris-chat._tcp"
        const val NEARBY_FRAME_HEADER_BYTES = 13
        const val MAX_FRAME_BODY_BYTES = 256 * 1024
        const val CONNECT_TIMEOUT_MILLIS = 5_000

        private fun normalizeServiceType(type: String?): String =
            type?.trim('.', ' ')?.removePrefix("_")?.let { "_$it" } ?: ""

        fun readExact(
            input: java.io.InputStream,
            count: Int,
        ): ByteArray? {
            val data = ByteArray(count)
            var offset = 0
            while (offset < count) {
                val read = input.read(data, offset, count - offset)
                if (read < 0) {
                    return null
                }
                offset += read
            }
            return data
        }

        fun isPrivateAddress(address: InetAddress): Boolean {
            if (address.isAnyLocalAddress || address.isLoopbackAddress || address.isLinkLocalAddress || address.isSiteLocalAddress) {
                return true
            }
            if (address is Inet6Address) {
                val first = address.address.firstOrNull()?.toInt()?.and(0xff) ?: return false
                return first and 0xfe == 0xfc
            }
            return false
        }

        data class NearbyBinding(val network: Network?, val address: InetAddress)

        fun localNearbyBinding(connectivityManager: ConnectivityManager?): NearbyBinding? {
            networkBindings(connectivityManager).forEach { binding ->
                preferredAddress(binding.addresses)?.let { return NearbyBinding(binding.network, it) }
            }
            return preferredAddress(interfaceAddresses())?.let { NearbyBinding(null, it) }
        }

        private fun preferredAddress(candidates: List<InetAddress>): InetAddress? {
            var fallback: InetAddress? = null
            candidates.forEach { address ->
                if (
                    !isPrivateAddress(address) ||
                        address.isAnyLocalAddress ||
                        address.isLoopbackAddress
                ) {
                    return@forEach
                }
                if (address is Inet4Address && !address.isLinkLocalAddress) {
                    return address
                }
                if (fallback == null) {
                    fallback = address
                }
            }
            return fallback
        }

        private data class NetworkBinding(val network: Network, val addresses: List<InetAddress>)

        @Suppress("DEPRECATION")
        private fun networkBindings(connectivityManager: ConnectivityManager?): List<NetworkBinding> {
            val manager = connectivityManager ?: return emptyList()
            return runCatching {
                manager.allNetworks.mapNotNull { network ->
                    val capabilities = manager.getNetworkCapabilities(network) ?: return@mapNotNull null
                    if (!isLocalNearbyTransport(capabilities)) {
                        return@mapNotNull null
                    }
                    val addresses =
                        manager
                            .getLinkProperties(network)
                            ?.linkAddresses
                            ?.map { it.address }
                            .orEmpty()
                    NetworkBinding(network, addresses)
                }
            }.getOrDefault(emptyList())
        }

        private fun interfaceAddresses(): List<InetAddress> {
            val interfaces = NetworkInterface.getNetworkInterfaces()?.toList() ?: return emptyList()
            return buildList {
                interfaces.forEach { networkInterface ->
                    val usable =
                        runCatching {
                            networkInterface.isUp &&
                                !networkInterface.isLoopback &&
                                isLocalNearbyInterface(networkInterface)
                        }.getOrDefault(false)
                    if (!usable) {
                        return@forEach
                    }
                    addAll(networkInterface.inetAddresses.toList())
                }
            }
        }

        private fun isLocalNearbyTransport(capabilities: NetworkCapabilities): Boolean {
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
                return false
            }
            return capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        }

        private fun isLocalNearbyInterface(networkInterface: NetworkInterface): Boolean {
            val name = networkInterface.name.lowercase()
            if (name.startsWith("wlan") || name.startsWith("wifi") || name.startsWith("eth")) {
                return true
            }
            val displayName = networkInterface.displayName.lowercase()
            return displayName.contains("wifi") ||
                displayName.contains("wi-fi") ||
                displayName.contains("ethernet")
        }
    }

    @Suppress("DEPRECATION")
    private fun resolveService(
        serviceInfo: NsdServiceInfo,
        listener: NsdManager.ResolveListener,
    ) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            nsdManager?.resolveService(serviceInfo, directExecutor, listener)
        } else {
            nsdManager?.resolveService(serviceInfo, listener)
        }
    }

    @Suppress("DEPRECATION")
    private fun resolvedHost(serviceInfo: NsdServiceInfo): InetAddress? = serviceInfo.host
}
