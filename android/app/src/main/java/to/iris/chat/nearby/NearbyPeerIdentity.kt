package to.iris.chat.nearby

import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot

internal fun List<ChatThreadSnapshot>.nearbyPeerChat(owner: String?): ChatThreadSnapshot? {
    val normalizedOwner = owner?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    return firstOrNull { it.kind == ChatKind.DIRECT && it.chatId.equals(normalizedOwner, ignoreCase = true) }
}
