package org.fips.ble

/** Bounds GATT discovery, including failures before FIPS learns about a peer. */
internal class BootstrapDiscoveryCache(
    private val capacity: Int = 64,
    private val clock: () -> Long = { System.nanoTime() / 1_000_000 },
) {
    private val pending = mutableMapOf<String, Long>()
    private val completed = linkedSetOf<String>()
    private data class Failure(val delay: Long, val retryAt: Long)
    private val failures = linkedMapOf<String, Failure>()

    init {
        require(capacity > 0)
    }

    fun begin(token: String): Boolean {
        if (token in pending || token in completed || pending.size == capacity) return false
        val now = clock()
        if (now < (failures[token]?.retryAt ?: Long.MIN_VALUE)) return false
        pending[token] = now + 15_000
        return true
    }

    fun complete(token: String): Boolean {
        if (pending.remove(token) == null) return false
        failures.remove(token)
        completed.add(token)
        if (completed.size > capacity) completed.remove(completed.first())
        return true
    }

    fun fail(token: String) {
        if (pending.remove(token) == null) return
        val delay = ((failures[token]?.delay ?: 2_500) * 2).coerceAtMost(60_000)
        failures[token] = Failure(delay, clock() + delay)
        if (failures.size > capacity) failures.remove(failures.keys.first())
    }

    fun expirePending(): List<String> {
        val now = clock()
        val expired = pending.filterValues { it <= now }.keys.toList()
        expired.forEach(::fail)
        return expired
    }

    fun dueRetries(): List<String> {
        if (pending.size == capacity) return emptyList()
        val now = clock()
        return failures.filter { (token, failure) -> token !in pending && failure.retryAt <= now }.keys.toList()
    }

    fun nextWakeupAt(): Long? {
        val retry = if (pending.size == capacity) null else {
            failures.filterKeys { it !in pending }.values.minOfOrNull { it.retryAt }
        }
        return listOfNotNull(pending.values.minOrNull(), retry).minOrNull()
    }

    fun invalidate(token: String) {
        // A rejected L2CAP open refreshes the PSM without cancelling a read or
        // bypassing the backoff from a failed GATT discovery.
        completed.remove(token)
    }

    fun clear() {
        pending.clear()
        completed.clear()
        failures.clear()
    }
}
