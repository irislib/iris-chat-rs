package to.iris.chat.calls

/** Keep the requested average cadence when the camera rate is not a multiple of it. */
internal class CallFramePacer {
    private var next = 0L
    private var previous = 0L
    private var rate = 0

    fun accept(timestampNs: Long, fps: Int): Boolean {
        val interval = 1_000_000_000L / fps.coerceAtLeast(1)
        if (fps != rate || timestampNs < previous) { next = 0; rate = fps }
        previous = timestampNs
        if (next != 0L && timestampNs < next) return false
        // Drop missed deadlines after pauses; never encode a catch-up burst.
        next = if (next == 0L || timestampNs - next >= interval) timestampNs + interval else next + interval
        return true
    }
}
