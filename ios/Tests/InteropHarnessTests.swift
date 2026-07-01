import Foundation
import SQLite3
import UserNotifications
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

typealias JsonArray = [Any]
typealias JsonObject = [String: Any]

enum HarnessError: Error, CustomStringConvertible {
    case missingEnv(String)
    case timeout(String)
    case unexpected(String)

    var description: String {
        switch self {
        case .missingEnv(let key):
            return "missing required env: \(key)"
        case .timeout(let label):
            return "timed out waiting for \(label)"
        case .unexpected(let detail):
            return detail
        }
    }
}

@MainActor
final class InteropHarnessTests: XCTestCase {
    let debugSnapshotFilename = "iris_chat_runtime_debug.json"

    func testHarnessAction() async throws {
        let env = ProcessInfo.processInfo.environment
        guard env["IRIS_IOS_HARNESS_ACTION"] != nil else {
            throw XCTSkip("Interop harness runs only via scripts/run_ios_harness.py")
        }
        let action = try requiredEnv("IRIS_IOS_HARNESS_ACTION", env: env)
        let runID = env["IRIS_IOS_HARNESS_RUN_ID"] ?? UUID().uuidString
        let useAppStorage = env["IRIS_IOS_HARNESS_USE_APP_STORAGE"] == "1"
        let service = env["IRIS_IOS_HARNESS_SERVICE"] ?? (
            useAppStorage ? "fi.siriusbusiness.irischat" : "fi.siriusbusiness.irischat.harness.\(runID)"
        )
        let account = "stored-account-bundle"
        let dataDir = useAppStorage
            ? AppPaths.dataDir(fileManager: .default, environment: [:])
            : isolatedHarnessDataDir(runID: runID, env: env)
        let reset = env["IRIS_IOS_HARNESS_RESET"] == "1"

        let secretStore: AccountSecretStore
        if useAppStorage || env["IRIS_IOS_HARNESS_USE_KEYCHAIN"] == "1" {
            secretStore = KeychainSecretStore(service: service, account: account)
        } else {
            secretStore = FileAccountSecretStore(
                url: dataDir.appendingPathComponent("account-secret.json"),
                fileManager: .default
            )
        }
        if reset {
            secretStore.clear()
            try? FileManager.default.removeItem(at: dataDir)
        }
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)

        let managerEnvironment = harnessManagerEnvironment(runID: runID)
        let manager = AppManager(
            secretStore: secretStore,
            dataDir: dataDir,
            environment: managerEnvironment
        )

        _ = try await waitFor(label: "bootstrap completion", timeout: 30) {
            manager.bootstrapInFlight ? nil : true
        }

        status("action", action)
        status("run_id", runID)
        status("data_dir", dataDir.path)

        switch action {
        case "create_account_and_report_identity":
            let snapshot = try await createOrLoadAccount(manager: manager, env: env)
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            reportIdentity(snapshot)
        case "report_logged_in_identity":
            let snapshot = try await ensureLoggedIn(manager: manager, env: env)
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            reportIdentity(snapshot)
        case "report_device_roster_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let roster: DeviceRosterSnapshot = try await waitFor(label: "device roster snapshot", timeout: 90) {
                manager.state.deviceRoster
            }
            reportDeviceRoster(roster)
        case "export_secret_key":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            guard let secretKey = manager.exportOwnerNsec() else {
                throw HarnessError.unexpected("secret key unavailable")
            }
            status("secret_key", secretKey)
        case "restore_session_from_args":
            let secretKey = try requiredEnv("IRIS_IOS_HARNESS_SECRET_KEY", env: env)
            let expectedPublicKeyHex = env["IRIS_IOS_HARNESS_EXPECTED_PUBLIC_KEY_HEX"]?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            manager.restoreSession(ownerNsec: secretKey)
            let restored: AccountSnapshot = try await waitFor(label: "restored account", timeout: 90) {
                guard let account = manager.state.account else {
                    return nil
                }
                if let expectedPublicKeyHex, !expectedPublicKeyHex.isEmpty,
                   account.publicKeyHex.caseInsensitiveCompare(expectedPublicKeyHex) != .orderedSame {
                    return nil
                }
                return account
            }
            if let toast = manager.state.toast,
               !toast.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                throw HarnessError.unexpected("Restore failed: \(toast)")
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            reportIdentity(restored)
            status("display_name", restored.displayName)
        case "wait_for_account_display_name_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let expectedName = try requiredEnv("IRIS_IOS_HARNESS_DISPLAY_NAME", env: env)
            let account: AccountSnapshot = try await waitFor(label: "account display name \(expectedName)", timeout: 180) {
                manager.state.account?.displayName == expectedName ? manager.state.account : nil
            }
            status("public_key_hex", account.publicKeyHex)
            status("display_name", account.displayName)
        case "update_profile_metadata_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let displayName = try requiredEnv("IRIS_IOS_HARNESS_DISPLAY_NAME", env: env)
            manager.updateProfileMetadata(name: displayName, pictureURL: env["IRIS_IOS_HARNESS_PICTURE_URL"], about: nil)
            let updated = try await waitFor(label: "profile metadata applied", timeout: 60) {
                manager.state.account?.displayName == displayName ? manager.state.account : nil
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("display_name", updated.displayName)
            status("public_key_hex", updated.publicKeyHex)
        case "start_linked_device_and_report_identity":
            try await applyHarnessRelaysIfProvided(manager: manager, env: env)
            let ownerInput = env["IRIS_IOS_HARNESS_OWNER_INPUT"] ?? ""
            manager.startLinkedDevice(ownerInput: ownerInput)
            let link = try await waitFor(label: "linked-device invite", timeout: 90) {
                manager.state.linkDevice
            }
            status("link_url", link.url)
            status("device_input", link.deviceInput)
        case "start_linked_device_wait_authorized_from_args":
            try await applyHarnessRelaysIfProvided(manager: manager, env: env)
            let ownerInput = env["IRIS_IOS_HARNESS_OWNER_INPUT"] ?? ""
            manager.startLinkedDevice(ownerInput: ownerInput)
            let link = try await waitFor(label: "linked-device invite", timeout: 90) {
                manager.state.linkDevice
            }
            status("link_url", link.url)
            status("device_input", link.deviceInput)
            let authorizationTimeout = TimeInterval(
                Double(env["IRIS_IOS_HARNESS_AUTHORIZATION_TIMEOUT_SECS"] ?? "") ?? 240
            )
            let snapshot: AccountSnapshot = try await waitFor(label: "linked-device authorization", timeout: authorizationTimeout) {
                guard let account = manager.state.account else {
                    return nil
                }
                let actual = String(describing: account.authorizationState).lowercased()
                return actual == "authorized" ? account : nil
            }
            reportIdentity(snapshot)
        case "add_authorized_device_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let deviceInput = try requiredEnv("IRIS_IOS_HARNESS_DEVICE_INPUT", env: env)
            let initialDeviceCount = manager.state.deviceRoster?.devices.count ?? 0
            manager.addAuthorizedDevice(deviceInput: deviceInput)
            _ = try await waitFor(label: "device roster update", timeout: 90) {
                let currentDeviceCount = manager.state.deviceRoster?.devices.count ?? 0
                if currentDeviceCount > initialDeviceCount {
                    return true
                }
                if let toast = manager.state.toast,
                   !toast.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    return true
                }
                return nil
            }
            if let roster = manager.state.deviceRoster {
                status("device_count", String(roster.devices.count))
                status("devices", roster.devices.map { $0.devicePubkeyHex }.joined(separator: ","))
            }
            status("toast", manager.state.toast ?? "")
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
        case "wait_for_authorization_state_from_args":
            let expected = try requiredEnv("IRIS_IOS_HARNESS_AUTHORIZATION_STATE", env: env).lowercased()
            let snapshot: AccountSnapshot = try await waitFor(label: "authorization state \(expected)", timeout: 180) {
                guard let account = manager.state.account else {
                    return nil
                }
                let actual = String(describing: account.authorizationState).lowercased()
                return actual == expected ? account : nil
            }
            reportIdentity(snapshot)
        case "remove_authorized_device_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let deviceInput = try requiredEnv("IRIS_IOS_HARNESS_DEVICE_INPUT", env: env)
            let normalizedDevice = normalizePeerInput(input: deviceInput)
            let initialRev = manager.state.rev
            manager.removeAuthorizedDevice(devicePubkeyHex: normalizedDevice)
            let roster = try await waitFor(label: "removed authorized device \(normalizedDevice)", timeout: 5) {
                manager.state.deviceRoster.flatMap { roster in
                    let stillListed = roster.devices.contains { device in
                        self.sameIdentifier(device.devicePubkeyHex, normalizedDevice)
                    }
                    return stillListed ? nil : roster
                }
            }
            status("device_pubkey_hex", normalizedDevice)
            status("device_removed", String(!roster.devices.contains { self.sameIdentifier($0.devicePubkeyHex, normalizedDevice) }))
            status("device_stale", String(roster.devices.first(where: { self.sameIdentifier($0.devicePubkeyHex, normalizedDevice) })?.isStale ?? false))
            status("device_count", String(roster.devices.count))
            status("devices", roster.devices.map { device in
                [
                    device.devicePubkeyHex,
                    device.deviceNpub,
                    String(device.isCurrentDevice),
                    String(device.isAuthorized),
                    String(device.isStale),
                ].joined(separator: ",")
            }.joined(separator: "|"))
            status("state_rev_changed", String(manager.state.rev > initialRev))
        case "wait_for_revoked_state":
            let snapshot: AccountSnapshot = try await waitFor(label: "revoked device state", timeout: 180) {
                guard let account = manager.state.account else {
                    return nil
                }
                return String(describing: account.authorizationState).lowercased() == "revoked" ? account : nil
            }
            reportIdentity(snapshot)
        case "create_public_invite_and_report_url":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            manager.dispatch(.createPublicInvite)
            let invite = try await waitFor(label: "public invite", timeout: 90) {
                manager.state.publicInvite
            }
            status("invite_url", invite.url)
        case "accept_invite_and_send_message_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let inviteURL = try requiredEnv("IRIS_IOS_HARNESS_INVITE_URL", env: env)
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let expectedChatID = env["IRIS_IOS_HARNESS_EXPECTED_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)

            manager.dispatch(.acceptInvite(inviteInput: inviteURL))
            var acceptedChat: CurrentChatSnapshot? = nil
            let acceptDeadline = Date().addingTimeInterval(180)
            while Date() < acceptDeadline {
                let state = manager.state
                if !state.busy.acceptingInvite,
                   let toast = state.toast,
                   !toast.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    throw HarnessError.unexpected("Invite accept failed: \(toast)")
                }
                if !state.busy.acceptingInvite,
                   let current = state.currentChat,
                   !current.chatId.isEmpty {
                    if let expectedChatID, !expectedChatID.isEmpty,
                       self.sameIdentifier(current.chatId, expectedChatID) == false {
                        try await Task.sleep(nanoseconds: 200_000_000)
                        continue
                    }
                    acceptedChat = current
                    break
                }
                try await Task.sleep(nanoseconds: 200_000_000)
            }
            guard let chat = acceptedChat else {
                throw HarnessError.timeout("accepted invite")
            }

            manager.dispatch(.sendMessage(chatId: chat.chatId, text: message))
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let finalizedDelivery: String = try await waitFor(label: "invite chat message publish", timeout: timeout) { () -> String? in
                guard let current = manager.state.currentChat else {
                    return nil
                }
                guard self.sameIdentifier(current.chatId, chat.chatId) else {
                    return nil
                }
                let messageEntry = current.messages.first { entry in
                    let isFinal = entry.delivery != .queued && entry.delivery != .pending
                    return entry.isOutgoing && entry.body == message && isFinal
                }
                guard let messageEntry else { return nil }
                return String(describing: messageEntry.delivery)
            }
            if finalizedDelivery.caseInsensitiveCompare("failed") == .orderedSame {
                throw HarnessError.unexpected("Invite chat message failed to publish")
            }
            status("chat_id", chat.chatId)
            status("message", message)
            status("delivery", finalizedDelivery)
        case "enable_nearby_and_report_peers":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            manager.nearbyIris.setVisible(true)
            try await Task.sleep(nanoseconds: 1_000_000_000)
            reportNearbySnapshot(manager: manager)
        case "enable_lan_nearby_and_report_peers":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            manager.nearbyIris.setVisible(false)
            manager.nearbyIris.setLanVisible(true)
            try await Task.sleep(nanoseconds: 1_000_000_000)
            reportNearbySnapshot(manager: manager)
        case "wait_for_nearby_peer_profile_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(
                manager: manager,
                peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env)
            )
            manager.nearbyIris.setVisible(true)
            let timeout = nearbyProfileTimeout(env: env)
            let peer = try await waitFor(label: "nearby peer profile \(peerOwnerHex)", timeout: timeout) {
                manager.nearbyIris.peers.first(where: { nearby in
                    nearby.ownerPubkeyHex?.caseInsensitiveCompare(peerOwnerHex) == .orderedSame
                })
            }
            status("nearby_visible", String(manager.nearbyIris.isVisible))
            status("nearby_status", manager.nearbyIris.status)
            status("nearby_peer_count", String(manager.nearbyIris.peers.count))
            status("nearby_peer_id", peer.id)
            status("nearby_peer_name", peer.name)
            status("nearby_peer_owner_hex", peer.ownerPubkeyHex ?? "")
            status("nearby_peer_profile_event_id", peer.profileEventID ?? "")
            try await holdNearbyIfRequested(env: env)
        case "wait_for_lan_nearby_peer_profile_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(
                manager: manager,
                peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env)
            )
            manager.nearbyIris.setVisible(false)
            manager.nearbyIris.setLanVisible(true)
            let timeout = nearbyProfileTimeout(env: env)
            let peer = try await waitFor(label: "LAN nearby peer profile \(peerOwnerHex)", timeout: timeout) {
                manager.nearbyIris.peers.first(where: { nearby in
                    nearby.ownerPubkeyHex?.caseInsensitiveCompare(peerOwnerHex) == .orderedSame
                })
            }
            status("nearby_visible", String(manager.nearbyIris.isVisible))
            status("nearby_status", manager.nearbyIris.status)
            status("nearby_lan_visible", String(manager.nearbyIris.isLanVisible))
            status("nearby_lan_status", manager.nearbyIris.lanStatus)
            status("nearby_peer_count", String(manager.nearbyIris.peers.count))
            status("nearby_peer_id", peer.id)
            status("nearby_peer_name", peer.name)
            status("nearby_peer_owner_hex", peer.ownerPubkeyHex ?? "")
            status("nearby_peer_profile_event_id", peer.profileEventID ?? "")
            try await holdNearbyIfRequested(env: env)
        case "report_runtime_debug_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            let settledLiveDebug = try await waitForRuntimeSnapshotIdleIfRequested(manager: manager, env: env)
            await reportRuntimeDebugSnapshot(manager: manager, dataDir: dataDir, liveDebugOverride: settledLiveDebug)
        case "report_persisted_protocol_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            reportPersistedProtocolSnapshot(dataDir: dataDir)
        case "report_mobile_push_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            reportMobilePushSnapshot(manager: manager)
        case "decrypt_notification_payload_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try decryptNotificationPayloadFromArgs(
                secretStore: secretStore,
                dataDir: dataDir,
                env: env
            )
        case "report_mobile_push_server_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await reportMobilePushServerSnapshot(manager: manager)
        case "clear_delivered_notifications":
            UNUserNotificationCenter.current().removeAllDeliveredNotifications()
            status("cleared", "true")
        case "assert_no_visible_delivered_notifications":
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 15)
            let delivered = try await waitForNoVisibleDeliveredNotifications(timeout: timeout)
            status("visible_notification_count", String(delivered.count))
            status("delivered_notifications", summarizeDeliveredNotifications(delivered))
        case "wait_for_visible_delivered_notification":
            let expectedBody = env["IRIS_IOS_HARNESS_EXPECTED_BODY"] ?? ""
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 30)
            let notification = try await waitForVisibleDeliveredNotification(
                expectedBody: expectedBody,
                timeout: timeout
            )
            status("notification_title", notification.request.content.title)
            status("notification_body", notification.request.content.body)
            status("notification_id", notification.request.identifier)
        case "wait_for_peer_roster_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env))
            let readiness = try await waitFor(label: "peer roster \(peerOwnerHex)", timeout: 180) { () -> (source: String, users: String)? in
                if self.splitPersistenceHasPeerRoster(dataDir: dataDir, peerOwnerHex: peerOwnerHex) {
                    return (
                        "persisted",
                        self.summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex)
                    )
                }
                if let debug = self.readRuntimeDebugSnapshot(dataDir: dataDir),
                   self.runtimeDebugHasPeerRoster(debug, peerOwnerHex: peerOwnerHex) {
                    return (
                        "runtime",
                        self.summarizeRuntimeKnownUsers(self.arrayValue(debug["known_users"]))
                    )
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("source", readiness.source)
            status("users", readiness.users)
        case "wait_for_known_peer_session_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env))
            let readiness = try await waitFor(label: "known peer session \(peerOwnerHex)", timeout: 180) { () -> (source: String, users: String)? in
                if self.splitPersistenceHasPeerSession(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex) {
                    return (
                        "persisted",
                        self.summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex)
                    )
                }
                if let debug = self.readRuntimeDebugSnapshot(dataDir: dataDir),
                   self.runtimeDebugHasPeerSession(debug, peerOwnerHex: peerOwnerHex) {
                    return (
                        "runtime",
                        self.summarizeRuntimeKnownUsers(self.arrayValue(debug["known_users"]))
                    )
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("source", readiness.source)
            status("users", readiness.users)
        case "wait_for_peer_transport_ready_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env))
            let readiness = try await waitFor(label: "peer transport ready \(peerOwnerHex)", timeout: 180) { () -> (source: String, users: String)? in
                if self.splitPersistenceHasPeerSession(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex) {
                    return (
                        "persisted",
                        self.summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex)
                    )
                }
                if let debug = self.readRuntimeDebugSnapshot(dataDir: dataDir),
                   self.runtimeDebugHasPeerTransportReady(debug, peerOwnerHex: peerOwnerHex) {
                    return (
                        "runtime",
                        self.summarizeRuntimeKnownUsers(self.arrayValue(debug["known_users"]))
                    )
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("source", readiness.source)
            status("users", readiness.users)
        case "wait_for_peer_profile_name_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerInput = env["IRIS_IOS_HARNESS_PEER_INPUT"] ?? env["IRIS_IOS_HARNESS_PEER_PUBKEY_HEX"] ?? ""
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: peerInput)
            let expectedName = try requiredEnv("IRIS_IOS_HARNESS_DISPLAY_NAME", env: env)
            let resolvedName = try await waitFor(label: "peer profile \(peerOwnerHex) == \(expectedName)", timeout: 60) {
                if let current = manager.state.currentChat,
                   self.sameIdentifier(current.chatId, peerOwnerHex),
                   current.displayName == expectedName {
                    return current.displayName
                }
                if let thread = manager.state.chatList.first(where: { self.sameIdentifier($0.chatId, peerOwnerHex) }),
                   thread.displayName == expectedName {
                    return thread.displayName
                }
                return nil
            }
            status("peer_pubkey_hex", peerOwnerHex)
            status("display_name", resolvedName)
        case "create_chat_from_args":
            let rawPeer = try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env)
            let chatID = try await ensureChatOpen(manager: manager, dataDir: dataDir, chatID: nil, peerInput: rawPeer)
            let subtitle =
                manager.state.currentChat?.subtitle ??
                manager.state.chatList.first(where: { self.sameIdentifier($0.chatId, chatID) })?.subtitle ??
                (rawPeer.lowercased().hasPrefix("npub1") ? rawPeer : "")
            status("chat_id", chatID)
            status("peer_npub", subtitle)
        case "send_message_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            manager.dispatch(.sendMessage(chatId: chatID, text: message))

            let waitForDeliveryRaw = (env["IRIS_IOS_HARNESS_WAIT_FOR_DELIVERY"] ?? "true")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            if ["0", "false", "no"].contains(waitForDeliveryRaw) {
                let localDelivery = try await waitFor(label: "queued outgoing message \(message)", timeout: 10) {
                    if let current = manager.state.currentChat,
                       self.sameIdentifier(current.chatId, chatID),
                       let messageEntry = current.messages.first(where: { $0.isOutgoing && $0.body == message }) {
                        return String(describing: messageEntry.delivery)
                    }
                    if let delivery = self.splitPersistenceMessageDelivery(
                        dataDir: dataDir,
                        chatID: chatID,
                        message: message,
                        direction: "outgoing"
                    ) {
                        return delivery
                    }
                    return nil
                }
                status("chat_id", chatID)
                status("message", message)
                status("delivery", localDelivery)
                break
            }

            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let finalizedDelivery = try await waitFor(label: "outgoing message \(message)", timeout: timeout) {
                if let current = manager.state.currentChat,
                   self.sameIdentifier(current.chatId, chatID),
                   let messageEntry = current.messages.first(where: { $0.isOutgoing && $0.body == message && $0.delivery != .queued && $0.delivery != .pending }) {
                    return String(describing: messageEntry.delivery)
                }
                if let delivery = self.splitPersistenceMessageDelivery(
                    dataDir: dataDir,
                    chatID: chatID,
                    message: message,
                    direction: "outgoing"
                ) {
                    return delivery
                }
                return nil
            }

            if finalizedDelivery.caseInsensitiveCompare("failed") == .orderedSame {
                throw HarnessError.unexpected("outgoing message failed to publish")
            }

            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("message", message)
            status("delivery", finalizedDelivery)
        case "send_nearby_message_from_args":
            try await maybeDisableRelays(manager: manager, env: env)
            manager.nearbyIris.setVisible(true)
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            manager.dispatch(.sendMessage(chatId: chatID, text: message))
            let delivery = try await waitFor(label: "nearby outgoing message \(message)", timeout: 30) {
                manager.state.currentChat?
                    .messages
                    .first(where: { $0.isOutgoing && $0.body == message })
                    .map { String(describing: $0.delivery) }
            }
            status("chat_id", chatID)
            status("message", message)
            status("delivery", delivery)
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
        case "disable_relays_and_report":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await disableRelays(manager: manager)
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
            status("relays", manager.state.preferences.nostrRelayUrls.joined(separator: "|"))
        case "reset_relays_and_report":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            manager.dispatch(.resetNostrRelays)
            _ = try await waitFor(label: "reset relays", timeout: 30) {
                manager.state.preferences.nostrRelayUrls.count >= 3 ? true : nil
            }
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
            status("relays", manager.state.preferences.nostrRelayUrls.joined(separator: "|"))
        case "add_relay_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let relayURL = normalizedHarnessRelayURL(try requiredEnv("IRIS_IOS_HARNESS_RELAY_URL", env: env))
            manager.dispatch(.addNostrRelay(relayUrl: relayURL))
            _ = try await waitFor(label: "added relay \(relayURL)", timeout: 30) {
                manager.state.preferences.nostrRelayUrls.contains(relayURL) ? true : nil
            }
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
            status("relays", manager.state.preferences.nostrRelayUrls.joined(separator: "|"))
        case "set_relays_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let relayURLs = parseList(env["IRIS_IOS_HARNESS_RELAY_URLS"] ?? env["IRIS_IOS_HARNESS_RELAY_URL"] ?? "")
                .map(normalizedHarnessRelayURL)
            manager.dispatch(.setNostrRelays(relayUrls: relayURLs))
            _ = try await waitFor(label: "set relays", timeout: 30) {
                manager.state.preferences.nostrRelayUrls == relayURLs ? true : nil
            }
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
            status("relays", manager.state.preferences.nostrRelayUrls.joined(separator: "|"))
        case "wait_for_connected_relay":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 30)
            let networkStatus: NetworkStatusSnapshot = try await waitFor(label: "connected relay", timeout: timeout) {
                guard let status = manager.state.networkStatus, status.connectedRelayCount > 0 else {
                    return nil
                }
                return status
            }
            status("network_connected_relay_count", String(networkStatus.connectedRelayCount))
            status("network_relay_urls", networkStatus.relayUrls.joined(separator: ","))
            status("network_relay_connections", summarizeRelayConnections(networkStatus.relayConnections))
        case "nearby_chat_exchange_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await maybeDisableRelays(manager: manager, env: env)
            let peerInput = try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env)
            let role = (env["IRIS_IOS_HARNESS_ROLE"] ?? "initiator").lowercased()
            let count = min(max(Int(env["IRIS_IOS_HARNESS_COUNT"] ?? "") ?? 10, 1), 50)
            let prefix = env["IRIS_IOS_HARNESS_PREFIX"] ?? "nearby"
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: peerInput)

            manager.nearbyIris.setVisible(true)
            _ = try await waitFor(label: "nearby peer \(peerOwnerHex)", timeout: 60) {
                manager.nearbyIris.peers.first(where: { peer in
                    peer.ownerPubkeyHex?.caseInsensitiveCompare(peerOwnerHex) == .orderedSame
                })
            }

            let chatID = try await ensureChatOpen(manager: manager, dataDir: dataDir, chatID: nil, peerInput: peerInput)
            let startedAt = Date()
            var sent = 0
            var received = 0
            for index in 1...count {
                let message = "\(prefix)-\(index)"
                let shouldSend = (role == "initiator") == (index % 2 == 1)
                if shouldSend {
                    manager.dispatch(.sendMessage(chatId: chatID, text: message))
                    _ = try await waitFor(label: "outgoing \(message)", timeout: 30) {
                        self.messageExists(
                            manager: manager,
                            dataDir: dataDir,
                            chatID: chatID,
                            message: message,
                            direction: "outgoing",
                            peerInput: peerInput
                        ) ? true : nil
                    }
                    sent += 1
                } else {
                    _ = try await waitFor(label: "incoming \(message)", timeout: 60) {
                        self.messageExists(
                            manager: manager,
                            dataDir: dataDir,
                            chatID: chatID,
                            message: message,
                            direction: "incoming",
                            peerInput: peerInput
                        ) ? true : nil
                    }
                    received += 1
                }
            }
            status("chat_id", chatID)
            status("role", role)
            status("sent", String(sent))
            status("received", String(received))
            status("elapsed_ms", String(Int(Date().timeIntervalSince(startedAt) * 1000)))
            status("relay_count", String(manager.state.preferences.nostrRelayUrls.count))
        case "wait_for_message_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "any").lowercased()
            let expectedCount = Int(env["IRIS_IOS_HARNESS_EXPECTED_COUNT"] ?? "")
            let requestedChatID = env["IRIS_IOS_HARNESS_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let peerInput = env["IRIS_IOS_HARNESS_PEER_INPUT"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let expectedChatID = requestedChatID?.isEmpty == false ? requestedChatID : nil
            let seededChatID: String?
            if expectedChatID != nil || peerInput?.isEmpty == false {
                seededChatID = try await ensureChatOpen(
                    manager: manager,
                    dataDir: dataDir,
                    chatID: expectedChatID,
                    peerInput: peerInput
                )
            } else {
                seededChatID = nil
            }
            let resolvedChatID = expectedChatID ?? seededChatID

            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let matchedChatID = try await waitFor(label: "message \(message)", timeout: timeout) {
                let state = manager.state
                if let current = state.currentChat,
                   self.chatMatchesExpectedChat(chatId: current.chatId, peerInput: peerInput, expectedChatID: resolvedChatID),
                   current.messages.contains(where: { $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction) }) {
                    return current.chatId
                }
                if let thread = state.chatList.first(where: {
                    $0.lastMessagePreview == message &&
                    self.chatMatchesExpectedChat(chatId: $0.chatId, peerInput: peerInput, expectedChatID: resolvedChatID)
                }) {
                    return thread.chatId
                }
                if let chatID = self.splitPersistenceThreadWithMessage(
                    dataDir: dataDir,
                    chatID: resolvedChatID,
                    expectedMessage: message,
                    direction: direction,
                    peerInput: peerInput
                ) {
                    return chatID
                }
                if let chatID = self.sqliteThreadWithMessage(
                    dataDir: dataDir,
                    chatID: resolvedChatID,
                    expectedMessage: message,
                    direction: direction,
                    peerInput: peerInput
                ) {
                    return chatID
                }
                return nil
            }

            manager.dispatch(.openChat(chatId: matchedChatID))
            if let expectedCount {
                try await Task.sleep(nanoseconds: 5_000_000_000)
                let actualCount = countMessages(
                    manager: manager,
                    dataDir: dataDir,
                    chatID: matchedChatID,
                    message: message,
                    direction: direction,
                    peerInput: peerInput
                )
                guard actualCount == expectedCount else {
                    throw HarnessError.unexpected(
                        "expected \(expectedCount) matching message(s), found \(actualCount) for `\(message)`"
                    )
                }
            }
            let matchingCount = countMessages(
                manager: manager,
                dataDir: dataDir,
                chatID: matchedChatID,
                message: message,
                direction: direction,
                peerInput: peerInput
            )
            status("chat_id", matchedChatID)
            status("message", message)
            status("matching_count", String(matchingCount))
        case "send_typing_from_args":
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            manager.dispatch(.setTypingIndicatorsEnabled(enabled: true))
            manager.dispatch(.sendTyping(chatId: chatID))
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("sent_typing", "true")
        case "wait_for_typing_from_args":
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 60)
            status("chat_id", chatID)
            status("typing_wait_ready", "true")
            let typing: [TypingIndicatorSnapshot] = try await waitFor(label: "typing indicator \(chatID)", timeout: timeout) {
                if let current = manager.state.currentChat,
                   self.sameIdentifier(current.chatId, chatID),
                   !current.typingIndicators.isEmpty {
                    return current.typingIndicators
                }
                if manager.state.chatList.contains(where: {
                    self.sameIdentifier($0.chatId, chatID) && $0.isTyping
                }) {
                    return []
                }
                return nil
            }
            status("chat_id", chatID)
            status("typing_count", String(typing.count))
            status("typing", "true")
        case "accept_message_request_from_args":
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            manager.dispatch(.setMessageRequestAccepted(chatId: chatID))
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            let accepted: Bool = try await waitFor(label: "message request accepted \(chatID)", timeout: 30) {
                if let current = manager.state.currentChat,
                   self.sameIdentifier(current.chatId, chatID) {
                    return current.isRequest ? nil : true
                }
                if let thread = manager.state.chatList.first(where: {
                    self.sameIdentifier($0.chatId, chatID)
                }) {
                    return thread.isRequest ? nil : true
                }
                return nil
            }
            status("chat_id", chatID)
            status("accepted", String(accepted))
        case "mark_message_seen_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "incoming").lowercased()
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let ids: [String] = try await waitFor(label: "message ids for seen \(message)", timeout: 60) {
                guard let current = manager.state.currentChat,
                      self.sameIdentifier(current.chatId, chatID) else {
                    return nil
                }
                let matching = current.messages.filter {
                    $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
                }.map(\.id)
                return matching.isEmpty ? nil : matching
            }
            manager.dispatch(.markMessagesSeen(chatId: chatID, messageIds: ids))
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("message", message)
            status("message_ids", ids.joined(separator: ","))
            status("seen", "true")
        case "wait_for_message_delivery_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let expectedDelivery = (env["IRIS_IOS_HARNESS_DELIVERY"] ?? "seen").lowercased()
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "outgoing").lowercased()
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let delivery: String = try await waitFor(label: "message delivery \(expectedDelivery) \(message)", timeout: timeout) {
                guard let current = manager.state.currentChat,
                      self.sameIdentifier(current.chatId, chatID) else {
                    return nil
                }
                guard let entry = current.messages.first(where: {
                    $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
                }) else {
                    return nil
                }
                if String(describing: entry.delivery).lowercased() == expectedDelivery {
                    return String(describing: entry.delivery)
                }
                if let recipient = entry.recipientDeliveries.first(where: {
                    String(describing: $0.delivery).lowercased() == expectedDelivery
                }) {
                    return String(describing: recipient.delivery)
                }
                return nil
            }
            status("chat_id", chatID)
            status("message", message)
            status("delivery", delivery)
        case "react_to_message_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let emoji = env["IRIS_IOS_HARNESS_EMOJI"] ?? "❤️"
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "incoming").lowercased()
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let messageID: String = try await waitFor(label: "message to react \(message)", timeout: 60) {
                manager.state.currentChat?.messages.first(where: {
                    $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
                })?.id
            }
            manager.dispatch(.toggleReaction(chatId: chatID, messageId: messageID, emoji: emoji))
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("message", message)
            status("message_id", messageID)
            status("emoji", emoji)
        case "wait_for_message_reaction_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let emoji = env["IRIS_IOS_HARNESS_EMOJI"] ?? "❤️"
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "any").lowercased()
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let reaction: MessageReactionSnapshot = try await waitFor(label: "reaction \(emoji) on \(message)", timeout: timeout) {
                manager.state.currentChat?.messages.first(where: {
                    $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
                })?.reactions.first(where: { $0.emoji == emoji && $0.count > 0 })
            }
            status("chat_id", chatID)
            status("message", message)
            status("emoji", reaction.emoji)
            status("reaction_count", String(reaction.count))
            status("reacted_by_me", String(reaction.reactedByMe))
        case "set_chat_settings_from_args":
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            if let muted = optionalBoolEnv("IRIS_IOS_HARNESS_MUTED", env: env) {
                manager.dispatch(.setChatMuted(chatId: chatID, muted: muted))
            }
            if let pinned = optionalBoolEnv("IRIS_IOS_HARNESS_PINNED", env: env) {
                manager.dispatch(.setChatPinned(chatId: chatID, pinned: pinned))
            }
            if let ttlRaw = env["IRIS_IOS_HARNESS_TTL_SECONDS"]?.trimmingCharacters(in: .whitespacesAndNewlines),
               !ttlRaw.isEmpty {
                manager.dispatch(.setChatMessageTtl(chatId: chatID, ttlSeconds: UInt64(ttlRaw)))
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            let settings = try await waitForChatSettings(manager: manager, chatID: chatID, env: env, timeout: 30)
            status("chat_id", chatID)
            status("muted", String(settings.muted))
            status("pinned", String(settings.pinned))
            status("ttl_seconds", settings.ttl.map { String($0) } ?? "")
        case "wait_for_chat_settings_from_args":
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let settings = try await waitForChatSettings(manager: manager, chatID: chatID, env: env, timeout: 60)
            status("chat_id", chatID)
            status("muted", String(settings.muted))
            status("pinned", String(settings.pinned))
            status("ttl_seconds", settings.ttl.map { String($0) } ?? "")
        case "send_disappearing_message_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let ttl = UInt64(env["IRIS_IOS_HARNESS_TTL_SECONDS"] ?? "") ?? 8
            let expiresAt = UInt64(Date().timeIntervalSince1970.rounded()) + ttl
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            manager.dispatch(.sendDisappearingMessage(chatId: chatID, text: message, expiresAtSecs: expiresAt))
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let entry: ChatMessageSnapshot = try await waitFor(label: "disappearing message \(message)", timeout: timeout) {
                manager.state.currentChat?.messages.first(where: {
                    $0.body == message && $0.isOutgoing && $0.expiresAtSecs != nil &&
                        $0.delivery != .queued && $0.delivery != .pending
                })
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("message", message)
            status("message_id", entry.id)
            status("expires_at_secs", String(entry.expiresAtSecs ?? expiresAt))
            status("delivery", String(describing: entry.delivery))
        case "wait_for_message_absent_from_args":
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let direction = (env["IRIS_IOS_HARNESS_DIRECTION"] ?? "any").lowercased()
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 60)
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            _ = try await waitFor(label: "message absent \(message)", timeout: timeout) {
                self.countMessages(
                    manager: manager,
                    dataDir: dataDir,
                    chatID: chatID,
                    message: message,
                    direction: direction,
                    peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
                ) == 0 ? true : nil
            }
            status("chat_id", chatID)
            status("message", message)
            status("absent", "true")
        case "create_group_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupName = try requiredEnv("IRIS_IOS_HARNESS_GROUP_NAME", env: env)
            let memberInputs = parseList(env["IRIS_IOS_HARNESS_MEMBER_INPUTS"] ?? "")

            manager.dispatch(.createGroup(name: groupName, memberInputs: memberInputs))
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "group \(groupName)", timeout: timeout) {
                manager.state.currentChat.flatMap { current in
                    current.groupId != nil && current.displayName == groupName ? current : nil
                }
            }

            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chat.chatId)
            status("group_id", chat.groupId ?? "")
            status("group_name", chat.displayName)
            status("member_count", String(chat.memberCount))
        case "wait_for_group_chat_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let chatID = try requiredEnv("IRIS_IOS_HARNESS_CHAT_ID", env: env)
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            _ = try await waitFor(label: "group thread \(chatID)", timeout: timeout) {
                manager.state.chatList.first(where: { self.sameIdentifier($0.chatId, chatID) })
            }
            manager.dispatch(.openChat(chatId: chatID))
            let chat = try await waitFor(label: "open group chat \(chatID)", timeout: 30) {
                manager.state.currentChat.flatMap { current in
                    self.sameIdentifier(current.chatId, chatID) ? current : nil
                }
            }
            status("chat_id", chat.chatId)
            status("group_id", chat.groupId ?? "")
            status("group_name", chat.displayName)
            status("member_count", String(chat.memberCount))
        case "wait_for_group_member_count_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let chatID = try requiredEnv("IRIS_IOS_HARNESS_CHAT_ID", env: env)
            let expectedMemberCount = UInt64(try requiredEnv("IRIS_IOS_HARNESS_MEMBER_COUNT", env: env)) ?? 0
            _ = try await ensureChatOpen(manager: manager, dataDir: dataDir, chatID: chatID, peerInput: nil)
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "group member count \(expectedMemberCount)", timeout: timeout) {
                manager.state.currentChat.flatMap { current in
                    self.sameIdentifier(current.chatId, chatID) && current.memberCount == expectedMemberCount ? current : nil
                }
            }
            status("chat_id", chat.chatId)
            status("group_id", chat.groupId ?? "")
            status("member_count", String(chat.memberCount))
        case "wait_for_group_name_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let chatID = try requiredEnv("IRIS_IOS_HARNESS_CHAT_ID", env: env)
            let expectedName = try requiredEnv("IRIS_IOS_HARNESS_GROUP_NAME", env: env)
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "group name \(expectedName)", timeout: timeout) {
                manager.state.chatList.first(where: {
                    self.sameIdentifier($0.chatId, chatID) && $0.displayName == expectedName
                })
            }
            manager.dispatch(.openChat(chatId: chat.chatId))
            status("chat_id", chat.chatId)
            status("group_name", chat.displayName)
            status("member_count", String(chat.memberCount))
        case "wait_for_group_admin_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = try requiredEnv("IRIS_IOS_HARNESS_GROUP_ID", env: env)
            let memberInput = normalizePeerInput(input: try requiredEnv("IRIS_IOS_HARNESS_MEMBER_INPUT", env: env))
            let isAdmin = ["1", "true", "yes"].contains((env["IRIS_IOS_HARNESS_IS_ADMIN"] ?? "true").lowercased())
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let details = try await waitForGroupDetails(manager: manager, groupID: groupID, timeout: timeout) { details in
                details.members.contains { member in
                    self.sameIdentifier(member.ownerPubkeyHex, memberInput) && member.isAdmin == isAdmin
                }
            }
            status("group_id", groupID)
            status("member_input", memberInput)
            status("is_admin", String(isAdmin))
            status("revision", String(details.revision))
        case "update_group_name_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = try requiredEnv("IRIS_IOS_HARNESS_GROUP_ID", env: env)
            let groupName = try requiredEnv("IRIS_IOS_HARNESS_GROUP_NAME", env: env)
            manager.dispatch(.updateGroupName(groupId: groupID, name: groupName))
            let chatID = "group:\(groupID)"
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "renamed group \(groupName)", timeout: timeout) {
                manager.state.chatList.first(where: {
                    self.sameIdentifier($0.chatId, chatID) && $0.displayName == groupName
                })
            }
            manager.dispatch(.openChat(chatId: chat.chatId))
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chat.chatId)
            status("group_id", groupID)
            status("group_name", chat.displayName)
            status("member_count", String(chat.memberCount))
        case "expect_group_name_update_rejected_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = try requiredEnv("IRIS_IOS_HARNESS_GROUP_ID", env: env)
            let rejectedName = try requiredEnv("IRIS_IOS_HARNESS_GROUP_NAME", env: env)
            let rawChatID = env["IRIS_IOS_HARNESS_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let chatID = rawChatID.isEmpty ? "group:\(groupID)" : rawChatID
            let rawExpectedName = env["IRIS_IOS_HARNESS_EXPECTED_GROUP_NAME"]?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let expectedName = rawExpectedName.isEmpty ? nil : rawExpectedName
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 30)
            let initialName = manager.state.chatList.first(where: {
                self.sameIdentifier($0.chatId, chatID)
            })?.displayName ?? ""

            manager.dispatch(.updateGroupName(groupId: groupID, name: rejectedName))
            let deadline = Date().addingTimeInterval(timeout)
            var rejectionToast = ""
            while Date() < deadline {
                if manager.state.chatList.contains(where: {
                    self.sameIdentifier($0.chatId, chatID) && $0.displayName == rejectedName
                }) {
                    throw HarnessError.unexpected("Rejected group rename unexpectedly applied \(rejectedName)")
                }
                if let toast = manager.state.toast?.trimmingCharacters(in: .whitespacesAndNewlines),
                   !toast.isEmpty {
                    rejectionToast = toast
                    break
                }
                try await Task.sleep(nanoseconds: 500_000_000)
            }
            let finalName = manager.state.chatList.first(where: {
                self.sameIdentifier($0.chatId, chatID)
            })?.displayName ?? initialName
            if let expectedName, finalName != expectedName {
                throw HarnessError.unexpected("Expected group name \(expectedName), found \(finalName)")
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chatID)
            status("group_id", groupID)
            status("rejected_group_name", rejectedName)
            status("group_name", finalName)
            status("toast", rejectionToast)
            status("rejected", "true")
        case "add_group_members_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = try requiredEnv("IRIS_IOS_HARNESS_GROUP_ID", env: env)
            let chatID = env["IRIS_IOS_HARNESS_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let expectedMemberCount = UInt64(env["IRIS_IOS_HARNESS_EXPECTED_MEMBER_COUNT"] ?? "")
            let memberInputs = parseList(try requiredEnv("IRIS_IOS_HARNESS_MEMBER_INPUTS", env: env))
            manager.dispatch(.addGroupMembers(groupId: groupID, memberInputs: memberInputs))
            let resolvedChatID = chatID?.isEmpty == false ? chatID! : "group:\(groupID)"
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "added group members", timeout: timeout) {
                manager.state.chatList.first(where: { thread in
                    guard self.sameIdentifier(thread.chatId, resolvedChatID) else { return false }
                    guard let expectedMemberCount else { return true }
                    return thread.memberCount == expectedMemberCount
                })
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chat.chatId)
            status("group_id", groupID)
            status("member_count", String(chat.memberCount))
        case "remove_group_member_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = env["IRIS_IOS_HARNESS_GROUP_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let chatID = env["IRIS_IOS_HARNESS_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let resolvedGroupID = groupID?.isEmpty == false ? groupID! : (chatID ?? "").replacingOccurrences(of: "group:", with: "")
            let resolvedChatID = chatID?.isEmpty == false ? chatID! : "group:\(resolvedGroupID)"
            let memberInput = normalizePeerInput(input: try requiredEnv("IRIS_IOS_HARNESS_MEMBER_INPUT", env: env))
            let expectedMemberCount = UInt64(env["IRIS_IOS_HARNESS_EXPECTED_MEMBER_COUNT"] ?? "")
            manager.dispatch(.removeGroupMember(groupId: resolvedGroupID, ownerPubkeyHex: memberInput))
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let chat = try await waitFor(label: "removed group member", timeout: timeout) {
                manager.state.chatList.first(where: { thread in
                    guard self.sameIdentifier(thread.chatId, resolvedChatID) else { return false }
                    guard let expectedMemberCount else { return true }
                    return thread.memberCount == expectedMemberCount
                })
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("chat_id", chat.chatId)
            status("group_id", resolvedGroupID)
            status("member_count", String(chat.memberCount))
        case "set_group_admin_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupID = try requiredEnv("IRIS_IOS_HARNESS_GROUP_ID", env: env)
            let memberInput = normalizePeerInput(input: try requiredEnv("IRIS_IOS_HARNESS_MEMBER_INPUT", env: env))
            let isAdmin = ["1", "true", "yes"].contains((env["IRIS_IOS_HARNESS_IS_ADMIN"] ?? "true").lowercased())
            manager.setGroupAdmin(groupId: groupID, ownerPubkeyHex: memberInput, isAdmin: isAdmin)
            let timeout = harnessTimeout(env: env, defaultSeconds: 60)
            let details = try await waitForGroupDetails(manager: manager, groupID: groupID, timeout: timeout) { details in
                details.members.contains { member in
                    self.sameIdentifier(member.ownerPubkeyHex, memberInput) && member.isAdmin == isAdmin
                }
            }
            try await waitForRelayDrainIfRequested(manager: manager, dataDir: dataDir, env: env)
            status("group_id", groupID)
            status("member_input", memberInput)
            status("is_admin", String(isAdmin))
            status("revision", String(details.revision))
        case "expect_send_rejected_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let initialCount = manager.state.currentChat?.messages.count ?? 0
            manager.dispatch(.sendMessage(chatId: chatID, text: message))
            let toast: String = try await waitFor(label: "rejected send", timeout: 60) { () -> String? in
                guard let current = manager.state.currentChat,
                      self.sameIdentifier(current.chatId, chatID) else {
                    return nil
                }
                if current.messages.count != initialCount || current.messages.contains(where: { $0.body == message }) {
                    return "unexpected-message-appended"
                }
                return manager.state.toast?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                    ? manager.state.toast
                    : nil
            }
            if toast == "unexpected-message-appended" {
                throw HarnessError.unexpected("Rejected send unexpectedly appended \(message)")
            }
            status("chat_id", chatID)
            status("message", message)
            status("toast", toast)
        case "assert_message_absent_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let message = try requiredEnv("IRIS_IOS_HARNESS_MESSAGE", env: env)
            let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_TIMEOUT_MS"] ?? "") ?? 30000) / 1000.0
            let chatID = try await ensureChatOpen(
                manager: manager,
                dataDir: dataDir,
                chatID: env["IRIS_IOS_HARNESS_CHAT_ID"],
                peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]
            )
            let deadline = Date().addingTimeInterval(timeout)
            while Date() < deadline {
                if self.messageExists(manager: manager, dataDir: dataDir, chatID: chatID, message: message, direction: "any", peerInput: env["IRIS_IOS_HARNESS_PEER_INPUT"]) {
                    throw HarnessError.unexpected("message unexpectedly present: \(message)")
                }
                try await Task.sleep(nanoseconds: 500_000_000)
            }
            status("chat_id", chatID)
            status("message", message)
            status("absent", "true")
        case "same_process_multi_device_mesh":
            try await runSameProcessMultiDeviceMesh(env: env, rootDataDir: dataDir)
        default:
            throw HarnessError.unexpected("unknown harness action: \(action)")
        }
    }

}
