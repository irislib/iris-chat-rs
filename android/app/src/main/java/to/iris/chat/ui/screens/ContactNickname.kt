package to.iris.chat.ui.screens

/** Match Rust's split_whitespace without Android-incompatible regex flags. */
internal fun normalizeContactNickname(value: String): String = buildString {
    var spacePending = false
    for (character in value) {
        // Unicode White_Space; all members are in the BMP. Keep other characters,
        // including surrogate pairs and zero-width characters, exactly as entered.
        val whitespace = when (character) {
            in '\u0009'..'\u000d', ' ', '\u0085', '\u00a0', '\u1680',
            in '\u2000'..'\u200a', '\u2028', '\u2029', '\u202f', '\u205f', '\u3000' -> true
            else -> false
        }
        if (whitespace) {
            if (isNotEmpty()) spacePending = true
        } else {
            if (spacePending) append(' ')
            append(character)
            spacePending = false
        }
    }
}
