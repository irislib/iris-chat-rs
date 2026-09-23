package to.iris.chat.calls

internal data class CallQuality(val profile: String = "auto", val maxBitrateBps: Int? = null) {
    val bitrate: Int get() = (maxBitrateBps ?: when (profile) {
        "high" -> 4_000_000
        "data" -> 400_000
        else -> 2_000_000
    }).coerceIn(100_000, 10_000_000)
    val fps: Int get() = if (profile == "data") 15 else 30
}
