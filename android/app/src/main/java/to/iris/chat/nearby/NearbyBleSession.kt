package to.iris.chat.nearby

/** The attached native endpoint captures LAN settings when its bridge starts. */
internal class NearbyBleSession<T : AutoCloseable>(
    private val create: () -> T,
) : AutoCloseable {
    private var runtime: T? = null
    private var nearbyLanEnabled: Boolean? = null
    private var closed = false

    @Synchronized
    fun update(enabled: Boolean, nearbyLanEnabled: Boolean) {
        if (closed || (enabled && runtime != null && this.nearbyLanEnabled == nearbyLanEnabled)) return
        // Detach finishes before a replacement can attach, including when
        // preference callbacks and client shutdown arrive on different threads.
        stop()
        if (enabled) {
            runtime = create()
            this.nearbyLanEnabled = nearbyLanEnabled
        }
    }

    private fun stop() {
        // A failed close must not leave a partly stopped bridge reusable.
        nearbyLanEnabled = null
        runtime?.close()
        runtime = null
    }

    @Synchronized
    override fun close() {
        closed = true
        stop()
    }

    @Synchronized
    fun shutdownCoreAndCleanup(shutdownAndWait: () -> Unit, cleanup: (T) -> Unit) {
        check(closed)
        // A timeout or failed FFI call is not permission to stop the platform.
        // Only a completed core join makes failed-detach cleanup safe.
        shutdownAndWait()
        runtime?.let(cleanup)
        runtime = null
    }
}
