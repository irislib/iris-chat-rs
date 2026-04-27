package to.iris.chat.ui.screens

import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.normalizePeerInput

internal fun List<ChatThreadSnapshot>.filterByQuery(query: String): List<ChatThreadSnapshot> {
    val raw = query.trim()
    if (raw.isEmpty()) return this
    val lower = raw.lowercase()
    val normalized = normalizePeerInput(raw).lowercase()
    return filter { chat ->
        chat.displayName.lowercase().contains(lower) ||
            chat.chatId.lowercase().contains(normalized) ||
            (chat.subtitle?.lowercase()?.contains(lower) == true)
    }
}
