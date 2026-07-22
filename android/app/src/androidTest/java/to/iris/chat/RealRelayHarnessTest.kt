package to.iris.chat

import android.Manifest
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.util.Base64
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
class RealRelayHarnessTest : RealRelayHarnessBase() {
    @get:Rule
    override val activityRule = ActivityScenarioRule(MainActivity::class.java)

    @Test
    fun create_account_and_report_identity() {
        val account = ensureLoggedIn(createIfMissing = true)
        waitForPersistedDeviceSecret()
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
        waitForRelayDrainIfRequested()
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
    fun export_secret_key() {
        ensureLoggedIn()
        val secretKey =
            kotlinx.coroutines.runBlocking { appManager().exportOwnerNsec() }
                ?: throw AssertionError("Secret key was not available for export")
        reportStatus(
            "secret_key" to secretKey,
        )
    }

    @Test
    fun restore_session_from_args() {
        val secretKey = requiredArg("secret_key")
        val expectedPublicKeyHex = optionalArg("expected_public_key_hex")

        appManager().restoreSession(secretKey)

        val account =
            waitForState("restored account", timeoutMs = 90_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { account ->
                        expectedPublicKeyHex?.let { expected ->
                            account.publicKeyHex.equals(expected, ignoreCase = true)
                        } ?: true
                    }
            }

        appManager().state.value.toast?.takeIf { it.isNotBlank() }?.let { toast ->
            fail("Restore failed: $toast")
        }

        waitForPersistedDeviceSecret()
        waitForRelayDrainIfRequested()
        reportStatus(
            "npub" to account.npub,
            "public_key_hex" to account.publicKeyHex,
            "device_npub" to account.deviceNpub,
            "device_public_key_hex" to account.devicePublicKeyHex,
            "display_name" to account.displayName,
            "authorization_state" to account.authorizationState.name,
            "app_package" to appPackageName(),
            "data_dir" to appFilesDir().absolutePath,
        )
    }

    @Test
    fun wait_for_account_display_name_from_args() {
        ensureLoggedIn()
        val expected = requiredArg("display_name")
        val account =
            waitForState("account display name $expected", timeoutMs = 180_000) {
                appManager()
                    .state
                    .value
                    .account
                    ?.takeIf { account -> account.displayName == expected }
            }

        reportStatus(
            "public_key_hex" to account.publicKeyHex,
            "display_name" to account.displayName,
        )
    }

    @Test
    fun enable_nearby_and_report_peers() {
        ensureLoggedIn(createIfMissing = true)
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
        SystemClock.sleep(1_000)
        reportNearbySnapshot(nearbyService().snapshot)
    }

    @Test
    fun enable_lan_nearby_and_report_peers() {
        ensureLoggedIn(createIfMissing = true)
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyLanEnabled(true))
        SystemClock.sleep(1_000)
        reportNearbySnapshot(nearbyService().snapshot)
    }

    @Test
    fun wait_for_nearby_peer_profile_from_args() {
        ensureLoggedIn()
        val peerOwnerHex = peerInputToHex(requiredArg("peer_input")).ifBlank {
            normalizePeerInput(requiredArg("peer_input"))
        }
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
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
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyLanEnabled(true))
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
            waitForState("invite chat message publish", timeoutMs = 60_000) {
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
    fun disable_mobile_push_and_wait() {
        ensureLoggedIn()
        appManager().dispatch(AppAction.SetDesktopNotificationsEnabled(false))
        waitForState("mobile push disabled", timeoutMs = 15_000) {
            appManager().state.value.preferences.takeIf { preferences ->
                !preferences.desktopNotificationsEnabled
            }
        }
        SystemClock.sleep(3_000)
        reportStatus("mobile_push_disabled" to "true")
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

        waitForPersistedDeviceSecret()
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
                    if (toast != "Device added") {
                        fail("Link invite accept failed: $toast")
                    }
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
            waitForState("device removal reflected in roster", timeoutMs = 5_000) {
                val state = appManager().state.value
                val roster = state.deviceRoster
                val removed =
                    roster?.devices?.none { device ->
                        deviceMatchesInput(device.devicePubkeyHex, device.deviceNpub, deviceInput)
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
            "device_count" to roster.devices.size.toString(),
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
        val state =
            waitForState("revoked device state", timeoutMs = 180_000) {
                wakeRelay()
                val state = appManager().state.value
                val account = state.account
                when {
                    account?.authorizationState == DeviceAuthorizationState.REVOKED -> state
                    account == null && state.router.defaultScreen is Screen.Welcome -> state
                    account == null && state.router.defaultScreen is Screen.DeviceRevoked -> state
                    else -> null
                }
            }
        val account = state.account

        reportStatus(
            "authorization_state" to (account?.authorizationState?.name ?: "LOGGED_OUT"),
            "device_npub" to (account?.deviceNpub ?: ""),
            "device_public_key_hex" to (account?.devicePublicKeyHex ?: ""),
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
        val settledLiveDebug = waitForRuntimeSnapshotIdleIfRequested()
        val state = appManager().state.value
        val fileDebug = readJsonObject(DEBUG_SNAPSHOT_FILENAME)
        val liveDebug = settledLiveDebug ?: readLiveRuntimeDebugSnapshot()
        val debug = liveDebug ?: fileDebug
        val plan = debug?.optJSONObject("current_protocol_plan") ?: debug?.optJSONObject("protocol")
        val protocolEngine = debug?.optJSONObject("protocol_engine")
        val pendingProtocolOutbound = protocolEngine.optStringArray("pending_outbound_targets")
        val pendingGroupFanouts = protocolEngine.optStringArray("pending_group_fanout_targets")
        val legacyPendingOutbound = summarizeRuntimePendingOutbound(debug?.optJSONArray("pending_outbound"))
        val localOwner =
            debug.optStringOrEmpty("local_owner_pubkey_hex").ifEmpty { state.account?.publicKeyHex.orEmpty() }
        val localDevice =
            debug.optStringOrEmpty("local_device_pubkey_hex").ifEmpty { state.account?.devicePublicKeyHex.orEmpty() }

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
            "runtime_file_present" to (fileDebug != null).toString(),
            "runtime_live_snapshot_present" to (liveDebug != null).toString(),
            "runtime_snapshot_source" to if (debug == null) "none" else if (liveDebug == null) "file" else "live",
            "runtime_support_bundle_timed_out" to
                (liveDebug?.optJSONObject("ffi_queue")?.optBoolean("core_support_bundle_timed_out") == true).toString(),
            "generated_at_secs" to debug.optStringOrEmpty("generated_at_secs"),
            "local_owner_pubkey_hex" to localOwner,
            "local_device_pubkey_hex" to localDevice,
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
            "pending_group_sender_key_message_count" to protocolEngine.optStringOrEmpty("pending_group_sender_key_message_count"),
            "pending_group_sender_key_retry_count" to protocolEngine.optStringOrEmpty("pending_group_sender_key_retry_count"),
            "pending_group_sender_key_unmapped_count" to protocolEngine.optStringOrEmpty("pending_group_sender_key_unmapped_count"),
            "pending_group_sender_key_repair_count" to protocolEngine.optStringOrEmpty("pending_group_sender_key_repair_count"),
            "pending_group_sender_key_repair_next_retry_at_secs" to protocolEngine.optStringOrEmpty("pending_group_sender_key_repair_next_retry_at_secs"),
            "pending_group_sender_key_repair_max_request_count" to protocolEngine.optStringOrEmpty("pending_group_sender_key_repair_max_request_count"),
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
        waitForRelayDrainIfRequested()
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
            waitForState("created group chat", timeoutMs = 60_000) {
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
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
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
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
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
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
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
    fun wait_for_group_admin_from_args() {
        ensureLoggedIn()
        val groupId = requiredArg("group_id")
        val memberInput = normalizePeerInput(requiredArg("member_input"))
        val isAdmin = optionalArg("is_admin")?.lowercase() !in setOf("0", "false", "no")
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)

        appManager().pushScreen(Screen.GroupDetails(groupId))
        val details =
            waitForState("group admin $memberInput=$isAdmin", timeoutMs = timeoutMs) {
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

        reportStatus(
            "group_id" to details.groupId,
            "member_input" to memberInput,
            "is_admin" to isAdmin.toString(),
            "revision" to details.revision.toString(),
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
            waitForState("renamed group $groupName", timeoutMs = 60_000) {
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
    fun expect_group_name_update_rejected_from_args() {
        ensureLoggedIn()
        val groupId = requiredArg("group_id")
        val rejectedName = requiredArg("group_name")
        val chatId = optionalArg("chat_id") ?: "group:$groupId"
        val expectedName = optionalArg("expected_group_name")?.takeIf { it.isNotBlank() }
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 30L) * 1_000L)
        val initialName =
            appManager()
                .state
                .value
                .chatList
                .firstOrNull { thread -> thread.chatId.equals(chatId, ignoreCase = true) }
                ?.displayName
                .orEmpty()

        appManager().updateGroupName(groupId, rejectedName)
        val rejectionToast =
            waitForOptionalState(timeoutMs = timeoutMs) {
                val state = appManager().state.value
                val renamed =
                    state.chatList.any { thread ->
                        thread.chatId.equals(chatId, ignoreCase = true) &&
                            thread.displayName == rejectedName
                    }
                if (renamed) {
                    fail("Rejected group rename unexpectedly applied $rejectedName")
                }
                state.toast?.takeIf { it.isNotBlank() }
            }.orEmpty()

        val finalName =
            appManager()
                .state
                .value
                .chatList
                .firstOrNull { thread -> thread.chatId.equals(chatId, ignoreCase = true) }
                ?.displayName
                ?: initialName
        if (expectedName != null && finalName != expectedName) {
            fail("Expected group name $expectedName, found $finalName")
        }

        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to chatId,
            "group_id" to groupId,
            "rejected_group_name" to rejectedName,
            "group_name" to finalName,
            "toast" to rejectionToast,
            "rejected" to "true",
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
            waitForState("added group members", timeoutMs = 60_000) {
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
            waitForState("removed group member from $resolvedChatId", timeoutMs = 60_000) {
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
            waitForState("group admin $memberInput=$isAdmin", timeoutMs = 60_000) {
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

        val outgoing =
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
                    ?.messages
                    ?.firstOrNull { entry ->
                        entry.isOutgoing && entry.body == message
                    }
            }

        val waitForDelivery =
            optionalArg("wait_for_delivery")?.lowercase() !in setOf("0", "false", "no")
        if (!waitForDelivery) {
            reportStatus(
                "chat_id" to chat.chatId,
                "message" to message,
                "delivery" to outgoing.delivery.name,
                "outer_event_ids" to outgoing.deliveryTrace.outerEventIds.joinToString(","),
                "recipient_deliveries" to outgoing.recipientDeliveries.joinToString("|") { recipient ->
                    "${recipient.ownerPubkeyHex},${recipient.delivery.name}"
                },
            )
            return
        }

        val finalized =
            waitForState("message publish", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current ->
                        current.chatId == chat.chatId
                    }
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
            "recipient_deliveries" to finalized.recipientDeliveries.joinToString("|") { recipient ->
                "${recipient.ownerPubkeyHex},${recipient.delivery.name}"
            },
        )
    }

    @Test
    fun send_nearby_message_from_args() {
        ensureLoggedIn()
        maybeDisableRelays()
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
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
    fun send_nearby_message_burst_from_args() {
        ensureLoggedIn()
        maybeDisableRelays()
        appManager().dispatch(AppAction.SetDebugLoggingEnabled(true))
        enableFipsBleForPhysicalPeer()
        val peerInput = requiredArg("peer_input")
        val messagePrefix = requiredArg("message_prefix")
        val messageCount = (optionalArg("message_count")?.toIntOrNull() ?: 24).coerceIn(1, 64)
        val messageSize = (optionalArg("message_size")?.toIntOrNull() ?: 512).coerceIn(32, 4_096)
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 180_000L
        val preSendDelayMs = optionalArg("pre_send_delay_ms")?.toLongOrNull() ?: 0L
        val chat = ensureChatOpen(peerInput)
        val initialPeerProtocol =
            kotlinx.coroutines.runBlocking { appManager().peerProfileDebug(peerInput) }
        if (initialPeerProtocol?.directSendReadiness == "MissingLocalAppKeys") {
            // The production New Chat screen performs this bootstrap action.
            // This harness opens chats directly, so mirror that side effect
            // before measuring BLE transport throughput.
            appManager().dispatch(AppAction.CreatePublicInvite)
        }
        val peerProtocol =
            waitForState("nearby peer protocol ready", timeoutMs = 90_000) {
                kotlinx.coroutines
                    .runBlocking { appManager().peerProfileDebug(peerInput) }
                    ?.takeIf { debug -> debug.directSendReadiness == "Ready" }
            }
        val messages =
            (1..messageCount).map { index ->
                val header = "$messagePrefix-${index.toString().padStart(3, '0')}-"
                header + "x".repeat((messageSize - header.length).coerceAtLeast(0))
            }

        if (preSendDelayMs > 0) {
            SystemClock.sleep(preSendDelayMs)
        }
        val sendStartedMs = SystemClock.elapsedRealtime()
        messages.forEach { message -> appManager().sendText(chat.chatId, message) }
        val enqueueDurationMs = SystemClock.elapsedRealtime() - sendStartedMs

        val seenMessages =
            waitForState("nearby burst receipts", timeoutMs = timeoutMs) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                    ?.messages
                    ?.filter { entry -> entry.isOutgoing && entry.body in messages }
                    ?.takeIf { entries ->
                        entries.size == messageCount &&
                            entries.all { entry ->
                                entry.delivery == DeliveryState.SEEN ||
                                    entry.recipientDeliveries.any { recipient ->
                                        recipient.delivery == DeliveryState.SEEN
                                    }
                            }
                    }
            }

        appManager().state.value.toast?.takeIf { it.isNotBlank() }?.let { toast ->
            fail("Unexpected toast after nearby burst: $toast")
        }
        SystemClock.sleep(3_000)
        reportStatus(
            "chat_id" to chat.chatId,
            "peer_protocol_ready" to peerProtocol.directSendReadiness,
            "message_count" to seenMessages.size.toString(),
            "message_size" to messageSize.toString(),
            "enqueue_duration_ms" to enqueueDurationMs.toString(),
            "all_seen" to "true",
            "relay_count" to appManager().state.value.preferences.nostrRelayUrls.size.toString(),
        )
    }

    @Test
    fun report_nearby_peer_protocol_from_args() {
        ensureLoggedIn()
        maybeDisableRelays()
        enableFipsBleForPhysicalPeer()
        val peerInput = requiredArg("peer_input")
        val debug =
            waitForState("nearby peer protocol diagnostics", timeoutMs = 30_000) {
                kotlinx.coroutines.runBlocking { appManager().peerProfileDebug(peerInput) }
            }
        reportStatus(
            "direct_send_readiness" to debug.directSendReadiness,
            "known_device_count" to debug.knownDeviceCount.toString(),
            "active_session_count" to debug.activeSessionCount.toString(),
            "session_count" to debug.sessionCount.toString(),
            "receiving_session_count" to debug.receivingSessionCount.toString(),
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
    fun set_relays_from_args() {
        ensureLoggedIn()
        val relayUrls =
            optionalListArg("relay_urls")
                .ifEmpty {
                    optionalArg("relay_url")
                        ?.takeIf { it.isNotBlank() }
                        ?.let(::listOf)
                        ?: emptyList()
                }
        appManager().dispatch(AppAction.SetNostrRelays(relayUrls))
        waitForState<Boolean>("set relays", timeoutMs = 30_000) {
            true.takeIf {
                appManager().state.value.preferences.nostrRelayUrls == relayUrls
            }
        }
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

        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
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

        appManager().dispatch(AppAction.SetTypingIndicatorsEnabled(true))
        appManager().dispatch(AppAction.SendTyping(chat.chatId))
        waitForRelayDrainIfRequested()

        reportStatus(
            "chat_id" to chat.chatId,
            "sent_typing" to "true",
        )
    }

    @Test
    fun wait_for_typing_from_args() {
        ensureLoggedIn()
        val chatIdArg = optionalArg("chat_id")
        val peerInput =
            optionalArg("peer_input")
                ?: if (chatIdArg.isNullOrBlank()) requiredArg("peer_input") else ""
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)

        reportStatus(
            "chat_id" to chat.chatId,
            "typing_wait_ready" to "true",
        )

        val typingCount =
            waitForState("typing indicator", timeoutMs = timeoutMs) {
                val state = appManager().state.value
                val currentTypingCount =
                    state.currentChat
                        ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                        ?.typingIndicators
                        ?.size
                        ?: 0
                val threadTyping =
                    state.chatList.any { thread ->
                        thread.chatId.equals(chat.chatId, ignoreCase = true) && thread.isTyping
                    }
                if (currentTypingCount > 0 || threadTyping) {
                    currentTypingCount
                } else {
                    null
                }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "typing_count" to typingCount.toString(),
            "typing" to "true",
        )
    }

    @Test
    fun accept_message_request_from_args() {
        ensureLoggedIn()
        val chatIdArg = optionalArg("chat_id")
        val peerInput =
            optionalArg("peer_input")
                ?: if (chatIdArg.isNullOrBlank()) requiredArg("peer_input") else ""
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        appManager().dispatch(AppAction.SetMessageRequestAccepted(chat.chatId))
        waitForRelayDrainIfRequested()
        val accepted =
            waitForState("message request accepted", timeoutMs = 30_000) {
                val state = appManager().state.value
                val currentAccepted =
                    state.currentChat
                        ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                        ?.let { current -> !current.isRequest }
                val threadAccepted =
                    state.chatList
                        .firstOrNull { thread -> thread.chatId.equals(chat.chatId, ignoreCase = true) }
                        ?.let { thread -> !thread.isRequest }
                true.takeIf { currentAccepted == true || threadAccepted == true }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "accepted" to accepted.toString(),
        )
    }

    @Test
    fun mark_message_seen_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val expectedMessage = requiredArg("message")
        val direction = optionalArg("direction")?.lowercase() ?: "incoming"
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val messageIds =
            waitForState("message ids for seen", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                    ?.messages
                    ?.filter { message ->
                        message.body == expectedMessage &&
                            messageDirectionMatches(message.isOutgoing, direction)
                    }
                    ?.map { it.id }
                    ?.takeIf { it.isNotEmpty() }
            }

        appManager().dispatch(AppAction.MarkMessagesSeen(chat.chatId, messageIds))
        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to chat.chatId,
            "message" to expectedMessage,
            "message_ids" to messageIds.joinToString(","),
            "seen" to "true",
        )
    }

    @Test
    fun accept_and_mark_incoming_message_seen_from_args() {
        ensureLoggedIn()
        enableFipsBleForPhysicalPeer()
        reportStatus("fips_ble_ready" to "true")
        val expectedMessage = requiredArg("message")
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 120_000L
        val chatId = acceptAndMarkIncomingMessageSeen(expectedMessage, timeoutMs)

        // Keep the host process alive long enough for the signed receipt event
        // to leave over the physical FIPS BLE link before instrumentation exits.
        SystemClock.sleep(3_000)
        reportStatus(
            "chat_id" to chatId,
            "message" to expectedMessage,
            "accepted" to "true",
            "seen" to "true",
        )
    }

    @Test
    fun accept_and_mark_two_incoming_messages_seen_from_args() {
        ensureLoggedIn()
        enableFipsBleForPhysicalPeer()
        reportStatus("fips_ble_ready" to "true")
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 180_000L
        val firstMessage = requiredArg("first_message")
        val secondMessage = requiredArg("second_message")

        val firstChatId = acceptAndMarkIncomingMessageSeen(firstMessage, timeoutMs)
        reportStatus("first_message_seen" to "true")
        val secondChatId = acceptAndMarkIncomingMessageSeen(secondMessage, timeoutMs)
        SystemClock.sleep(3_000)
        reportStatus(
            "chat_id" to secondChatId,
            "same_chat" to firstChatId.equals(secondChatId, ignoreCase = true).toString(),
            "first_message" to firstMessage,
            "second_message" to secondMessage,
            "seen" to "true",
        )
    }

    private fun acceptAndMarkIncomingMessageSeen(expectedMessage: String, timeoutMs: Long): String {

        val chatId =
            waitForState("incoming message", timeoutMs = timeoutMs) {
                val state = appManager().state.value
                state.currentChat
                    ?.takeIf { chat ->
                        chat.messages.any { message ->
                            !message.isOutgoing && message.body == expectedMessage
                        }
                    }
                    ?.chatId
                    ?: state.chatList
                        .firstOrNull { thread ->
                            thread.lastMessageIsOutgoing == false &&
                                thread.lastMessagePreview == expectedMessage
                        }
                        ?.chatId
            }

        appManager().openChat(chatId)
        val messageIds =
            waitForState("incoming message in open chat", timeoutMs = 30_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                    ?.messages
                    ?.filter { message ->
                        !message.isOutgoing && message.body == expectedMessage
                    }
                    ?.map { message -> message.id }
                    ?.takeIf { ids -> ids.isNotEmpty() }
            }

        appManager().dispatch(AppAction.SetMessageRequestAccepted(chatId))
        waitForState("message request accepted", timeoutMs = 30_000) {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                ?.let { chat -> true.takeIf { !chat.isRequest } }
        }
        appManager().dispatch(AppAction.MarkMessagesSeen(chatId, messageIds))
        waitForState("incoming message marked seen", timeoutMs = 30_000) {
            appManager()
                .state
                .value
                .currentChat
                ?.takeIf { chat -> chat.chatId.equals(chatId, ignoreCase = true) }
                ?.messages
                ?.filter { message -> message.id in messageIds }
                ?.takeIf { messages ->
                    messages.isNotEmpty() &&
                        messages.all { message -> message.delivery == DeliveryState.SEEN }
                }
        }
        return chatId
    }

    private fun enableFipsBleForPhysicalPeer() {
        val permissions =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                listOf(
                    Manifest.permission.BLUETOOTH_SCAN,
                    Manifest.permission.BLUETOOTH_CONNECT,
                    Manifest.permission.BLUETOOTH_ADVERTISE,
                )
            } else {
                listOf(Manifest.permission.ACCESS_FINE_LOCATION)
            }
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        permissions.forEach { permission ->
            runCatching {
                instrumentation.uiAutomation.grantRuntimePermission(appPackageName(), permission)
            }
        }
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(false))
        waitForState("FIPS Bluetooth reset", timeoutMs = 30_000) {
            appManager().state.value.preferences.takeIf { preferences ->
                !preferences.nearbyBluetoothEnabled
            }
        }
        appManager().dispatch(AppAction.SetNearbyEnabled(true))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(true))
        waitForState("FIPS Bluetooth enabled", timeoutMs = 30_000) {
            appManager().state.value.preferences.takeIf { preferences ->
                preferences.nearbyEnabled && preferences.nearbyBluetoothEnabled
            }
        }
    }

    @Test
    fun wait_for_message_delivery_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val expectedMessage = requiredArg("message")
        val expectedDelivery = (optionalArg("delivery") ?: "seen").uppercase()
        val direction = optionalArg("direction")?.lowercase() ?: "outgoing"
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val delivery =
            waitForState("message delivery $expectedDelivery", timeoutMs = 60_000) {
                val message =
                    appManager()
                        .state
                        .value
                        .currentChat
                        ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                        ?.messages
                        ?.firstOrNull { entry ->
                            entry.body == expectedMessage &&
                                messageDirectionMatches(entry.isOutgoing, direction)
                        }
                        ?: return@waitForState null
                if (message.delivery.name.equals(expectedDelivery, ignoreCase = true)) {
                    message.delivery.name
                } else {
                    message.recipientDeliveries
                        .firstOrNull { recipient ->
                            recipient.delivery.name.equals(expectedDelivery, ignoreCase = true)
                        }
                        ?.delivery
                        ?.name
                }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to expectedMessage,
            "delivery" to delivery,
        )
    }

    @Test
    fun react_to_message_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val expectedMessage = requiredArg("message")
        val emoji = optionalArg("emoji") ?: "❤️"
        val direction = optionalArg("direction")?.lowercase() ?: "incoming"
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val messageId =
            waitForState("message to react", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                    ?.messages
                    ?.firstOrNull { entry ->
                        entry.body == expectedMessage &&
                            messageDirectionMatches(entry.isOutgoing, direction)
                    }
                    ?.id
            }

        appManager().dispatch(AppAction.ToggleReaction(chat.chatId, messageId, emoji))
        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to chat.chatId,
            "message" to expectedMessage,
            "message_id" to messageId,
            "emoji" to emoji,
        )
    }

    @Test
    fun wait_for_message_reaction_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val expectedMessage = requiredArg("message")
        val emoji = optionalArg("emoji") ?: "❤️"
        val direction = optionalArg("direction")?.lowercase() ?: "any"
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val reaction =
            waitForState("reaction $emoji", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                    ?.messages
                    ?.firstOrNull { entry ->
                        entry.body == expectedMessage &&
                            messageDirectionMatches(entry.isOutgoing, direction)
                    }
                    ?.reactions
                    ?.firstOrNull { reaction -> reaction.emoji == emoji && reaction.count > 0UL }
            }

        reportStatus(
            "chat_id" to chat.chatId,
            "message" to expectedMessage,
            "emoji" to reaction.emoji,
            "reaction_count" to reaction.count.toString(),
            "reacted_by_me" to reaction.reactedByMe.toString(),
        )
    }

    @Test
    fun set_chat_settings_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        optionalBoolArg("muted")?.let { muted ->
            appManager().dispatch(AppAction.SetChatMuted(chat.chatId, muted))
        }
        optionalBoolArg("pinned")?.let { pinned ->
            appManager().dispatch(AppAction.SetChatPinned(chat.chatId, pinned))
        }
        optionalArg("ttl_seconds")?.let { ttl ->
            appManager().dispatch(AppAction.SetChatMessageTtl(chat.chatId, ttl.toULongOrNull()))
        }

        waitForRelayDrainIfRequested()
        val settings = waitForChatSettings(chat.chatId, timeoutMs = 30_000)
        reportStatus(
            "chat_id" to chat.chatId,
            "muted" to settings.muted.toString(),
            "pinned" to settings.pinned.toString(),
            "ttl_seconds" to settings.ttlSeconds?.toString().orEmpty(),
        )
    }

    @Test
    fun wait_for_chat_settings_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        val settings = waitForChatSettings(chat.chatId, timeoutMs = 60_000)
        reportStatus(
            "chat_id" to chat.chatId,
            "muted" to settings.muted.toString(),
            "pinned" to settings.pinned.toString(),
            "ttl_seconds" to settings.ttlSeconds?.toString().orEmpty(),
        )
    }

    @Test
    fun send_disappearing_message_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val message = requiredArg("message")
        val ttlSeconds = optionalArg("ttl_seconds")?.toLongOrNull() ?: 8L
        val expiresAtSecs = ((System.currentTimeMillis() / 1_000L) + ttlSeconds).toULong()
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        appManager().dispatch(AppAction.SendDisappearingMessage(chat.chatId, message, expiresAtSecs))
        val sent =
            waitForState("disappearing message", timeoutMs = 60_000) {
                appManager()
                    .state
                    .value
                    .currentChat
                    ?.takeIf { current -> current.chatId.equals(chat.chatId, ignoreCase = true) }
                    ?.messages
                    ?.firstOrNull { entry ->
                        entry.isOutgoing &&
                            entry.body == message &&
                            entry.expiresAtSecs != null &&
                            entry.delivery != DeliveryState.QUEUED &&
                            entry.delivery != DeliveryState.PENDING
                    }
            }

        waitForRelayDrainIfRequested()
        reportStatus(
            "chat_id" to chat.chatId,
            "message" to message,
            "message_id" to sent.id,
            "expires_at_secs" to (sent.expiresAtSecs ?: expiresAtSecs).toString(),
            "delivery" to sent.delivery.name,
        )
    }

    @Test
    fun wait_for_message_absent_from_args() {
        ensureLoggedIn()
        val peerInput = optionalArg("peer_input").orEmpty()
        val chatIdArg = optionalArg("chat_id")
        val expectedMessage = requiredArg("message")
        val direction = optionalArg("direction")?.lowercase() ?: "any"
        val timeoutMs =
            optionalArg("timeout_ms")?.toLongOrNull()
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
        val chat =
            chatIdArg
                ?.let { ensureChatOpenById(it) }
                ?: ensureChatOpen(peerInput)

        waitForState("message absent", timeoutMs = timeoutMs) {
            true.takeIf { countMessages(chat.chatId, expectedMessage, direction) == 0 }
        }
        reportStatus(
            "chat_id" to chat.chatId,
            "message" to expectedMessage,
            "absent" to "true",
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
                ?: ((optionalArg("timeout_secs")?.toLongOrNull() ?: 60L) * 1_000L)
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
        val timeoutMs = optionalArg("timeout_ms")?.toLong() ?: 60_000L

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


}
