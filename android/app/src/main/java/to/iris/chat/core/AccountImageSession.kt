package to.iris.chat.core

/** Rejects late image loads when the local account is removed. */
internal object AccountImageSession {
    private var generation = 0L
    private val clearers = mutableListOf<() -> Unit>()

    @Synchronized fun current(): Long = generation
    @Synchronized fun isCurrent(value: Long): Boolean = generation == value
    @Synchronized fun register(clear: () -> Unit) { clearers += clear }
    @Synchronized fun <T> ifCurrent(value: Long, action: () -> T): T? =
        if (generation == value) action() else null

    @Synchronized fun clear() {
        generation += 1
        clearers.forEach { it() }
    }
}
