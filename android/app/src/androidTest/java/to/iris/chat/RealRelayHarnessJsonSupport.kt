package to.iris.chat

import org.json.JSONArray
import org.json.JSONObject
import to.iris.chat.rust.CurrentChatSnapshot

fun summarizeProtocolEnginePending(protocolEngine: JSONObject): String {
    val senderMessageCountKey =
        if (protocolEngine.has("pending_group_sender_key_retry_count")) {
            "pending_group_sender_key_retry_count"
        } else {
            "pending_group_sender_key_message_count"
        }
    val counts =
        listOf(
            "out" to "pending_outbound_count",
            "in" to "pending_inbound_count",
            "fanout" to "pending_group_fanout_count",
            "pairwise" to "pending_group_pairwise_payload_count",
            "senderMsg" to senderMessageCountKey,
            "senderRepair" to "pending_group_sender_key_repair_count",
        ).mapNotNull { (label, key) ->
            protocolEngine.optInt(key).takeIf { it > 0 }?.let { "$label=$it" }
        }.joinToString(",")
    val senderUnmapped = protocolEngine.optInt("pending_group_sender_key_unmapped_count")
    val senderRepairNext = protocolEngine.optInt("pending_group_sender_key_repair_next_retry_at_secs")
    val senderRepairRequests = protocolEngine.optInt("pending_group_sender_key_repair_max_request_count")
    val outboundTargets = protocolEngine.optJSONArray("pending_outbound_targets").joinValues(limit = 6)
    val fanoutTargets = protocolEngine.optJSONArray("pending_group_fanout_targets").joinValues(limit = 6)
    val outboundDetails =
        protocolEngine.optJSONArray("pending_outbound_details").joinObjects(limit = 3) { entry ->
            listOf(
                entry.optString("reason"),
                entry.optString("chat_id"),
                entry.optString("recipient_owner_hex"),
                entry.optJSONArray("queued_targets").joinValues(limit = 4),
            ).filter { it.isNotEmpty() }.joinToString(",")
        }
    return listOf(
        if (counts.isEmpty()) "" else "counts=$counts",
        if (senderUnmapped == 0) "" else "senderUnmapped=$senderUnmapped",
        if (senderRepairNext == 0) "" else "senderRepairNext=$senderRepairNext",
        if (senderRepairRequests == 0) "" else "senderRepairRequests=$senderRepairRequests",
        if (outboundTargets.isEmpty()) "" else "outboundTargets=$outboundTargets",
        if (outboundDetails.isEmpty()) "" else "outboundDetails=$outboundDetails",
        if (fanoutTargets.isEmpty()) "" else "fanoutTargets=$fanoutTargets",
    ).filter { it.isNotEmpty() }.joinToString(" ")
}

fun protocolEnginePendingCount(protocolEngine: JSONObject): Int {
    val senderMessageCount =
        if (protocolEngine.has("pending_group_sender_key_retry_count")) {
            protocolEngine.optInt("pending_group_sender_key_retry_count")
        } else {
            protocolEngine.optInt("pending_group_sender_key_message_count")
        }
    return senderMessageCount + listOf(
        "pending_outbound_count",
        "pending_inbound_count",
        "pending_group_fanout_count",
        "pending_group_pairwise_payload_count",
        "pending_group_sender_key_repair_count",
    ).sumOf { key -> protocolEngine.optInt(key) }
}

fun summarizeKnownUsers(
    snapshot: JSONObject,
    source: String,
): String =
    if (source == "runtime") {
        summarizeRuntimeKnownUsers(snapshot.optJSONArray("known_users"))
    } else {
        summarizePersistedUsers(snapshot.optJSONObject("session_manager")?.optJSONArray("users"))
    }

fun summarizeCurrentChat(chat: CurrentChatSnapshot?): String =
    chat?.let {
        listOf(
            it.chatId,
            it.displayName,
            it.groupId.orEmpty(),
            it.memberCount.toString(),
            it.messages.size.toString(),
        ).joinToString(",")
    }.orEmpty()

fun summarizeChatList(threads: List<to.iris.chat.rust.ChatThreadSnapshot>): String =
    threads.joinToString("|") { thread ->
        listOf(
            thread.chatId,
            thread.kind.name,
            thread.displayName,
            thread.memberCount.toString(),
            thread.lastMessagePreview.orEmpty(),
            thread.unreadCount.toString(),
        ).joinToString(",")
    }

fun summarizeRuntimeKnownUsers(users: JSONArray?): String =
    users.joinObjects { user ->
        listOf(
            user.optString("owner_pubkey_hex"),
            "roster=${user.optBoolean("has_roster")}",
            "rosterDevices=${user.optInt("roster_device_count")}",
            "devices=${user.optInt("device_count")}",
            "authorized=${user.optInt("authorized_device_count")}",
            "active=${user.optInt("active_session_device_count")}",
            "inactive=${user.optInt("inactive_session_count")}",
        ).joinToString(",")
    }

fun summarizeRuntimePendingOutbound(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("message_id"),
            entry.optString("chat_id"),
            entry.optString("reason"),
            entry.optString("publish_mode"),
            "inFlight=${entry.optBoolean("in_flight")}",
        ).joinToString(",")
    }

fun summarizeRuntimePendingRelayPublishes(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("event_id"),
            entry.optString("label"),
            entry.optString("chat_id"),
            entry.optString("inner_event_id"),
            "attempts=${entry.optInt("attempt_count")}",
            "error=${entry.optString("last_error")}",
        ).joinToString(",")
    }

fun summarizeRuntimePendingGroupControls(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("operation_id"),
            entry.optString("group_id"),
            entry.optString("reason"),
            entry.optString("kind"),
            "targets=${entry.optStringArray("target_owner_hexes")}",
            "inFlight=${entry.optBoolean("in_flight")}",
        ).joinToString(",")
    }

fun summarizeRecentHandshakePeers(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("owner_hex"),
            entry.optString("device_hex"),
            entry.optString("observed_at_secs"),
        ).joinToString(",")
    }

fun summarizeEventCounts(eventCounts: JSONObject?): String =
    if (eventCounts == null) {
        ""
    } else {
        listOf(
            "roster=${eventCounts.optInt("roster_events")}",
            "invite=${eventCounts.optInt("invite_events")}",
            "inviteResponse=${eventCounts.optInt("invite_response_events")}",
            "message=${eventCounts.optInt("message_events")}",
            "other=${eventCounts.optInt("other_events")}",
        ).joinToString(",")
    }

fun summarizeRecentLog(entries: JSONArray?): String =
    entries.joinObjects(limit = 80) { entry ->
        listOf(
            entry.optString("timestamp_secs"),
            entry.optString("category"),
            entry.optString("detail"),
        ).joinToString(",")
    }

fun summarizePersistedUsers(users: JSONArray?): String =
    users.joinObjects { user ->
        val devices = user.optJSONArray("devices")
        val activeSessions =
            devices.countObjects { device ->
                !device.isNull("active_session")
            }
        val inactiveSessions =
            devices.sumObjects { device ->
                device.optJSONArray("inactive_sessions")?.length() ?: 0
            }
        listOf(
            user.optString("owner_pubkey"),
            "roster=${!user.isNull("roster")}",
            "devices=${devices?.length() ?: 0}",
            "active=${activeSessions}",
            "inactive=${inactiveSessions}",
        ).joinToString(",")
    }

fun summarizePersistedGroups(groups: JSONArray?): String =
    groups.joinObjects { group ->
        listOf(
            group.optString("group_id"),
            group.optString("name"),
            "revision=${group.optLong("revision")}",
            "members=${group.optJSONArray("members")?.length() ?: 0}",
            "admins=${group.optJSONArray("admins")?.length() ?: 0}",
        ).joinToString(",")
    }

fun summarizePersistedPendingOutbound(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("message_id"),
            entry.optString("chat_id"),
            entry.optString("reason"),
            entry.optString("publish_mode"),
            "inFlight=${entry.optBoolean("in_flight")}",
        ).joinToString(",")
    }

fun summarizePersistedPendingGroupControls(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("operation_id"),
            entry.optString("group_id"),
            entry.optString("reason"),
            entry.opt("kind")?.toString().orEmpty(),
            "inFlight=${entry.optBoolean("in_flight")}",
        ).joinToString(",")
    }

fun summarizePersistedThreads(entries: JSONArray?): String =
    entries.joinObjects { entry ->
        listOf(
            entry.optString("chat_id"),
            "messages=${entry.optJSONArray("messages")?.length() ?: 0}",
            "unread=${entry.optLong("unread_count")}",
        ).joinToString(",")
    }

fun JSONObject?.optStringOrEmpty(key: String): String =
    if (this == null || !has(key) || isNull(key)) {
        ""
    } else {
        opt(key)?.toString().orEmpty()
    }

fun JSONObject?.optStringArray(key: String): String =
    this?.optJSONArray(key).joinValues().orEmpty()

fun JSONArray?.joinObjects(
    limit: Int = Int.MAX_VALUE,
    block: (JSONObject) -> String,
): String {
    if (this == null) {
        return ""
    }
    val values = mutableListOf<String>()
    for (index in 0 until minOf(length(), limit)) {
        val obj = optJSONObject(index) ?: continue
        values += block(obj)
    }
    return values.joinToString("|")
}

fun JSONArray?.joinValues(limit: Int = Int.MAX_VALUE): String {
    if (this == null) {
        return ""
    }
    val values = mutableListOf<String>()
    for (index in 0 until minOf(length(), limit)) {
        values += opt(index)?.toString().orEmpty()
    }
    return values.joinToString("|")
}

fun JSONArray?.countObjects(predicate: (JSONObject) -> Boolean): Int {
    if (this == null) {
        return 0
    }
    var count = 0
    for (index in 0 until length()) {
        val obj = optJSONObject(index) ?: continue
        if (predicate(obj)) {
            count += 1
        }
    }
    return count
}

fun JSONArray?.sumObjects(transform: (JSONObject) -> Int): Int {
    if (this == null) {
        return 0
    }
    var sum = 0
    for (index in 0 until length()) {
        val obj = optJSONObject(index) ?: continue
        sum += transform(obj)
    }
    return sum
}
