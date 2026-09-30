package to.iris.chat.core

import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.CurrentChatSnapshot

val CurrentChatSnapshot.isRemovedFromGroup: Boolean
    get() = kind == ChatKind.GROUP && participants.none { it.isLocalOwner }

internal fun blocksRemovedGroupAction(chat: CurrentChatSnapshot?, action: AppAction): Boolean {
    if (chat?.isRemovedFromGroup != true) return false
    val target = when (action) {
        is AppAction.SendMessage -> action.chatId
        is AppAction.SendDisappearingMessage -> action.chatId
        is AppAction.SendAttachment -> action.chatId
        is AppAction.SendAttachments -> action.chatId
        is AppAction.SendTyping -> action.chatId
        is AppAction.ToggleReaction -> action.chatId
        else -> null
    }
    return target == chat.chatId
}
