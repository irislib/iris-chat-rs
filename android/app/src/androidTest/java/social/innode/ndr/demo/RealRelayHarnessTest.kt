package social.innode.ndr.demo

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
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.account.AccountBootstrapState
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.CurrentChatSnapshot
import social.innode.ndr.demo.rust.DeliveryState
import social.innode.ndr.demo.rust.DeviceAuthorizationState
import social.innode.ndr.demo.rust.normalizePeerInput
import social.innode.ndr.demo.rust.peerInputToHex
import java.io.File

@RunWith(AndroidJUnit4::class)
class RealRelayHarnessTest {
    @get:Rule
    val activityRule = ActivityScenarioRule(MainActivity::class.java)

    private val instrumentation
        get() = InstrumentationRegistry.getInstrumentation()

    private val arguments
        get() = InstrumentationRegistry.getArguments()

    private fun appManager(): AppManager =
        (instrumentation.targetContext.applicationContext as IrisChatApp).container.appManager

    private fun appFilesDir(): File = instrumentation.targetContext.filesDir

    private fun appPackageName(): String = instrumentation.targetContext.packageName

    private fun <T> withActivity(block: (MainActivity) -> T): T {
        var result: Result<T>? = null
        activityRule.scenario.onActivity { activity ->
            result = runCatching { block(activity) }
        }
        return result?.getOrThrow() ?: error("Activity was not available")
    }

    @Test
    fun create_account_and_report_identity() {
        val account = ensureLoggedIn()
        reportStatus(
            "npub" to account.npub,
            "public_key_hex" to account.publicKeyHex,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
            "app_package" to appPackageName(),
            "data_dir" to appFilesDir().absolutePath,
        )
    }

    @Test
    fun report_logged_in_identity() {
        val account = ensureLoggedIn()
        reportStatus(
            "npub" to account.npub,
            "public_key_hex" to account.publicKeyHex,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
            "authorization_state" to account.authorizationState.name,
            "app_package" to appPackageName(),
            "data_dir" to appFilesDir().absolutePath,
        )
    }

    @Test
    fun create_public_invite_and_report_url() {
        ensureLoggedIn()
        appManager().dispatch(AppAction.CreatePublicInvite)

        val invite =
            waitForState("public invite", timeoutMs = 90_000) {
                appManager().state.value.publicInvite
            }

        reportStatus(
            "invite_url" to invite.url,
        )
    }

    @Test
    fun accept_invite_and_send_message_from_args() {
        ensureLoggedIn()
        val inviteUrl = requiredArg("invite_url")
        val message = requiredArg("message")
        val expectedChatId = optionalArg("expected_chat_id")

        appManager().dispatch(AppAction.AcceptInvite(inviteUrl))
        val chat =
            waitForState("accepted invite", timeoutMs = 180_000) {
                val state = appManager().state.value
                if (!state.busy.acceptingInvite) {
                    state.toast?.takeIf { it.isNotBlank() }?.let { toast ->
                        fail("Invite accept failed: $toast")
                    }
                }
                state.currentChat?.takeIf { current ->
                    !state.busy.acceptingInvite &&
                        current.chatId.isNotBlank() &&
                        (expectedChatId?.let { expected ->
                            current.chatId.equals(expected, ignoreCase = true)
                        } ?: true)
                }
            }

        appManager().sendText(chat.chatId, message)

        val finalized =
            waitForState("invite chat message publish", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId == chat.chatId }
                    ?.messages
                    ?.find { entry ->
                        entry.isOutgoing &&
                            entry.body == message &&
                            entry.delivery != DeliveryState.QUEUED &&
                            entry.delivery != DeliveryState.PENDING
                    }
            }

        if (finalized.delivery == DeliveryState.FAILED) {
            fail("Invite chat message failed to publish")
        }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "delivery" to finalized.delivery.name,
        )
    }

    @Test
    fun wait_for_mobile_push_author_from_args() {
        ensureLoggedIn()
        val authorInput = requiredArg("author_input")
        val authorHex = normalizePeerInput(authorInput)

        val snapshot =
            waitForState("mobile push author $authorHex", timeoutMs = 90_000) {
                appManager().state.value.mobilePush.takeIf { mobilePush ->
                    mobilePush.messageAuthorPubkeys.any { it.equals(authorHex, ignoreCase = true) }
                }
            }

        reportStatus(
            "owner_pubkey_hex" to snapshot.ownerPubkeyHex.orEmpty(),
            "author_pubkey_hex" to authorHex,
            "message_author_pubkeys" to snapshot.messageAuthorPubkeys.joinToString(","),
            "session_count" to snapshot.sessions.size.toString(),
        )
    }

    @Test
    fun report_mobile_push_snapshot() {
        ensureLoggedIn()
        val snapshot =
            waitForState("mobile push author snapshot", timeoutMs = 90_000) {
                appManager().state.value.mobilePush.takeIf { mobilePush ->
                    mobilePush.messageAuthorPubkeys.isNotEmpty()
                }
            }

        reportStatus(
            "owner_pubkey_hex" to snapshot.ownerPubkeyHex.orEmpty(),
            "message_author_pubkeys" to snapshot.messageAuthorPubkeys.joinToString(","),
            "session_count" to snapshot.sessions.size.toString(),
        )
    }

    @Test
    fun start_linked_device_and_report_identity() {
        val ownerInput = requiredArg("owner_input")
        val account = ensureLinkedDeviceStarted(ownerInput)
        reportStatus(
            "npub" to account.npub,
            "public_key_hex" to account.publicKeyHex,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
            "authorization_state" to account.authorizationState.name,
        )
    }

    @Test
    fun add_authorized_device_from_args() {
        ensureLoggedIn()
        val deviceInput = requiredArg("device_input")
        val initialRev = appManager().state.value.rev

        appManager().addAuthorizedDevice(deviceInput)

        val roster =
            waitForState("authorized device in roster", timeoutMs = 90_000) {
                val state = appManager().state.value
                val roster = state.deviceRoster
                val matched =
                    roster?.devices?.any { device ->
                        deviceMatchesInput(device.devicePubkeyHex, device.deviceNpub, deviceInput) &&
                            device.isAuthorized &&
                            !device.isStale
                    } == true
                if (matched) {
                    return@waitForState roster
                }
                if (state.rev > initialRev && !state.busy.updatingRoster) {
                    val rosterSummary =
                        roster
                            ?.devices
                            ?.joinToString("|") { device ->
                                listOf(
                                    device.devicePubkeyHex,
                                    device.isAuthorized.toString(),
                                    device.isStale.toString(),
                                ).joinToString(",")
                            }
                            ?: "<none>"
                    fail(
                        buildString {
                            append("Device add completed without authorizing $deviceInput.")
                            state.toast?.takeIf { it.isNotBlank() }?.let { toast ->
                                append(" toast=")
                                append(toast)
                            }
                            append(" roster=")
                            append(rosterSummary)
                        },
                    )
                }
                null
            }

        reportStatus(
            "device_pubkey_hex" to normalizePeerInput(deviceInput),
            "device_count" to roster.devices.size.toString(),
        )
    }

    @Test
    fun remove_authorized_device_from_args() {
        ensureLoggedIn()
        val deviceInput = requiredArg("device_input")
        val initialRev = appManager().state.value.rev

        val normalizedDeviceHex = normalizePeerInput(deviceInput)
        appManager().removeAuthorizedDevice(normalizedDeviceHex)

        val roster =
            waitForState("device removal reflected in roster", timeoutMs = 90_000) {
                val state = appManager().state.value
                val roster = state.deviceRoster
                val removed =
                    roster?.devices?.none { device ->
                        deviceMatchesInput(device.devicePubkeyHex, device.deviceNpub, deviceInput) &&
                            device.isAuthorized &&
                            !device.isStale
                    } == true
                if (removed) {
                    return@waitForState roster
                }
                if (state.rev > initialRev && !state.busy.updatingRoster) {
                    val rosterSummary =
                        roster
                            ?.devices
                            ?.joinToString("|") { device ->
                                listOf(
                                    device.devicePubkeyHex,
                                    device.isAuthorized.toString(),
                                    device.isStale.toString(),
                                ).joinToString(",")
                            }
                            ?: "<none>"
                    fail(
                        buildString {
                            append("Device removal completed without removing $deviceInput.")
                            state.toast?.takeIf { it.isNotBlank() }?.let { toast ->
                                append(" toast=")
                                append(toast)
                            }
                            append(" roster=")
                            append(rosterSummary)
                        },
                    )
                }
                null
            }

        val removedEntry =
            roster.devices.firstOrNull { entry ->
                entry.devicePubkeyHex.equals(normalizedDeviceHex, ignoreCase = true)
            }

        reportStatus(
            "device_pubkey_hex" to normalizedDeviceHex,
            "device_removed" to (removedEntry == null).toString(),
            "device_stale" to (removedEntry?.isStale ?: false).toString(),
        )
    }

    @Test
    fun wait_for_authorization_state_from_args() {
        val expectedState = requiredAuthorizationState()
        val account =
            waitForState("authorization state ${expectedState.name}", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { it.authorizationState == expectedState }
            }

        reportStatus(
            "authorization_state" to account.authorizationState.name,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
        )
    }

    @Test
    fun wait_for_revoked_state() {
        val account =
            waitForState("revoked device state", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { it.authorizationState == DeviceAuthorizationState.REVOKED }
            }

        reportStatus(
            "authorization_state" to account.authorizationState.name,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
        )
    }

    @Test
    fun report_device_roster_snapshot() {
        val roster =
            waitForState("device roster snapshot", timeoutMs = 90_000) {
                appManager().state.value.deviceRoster
            }

        reportStatus(
            "owner_npub" to roster.ownerNpub,
            "current_device_npub" to roster.currentDeviceNpub,
            "authorization_state" to roster.authorizationState.name,
            "can_manage_devices" to roster.canManageDevices.toString(),
            "devices" to roster.devices.joinToString("|") { device ->
                listOf(
                    device.devicePubkeyHex,
                    device.deviceNpub,
                    device.isCurrentDevice.toString(),
                    device.isAuthorized.toString(),
                    device.isStale.toString(),
                ).joinToString(",")
            },
        )
    }

    @Test
    fun report_runtime_debug_snapshot() {
        ensureLoggedIn()
        val state = appManager().state.value
        val debug = readJsonObject(DEBUG_SNAPSHOT_FILENAME)
        val plan = debug?.optJSONObject("current_protocol_plan")

        reportStatus(
            "data_dir" to appFilesDir().absolutePath,
            "rev" to state.rev.toString(),
            "default_screen" to state.router.defaultScreen.toString(),
            "screen_stack" to state.router.screenStack.joinToString("|") { screen -> screen.toString() },
            "current_chat" to summarizeCurrentChat(state.currentChat),
            "chat_list" to summarizeChatList(state.chatList),
            "toast" to state.toast.orEmpty(),
            "runtime_file_present" to (debug != null).toString(),
            "generated_at_secs" to debug.optStringOrEmpty("generated_at_secs"),
            "local_owner_pubkey_hex" to debug.optStringOrEmpty("local_owner_pubkey_hex"),
            "local_device_pubkey_hex" to debug.optStringOrEmpty("local_device_pubkey_hex"),
            "authorization_state" to debug.optStringOrEmpty("authorization_state"),
            "tracked_owner_hexes" to debug.optStringArray("tracked_owner_hexes"),
            "plan_roster_authors" to plan.optStringArray("roster_authors"),
            "plan_invite_authors" to plan.optStringArray("invite_authors"),
            "plan_message_authors" to plan.optStringArray("message_authors"),
            "plan_invite_response_recipient" to plan.optStringOrEmpty("invite_response_recipient"),
            "known_users" to summarizeRuntimeKnownUsers(debug?.optJSONArray("known_users")),
            "pending_outbound" to summarizeRuntimePendingOutbound(debug?.optJSONArray("pending_outbound")),
            "pending_group_controls" to summarizeRuntimePendingGroupControls(debug?.optJSONArray("pending_group_controls")),
            "recent_handshake_peers" to summarizeRecentHandshakePeers(debug?.optJSONArray("recent_handshake_peers")),
            "event_counts" to summarizeEventCounts(debug?.optJSONObject("event_counts")),
            "recent_log" to summarizeRecentLog(debug?.optJSONArray("recent_log")),
        )
    }

    @Test
    fun report_persisted_protocol_snapshot() {
        ensureLoggedIn()
        val persisted = readJsonObject(PERSISTED_STATE_FILENAME)
        val sessionManager = persisted?.optJSONObject("session_manager")
        val groupManager = persisted?.optJSONObject("group_manager")

        reportStatus(
            "data_dir" to appFilesDir().absolutePath,
            "persisted_file_present" to (persisted != null).toString(),
            "version" to persisted.optStringOrEmpty("version"),
            "active_chat_id" to persisted.optStringOrEmpty("active_chat_id"),
            "authorization_state" to persisted.optStringOrEmpty("authorization_state"),
            "users" to summarizePersistedUsers(sessionManager?.optJSONArray("users")),
            "groups" to summarizePersistedGroups(groupManager?.optJSONArray("groups")),
            "pending_outbound" to summarizePersistedPendingOutbound(persisted?.optJSONArray("pending_outbound")),
            "pending_group_controls" to summarizePersistedPendingGroupControls(persisted?.optJSONArray("pending_group_controls")),
            "seen_event_ids_count" to (persisted?.optJSONArray("seen_event_ids")?.length() ?: 0).toString(),
            "threads" to summarizePersistedThreads(persisted?.optJSONArray("threads")),
        )
    }

    @Test
    fun wait_for_peer_roster_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val peerOwnerHex = resolvePeerOwnerHex(peerInput)

        val persisted =
            waitForState("peer roster for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerRoster(json, peerOwnerHex) }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "users" to summarizePersistedUsers(persisted.optJSONObject("session_manager")?.optJSONArray("users")),
        )
    }

    @Test
    fun wait_for_known_peer_session_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val peerOwnerHex = resolvePeerOwnerHex(peerInput)

        val persisted =
            waitForState("known peer session for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerSession(json, peerOwnerHex) }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "users" to summarizePersistedUsers(persisted.optJSONObject("session_manager")?.optJSONArray("users")),
        )
    }

    @Test
    fun wait_for_peer_transport_ready_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val peerOwnerHex = resolvePeerOwnerHex(peerInput)

        val persisted =
            waitForState("peer transport ready for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerTransportReady(json, peerOwnerHex) }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "users" to summarizePersistedUsers(persisted.optJSONObject("session_manager")?.optJSONArray("users")),
        )
    }

    @Test
    fun create_chat_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val chat = ensureChatOpen(peerInput)
        reportStatus(
            "chat_id" to chat.chatId,
            "peer_npub" to chat.subtitle.orEmpty(),
        )
    }

    @Test
    fun create_group_from_args() {
        ensureLoggedIn()
        val groupName = requiredArg("group_name")
        val memberInputs = requiredListArg("member_inputs")

        appManager().createGroup(groupName, memberInputs)

        val chat =
            waitForState("created group chat", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current ->
                        current.groupId != null &&
                            current.displayName == groupName
                    }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "group_id" to chat.groupId.orEmpty(),
            "group_name" to chat.displayName,
            "member_count" to chat.memberCount.toString(),
        )
    }

    @Test
    fun wait_for_group_chat_from_args() {
        ensureLoggedIn()
        val chatId = requiredArg("chat_id")

        val existing =
            waitForState("group thread in chat list", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .chatList
                    .firstOrNull { thread -> thread.chatId == chatId }
            }

        appManager().openChat(existing.chatId)
        val current =
            waitForState("opened group chat", timeoutMs = 30_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { chat -> chat.chatId == chatId }
            }

        reportStatus(
            "chat_id" to current.chatId,
            "group_id" to current.groupId.orEmpty(),
            "group_name" to current.displayName,
            "member_count" to current.memberCount.toString(),
        )
    }

    @Test
    fun wait_for_group_member_count_from_args() {
        ensureLoggedIn()
        val chatId = optionalArg("chat_id")
        val groupId = optionalArg("group_id")
        val expectedMemberCount = requiredArg("member_count").toULong()
        val resolvedChatId =
            when {
                !chatId.isNullOrBlank() -> chatId
                !groupId.isNullOrBlank() -> "group:$groupId"
                else -> throw AssertionError("Missing instrumentation argument: chat_id or group_id")
            }

        ensureChatOpenById(resolvedChatId)
        val current =
            waitForState("group member count $expectedMemberCount", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { chat ->
                        chat.chatId == resolvedChatId &&
                            chat.memberCount == expectedMemberCount
                    }
            }

        reportStatus(
            "chat_id" to current.chatId,
            "group_id" to current.groupId.orEmpty(),
            "member_count" to current.memberCount.toString(),
        )
    }

    @Test
    fun remove_group_member_from_args() {
        ensureLoggedIn()
        val chatId = optionalArg("chat_id")
        val groupIdArg = optionalArg("group_id")
        val memberInput = requiredArg("member_input")
        val expectedMemberCount = optionalArg("expected_member_count")?.toULong()
        val resolvedChatId =
            when {
                !chatId.isNullOrBlank() -> chatId
                !groupIdArg.isNullOrBlank() -> "group:$groupIdArg"
                else -> throw AssertionError("Missing instrumentation argument: chat_id or group_id")
            }
        val groupId = groupIdArg ?: resolvedChatId.removePrefix("group:")

        val existing = ensureChatOpenById(resolvedChatId)
        val initialRev = appManager().state.value.rev
        val initialMemberCount = existing.memberCount

        appManager().removeGroupMember(groupId, normalizePeerInput(memberInput))

        val current =
            waitForState("removed group member from $resolvedChatId", timeoutMs = 180_000) {
                val state = appManager().state.value
                val chat =
                    state.currentChat
                        ?.takeIf { current -> current.chatId == resolvedChatId }
                        ?: return@waitForState null

                expectedMemberCount?.let { expected ->
                    return@waitForState chat.takeIf { current -> current.memberCount == expected }
                }

                chat.takeIf { current ->
                    state.rev > initialRev &&
                        !state.busy.updatingGroup &&
                        current.memberCount < initialMemberCount
                }
            }

        appManager().state.value.toast?.takeIf { it.isNotBlank() }?.let { toast ->
            fail("Unexpected toast after remove member: $toast")
        }

        reportStatus(
            "chat_id" to current.chatId,
            "group_id" to current.groupId.orEmpty(),
            "member_count" to current.memberCount.toString(),
        )
    }

    @Test
    fun send_message_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val message = requiredArg("message")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        appManager().sendText(chat.chatId, message)

        waitForState("outgoing message") {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { current ->
                    current.chatId == chat.chatId &&
                        current.messages.any { entry ->
                            entry.isOutgoing && entry.body == message
                        }
                }
        }

        val finalized =
            waitForState("message publish", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId == chat.chatId }
                    ?.messages
                    ?.find { entry ->
                        entry.isOutgoing &&
                            entry.body == message &&
                            entry.delivery != DeliveryState.QUEUED &&
                            entry.delivery != DeliveryState.PENDING
                    }
            }

        if (finalized.delivery == DeliveryState.FAILED) {
            fail("Outgoing message failed to publish")
        }

        appManager().state.value.toast?.takeIf { it.isNotBlank() }?.let { toast ->
            fail("Unexpected toast after send: $toast")
        }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "delivery" to finalized.delivery.name,
        )
    }

    @Test
    fun expect_send_rejected_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val message = requiredArg("message")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val initialMessageCount = chat.messages.size
        appManager().sendText(chat.chatId, message)

        val rejectionToast =
            waitForState("rejected send", timeoutMs = 60_000) {
                val state = appManager().state.value
                val current =
                    state.currentChat
                        ?.takeIf { current -> current.chatId == chat.chatId }
                        ?: return@waitForState null
                if (current.messages.size != initialMessageCount || current.messages.any { it.body == message }) {
                    fail("Rejected send unexpectedly appended a message")
                }
                state.toast?.takeIf { it.isNotBlank() }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "toast" to rejectionToast,
        )
    }

    @Test
    fun wait_for_message_from_args() {
        ensureLoggedIn()
        val expectedMessage = requiredArg("message")
        val peerInput = arguments.getString("peer_input").orEmpty()
        val expectedChatId = arguments.getString("chat_id").orEmpty().takeIf { it.isNotBlank() }
        val direction = arguments.getString("direction").orEmpty().lowercase()
        val expectedCount = optionalArg("expected_count")?.toIntOrNull()
        val seededChat =
            when {
                !expectedChatId.isNullOrBlank() -> ensureChatOpenById(expectedChatId)
                peerInput.isNotBlank() -> ensureChatOpen(peerInput)
                else -> null
            }
        val resolvedChatId = expectedChatId ?: seededChat?.chatId

        val matchedChatId =
            waitForState("incoming message", timeoutMs = 180_000) {
                fun matchesResolvedChat(chatId: String): Boolean =
                    resolvedChatId?.let { expected -> chatId.equals(expected, ignoreCase = true) }
                        ?: chatMatchesExpectedChat(chatId, peerInput, expectedChatId)

                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.let { persisted ->
                        persistedThreadWithMessage(
                            persisted = persisted,
                            chatId = resolvedChatId,
                            expectedMessage = expectedMessage,
                            direction = direction,
                        )
                    }
                    ?.let { return@waitForState it }

            val state = appManager().state.value
                state.currentChat?.takeIf { chat ->
                    matchesResolvedChat(chat.chatId) &&
                        chat.messages.any { entry ->
                            entry.body == expectedMessage &&
                                messageDirectionMatches(entry.isOutgoing, direction)
                        }
                }?.chatId
                    ?: state.chatList.firstOrNull { thread ->
                        thread.lastMessagePreview == expectedMessage &&
                            matchesResolvedChat(thread.chatId)
                    }?.also { thread ->
                        appManager().openChat(thread.chatId)
                    }?.chatId
            }

        resolvedChatId?.let(appManager()::openChat)
        val finalChatId = resolvedChatId ?: matchedChatId
        if (expectedCount != null) {
            SystemClock.sleep(5_000)
            val actualCount = countMessages(finalChatId, expectedMessage, direction)
            if (actualCount != expectedCount) {
                fail("Expected $expectedCount matching message(s), found $actualCount for `$expectedMessage` in $finalChatId")
            }
        }

        reportStatus(
            "chat_id" to finalChatId,
            "message" to expectedMessage,
            "matching_count" to countMessages(finalChatId, expectedMessage, direction).toString(),
        )
    }

    /**
     * Set the local owner's `name` (kind:0 metadata) so peers can
     * resolve the sender's display name in their notification title.
     */
    @Test
    fun update_profile_metadata_from_args() {
        ensureLoggedIn()
        val displayName = requiredArg("display_name")
        appManager().updateProfileMetadata(name = displayName, pictureUrl = null)
        val updated =
            waitForState("profile metadata applied", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { account -> account.displayName == displayName }
            }
        reportStatus(
            "display_name" to updated.displayName,
            "public_key_hex" to updated.publicKeyHex,
        )
    }

    /**
     * Wait until the local owner's `core/profiles.json` carries an
     * entry for `peer_pubkey_hex` whose name matches `display_name`.
     * Used by the notification-decrypt smoke to confirm that Bob has
     * received and persisted Alice's kind:0 before we feed her
     * encrypted DM through the decryption path.
     */
    @Test
    fun wait_for_peer_profile_name_from_args() {
        ensureLoggedIn()
        val peerPubkeyHex = requiredArg("peer_pubkey_hex").lowercase()
        val expected = requiredArg("display_name")
        val timeoutMs = optionalArg("timeout_ms")?.toLong() ?: 60_000

        val resolved =
            waitForState("peer profile $peerPubkeyHex == $expected", timeoutMs = timeoutMs) {
                val profiles = readJsonObject("core/profiles.json") ?: return@waitForState null
                val entry = profiles.optJSONObject(peerPubkeyHex) ?: return@waitForState null
                val candidate = entry.optString("display_name").takeIf { it.isNotEmpty() }
                    ?: entry.optString("name").takeIf { it.isNotEmpty() }
                candidate?.takeIf { it == expected }
            }

        reportStatus(
            "peer_pubkey_hex" to peerPubkeyHex,
            "display_name" to resolved,
        )
    }

    /**
     * Verifies that the FCM/APNs notification-decryption path turns an
     * encrypted Nostr event into a notification with the sender's
     * display name as title and the plaintext message as body — what
     * `IrisFirebaseMessagingService` shows the user.
     *
     * Driven from a smoke script that has already established a real
     * DR session between this device and a peer, then waited for the
     * peer's outgoing kind:1060 wrapper to land in this device's
     * persisted `seen_event_ids` (so we know the relay actually
     * delivered it). The script reads the event JSON out of the
     * persisted state and passes it back as `outer_event_json` here.
     *
     * Expected args:
     *   - outer_event_json: serialized Nostr event the notification
     *     server would forward in `payload['event']`
     *   - expected_body: plaintext the rumor carries
     *   - expected_title: sender's display name (or "<sender> in <group>")
     */
    @Test
    fun decrypt_notification_payload_from_args() {
        ensureLoggedIn()
        val outerEventJson = requiredArg("outer_event_json")
        val expectedBody = requiredArg("expected_body")
        val expectedTitle = requiredArg("expected_title")

        val payload = JSONObject().apply {
            put("event", outerEventJson)
            put("sender_name", "Iris Chat")
            put("title", "New message")
            put("body", "New activity")
        }.toString()

        val resolution =
            kotlinx.coroutines.runBlocking {
                appManager().decryptOrResolveNotificationPayload(payload)
            }

        if (!resolution.shouldShow) {
            fail(
                "Decrypted notification was suppressed (should_show=false). " +
                    "Resolution payload=${resolution.payloadJson}",
            )
        }
        if (resolution.body != expectedBody) {
            fail(
                "Notification body did not match decrypted plaintext. " +
                    "expected=`$expectedBody` got=`${resolution.body}` " +
                    "title=`${resolution.title}` payload=${resolution.payloadJson}",
            )
        }
        if (resolution.title != expectedTitle) {
            fail(
                "Notification title did not match expected sender label. " +
                    "expected=`$expectedTitle` got=`${resolution.title}` " +
                    "body=`${resolution.body}` payload=${resolution.payloadJson}",
            )
        }

        reportStatus(
            "title" to resolution.title,
            "body" to resolution.body,
            "payload" to resolution.payloadJson,
        )
    }

/**
     * Strict variant of [wait_for_message_from_args]. The other helper falls
     * back to opening the matching chat from the chat list when the message
     * doesn't surface in `state.currentChat` — that hides exactly the
     * "messages only appear after navigating away and back" bug, because
     * `OpenChat` triggers `fetch_recent_protocol_state` which forces a fresh
     * relay catch-up.
     *
     * This test sets up the chat (peer_input or chat_id) and then waits
     * strictly for the body to land in `state.currentChat.messages`. No
     * fallback to `chatList`, no `openChat` after the initial setup. If the
     * message never lands, the test fails — with a snapshot of how things
     * looked elsewhere in state to make it obvious that the message *did*
     * arrive on the wire but the open-chat projection didn't get the update.
     */
    @Test
    fun wait_for_incoming_message_in_open_chat_strict_from_args() {
        ensureLoggedIn()
        val expectedMessage = requiredArg("message")
        // Args go through run_harness.py which base64-encodes everything
        // under `<name>_b64`, so use the helpers (`optionalArg` /
        // `requiredArg`) — `arguments.getString("peer_input")` is always
        // null here. This was breaking the strict variant on first run
        // even though the smoke was passing the args correctly.
        val peerInput = optionalArg("peer_input").orEmpty()
        val expectedChatId = optionalArg("chat_id")?.takeIf { it.isNotBlank() }
        val timeoutMs = optionalArg("timeout_ms")?.toLong() ?: 180_000L

        val seededChat =
            when {
                !expectedChatId.isNullOrBlank() -> ensureChatOpenById(expectedChatId)
                peerInput.isNotBlank() -> ensureChatOpen(peerInput)
                else -> error("wait_for_incoming_message_in_open_chat_strict_from_args needs peer_input or chat_id")
            }
        val resolvedChatId =
            expectedChatId
                ?: seededChat?.chatId
                ?: error("Could not resolve chat id for strict wait")

        // Make sure the open chat is actually the one we're going to assert
        // against. ensureChatOpen() already calls OpenChat once at setup; we
        // do NOT call OpenChat again from here — that's the whole point.
        val initialState = appManager().state.value
        val initialOpen = initialState.currentChat
        if (initialOpen == null || !initialOpen.chatId.equals(resolvedChatId, ignoreCase = true)) {
            fail(
                "Expected currentChat to be `$resolvedChatId` before the message arrives, " +
                    "got `${initialOpen?.chatId}`. ensureChatOpen failed.",
            )
        }

        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        var landedInCurrentChat = false
        var landedInChatListOnly = false
        var lastObserved: CurrentChatSnapshot? = initialOpen
        while (SystemClock.elapsedRealtime() < deadline) {
            val state = appManager().state.value
            val openChat =
                state.currentChat?.takeIf { it.chatId.equals(resolvedChatId, ignoreCase = true) }
            lastObserved = openChat
            val inOpenChat =
                openChat?.messages?.any { entry ->
                    !entry.isOutgoing && entry.body == expectedMessage
                } == true
            if (inOpenChat) {
                landedInCurrentChat = true
                break
            }
            // Track whether the message reached the chat list / threads but
            // *not* the open-chat projection. If we observe that without
            // the open chat updating, that's the regression we're chasing.
            val inChatListOnly =
                state.chatList.any { thread ->
                    thread.chatId.equals(resolvedChatId, ignoreCase = true) &&
                        thread.lastMessagePreview == expectedMessage
                }
            if (inChatListOnly && !landedInChatListOnly) {
                landedInChatListOnly = true
            }
            SystemClock.sleep(100)
        }

        if (!landedInCurrentChat) {
            val openMessages =
                lastObserved
                    ?.messages
                    ?.takeLast(5)
                    ?.joinToString(", ") { msg ->
                        "${if (msg.isOutgoing) "out" else "in"}:${msg.body}"
                    }
                    .orEmpty()
            val matchingThread =
                appManager().state.value.chatList.firstOrNull { thread ->
                    thread.chatId.equals(resolvedChatId, ignoreCase = true)
                }
            val msg =
                buildString {
                    append("Expected `")
                    append(expectedMessage)
                    append("` to appear in state.currentChat.messages while chat ")
                    append(resolvedChatId)
                    append(" was open, but it never did within ")
                    append(timeoutMs)
                    append("ms.")
                    if (landedInChatListOnly) {
                        append(
                            " The message DID appear in chatList.lastMessagePreview, " +
                                "so it landed in `threads` but the current_chat projection " +
                                "stayed stale — this is the rerender regression.",
                        )
                    }
                    append(" Latest open-chat tail: [")
                    append(openMessages)
                    append("]. ChatList preview for chat: ")
                    append(matchingThread?.lastMessagePreview)
                }
            fail(msg)
        }

        reportStatus(
            "chat_id" to resolvedChatId,
            "message" to expectedMessage,
            "landed_in_current_chat" to landedInCurrentChat.toString(),
            "landed_in_chat_list_only" to landedInChatListOnly.toString(),
        )
    }

    @Test
    fun assert_message_absent_from_args() {
        ensureLoggedIn()
        val expectedMessage = requiredArg("message")
        val peerInput = arguments.getString("peer_input").orEmpty()
        val expectedChatId = arguments.getString("chat_id").orEmpty().takeIf { it.isNotBlank() }
        val direction = arguments.getString("direction").orEmpty().lowercase()
        val timeoutMs = optionalArg("timeout_ms")?.toLong() ?: 30_000

        if (peerInput.isNotBlank()) {
            ensureChatOpen(peerInput)
        } else if (!expectedChatId.isNullOrBlank()) {
            ensureChatOpenById(expectedChatId)
        }

        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            val state = appManager().state.value
            val foundInCurrent =
                state.currentChat?.let { chat ->
                    chatMatchesExpectedChat(chat.chatId, peerInput, expectedChatId) &&
                        chat.messages.any { entry ->
                            entry.body == expectedMessage &&
                                messageDirectionMatches(entry.isOutgoing, direction)
                        }
                } == true
            if (foundInCurrent) {
                fail("Unexpected message `$expectedMessage` appeared in current chat")
            }

            val foundInList =
                state.chatList.any { thread ->
                    chatMatchesExpectedChat(thread.chatId, peerInput, expectedChatId) &&
                        thread.lastMessagePreview == expectedMessage
                }
            if (foundInList) {
                fail("Unexpected message `$expectedMessage` appeared in chat list")
            }

            SystemClock.sleep(100)
        }

        reportStatus(
            "chat_id" to expectedChatId.orEmpty(),
            "message" to expectedMessage,
            "timeout_ms" to timeoutMs.toString(),
        )
    }

    @Test
    fun logout_and_create_account_and_report_identity() {
        val oldAccount = ensureLoggedIn()
        appManager().logout()

        waitForState("logged out state", timeoutMs = 60_000) {
            appManager().state.value.takeIf { it.account == null }
        }

        val filesEntries = storageEntries(appFilesDir())
        if (filesEntries.isNotEmpty()) {
            fail("Expected filesDir to be empty after logout, found: $filesEntries")
        }

        appManager().createAccount()

        val newAccount = waitForState("new account") { appManager().state.value.account }
        if (newAccount.publicKeyHex.equals(oldAccount.publicKeyHex, ignoreCase = true)) {
            fail("Expected a fresh identity after logout")
        }

        reportStatus(
            "old_public_key_hex" to oldAccount.publicKeyHex,
            "new_public_key_hex" to newAccount.publicKeyHex,
            "new_npub" to newAccount.npub,
        )
    }

    private fun ensureLoggedIn(): social.innode.ndr.demo.rust.AccountSnapshot {
        var createRequested = false
        return waitForState("logged in account", timeoutMs = 90_000) {
            val manager = appManager()
            manager.state.value.account?.let { return@waitForState it }

            when (manager.bootstrapState.value) {
                AccountBootstrapState.Loading -> null
                AccountBootstrapState.NeedsLogin -> {
                    if (!createRequested) {
                        createRequested = true
                        manager.createAccount()
                    }
                    null
                }
                is AccountBootstrapState.LoggedIn -> null
            }
        }
    }

    private fun ensureLinkedDeviceStarted(ownerInput: String): social.innode.ndr.demo.rust.AccountSnapshot {
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

    private fun ensureChatOpen(peerInput: String): CurrentChatSnapshot {
        val existing =
            appManager().state.value.chatList.firstOrNull { thread ->
                matchesPeerInput(
                    chatId = thread.chatId,
                    peerNpub = thread.subtitle.orEmpty(),
                    peerInput = peerInput,
                )
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

    private fun ensureChatOpenById(chatId: String): CurrentChatSnapshot {
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

    private fun resolvePeerOwnerHex(peerInput: String): String =
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

    private fun matchesPeerInput(
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

    private fun deviceMatchesInput(
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

    private fun chatMatchesExpectedChat(
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

    private fun messageDirectionMatches(
        isOutgoing: Boolean,
        direction: String,
    ): Boolean =
        when (direction) {
            "", "incoming" -> !isOutgoing
            "outgoing" -> isOutgoing
            "any" -> true
            else -> !isOutgoing
        }

    private fun requiredAuthorizationState(): DeviceAuthorizationState =
        when (requiredArg("authorization_state").trim().uppercase()) {
            "AUTHORIZED" -> DeviceAuthorizationState.AUTHORIZED
            "AWAITING_APPROVAL" -> DeviceAuthorizationState.AWAITING_APPROVAL
            "REVOKED" -> DeviceAuthorizationState.REVOKED
            else -> throw AssertionError("Unsupported authorization_state argument")
        }

    private fun optionalArg(name: String): String? =
        arguments.getString("${name}_b64")
            ?.takeIf { it.isNotBlank() }
            ?.let(::decodeBase64Arg)
            ?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?: arguments.getString(name)?.trim()?.takeIf { it.isNotEmpty() }

    private fun requiredArg(name: String): String =
        optionalArg(name) ?: throw AssertionError("Missing instrumentation argument: $name")

    private fun requiredListArg(name: String): List<String> =
        requiredArg(name)
            .split(',', '\n', '|')
            .map(String::trim)
            .filter(String::isNotEmpty)
            .takeIf { it.isNotEmpty() }
            ?: throw AssertionError("Missing non-empty list argument: $name")

    private fun decodeBase64Arg(value: String): String =
        String(Base64.decode(value, Base64.NO_WRAP or Base64.URL_SAFE), Charsets.UTF_8)

    private fun storageEntries(root: File): List<String> =
        root
            .listFiles()
            ?.sortedBy { it.name }
            ?.map { it.relativeTo(root).path.ifBlank { it.name } }
            ?: emptyList()

    private fun readJsonObject(fileName: String): JSONObject? {
        val file = File(appFilesDir(), fileName)
        if (!file.exists()) {
            return null
        }
        return runCatching { JSONObject(file.readText()) }.getOrNull()
    }

    private fun persistedThreadWithMessage(
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

    private fun countMessages(
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

    private fun countPersistedMessages(
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

    private fun persistedHasPeerRoster(
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

    private fun persistedHasPeerSession(
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

    private fun persistedHasPeerTransportReady(
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

    private fun summarizeCurrentChat(chat: CurrentChatSnapshot?): String =
        chat?.let {
            listOf(
                it.chatId,
                it.displayName,
                it.groupId.orEmpty(),
                it.memberCount.toString(),
                it.messages.size.toString(),
            ).joinToString(",")
        }.orEmpty()

    private fun summarizeChatList(threads: List<social.innode.ndr.demo.rust.ChatThreadSnapshot>): String =
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

    private fun summarizeRuntimeKnownUsers(users: JSONArray?): String =
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

    private fun summarizeRuntimePendingOutbound(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("message_id"),
                entry.optString("chat_id"),
                entry.optString("reason"),
                entry.optString("publish_mode"),
                "inFlight=${entry.optBoolean("in_flight")}",
            ).joinToString(",")
        }

    private fun summarizeRuntimePendingGroupControls(entries: JSONArray?): String =
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

    private fun summarizeRecentHandshakePeers(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("owner_hex"),
                entry.optString("device_hex"),
                entry.optString("observed_at_secs"),
            ).joinToString(",")
        }

    private fun summarizeEventCounts(eventCounts: JSONObject?): String =
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

    private fun summarizeRecentLog(entries: JSONArray?): String =
        entries.joinObjects(limit = 20) { entry ->
            listOf(
                entry.optString("timestamp_secs"),
                entry.optString("category"),
                entry.optString("detail"),
            ).joinToString(",")
        }

    private fun summarizePersistedUsers(users: JSONArray?): String =
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

    private fun summarizePersistedGroups(groups: JSONArray?): String =
        groups.joinObjects { group ->
            listOf(
                group.optString("group_id"),
                group.optString("name"),
                "revision=${group.optLong("revision")}",
                "members=${group.optJSONArray("members")?.length() ?: 0}",
                "admins=${group.optJSONArray("admins")?.length() ?: 0}",
            ).joinToString(",")
        }

    private fun summarizePersistedPendingOutbound(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("message_id"),
                entry.optString("chat_id"),
                entry.optString("reason"),
                entry.optString("publish_mode"),
                "inFlight=${entry.optBoolean("in_flight")}",
            ).joinToString(",")
        }

    private fun summarizePersistedPendingGroupControls(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("operation_id"),
                entry.optString("group_id"),
                entry.optString("reason"),
                entry.opt("kind")?.toString().orEmpty(),
                "inFlight=${entry.optBoolean("in_flight")}",
            ).joinToString(",")
        }

    private fun summarizePersistedThreads(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("chat_id"),
                "messages=${entry.optJSONArray("messages")?.length() ?: 0}",
                "unread=${entry.optLong("unread_count")}",
            ).joinToString(",")
        }

    private fun JSONObject?.optStringOrEmpty(key: String): String =
        if (this == null || !has(key) || isNull(key)) {
            ""
        } else {
            opt(key)?.toString().orEmpty()
        }

    private fun JSONObject?.optStringArray(key: String): String =
        this?.optJSONArray(key).joinValues().orEmpty()

    private fun JSONArray?.joinObjects(
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

    private fun JSONArray?.joinValues(limit: Int = Int.MAX_VALUE): String {
        if (this == null) {
            return ""
        }
        val values = mutableListOf<String>()
        for (index in 0 until minOf(length(), limit)) {
            values += opt(index)?.toString().orEmpty()
        }
        return values.joinToString("|")
    }

    private fun JSONArray?.countObjects(predicate: (JSONObject) -> Boolean): Int {
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

    private fun JSONArray?.sumObjects(transform: (JSONObject) -> Int): Int {
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

    private fun reportStatus(vararg fields: Pair<String, String>) {
        val bundle = Bundle()
        fields.forEach { (key, value) -> bundle.putString(key, value) }
        instrumentation.sendStatus(0, bundle)
    }

    private fun <T> waitForState(
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

    private companion object {
        const val DEBUG_SNAPSHOT_FILENAME = "ndr_demo_runtime_debug.json"
        const val PERSISTED_STATE_FILENAME = "ndr_demo_core_state.json"
    }
}
