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

    private fun nearbyService(): IrisNearbyService =
        (instrumentation.targetContext.applicationContext as IrisChatApp).container.nearbyIrisService

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
        waitForRelayDrainIfRequested()
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
    fun enable_nearby_and_report_peers() {
        ensureLoggedIn()
        withActivity {
            nearbyService().setVisible(true)
        }
        SystemClock.sleep(1_000)
        reportNearbySnapshot(nearbyService().snapshot)
    }

    @Test
    fun enable_lan_nearby_and_report_peers() {
        ensureLoggedIn()
        withActivity {
            nearbyService().setVisible(false)
            nearbyService().setLocalNetworkVisible(true)
        }
        SystemClock.sleep(1_000)
        reportNearbySnapshot(nearbyService().snapshot)
    }

    @Test
    fun wait_for_nearby_peer_profile_from_args() {
        ensureLoggedIn()
        val peerOwnerHex = peerInputToHex(requiredArg("peer_input")).ifBlank {
            normalizePeerInput(requiredArg("peer_input"))
        }
        withActivity {
            nearbyService().setVisible(true)
        }
        val timeoutMs =
            (optionalArg("timeout_ms")?.toLongOrNull() ?: NEARBY_PROFILE_TIMEOUT_MS)
                .coerceIn(1_000, NEARBY_PROFILE_TIMEOUT_MS)
        val peer =
            waitForState("nearby peer profile $peerOwnerHex", timeoutMs = timeoutMs) {
                nearbyService()
                    .snapshot
                    .peers
                    .firstOrNull { nearby ->
                        nearby.ownerPubkeyHex?.equals(peerOwnerHex, ignoreCase = true) == true
                    }
            }
        reportStatus(
            "nearby_visible" to nearbyService().snapshot.visible.toString(),
            "nearby_status" to nearbyService().snapshot.status,
            "nearby_peer_count" to nearbyService().snapshot.peerCount.toString(),
            "nearby_peer_id" to peer.id,
            "nearby_peer_name" to peer.name,
            "nearby_peer_owner_hex" to (peer.ownerPubkeyHex ?: ""),
            "nearby_peer_profile_event_id" to (peer.profileEventId ?: ""),
        )
        holdNearbyIfRequested()
    }

    @Test
    fun wait_for_lan_nearby_peer_profile_from_args() {
        ensureLoggedIn()
        val peerOwnerHex = peerInputToHex(requiredArg("peer_input")).ifBlank {
            normalizePeerInput(requiredArg("peer_input"))
        }
        withActivity {
            nearbyService().setVisible(false)
            nearbyService().setLocalNetworkVisible(true)
        }
        val timeoutMs =
            (optionalArg("timeout_ms")?.toLongOrNull() ?: NEARBY_PROFILE_TIMEOUT_MS)
                .coerceIn(1_000, NEARBY_PROFILE_TIMEOUT_MS)
        val peer =
            waitForState("LAN nearby peer profile $peerOwnerHex", timeoutMs = timeoutMs) {
                nearbyService()
                    .snapshot
                    .peers
                    .firstOrNull { nearby ->
                        nearby.ownerPubkeyHex?.equals(peerOwnerHex, ignoreCase = true) == true
                    }
            }
        val snapshot = nearbyService().snapshot
        reportStatus(
            "nearby_visible" to snapshot.visible.toString(),
            "nearby_status" to snapshot.status,
            "nearby_lan_visible" to snapshot.localNetworkVisible.toString(),
            "nearby_lan_status" to snapshot.localNetworkStatus,
            "nearby_lan_permission_granted" to snapshot.localNetworkPermissionGranted.toString(),
            "nearby_peer_count" to snapshot.peerCount.toString(),
            "nearby_peer_id" to peer.id,
            "nearby_peer_name" to peer.name,
            "nearby_peer_owner_hex" to (peer.ownerPubkeyHex ?: ""),
            "nearby_peer_profile_event_id" to (peer.profileEventId ?: ""),
        )
        holdNearbyIfRequested()
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
            "outer_event_ids" to finalized.deliveryTrace.outerEventIds.joinToString(","),
            "target_device_ids" to finalized.deliveryTrace.targetDeviceIds.joinToString(","),
            "recipient_deliveries" to finalized.recipientDeliveries.joinToString("|") { recipient ->
                "${recipient.ownerPubkeyHex},${recipient.delivery.name}"
            },
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
        requireHarnessInvocation("mobile push snapshot is driven by targeted harness scripts")
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
    fun start_link_invite_and_wait_for_authorization_from_args() {
        val ownerInput = optionalArg("owner_input").orEmpty()
        val expectedState = requiredAuthorizationState()
        var linkRequested = false

        val linkDevice =
            waitForState("link device invite", timeoutMs = 90_000) {
                val manager = appManager()
                manager.state.value.linkDevice?.let { return@waitForState it }

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

        reportStatus(
            "invite_url" to linkDevice.url,
            "device_input" to linkDevice.deviceInput,
        )

        val account =
            waitForState("authorization state ${expectedState.name}", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { it.authorizationState == expectedState }
            }

        waitForRelayDrainIfRequested()
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
        val initialAuthorizedDeviceCount =
            appManager()
                .state
                .value
                .deviceRoster
                ?.devices
                ?.count { device -> device.isAuthorized && !device.isStale }
                ?: 0

        appManager().addAuthorizedDevice(deviceInput)

        val deviceCount =
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
                    return@waitForState roster?.devices?.size ?: 0
                }

                val debug = readJsonObject(DEBUG_SNAPSHOT_FILENAME)
                val localOwner = debug.optStringOrEmpty("local_owner_pubkey_hex")
                val runtimeAuthorizedCount =
                    runtimeDebugAuthorizedDeviceCount(debug, localOwner)
                if (runtimeAuthorizedCount != null &&
                    runtimeAuthorizedCount > initialAuthorizedDeviceCount
                ) {
                    return@waitForState runtimeAuthorizedCount
                }
                null
            }

        waitForRelayDrainIfRequested()
        reportStatus(
            "device_pubkey_hex" to normalizePeerInput(deviceInput),
            "device_count" to deviceCount.toString(),
        )
    }

    @Test
    fun accept_link_invite_from_args() {
        ensureLoggedIn()
        val inviteUrl = requiredArg("invite_url")
        val initialDeviceCount = appManager().state.value.deviceRoster?.devices?.size ?: 0

        appManager().addAuthorizedDevice(inviteUrl)

        val roster =
            waitForState("accepted link invite", timeoutMs = 90_000) {
                val state = appManager().state.value
                state.toast?.takeIf { it.isNotBlank() }?.let { toast ->
                    fail("Link invite accept failed: $toast")
                }
                state.deviceRoster?.takeIf { roster ->
                    !state.busy.updatingRoster && roster.devices.size > initialDeviceCount
                }
            }

        reportStatus(
            "accepted" to "true",
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

        waitForRelayDrainIfRequested()
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
        requireHarnessInvocation("revoked-state wait is driven by the relay matrix")
        val wakeRelay = relayWakeCallback()
        wakeRelay()
        val account =
            waitForState("revoked device state", timeoutMs = 180_000) {
                wakeRelay()
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
        waitForRelayDrainIfRequested()
        val state = appManager().state.value
        val debug = readJsonObject(DEBUG_SNAPSHOT_FILENAME)
        val plan = debug?.optJSONObject("current_protocol_plan")
        val protocolEngine = debug?.optJSONObject("protocol_engine")
        val pendingProtocolOutbound = protocolEngine.optStringArray("pending_outbound_targets")
        val pendingGroupFanouts = protocolEngine.optStringArray("pending_group_fanout_targets")
        val legacyPendingOutbound = summarizeRuntimePendingOutbound(debug?.optJSONArray("pending_outbound"))

        reportStatus(
            "data_dir" to appFilesDir().absolutePath,
            "rev" to state.rev.toString(),
            "default_screen" to state.router.defaultScreen.toString(),
            "screen_stack" to state.router.screenStack.joinToString("|") { screen -> screen.toString() },
            "current_chat" to summarizeCurrentChat(state.currentChat),
            "chat_list" to summarizeChatList(state.chatList),
            "mobile_push_owner_pubkey_hex" to state.mobilePush.ownerPubkeyHex.orEmpty(),
            "mobile_push_message_author_pubkeys" to state.mobilePush.messageAuthorPubkeys.joinToString(","),
            "mobile_push_session_count" to state.mobilePush.sessions.size.toString(),
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
            "pending_protocol_outbound_count" to protocolEngine.optStringOrEmpty("pending_outbound_count"),
            "pending_protocol_outbound" to pendingProtocolOutbound,
            "pending_group_fanout_count" to protocolEngine.optStringOrEmpty("pending_group_fanout_count"),
            "pending_group_fanouts" to pendingGroupFanouts,
            "pending_outbound" to (legacyPendingOutbound.ifEmpty { pendingProtocolOutbound }),
            "pending_relay_publishes" to summarizeRuntimePendingRelayPublishes(debug?.optJSONArray("pending_relay_publishes")),
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
        val sqlite = readSqliteCoreSnapshot()

        reportStatus(
            "data_dir" to appFilesDir().absolutePath,
            "sqlite_file_present" to sqlite.filePresent.toString(),
            "sqlite_app_meta" to sqlite.appMeta,
            "sqlite_app_keys" to sqlite.appKeys,
            "sqlite_groups" to sqlite.groups,
            "sqlite_threads" to sqlite.threads,
            "sqlite_messages" to sqlite.messages,
            "sqlite_pending_relay_publishes" to sqlite.pendingRelayPublishes,
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
        var source = "persisted"

        val snapshot =
            waitForState("peer roster for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerRoster(json, peerOwnerHex) }
                    ?.also { source = "persisted" }
                    ?: readJsonObject(DEBUG_SNAPSHOT_FILENAME)
                        ?.takeIf { json -> runtimeDebugHasPeerRoster(json, peerOwnerHex) }
                        ?.also { source = "runtime" }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "source" to source,
            "users" to summarizeKnownUsers(snapshot, source),
        )
    }

    @Test
    fun wait_for_known_peer_session_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val peerOwnerHex = resolvePeerOwnerHex(peerInput)
        var source = "persisted"

        val snapshot =
            waitForState("known peer session for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerSession(json, peerOwnerHex) }
                    ?.also { source = "persisted" }
                    ?: readJsonObject(DEBUG_SNAPSHOT_FILENAME)
                        ?.takeIf { json -> runtimeDebugHasPeerSession(json, peerOwnerHex) }
                        ?.also { source = "runtime" }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "source" to source,
            "users" to summarizeKnownUsers(snapshot, source),
        )
    }

    @Test
    fun wait_for_peer_transport_ready_from_args() {
        ensureLoggedIn()
        val peerInput = requiredArg("peer_input")
        val peerOwnerHex = resolvePeerOwnerHex(peerInput)
        var source = "persisted"

        val snapshot =
            waitForState("peer transport ready for $peerOwnerHex", timeoutMs = 180_000) {
                readJsonObject(PERSISTED_STATE_FILENAME)
                    ?.takeIf { json -> persistedHasPeerTransportReady(json, peerOwnerHex) }
                    ?.also { source = "persisted" }
                    ?: readJsonObject(DEBUG_SNAPSHOT_FILENAME)
                        ?.takeIf { json -> runtimeDebugHasPeerTransportReady(json, peerOwnerHex) }
                        ?.also { source = "runtime" }
            }

        reportStatus(
            "peer_owner_hex" to peerOwnerHex,
            "source" to source,
            "users" to summarizeKnownUsers(snapshot, source),
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
        val memberInputs = optionalListArg("member_inputs")

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

        waitForRelayDrainIfRequested()
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
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 180L) * 1_000L)
        val wakeRelay = relayWakeCallback(chatId)

        val existing =
            waitForState("group thread in chat list", timeoutMs = timeoutMs) {
                wakeRelay()
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
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 180L) * 1_000L)
        val resolvedChatId =
            when {
                !chatId.isNullOrBlank() -> chatId
                !groupId.isNullOrBlank() -> "group:$groupId"
                else -> throw AssertionError("Missing instrumentation argument: chat_id or group_id")
            }
        val wakeRelay = relayWakeCallback(resolvedChatId)

        val current =
            waitForState("group member count $expectedMemberCount", timeoutMs = timeoutMs) {
                wakeRelay()
                val state = appManager().state.value
                state.currentChat?.takeIf { chat ->
                    chat.chatId == resolvedChatId &&
                        chat.memberCount == expectedMemberCount
                } ?: state.chatList
                    .firstOrNull { thread -> thread.chatId == resolvedChatId }
                    ?.also { thread -> appManager().openChat(thread.chatId) }
                    ?.let { null }
            }

        reportStatus(
            "chat_id" to current.chatId,
            "group_id" to current.groupId.orEmpty(),
            "member_count" to current.memberCount.toString(),
        )
    }

    @Test
    fun wait_for_group_name_from_args() {
        ensureLoggedIn()
        val chatId = requiredArg("chat_id")
        val groupName = requiredArg("group_name")
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 180L) * 1_000L)
        val wakeRelay = relayWakeCallback(chatId)
        val thread =
            waitForState("group name $groupName", timeoutMs = timeoutMs) {
                wakeRelay()
                appManager()
                    .state
                    .value
                    .chatList
                    .firstOrNull { thread ->
                        thread.chatId.equals(chatId, ignoreCase = true) &&
                            thread.displayName == groupName
                    }
            }

        appManager().openChat(thread.chatId)
        reportStatus(
            "chat_id" to thread.chatId,
            "group_name" to thread.displayName,
            "member_count" to thread.memberCount.toString(),
        )
    }

    @Test
    fun update_group_name_from_args() {
        ensureLoggedIn()
        val groupId = requiredArg("group_id")
        val groupName = requiredArg("group_name")
        val chatId = optionalArg("chat_id") ?: "group:$groupId"

        appManager().updateGroupName(groupId, groupName)
        val thread =
            waitForState("renamed group $groupName", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .chatList
                    .firstOrNull { thread ->
                        thread.chatId.equals(chatId, ignoreCase = true) &&
                            thread.displayName == groupName
                    }
            }

        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to thread.chatId,
            "group_id" to groupId,
            "group_name" to thread.displayName,
            "member_count" to thread.memberCount.toString(),
        )
    }

    @Test
    fun add_group_members_from_args() {
        ensureLoggedIn()
        val groupId = requiredArg("group_id")
        val chatId = optionalArg("chat_id") ?: "group:$groupId"
        val memberInputs = requiredListArg("member_inputs")
        val expectedMemberCount = optionalArg("expected_member_count")?.toULong()

        appManager().addGroupMembers(groupId, memberInputs)
        val thread =
            waitForState("added group members", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .chatList
                    .firstOrNull { thread ->
                        thread.chatId.equals(chatId, ignoreCase = true) &&
                            (expectedMemberCount == null || thread.memberCount == expectedMemberCount)
                    }
            }

        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to thread.chatId,
            "group_id" to groupId,
            "member_count" to thread.memberCount.toString(),
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
    fun set_group_admin_from_args() {
        ensureLoggedIn()
        val groupId = requiredArg("group_id")
        val memberInput = normalizePeerInput(requiredArg("member_input"))
        val isAdmin = optionalArg("is_admin")?.lowercase() !in setOf("0", "false", "no")

        appManager().setGroupAdmin(groupId, memberInput, isAdmin)
        appManager().pushScreen(Screen.GroupDetails(groupId))
        val details =
            waitForState("group admin $memberInput=$isAdmin", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .groupDetails
                    ?.takeIf { details ->
                        details.groupId == groupId &&
                            details.members.any { member ->
                                member.ownerPubkeyHex.equals(memberInput, ignoreCase = true) &&
                                    member.isAdmin == isAdmin
                            }
                    }
            }

        waitForRelayDrainIfRequested()
        reportStatus(
            "group_id" to details.groupId,
            "member_input" to memberInput,
            "is_admin" to isAdmin.toString(),
            "revision" to details.revision.toString(),
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

        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "delivery" to finalized.delivery.name,
            "outer_event_ids" to finalized.deliveryTrace.outerEventIds.joinToString(","),
            "target_device_ids" to finalized.deliveryTrace.targetDeviceIds.joinToString(","),
            "recipient_deliveries" to finalized.recipientDeliveries.joinToString("|") { recipient ->
                "${recipient.ownerPubkeyHex},${recipient.delivery.name}"
            },
        )
    }

    @Test
    fun send_nearby_message_from_args() {
        ensureLoggedIn()
        maybeDisableRelays()
        withActivity {
            nearbyService().setVisible(true)
        }
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val message = requiredArg("message")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        appManager().sendText(chat.chatId, message)
        val outgoing =
            waitForState("nearby outgoing message", timeoutMs = 30_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId == chat.chatId }
                    ?.messages
                    ?.find { entry -> entry.isOutgoing && entry.body == message }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "delivery" to outgoing.delivery.name,
            "relay_count" to appManager().state.value.preferences.nostrRelayUrls.size.toString(),
        )
    }

    @Test
    fun disable_relays_and_report() {
        ensureLoggedIn()
        disableRelays()
        reportStatus(
            "relay_count" to appManager().state.value.preferences.nostrRelayUrls.size.toString(),
            "relays" to JSONArray(appManager().state.value.preferences.nostrRelayUrls).toString(),
        )
    }

    @Test
    fun nearby_chat_exchange_from_args() {
        ensureLoggedIn()
        maybeDisableRelays()
        val peerInput = requiredArg("peer_input")
        val role = optionalArg("role")?.lowercase() ?: "initiator"
        val count = (optionalArg("count")?.toIntOrNull() ?: 10).coerceIn(1, 50)
        val prefix = optionalArg("prefix") ?: "nearby"
        val peerOwnerHex = peerInputToHex(peerInput).ifBlank { normalizePeerInput(peerInput) }

        withActivity {
            nearbyService().setVisible(true)
        }
        waitForState("nearby peer $peerOwnerHex", timeoutMs = 60_000) {
            nearbyService().snapshot.peers.firstOrNull { peer ->
                peer.ownerPubkeyHex?.equals(peerOwnerHex, ignoreCase = true) == true
            }
        }

        val chat = ensureChatOpen(peerInput)
        val startedAt = SystemClock.elapsedRealtime()
        var sent = 0
        var received = 0
        for (index in 1..count) {
            val message = "$prefix-$index"
            val shouldSend = (role == "initiator") == (index % 2 == 1)
            if (shouldSend) {
                appManager().sendText(chat.chatId, message)
                waitForState("outgoing $message", timeoutMs = 30_000) {
                    true.takeIf { countMessages(chat.chatId, message, "outgoing") > 0 }
                }
                sent += 1
            } else {
                waitForState("incoming $message", timeoutMs = 60_000) {
                    true.takeIf { countMessages(chat.chatId, message, "incoming") > 0 }
                }
                received += 1
            }
        }

        reportStatus(
            "chat_id" to chat.chatId,
            "role" to role,
            "sent" to sent.toString(),
            "received" to received.toString(),
            "elapsed_ms" to (SystemClock.elapsedRealtime() - startedAt).toString(),
            "relay_count" to appManager().state.value.preferences.nostrRelayUrls.size.toString(),
        )
    }

    @Test
    fun send_typing_from_args() {
        ensureLoggedIn()
        val chatIdArg = optionalArg("chat_id")
        val peerInput =
            optionalArg("peer_input")
                ?: if (chatIdArg.isNullOrBlank()) requiredArg("peer_input") else ""
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        appManager().dispatch(AppAction.SendTyping(chat.chatId))

        reportStatus(
            "chat_id" to chat.chatId,
            "sent_typing" to "true",
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
        val peerInput = optionalArg("peer_input").orEmpty()
        val expectedChatId = optionalArg("chat_id")?.takeIf { it.isNotBlank() }
        val direction = optionalArg("direction").orEmpty().lowercase()
        val expectedCount = optionalArg("expected_count")?.toIntOrNull()
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 180L) * 1_000L)
        val seededChat =
            when {
                !expectedChatId.isNullOrBlank() -> ensureChatOpenById(expectedChatId)
                peerInput.isNotBlank() -> ensureChatOpen(peerInput)
                else -> null
            }
        val resolvedChatId = expectedChatId ?: seededChat?.chatId
        val wakeRelay = relayWakeCallback(resolvedChatId)

        val matchedChatId =
            waitForState("${direction.ifBlank { "incoming" }} message", timeoutMs = timeoutMs) {
                wakeRelay()
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

                sqliteThreadWithMessage(
                    chatId = resolvedChatId,
                    expectedMessage = expectedMessage,
                    direction = direction,
                )?.let { return@waitForState it }

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

    @Test
    fun report_chat_messages_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val current =
            waitForState("opened chat messages", timeoutMs = 30_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
            }

        reportStatus(
            "chat_id" to current.chatId,
            "messages" to current.messages.joinToString("|") { message ->
                listOf(
                    message.id,
                    message.body,
                    message.isOutgoing.toString(),
                    message.delivery.name,
                ).joinToString(",")
            },
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
        appManager().updateProfileMetadata(name = displayName, pictureUrl = null, about = null)
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
     * Wait until the local owner's persisted profile store carries an
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
                val candidate =
                    readOwnerProfileDisplayName(peerPubkeyHex)
                        ?: readLegacyOwnerProfileDisplayName(peerPubkeyHex)
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
     * display name as title and the plaintext message as body for direct
     * chats, or the group name as title with the sender-prefixed body
     * for group chats — what `IrisFirebaseMessagingService` shows the user.
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
     *   - expected_title: sender's display name or group name
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
     * Verifies the foreground Android notification gate with the same
     * decrypted payload the FCM path uses. The ActivityScenario rule keeps
     * the app foregrounded; opening the matching chat should therefore
     * suppress the notification, while background/killed app delivery still
     * falls through to the normal notifier.
     */
    @Test
    fun suppress_notification_for_open_chat_from_args() {
        ensureLoggedIn()
        val outerEventJson = requiredArg("outer_event_json")
        val expectedBody = requiredArg("expected_body")
        val chatId = requiredArg("chat_id")

        val openChat = ensureChatOpenById(chatId)
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
            fail("Notification resolver suppressed before active-chat gate: ${resolution.payloadJson}")
        }
        if (resolution.body != expectedBody) {
            fail(
                "Notification body did not match decrypted plaintext. " +
                    "expected=`$expectedBody` got=`${resolution.body}` payload=${resolution.payloadJson}",
            )
        }
        if (!appManager().shouldSuppressNotificationForActiveChat(resolution)) {
            fail(
                "Expected active chat `${openChat.chatId}` to suppress notification. " +
                    "Resolution payload=${resolution.payloadJson}",
            )
        }

        reportStatus(
            "chat_id" to openChat.chatId,
            "body" to resolution.body,
            "suppressed" to "true",
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
        val peerInput = optionalArg("peer_input").orEmpty()
        val expectedChatId = optionalArg("chat_id")
        val direction = optionalArg("direction").orEmpty().lowercase()
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
        requireHarnessInvocation("logout/account reset is driven by targeted harness scripts")
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

    private fun ensureLoggedIn(): to.iris.chat.rust.AccountSnapshot {
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

    private fun maybeDisableRelays() {
        if (optionalArg("disable_relays") != "0") {
            disableRelays()
        }
    }

    private fun disableRelays() {
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

    private fun ensureLinkedDeviceStarted(ownerInput: String): to.iris.chat.rust.AccountSnapshot {
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

    private fun findChatMatchingPeerInput(peerInput: String): ChatThreadSnapshot? =
        appManager().state.value.chatList.firstOrNull { thread ->
            matchesPeerInput(
                chatId = thread.chatId,
                peerNpub = thread.subtitle.orEmpty(),
                peerInput = peerInput,
            )
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

    private fun requiredArg(name: String): String {
        optionalArg(name)?.let { return it }
        if (arguments.getString("class").isNullOrBlank()) {
            assumeTrue("Harness action requires instrumentation argument: $name", false)
        }
        throw AssertionError("Missing instrumentation argument: $name")
    }

    private fun requireHarnessInvocation(reason: String) {
        if (arguments.getString("class").isNullOrBlank()) {
            assumeTrue(reason, false)
        }
    }

    private fun waitForRelayDrainIfRequested() {
        val raw = optionalArg("wait_for_relay_drain")?.lowercase() ?: return
        if (raw !in setOf("1", "true", "yes")) {
            return
        }

        SystemClock.sleep(500)
        val runtimeOnly =
            optionalArg("relay_drain_runtime_only")?.lowercase() in setOf("1", "true", "yes")
        val timeoutMs =
            ((optionalArg("relay_drain_timeout_secs")?.toLongOrNull() ?: 180L) * 1_000L)
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
                            pendingDurablePublishCount == 0 &&
                            (runtimeOnly || status.pendingOutboundCount == 0UL) &&
                            status.pendingGroupControlCount == 0UL
                    }
            }
        reportStatus(
            "pending_outbound_count" to status.pendingOutboundCount.toString(),
            "pending_runtime_outbound_count" to pendingRelayPublishCount().toString(),
            "pending_group_control_count" to status.pendingGroupControlCount.toString(),
        )
    }

    private fun requiredListArg(name: String): List<String> =
        requiredArg(name)
            .split(',', '\n', '|')
            .map(String::trim)
            .filter(String::isNotEmpty)
            .takeIf { it.isNotEmpty() }
            ?: throw AssertionError("Missing non-empty list argument: $name")

    private fun optionalListArg(name: String): List<String> =
        optionalArg(name)
            ?.split(',', '\n', '|')
            ?.map(String::trim)
            ?.filter(String::isNotEmpty)
            ?: emptyList()

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

    private data class SqliteCoreSnapshot(
        val filePresent: Boolean,
        val appMeta: String = "",
        val appKeys: String = "",
        val groups: String = "",
        val threads: String = "",
        val messages: String = "",
        val pendingRelayPublishes: String = "",
    )

    private fun readSqliteCoreSnapshot(): SqliteCoreSnapshot {
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
                                    SELECT label, target_owner_pubkey_hex, target_device_id, chat_id, message_id, attempt_count
                                    FROM pending_relay_publishes
                                    ORDER BY created_at_secs DESC
                                    LIMIT 30
                                """.trimIndent(),
                            ) { cursor ->
                                listOf(
                                    cursor.getString(0),
                                    cursor.stringOrEmpty(1),
                                    cursor.stringOrEmpty(2),
                                    cursor.stringOrEmpty(3),
                                    cursor.stringOrEmpty(4),
                                    cursor.getLong(5).toString(),
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

    private fun summarizeRows(
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

    private fun android.database.Cursor.stringOrEmpty(index: Int): String =
        if (isNull(index)) "" else getString(index)

    private fun pendingRelayPublishCount(label: String? = null): Int {
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

    private fun readOwnerProfileDisplayName(ownerPubkeyHex: String): String? {
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

    private fun readLegacyOwnerProfileDisplayName(ownerPubkeyHex: String): String? {
        val profiles = readJsonObject("core/profiles.json") ?: return null
        val entry = profiles.optJSONObject(ownerPubkeyHex) ?: return null
        return entry.optString("display_name").takeIf { it.isNotEmpty() }
            ?: entry.optString("name").takeIf { it.isNotEmpty() }
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
        val sqliteCount = countSqliteMessages(chatId, expectedMessage, direction)
        return maxOf(persistedCount, sqliteCount, stateCount)
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

    private fun sqliteThreadWithMessage(
        chatId: String?,
        expectedMessage: String,
        direction: String,
    ): String? {
        val dbFile = File(appFilesDir(), CORE_DB_FILENAME)
        if (!dbFile.exists()) {
            return null
        }
        return runCatching {
            SQLiteDatabase
                .openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
                .use { db ->
                    val args = mutableListOf(expectedMessage)
                    val clauses = mutableListOf("body = ?")
                    if (!chatId.isNullOrBlank()) {
                        clauses += "chat_id = ?"
                        args += chatId
                    }
                    sqliteDirectionValue(direction)?.let { outgoing ->
                        clauses += "is_outgoing = ?"
                        args += outgoing
                    }
                    val sql =
                        """
                            SELECT chat_id
                            FROM messages
                            WHERE ${clauses.joinToString(" AND ")}
                            ORDER BY created_at_secs DESC, id DESC
                            LIMIT 1
                        """.trimIndent()
                    db.rawQuery(sql, args.toTypedArray()).use { cursor ->
                        if (cursor.moveToFirst()) cursor.getString(0) else null
                    }
                }
        }.getOrNull()
    }

    private fun countSqliteMessages(
        chatId: String,
        expectedMessage: String,
        direction: String,
    ): Int {
        val dbFile = File(appFilesDir(), CORE_DB_FILENAME)
        if (!dbFile.exists()) {
            return 0
        }
        return runCatching {
            SQLiteDatabase
                .openDatabase(dbFile.absolutePath, null, SQLiteDatabase.OPEN_READONLY)
                .use { db ->
                    val args = mutableListOf(chatId, expectedMessage)
                    val clauses = mutableListOf("chat_id = ?", "body = ?")
                    sqliteDirectionValue(direction)?.let { outgoing ->
                        clauses += "is_outgoing = ?"
                        args += outgoing
                    }
                    val sql =
                        """
                            SELECT COUNT(*)
                            FROM messages
                            WHERE ${clauses.joinToString(" AND ")}
                        """.trimIndent()
                    db.rawQuery(sql, args.toTypedArray()).use { cursor ->
                        if (cursor.moveToFirst()) cursor.getInt(0) else 0
                    }
                }
        }.getOrDefault(0)
    }

    private fun holdNearbyIfRequested() {
        val holdMs = (optionalArg("hold_ms")?.toLongOrNull() ?: 0L).coerceIn(0L, 60_000L)
        if (holdMs <= 0L) return
        reportStatus("nearby_hold_ms" to holdMs.toString())
        SystemClock.sleep(holdMs)
    }

    private fun sqliteDirectionValue(direction: String): String? =
        when (direction.lowercase()) {
            "incoming" -> "0"
            "outgoing" -> "1"
            else -> null
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

    private fun runtimeDebugHasPeerRoster(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optBoolean("has_roster") && user.optInt("roster_device_count") > 0
        }

    private fun runtimeDebugHasPeerSession(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optInt("active_session_device_count") > 0 ||
                user.optInt("inactive_session_count") > 0
        }

    private fun runtimeDebugHasPeerTransportReady(
        debug: JSONObject,
        peerOwnerHex: String,
    ): Boolean =
        runtimeDebugKnownPeer(debug, peerOwnerHex) { user ->
            user.optBoolean("has_roster") &&
                user.optInt("roster_device_count") > 0 &&
                user.optInt("device_count") > 0 &&
                user.optInt("authorized_device_count") > 0
        }

    private fun runtimeDebugKnownPeer(
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

    private fun runtimeDebugAuthorizedDeviceCount(
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

    private fun summarizeKnownUsers(
        snapshot: JSONObject,
        source: String,
    ): String =
        if (source == "runtime") {
            summarizeRuntimeKnownUsers(snapshot.optJSONArray("known_users"))
        } else {
            summarizePersistedUsers(snapshot.optJSONObject("session_manager")?.optJSONArray("users"))
        }

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

    private fun summarizeChatList(threads: List<to.iris.chat.rust.ChatThreadSnapshot>): String =
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

    private fun summarizeRuntimePendingRelayPublishes(entries: JSONArray?): String =
        entries.joinObjects { entry ->
            listOf(
                entry.optString("event_id"),
                entry.optString("label"),
                entry.optString("target_owner_pubkey_hex"),
                entry.optString("target_device_id"),
                "attempts=${entry.optInt("attempt_count")}",
                "error=${entry.optString("last_error")}",
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
        entries.joinObjects(limit = 80) { entry ->
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

    private fun reportNearbySnapshot(snapshot: IrisNearbyService.Snapshot) {
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

    private fun <T> waitForOptionalState(timeoutMs: Long, condition: () -> T?): T? {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            condition()?.let { return it }
            SystemClock.sleep(100)
        }
        return null
    }

    private fun relayWakeCallback(openChatId: String? = null): () -> Unit {
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

    private companion object {
        const val DEBUG_SNAPSHOT_FILENAME = "iris_chat_runtime_debug.json"
        const val CORE_DB_FILENAME = "core.sqlite3"
        const val PERSISTED_STATE_FILENAME = "iris_chat_core_state.json"
        const val NEARBY_PROFILE_TIMEOUT_MS = 180_000L
        const val HARNESS_RELAY_WAKE_INTERVAL_MS = 3_000L
    }
}
