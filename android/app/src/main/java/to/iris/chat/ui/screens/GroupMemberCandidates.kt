package to.iris.chat.ui.screens

import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot

internal fun groupMemberCandidates(
    chats: List<ChatThreadSnapshot>,
    localOwner: String?,
    members: Set<String>,
    query: String,
): List<ChatThreadSnapshot> = chats.filter {
    it.kind == ChatKind.DIRECT && it.chatId != localOwner && it.chatId !in members
}.filterByQuery(query)
