package to.iris.chat

import android.database.sqlite.SQLiteDatabase
import android.util.Base64
import android.os.Bundle
import android.os.SystemClock
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.core.AppManager
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.rust.peerInputToHex
import to.iris.chat.rust.Screen
import to.iris.chat.nearby.IrisNearbyService
import java.io.File

abstract class RealRelayHarnessBase {
    abstract val activityRule: ActivityScenarioRule<MainActivity>

    protected val instrumentation
        get() = InstrumentationRegistry.getInstrumentation()

    protected val arguments
        get() = InstrumentationRegistry.getArguments()

    protected fun appManager(): AppManager =
        (instrumentation.targetContext.applicationContext as IrisChatApp).container.appManager

    protected fun nearbyService(): IrisNearbyService =
        (instrumentation.targetContext.applicationContext as IrisChatApp).container.nearbyIrisService

    protected fun appFilesDir(): File = instrumentation.targetContext.filesDir

    protected fun appPackageName(): String = instrumentation.targetContext.packageName

    protected fun <T> withActivity(block: (MainActivity) -> T): T {
        var result: Result<T>? = null
        activityRule.scenario.onActivity { activity ->
            result = runCatching { block(activity) }
        }
        return result?.getOrThrow() ?: error("Activity was not available")
    }

    protected fun ensureLoggedIn(createIfMissing: Boolean = false): to.iris.chat.rust.AccountSnapshot {
        var createRequested = false
        return waitForState("logged in account", timeoutMs = 90_000) {
            val manager = appManager()
            manager.state.value.account?.let { return@waitForState it }

            when (manager.bootstrapState.value) {
                AccountBootstrapState.Loading -> null
                AccountBootstrapState.NeedsLogin -> {
                    if (createIfMissing && !createRequested) {
                        createRequested = true
                        manager.createAccount()
                    }
                    null
                }
                is AccountBootstrapState.LoggedIn -> null
            }
        }
    }

    protected fun waitForPersistedDeviceSecret() {
        waitForState("persisted device secret", timeoutMs = 90_000) {
            val ready = kotlinx.coroutines.runBlocking { appManager().hasPersistedDeviceSecret() }
            true.takeIf { ready }
        }
    }

    protected fun maybeDisableRelays() {
        if (optionalArg("disable_relays") != "0") {
            disableRelays()
        }
    }

    protected fun disableRelays() {
        while (true) {
            val relays = appManager().state.value.preferences.nostrRelayUrls
            if (relays.isEmpty()) {
                return
            }
            val relayUrl = relays.first()
            appManager().dispatch(AppAction.RemoveNostrRelay(relayUrl))
            waitForState<Boolean>("removed relay $relayUrl", timeoutMs = 30_000) {
                true.takeIf {
                    !appManager().state.value.preferences.nostrRelayUrls.contains(relayUrl)
                }
            }
        }
    }

    protected fun ensureLinkedDeviceStarted(ownerInput: String): to.iris.chat.rust.AccountSnapshot {
        var linkRequested = false
        return waitForState("linked device account", timeoutMs = 90_000) {
            val manager = appManager()
            manager.state.value.account?.let { account ->
                if (account.authorizationState == DeviceAuthorizationState.AWAITING_APPROVAL ||
                    account.authorizationState == DeviceAuthorizationState.AUTHORIZED
                ) {
                    return@waitForState account
                }
            }

            when (manager.bootstrapState.value) {
                AccountBootstrapState.Loading -> null
                AccountBootstrapState.NeedsLogin -> {
                    if (!linkRequested) {
                        linkRequested = true
                        manager.startLinkedDevice(ownerInput)
                    }
                    null
                }
                is AccountBootstrapState.LoggedIn -> null
            }
        }
    }

    protected fun ensureChatOpen(peerInput: String): CurrentChatSnapshot {
        val existing =
            waitForOptionalState(timeoutMs = 5_000) {
                findChatMatchingPeerInput(peerInput)
            }
        if (existing != null) {
            appManager().openChat(existing.chatId)
            return waitForState("existing chat") {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> matchesPeerInput(current.chatId, current.subtitle.orEmpty(), peerInput) }
            }
        }

        appManager().createChat(peerInput)
        return waitForState("created chat") {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { current -> matchesPeerInput(current.chatId, current.subtitle.orEmpty(), peerInput) }
        }
    }

    protected fun findChatMatchingPeerInput(peerInput: String): ChatThreadSnapshot? =
        appManager().state.value.chatList.firstOrNull { thread ->
            matchesPeerInput(
                chatId = thread.chatId,
                peerNpub = thread.subtitle.orEmpty(),
                peerInput = peerInput,
            )
        }

    protected fun ensureChatOpenById(chatId: String): CurrentChatSnapshot {
        val trimmed = chatId.trim()
        require(trimmed.isNotEmpty()) { "chat id must not be blank" }
        appManager().openChat(trimmed)
        return waitForState("opened chat by id") {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { current -> current.chatId == trimmed }
            }
    }

    protected fun resolvePeerOwnerHex(peerInput: String): String =
        appManager()
            .state
            .value
            .chatList
            .firstOrNull { thread ->
                matchesPeerInput(
                    chatId = thread.chatId,
                    peerNpub = thread.subtitle.orEmpty(),
                    peerInput = peerInput,
                )
            }
            ?.chatId
            ?: normalizePeerInput(peerInput)

    protected fun matchesPeerInput(
        chatId: String,
        peerNpub: String,
        peerInput: String,
    ): Boolean {
        // chatId for direct chats is canonical lowercase hex; peerInput
        // is whatever the caller had handy (npub / hex / nprofile…).
        // `normalizePeerInput` returns an npub when the input is
        // npub-shaped and hex when it's already hex, so it's not a
        // single canonical form — compare against both via
        // `peerInputToHex`. Without this the harness silently fails to
        // find existing chats by npub, falls back to `createChat`, and
        // times out waiting for a currentChat that already matches.
        val normalizedDisplay = normalizePeerInput(peerInput)
        val hex = peerInputToHex(peerInput)
        if (hex.isNotEmpty() && chatId.equals(hex, ignoreCase = true)) {
            return true
        }
        return chatId.equals(normalizedDisplay, ignoreCase = true) ||
            peerNpub.equals(normalizedDisplay, ignoreCase = true)
    }

    protected fun deviceMatchesInput(
        devicePubkeyHex: String,
        deviceNpub: String,
        deviceInput: String,
    ): Boolean {
        val trimmed = deviceInput.trim()
        if (trimmed.isEmpty()) {
            return false
        }
        val normalized = normalizePeerInput(trimmed)
        return devicePubkeyHex.equals(normalized, ignoreCase = true) ||
            deviceNpub.equals(trimmed, ignoreCase = true) ||
            deviceNpub.equals(normalized, ignoreCase = true)
    }

    protected fun chatMatchesExpectedChat(
        chatId: String,
        peerInput: String,
        expectedChatId: String?,
    ): Boolean {
        if (!expectedChatId.isNullOrBlank()) {
            return chatId.equals(expectedChatId, ignoreCase = true)
        }
        if (peerInput.isBlank()) {
            return true
        }
        val hex = peerInputToHex(peerInput)
        if (hex.isNotEmpty() && chatId.equals(hex, ignoreCase = true)) {
            return true
        }
        return chatId.equals(normalizePeerInput(peerInput), ignoreCase = true)
    }

    protected fun messageDirectionMatches(
        isOutgoing: Boolean,
        direction: String,
    ): Boolean =
        when (direction) {
            "", "incoming" -> !isOutgoing
            "outgoing" -> isOutgoing
            "any" -> true
            else -> !isOutgoing
        }

    protected data class ChatSettings(
        val muted: Boolean,
        val pinned: Boolean,
        val ttlSeconds: ULong?,
    )

    protected fun waitForChatSettings(
        chatId: String,
        timeoutMs: Long,
    ): ChatSettings {
        val expectedMuted = optionalBoolArg("muted")
        val expectedPinned = optionalBoolArg("pinned")
        val expectedTtl = optionalArg("ttl_seconds")?.toULongOrNull()
        return waitForState("chat settings", timeoutMs = timeoutMs) {
            val state = appManager().state.value
            val current =
                state.currentChat?.takeIf { current ->
                    current.chatId.equals(chatId, ignoreCase = true)
                }
            val thread =
                state.chatList.firstOrNull { thread ->
                    thread.chatId.equals(chatId, ignoreCase = true)
                }
            val muted = current?.isMuted ?: thread?.isMuted ?: false
            val pinned = thread?.isPinned ?: false
            val ttl = current?.messageTtlSeconds
            if (expectedMuted != null && muted != expectedMuted) {
                return@waitForState null
            }
            if (expectedPinned != null && pinned != expectedPinned) {
                return@waitForState null
            }
            if (expectedTtl != null && ttl != expectedTtl) {
                return@waitForState null
            }
            ChatSettings(muted = muted, pinned = pinned, ttlSeconds = ttl)
        }
    }

    protected fun requiredAuthorizationState(): DeviceAuthorizationState =
        when (requiredArg("authorization_state").trim().uppercase()) {
            "AUTHORIZED" -> DeviceAuthorizationState.AUTHORIZED
            "AWAITING_APPROVAL" -> DeviceAuthorizationState.AWAITING_APPROVAL
            "REVOKED" -> DeviceAuthorizationState.REVOKED
            else -> throw AssertionError("Unsupported authorization_state argument")
        }

    protected fun optionalArg(name: String): String? =
        arguments.getString("${name}_b64")
            ?.takeIf { it.isNotBlank() }
            ?.let(::decodeBase64Arg)
            ?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?: arguments.getString(name)?.trim()?.takeIf { it.isNotEmpty() }

    protected fun optionalBoolArg(name: String): Boolean? =
        when (optionalArg(name)?.lowercase()) {
            "1", "true", "yes", "on" -> true
            "0", "false", "no", "off" -> false
            else -> null
        }

    protected fun requiredArg(name: String): String {
        optionalArg(name)?.let { return it }
        if (arguments.getString("class").isNullOrBlank()) {
            assumeTrue("Harness action requires instrumentation argument: $name", false)
        }
        throw AssertionError("Missing instrumentation argument: $name")
    }

    protected fun requireHarnessInvocation(reason: String) {
        if (arguments.getString("class").isNullOrBlank()) {
            assumeTrue(reason, false)
        }
    }

    protected fun waitForRelayDrainIfRequested() {
        val raw = optionalArg("wait_for_relay_drain")?.lowercase() ?: return
        if (raw !in setOf("1", "true", "yes")) {
            return
        }

        SystemClock.sleep(500)
        val runtimeOnly =
            optionalArg("relay_drain_runtime_only")?.lowercase() in setOf("1", "true", "yes")
        val timeoutMs =
            ((optionalArg("relay_drain_timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
                .coerceAtLeast(1_000L)
        val wakeRelay = relayWakeCallback()
        wakeRelay()
        val status =
            waitForState("relay publish drain", timeoutMs = timeoutMs) {
                wakeRelay()
                val pendingDurablePublishCount = pendingRelayPublishCount()
                appManager()
                    .state
                    .value
                    .networkStatus
                    ?.takeIf { status ->
                        (status.relayUrls.isEmpty() || status.connectedRelayCount > 0UL) &&
                            !status.syncing &&
                            pendingDurablePublishCount == 0 &&
                            (runtimeOnly || status.pendingOutboundCount == 0UL) &&
                            status.pendingGroupControlCount == 0UL
                    }
            }
        reportStatus(
            "pending_outbound_count" to status.pendingOutboundCount.toString(),
            "pending_runtime_outbound_count" to pendingRelayPublishCount().toString(),
            "pending_group_control_count" to status.pendingGroupControlCount.toString(),
            "network_syncing" to status.syncing.toString(),
        )
    }

    protected fun waitForRuntimeSnapshotIdleIfRequested(): JSONObject? {
        val waitForRelayDrain = optionalArg("wait_for_relay_drain")?.lowercase() in setOf("1", "true", "yes")
        val waitForRuntimeIdle = optionalArg("wait_for_runtime_idle")?.lowercase() in setOf("1", "true", "yes")
        val runtimeOnly =
            optionalArg("relay_drain_runtime_only")?.lowercase() in setOf("1", "true", "yes")
        if ((!waitForRelayDrain && !waitForRuntimeIdle) || runtimeOnly) {
            return null
        }

        val timeoutMs =
            ((optionalArg("runtime_idle_timeout_secs")
                ?: optionalArg("relay_drain_timeout_secs"))
                ?.toLongOrNull()
                ?: 60L) * 1_000L
        val deadline = SystemClock.elapsedRealtime() + timeoutMs.coerceAtLeast(1_000L)
        val wakeRelay = relayWakeCallback()
        var lastDebug: JSONObject? = null
        while (SystemClock.elapsedRealtime() < deadline) {
            wakeRelay()
            val debug = readLiveRuntimeDebugSnapshot()
            if (debug != null) {
                lastDebug = debug
                if (runtimeSnapshotIsIdle(debug)) {
                    reportStatus(
                        "runtime_settled" to "true",
                        "runtime_pending_summary" to runtimePendingSummary(debug),
                    )
                    return debug
                }
            }
            SystemClock.sleep(200)
        }

        reportStatus(
            "runtime_settled" to "false",
            "runtime_pending_summary" to (lastDebug?.let(::runtimePendingSummary) ?: "snapshot=unavailable"),
        )
        throw AssertionError("Timed out waiting for runtime snapshot idle")
    }

    protected fun requiredListArg(name: String): List<String> =
        requiredArg(name)
            .split(',', '\n', '|')
            .map(String::trim)
            .filter(String::isNotEmpty)
            .takeIf { it.isNotEmpty() }
            ?: throw AssertionError("Missing non-empty list argument: $name")

    protected fun optionalListArg(name: String): List<String> =
        optionalArg(name)
            ?.split(',', '\n', '|')
            ?.map(String::trim)
            ?.filter(String::isNotEmpty)
            ?: emptyList()

    protected fun decodeBase64Arg(value: String): String =
        String(Base64.decode(value, Base64.NO_WRAP or Base64.URL_SAFE), Charsets.UTF_8)

    protected fun storageEntries(root: File): List<String> =
        root
            .listFiles()
            ?.sortedBy { it.name }
            ?.map { it.relativeTo(root).path.ifBlank { it.name } }
            ?: emptyList()

    protected fun readJsonObject(fileName: String): JSONObject? {
        val file = File(appFilesDir(), fileName)
        if (!file.exists()) {
            return null
        }
        return runCatching { JSONObject(file.readText()) }.getOrNull()
    }

    protected fun readLiveRuntimeDebugSnapshot(): JSONObject? =
        runCatching {
            runtimeDebugObject(JSONObject(kotlinx.coroutines.runBlocking { appManager().exportSupportBundleJson() }))
        }.getOrNull()

    protected fun runtimeDebugObject(root: JSONObject): JSONObject {
        root.optJSONObject("runtime_debug")?.let { return it }
        root.optJSONObject("runtime")?.let { return it }
        root.optJSONObject("rust")?.let { rust ->
            rust.optJSONObject("runtime_debug")?.let { return it }
            rust.optJSONObject("runtime")?.let { return it }
        }
        return root
    }

    protected fun runtimeSnapshotIsIdle(debug: JSONObject): Boolean {
        if (debug.optJSONObject("ffi_queue")?.optBoolean("core_support_bundle_timed_out") == true) {
            return false
        }
        if ((debug.optJSONArray("pending_relay_publishes")?.length() ?: 0) > 0) {
            return false
        }
        val protocolEngine = debug.optJSONObject("protocol_engine") ?: return true
        return protocolEnginePendingCount(protocolEngine) == 0
    }

    protected fun runtimePendingSummary(debug: JSONObject): String {
        val protocolEngine = debug.optJSONObject("protocol_engine")
        val protocolCount = protocolEngine?.let(::protocolEnginePendingCount) ?: 0
        val protocolSummary = protocolEngine?.let(::summarizeProtocolEnginePending).orEmpty()
        val relayPublishes = debug.optJSONArray("pending_relay_publishes")
        val relaySummary = summarizeRuntimePendingRelayPublishes(relayPublishes)
        val coreSupportBundleTimedOut =
            debug.optJSONObject("ffi_queue")?.optBoolean("core_support_bundle_timed_out") == true
        return listOf(
            "protocol=$protocolCount",
            if (protocolSummary.isEmpty()) "" else "protocolPending=$protocolSummary",
            "relay=${relayPublishes?.length() ?: 0}",
            "ffiTimedOut=$coreSupportBundleTimedOut",
            if (relaySummary.isEmpty()) "" else "pending=$relaySummary",
        ).filter { it.isNotEmpty() }.joinToString(" ")
    }

    protected data class SqliteCoreSnapshot(
        val filePresent: Boolean,
        val appMeta: String = "",
        val appKeys: String = "",
        val groups: String = "",
        val threads: String = "",
        val messages: String = "",
        val pendingRelayPublishes: String = "",
    )

    protected fun readSqliteCoreSnapshot(): SqliteCoreSnapshot {
        val dbFile = File(appFilesDir(), CORE_DB_FILENAME)
        if (!dbFile.exists()) {
            return SqliteCoreSnapshot(filePresent = false)
        }
        return runCatching {
            SQLiteDatabase
                .openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
                .use { db ->
                    SqliteCoreSnapshot(
                        filePresent = true,
                        appMeta =
                            summarizeRows(
                                db,
                                "SELECT key, value FROM app_meta ORDER BY key",
                            ) { cursor ->
                                "${cursor.getString(0)}=${cursor.getString(1)}"
                            },
                        appKeys =
                            summarizeRows(
                                db,
                                """
                                    SELECT owner_pubkey_hex, created_at_secs, devices_json
                                    FROM app_keys
                                    ORDER BY owner_pubkey_hex
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.getLong(1).toString(),
                                    cursor.getString(2).take(160),
                                ).joinToString(",")
                            },
                        groups =
                            summarizeRows(
                                db,
                                """
                                    SELECT group_id, name, updated_at_secs
                                    FROM groups
                                    ORDER BY updated_at_secs DESC, group_id
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.getString(1),
                                    cursor.getLong(2).toString(),
                                ).joinToString(",")
                            },
                        threads =
                            summarizeRows(
                                db,
                                """
                                    SELECT chat_id, unread_count, updated_at_secs
                                    FROM threads
                                    ORDER BY updated_at_secs DESC, chat_id
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.getLong(1).toString(),
                                    cursor.getLong(2).toString(),
                                ).joinToString(",")
                            },
                        messages =
                            summarizeRows(
                                db,
                                """
                                    SELECT chat_id, id, delivery, is_outgoing, body
                                    FROM messages
                                    ORDER BY created_at_secs DESC, id DESC
                                    LIMIT 20
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.getString(1),
                                    cursor.getString(2),
                                    cursor.getLong(3).toString(),
                                    cursor.getString(4).replace('|', '/').take(120),
                                ).joinToString(",")
                            },
                        pendingRelayPublishes =
                            summarizeRows(
                                db,
                                """
                                    SELECT label, chat_id, inner_event_id, attempt_count
                                    FROM pending_relay_publishes
                                    ORDER BY created_at_secs DESC
                                    LIMIT 30
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.stringOrEmpty(1),
                                    cursor.stringOrEmpty(2),
                                    cursor.getLong(3).toString(),
                                ).joinToString(",")
                            },
                    )
                }
        }.getOrElse {
            SqliteCoreSnapshot(
                filePresent = true,
                appMeta = "read_error=${it.message.orEmpty()}",
            )
        }
    }

    protected fun summarizeRows(
        db: SQLiteDatabase,
        sql: String,
        args: Array<String> = emptyArray(),
        row: (android.database.Cursor) -> String,
    ): String =
        db.rawQuery(sql, args).use { cursor ->
            buildList {
                while (cursor.moveToNext()) {
                    add(row(cursor))
                }
            }.joinToString("|")
        }

    protected fun android.database.Cursor.stringOrEmpty(index: Int): String =
        if (isNull(index)) "" else getString(index)

    protected fun pendingRelayPublishCount(label: String? = null): Int {
        val dbFile = File(appFilesDir(), CORE_DB_FILENAME)
        if (!dbFile.exists()) {
            return 0
        }
        return runCatching {
            SQLiteDatabase
                .openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
                .use { db ->
                    val (sql, args) =
                        if (label.isNullOrBlank()) {
                            "SELECT COUNT(*) FROM pending_relay_publishes" to emptyArray<String>()
                        } else {
                            "SELECT COUNT(*) FROM pending_relay_publishes WHERE label = ?" to
                                arrayOf(label)
                        }
                    db.rawQuery(sql, args).use { cursor ->
                        if (cursor.moveToFirst()) cursor.getInt(0) else 0
                    }
                }
        }.getOrDefault(0)
    }

    protected fun readOwnerProfileDisplayName(ownerPubkeyHex: String): String? {
        val dbFile = File(appFilesDir(), CORE_DB_FILENAME)
        if (!dbFile.exists()) {
            return null
        }
        return runCatching {
            SQLiteDatabase
                .openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
                .use { db ->
                    db.rawQuery(
                        """
                            SELECT display_name, name
                            FROM owner_profiles
                            WHERE owner_pubkey_hex = ?
                            LIMIT 1
                        """.trimIndent(),
                        arrayOf(ownerPubkeyHex),
                    ).use { cursor ->
                        if (!cursor.moveToFirst()) {
                            null
                        } else {
                            cursor.getString(0)?.takeIf { it.isNotEmpty() }
                                ?: cursor.getString(1)?.takeIf { it.isNotEmpty() }
                        }
                    }
                }
        }.getOrNull()
    }

    protected fun readLegacyOwnerProfileDisplayName(ownerPubkeyHex: String): String? {
        val profiles = readJsonObject("core/profiles.json") ?: return null
        val entry = profiles.optJSONObject(ownerPubkeyHex) ?: return null
        return entry.optString("display_name").takeIf { it.isNotEmpty() }
            ?: entry.optString("name").takeIf { it.isNotEmpty() }
    }

    protected fun persistedThreadWithMessage(
        persisted: JSONObject,
        chatId: String?,
        expectedMessage: String,
        direction: String,
    ): String? {
        val threads = persisted.optJSONArray("threads") ?: return null
        for (index in 0 until threads.length()) {
            val thread = threads.optJSONObject(index) ?: continue
            val threadChatId = thread.optString("chat_id")
            if (!chatId.isNullOrBlank() && !threadChatId.equals(chatId, ignoreCase = true)) {
                continue
            }
            val messages = thread.optJSONArray("messages") ?: continue
            val found =
                (0 until messages.length()).any { messageIndex ->
                    val message = messages.optJSONObject(messageIndex) ?: return@any false
                    message.optString("body") == expectedMessage &&
                        messageDirectionMatches(message.optBoolean("is_outgoing"), direction)
                }
            if (found) {
                return threadChatId
            }
        }
        return null
    }

    protected fun countMessages(
        chatId: String,
        expectedMessage: String,
        direction: String,
    ): Int {
        val persistedCount =
            readJsonObject(PERSISTED_STATE_FILENAME)?.let { persisted ->
                countPersistedMessages(persisted, chatId, expectedMessage, direction)
            } ?: 0
        val stateCount =
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                ?.messages
                ?.count { message ->
                    message.body == expectedMessage &&
                        messageDirectionMatches(message.isOutgoing, direction)
                } ?: 0
        return maxOf(persistedCount, stateCount)
    }

    protected fun countPersistedMessages(
        persisted: JSONObject,
        chatId: String,
        expectedMessage: String,
        direction: String,
    ): Int {
        val threads = persisted.optJSONArray("threads") ?: return 0
        for (index in 0 until threads.length()) {
            val thread = threads.optJSONObject(index) ?: continue
            val threadChatId = thread.optString("chat_id")
            if (!threadChatId.equals(chatId, ignoreCase = true)) {
                continue
            }
            val messages = thread.optJSONArray("messages") ?: return 0
            return (0 until messages.length()).count { messageIndex ->
                val message = messages.optJSONObject(messageIndex) ?: return@count false
                message.optString("body") == expectedMessage &&
                    messageDirectionMatches(message.optBoolean("is_outgoing"), direction)
            }
        }
        return 0
    }

    protected fun holdNearbyIfRequested() {
        val holdMs = (optionalArg("hold_ms")?.toLongOrNull() ?: 0L).coerceIn(0L, 60_000L)
        if (holdMs <= 0L) return
        reportStatus("nearby_hold_ms" to holdMs.toString())
        SystemClock.sleep(holdMs)
    }

    protected fun sqliteDirectionValue(direction: String): String? =
        when (direction.lowercase()) {
            "incoming" -> "0"
            "outgoing" -> "1"
            else -> null
        }

    protected fun persistedHasPeerRoster(
        persisted: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        persisted
            .optJSONObject("session_manager")
            ?.optJSONArray("users")
            ?.let { users ->
                (0 until users.length()).any { index ->
                    users.optJSONObject(index)?.let { user ->
                        user.optString("owner_pubkey").equals(peerOwnerHex, ignoreCase = true) &&
                            !user.isNull("roster")
                    } == true
                }
            } == true

    protected fun persistedHasPeerSession(
        persisted: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        persisted
            .optJSONObject("session_manager")
            ?.optJSONArray("users")
            ?.let { users ->
                (0 until users.length()).any { index ->
                    val user = users.optJSONObject(index) ?: return@any false
                    if (!user.optString("owner_pubkey").equals(peerOwnerHex, ignoreCase = true)) {
                        return@any false
                    }
                    val devices = user.optJSONArray("devices") ?: return@any false
                    (0 until devices.length()).any { deviceIndex ->
                        val device = devices.optJSONObject(deviceIndex) ?: return@any false
                        !device.isNull("active_session") ||
                            (device.optJSONArray("inactive_sessions")?.length() ?: 0) > 0
                    }
                }
            } == true

    protected fun persistedHasPeerTransportReady(
        persisted: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        persisted
            .optJSONObject("session_manager")
            ?.optJSONArray("users")
            ?.let { users ->
                (0 until users.length()).any { index ->
                    val user = users.optJSONObject(index) ?: return@any false
                    if (!user.optString("owner_pubkey").equals(peerOwnerHex, ignoreCase = true)) {
                        return@any false
                    }
                    val rosterDevices = user.optJSONObject("roster")?.optJSONArray("devices") ?: return@any false
                    val devices = user.optJSONArray("devices") ?: return@any false
                    if (rosterDevices.length() == 0) {
                        return@any false
                    }

                    (0 until rosterDevices.length()).all { rosterIndex ->
                        val rosterDevice = rosterDevices.optJSONObject(rosterIndex) ?: return@all false
                        val rosterDeviceHex = rosterDevice.optString("device_pubkey")
                        (0 until devices.length()).any { deviceIndex ->
                            val device = devices.optJSONObject(deviceIndex) ?: return@any false
                            device.optString("device_pubkey").equals(rosterDeviceHex, ignoreCase = true) &&
                                !device.isNull("public_invite")
                        }
                    }
                }
            } == true

    protected fun runtimeDebugHasPeerRoster(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optBoolean("has_roster") && user.optInt("roster_device_count") > 0
        }

    protected fun runtimeDebugHasPeerSession(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optInt("active_session_device_count") > 0 ||
                user.optInt("inactive_session_count") > 0
        }

    protected fun runtimeDebugHasPeerTransportReady(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optBoolean("has_roster") &&
                user.optInt("roster_device_count") > 0 &&
                user.optInt("device_count") > 0 &&
                user.optInt("authorized_device_count") > 0
        }

    protected fun runtimeDebugKnownPeer(
        debug: JSONObject,
        peerOwnerHex: String,
        predicate: (JSONObject) -> Boolean,
    ): Boolean {
        val users = debug.optJSONArray("known_users") ?: return false
        return (0 until users.length()).any { index ->
            val user = users.optJSONObject(index) ?: return@any false
            user.optString("owner_pubkey_hex").equals(peerOwnerHex, ignoreCase = true) &&
                predicate(user)
        }
    }

    protected fun runtimeDebugAuthorizedDeviceCount(
        debug: JSONObject?,
        ownerHex: String,
    ): Int? {
        if (debug == null || ownerHex.isBlank()) {
            return null
        }
        val users = debug.optJSONArray("known_users") ?: return null
        for (index in 0 until users.length()) {
            val user = users.optJSONObject(index) ?: continue
            if (user.optString("owner_pubkey_hex").equals(ownerHex, ignoreCase = true)) {
                return user.optInt("authorized_device_count")
            }
        }
        return null
    }

    protected fun reportNearbySnapshot(snapshot: IrisNearbyService.Snapshot) {
        val peers = JSONArray()
        snapshot.peers.forEach { peer ->
            peers.put(
                JSONObject()
                    .put("id", peer.id)
                    .put("name", peer.name)
                    .put("owner_pubkey_hex", peer.ownerPubkeyHex ?: "")
                    .put("profile_event_id", peer.profileEventId ?: ""),
            )
        }
        reportStatus(
            "nearby_visible" to snapshot.visible.toString(),
            "nearby_status" to snapshot.status,
            "nearby_lan_visible" to snapshot.localNetworkVisible.toString(),
            "nearby_lan_status" to snapshot.localNetworkStatus,
            "nearby_lan_permission_granted" to snapshot.localNetworkPermissionGranted.toString(),
            "nearby_peer_count" to snapshot.peerCount.toString(),
            "nearby_peers" to peers.toString(),
        )
    }

    protected fun reportStatus(vararg fields: Pair<String, String>) {
        val bundle = Bundle()
        fields.forEach { (key, value) -> bundle.putString(key, value) }
        instrumentation.sendStatus(0, bundle)
    }

    protected fun <T> waitForState(
        label: String,
        timeoutMs: Long = 60_000,
        condition: () -> T?,
    ): T {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            condition()?.let { return it }
            SystemClock.sleep(100)
        }
        throw AssertionError("Timed out waiting for $label")
    }

    protected fun <T> waitForOptionalState(timeoutMs: Long, condition: () -> T?): T? {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            condition()?.let { return it }
            SystemClock.sleep(100)
        }
        return null
    }

    protected fun relayWakeCallback(openChatId: String? = null): () -> Unit {
        var lastRelayWakeAt = 0L
        return {
            val now = SystemClock.elapsedRealtime()
            if (now - lastRelayWakeAt >= HARNESS_RELAY_WAKE_INTERVAL_MS) {
                lastRelayWakeAt = now
                appManager().appForegrounded()
                openChatId?.let(appManager()::openChat)
            }
        }
    }

    companion object {
        const val DEBUG_SNAPSHOT_FILENAME = "iris_chat_runtime_debug.json"
        const val CORE_DB_FILENAME = "core.sqlite3"
        const val PERSISTED_STATE_FILENAME = "iris_chat_core_state.json"
        const val NEARBY_PROFILE_TIMEOUT_MS = 180_000L
        const val HARNESS_RELAY_WAKE_INTERVAL_MS = 3_000L
    }
}
