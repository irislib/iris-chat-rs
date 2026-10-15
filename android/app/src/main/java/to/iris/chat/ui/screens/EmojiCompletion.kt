package to.iris.chat.ui.screens

import java.util.Locale

internal data class ComposerEmojiToken(val start: Int, val end: Int, val query: String, val closed: Boolean) {
    companion object {
        fun find(text: String, caret: Int, selectionEnd: Int = caret): ComposerEmojiToken? {
            if (caret != selectionEnd || caret !in 0..text.length ||
                (caret > 0 && caret < text.length && Character.isSurrogatePair(text[caret - 1], text[caret]))) return null
            val prefix = text.substring(0, caret)
            val start = prefix.indexOfLast { it.isWhitespace() } + 1
            if (start >= caret || text[start] != ':') return null
            val closed = caret > start + 1 && text[caret - 1] == ':'
            val query = text.substring(start + 1, if (closed) caret - 1 else caret)
            if (query.length > 64 || (closed && query.isEmpty()) || !query.all(::tokenCharacter)) return null
            var end = caret
            if (!closed) {
                while (end < text.length && tokenCharacter(text[end])) {
                    if (end - start > 64) return null
                    end++
                }
                if (end < text.length && text[end] == ':') end++
            }
            return ComposerEmojiToken(start, end, query, closed)
        }

        private fun tokenCharacter(char: Char) = char.isLetterOrDigit() || char in "_+-"
    }
}

internal data class ComposerEmojiEntry(val emoji: String, val names: List<String>) {
    val label: String get() = names.first()
    fun matchesExactly(query: String) = names.any { normalize(it) == normalize(query) }

    companion object {
        private fun normalize(value: String) = value.lowercase(Locale.ROOT).replace('-', '_')
        fun search(query: String, entries: List<ComposerEmojiEntry>): List<ComposerEmojiEntry> {
            val normalized = normalize(query)
            if (normalized.length < 2) return emptyList()
            return entries.mapIndexedNotNull { index, entry ->
                val names = entry.names.map(::normalize)
                val rank = when {
                    normalized in names -> 0
                    names.any { it.startsWith(normalized) } -> 1
                    names.any { it.contains(normalized) } -> 2
                    else -> return@mapIndexedNotNull null
                }
                Triple(rank, index, entry)
            }.sortedWith(compareBy({ it.first }, { it.second })).take(6).map { it.third }
        }
    }
}
