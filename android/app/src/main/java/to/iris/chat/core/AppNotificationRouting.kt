package to.iris.chat.core

import java.util.Locale
import org.json.JSONObject
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.Router

internal const val MOBILE_PUSH_GROUP_CHAT_PREFIX = "group:"

internal fun activeNotificationChatIds(
    currentChat: CurrentChatSnapshot?,
    router: Router,
): Set<String> =
    buildSet {
        // Single source of truth — the Rust core knows what "open chat"
        // means for the router stack. Keep the alternate currentChat
        // hints below to bridge the brief moment after we navigated but
        // before Rust has emitted the new state.
        to.iris.chat.rust
            .routerOpenChatId(router)
            ?.let(::normalizedNotificationId)
            ?.let(::add)
        normalizedNotificationId(currentChat?.chatId.orEmpty())?.let(::add)
        currentChat?.groupId
            ?.let(::normalizedNotificationId)
            ?.let { groupId ->
                add("$MOBILE_PUSH_GROUP_CHAT_PREFIX$groupId")
            }
    }

internal fun pushNotificationChatCandidates(payload: JSONObject): Set<String> =
    buildSet {
        listOf(
            "chat_id",
            "chatId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ).forEach { key ->
            normalizedNotificationId(payload.optString(key))?.let(::add)
        }
        listOf("group_id", "groupId", "group_chat_id", "groupChatId").forEach { key ->
            normalizedNotificationId(payload.optString(key))?.let { groupId ->
                if (groupId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX)) {
                    add(groupId)
                } else {
                    add("$MOBILE_PUSH_GROUP_CHAT_PREFIX$groupId")
                }
            }
        }
        listOf("sender_pubkey", "senderPubkey", "author_pubkey", "authorPubkey").forEach { key ->
            normalizedNotificationId(payload.optString(key))?.let(::add)
        }
    }

internal fun notificationChatIdMatches(
    activeChatId: String,
    pushChatId: String,
): Boolean {
    if (activeChatId == pushChatId) {
        return true
    }
    val activeGroupId = activeChatId.removePrefix(MOBILE_PUSH_GROUP_CHAT_PREFIX)
        .takeIf { activeChatId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX) }
    val pushGroupId = pushChatId.removePrefix(MOBILE_PUSH_GROUP_CHAT_PREFIX)
        .takeIf { pushChatId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX) }
    return when {
        activeGroupId != null && pushGroupId != null -> activeGroupId == pushGroupId
        activeGroupId != null -> activeGroupId == pushChatId
        pushGroupId != null -> activeChatId == pushGroupId
        else -> false
    }
}

private fun normalizedNotificationId(value: String): String? =
    value
        .trim()
        .takeIf { it.isNotEmpty() }
        ?.lowercase(Locale.ROOT)
