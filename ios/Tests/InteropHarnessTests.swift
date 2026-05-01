import Foundation
import SQLite3
import UserNotifications
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

private typealias JsonArray = [Any]
private typealias JsonObject = [String: Any]

private enum HarnessError: Error, CustomStringConvertible {
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
    private let debugSnapshotFilename = "iris_chat_runtime_debug.json"

    func testHarnessAction() async throws {
        let env = ProcessInfo.processInfo.environment
        guard env["IRIS_IOS_HARNESS_ACTION"] != nil else {
            throw XCTSkip("Interop harness runs only via scripts/run_ios_harness.py")
        }
        let action = try requiredEnv("IRIS_IOS_HARNESS_ACTION", env: env)
        let runID = env["IRIS_IOS_HARNESS_RUN_ID"] ?? UUID().uuidString
        let useAppStorage = env["IRIS_IOS_HARNESS_USE_APP_STORAGE"] == "1"
        let service = env["IRIS_IOS_HARNESS_SERVICE"] ?? (useAppStorage ? "to.iris.chat" : "to.iris.chat.harness.\(runID)")
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

        let manager = AppManager(
            secretStore: secretStore,
            dataDir: dataDir,
            environment: [:]
        )

        _ = try await waitFor(label: "bootstrap completion", timeout: 30) {
            manager.bootstrapInFlight ? nil : true
        }

        status("action", action)
        status("run_id", runID)
        status("data_dir", dataDir.path)

        switch action {
        case "create_account_and_report_identity", "report_logged_in_identity":
            let snapshot = try await ensureLoggedIn(manager: manager, env: env)
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
            let finalizedDelivery: String = try await waitFor(label: "invite chat message publish", timeout: 180) { () -> String? in
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
        case "report_runtime_debug_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            reportRuntimeDebugSnapshot(manager: manager, dataDir: dataDir)
        case "report_persisted_protocol_snapshot":
            _ = try await ensureLoggedIn(manager: manager, env: env)
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
            _ = try await waitFor(label: "peer roster \(peerOwnerHex)", timeout: 180) {
                if self.splitPersistenceHasPeerRoster(dataDir: dataDir, peerOwnerHex: peerOwnerHex) {
                    return true
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("users", summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex))
        case "wait_for_known_peer_session_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env))
            _ = try await waitFor(label: "known peer session \(peerOwnerHex)", timeout: 180) {
                if self.splitPersistenceHasPeerSession(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex) {
                    return true
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("users", summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex))
        case "wait_for_peer_transport_ready_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let peerOwnerHex = resolvePeerOwnerHex(manager: manager, peerInput: try requiredEnv("IRIS_IOS_HARNESS_PEER_INPUT", env: env))
            _ = try await waitFor(label: "peer transport ready \(peerOwnerHex)", timeout: 180) {
                if self.splitPersistenceHasPeerSession(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex) {
                    return true
                }
                return nil
            }
            status("peer_owner_hex", peerOwnerHex)
            status("users", summarizeSplitPersistedPeer(dataDir: dataDir, manager: manager, peerOwnerHex: peerOwnerHex))
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

            let finalizedDelivery = try await waitFor(label: "outgoing message \(message)", timeout: 180) {
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
            let requestedChatID = env["IRIS_IOS_HARNESS_CHAT_ID"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let peerInput = env["IRIS_IOS_HARNESS_PEER_INPUT"]?.trimmingCharacters(in: .whitespacesAndNewlines)
            let expectedChatID = requestedChatID?.isEmpty == false ? requestedChatID : nil

            let matchedChatID = try await waitFor(label: "message \(message)", timeout: 180) {
                let state = manager.state
                if let current = state.currentChat,
                   self.chatMatchesExpectedChat(chatId: current.chatId, peerInput: peerInput, expectedChatID: expectedChatID),
                   current.messages.contains(where: { $0.body == message && self.directionMatches(isOutgoing: $0.isOutgoing, direction: direction) }) {
                    return current.chatId
                }
                if let thread = state.chatList.first(where: {
                    $0.lastMessagePreview == message &&
                    self.chatMatchesExpectedChat(chatId: $0.chatId, peerInput: peerInput, expectedChatID: expectedChatID)
                }) {
                    return thread.chatId
                }
                if let chatID = self.splitPersistenceThreadWithMessage(
                    dataDir: dataDir,
                    chatID: expectedChatID,
                    expectedMessage: message,
                    direction: direction,
                    peerInput: peerInput
                ) {
                    return chatID
                }
                return nil
            }

            manager.dispatch(.openChat(chatId: matchedChatID))
            status("chat_id", matchedChatID)
            status("message", message)
        case "create_group_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let groupName = try requiredEnv("IRIS_IOS_HARNESS_GROUP_NAME", env: env)
            let memberInputs = parseList(env["IRIS_IOS_HARNESS_MEMBER_INPUTS"] ?? "")

            manager.dispatch(.createGroup(name: groupName, memberInputs: memberInputs))
            let chat = try await waitFor(label: "group \(groupName)", timeout: 180) {
                manager.state.currentChat.flatMap { current in
                    current.groupId != nil && current.displayName == groupName ? current : nil
                }
            }

            status("chat_id", chat.chatId)
            status("group_id", chat.groupId ?? "")
            status("group_name", chat.displayName)
            status("member_count", String(chat.memberCount))
        case "wait_for_group_chat_from_args":
            _ = try await ensureLoggedIn(manager: manager, env: env)
            let chatID = try requiredEnv("IRIS_IOS_HARNESS_CHAT_ID", env: env)
            _ = try await waitFor(label: "group thread \(chatID)", timeout: 180) {
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
            let chat = try await waitFor(label: "group member count \(expectedMemberCount)", timeout: 180) {
                manager.state.currentChat.flatMap { current in
                    self.sameIdentifier(current.chatId, chatID) && current.memberCount == expectedMemberCount ? current : nil
                }
            }
            status("chat_id", chat.chatId)
            status("group_id", chat.groupId ?? "")
            status("member_count", String(chat.memberCount))
        default:
            throw HarnessError.unexpected("unknown harness action: \(action)")
        }
    }

    private func ensureLoggedIn(manager: AppManager, env: [String: String]) async throws -> AccountSnapshot {
        if let account = manager.state.account {
            return account
        }

        manager.dispatch(.createAccount(name: env["IRIS_IOS_HARNESS_DISPLAY_NAME"] ?? ""))
        return try await waitFor(label: "logged in account", timeout: 90) {
            manager.state.account
        }
    }

    private func maybeDisableRelays(manager: AppManager, env: [String: String]) async throws {
        if env["IRIS_IOS_HARNESS_DISABLE_RELAYS"] != "0" {
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await disableRelays(manager: manager)
        }
    }

    private func disableRelays(manager: AppManager) async throws {
        while true {
            let relays = manager.state.preferences.nostrRelayUrls
            if relays.isEmpty {
                return
            }
            let relay = relays[0]
            manager.dispatch(.removeNostrRelay(relayUrl: relay))
            _ = try await waitFor(label: "removed relay \(relay)", timeout: 30) {
                manager.state.preferences.nostrRelayUrls.contains(relay) ? nil : true
            }
        }
    }

    private func ensureChatOpen(
        manager: AppManager,
        dataDir: URL,
        chatID: String?,
        peerInput: String?
    ) async throws -> String {
        _ = try await ensureLoggedIn(manager: manager, env: ProcessInfo.processInfo.environment)

        if let chatID, !chatID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let trimmedChatID = chatID.trimmingCharacters(in: .whitespacesAndNewlines)
            if manager.state.currentChat?.chatId.caseInsensitiveCompare(trimmedChatID) != .orderedSame {
                manager.dispatch(.openChat(chatId: trimmedChatID))
            }
            return try await waitForCurrentChat(manager: manager, chatID: trimmedChatID, timeout: 30)
        }

        let rawPeer = try requiredEnv(
            "IRIS_IOS_HARNESS_PEER_INPUT",
            env: ProcessInfo.processInfo.environment,
            fallback: peerInput
        )
        let normalizedPeer = normalizePeerInput(input: rawPeer)
        guard !normalizedPeer.isEmpty, isValidPeerInput(input: rawPeer) else {
            throw HarnessError.unexpected("invalid peer input: \(rawPeer)")
        }

        if let current = manager.state.currentChat,
           chatMatchesPeerReference(chatId: current.chatId, peerLabel: current.subtitle, peerInput: rawPeer) {
            return current.chatId
        }

        if let existing = manager.state.chatList.first(where: {
            chatMatchesPeerReference(chatId: $0.chatId, peerLabel: $0.subtitle, peerInput: rawPeer)
        }) {
            manager.dispatch(.openChat(chatId: existing.chatId))
            return try await waitForCurrentChat(manager: manager, chatID: existing.chatId, timeout: 90)
        }

        let previousThreadCount = splitPersistenceThreadFiles(dataDir: dataDir).count
        let previousActiveChatID = stringValue(readJsonObject(at: dataDir.appendingPathComponent("core/meta.json"))?["active_chat_id"])
        manager.dispatch(.createChat(peerInput: rawPeer))

        let createdChatID = try await waitForCreatedChat(
            manager: manager,
            dataDir: dataDir,
            peerInput: rawPeer,
            previousActiveChatID: previousActiveChatID,
            previousThreadCount: previousThreadCount,
            timeout: 90
        )
        manager.dispatch(.openChat(chatId: createdChatID))
        return try await waitForCurrentChat(manager: manager, chatID: createdChatID, timeout: 30)
    }

    private func waitForCurrentChat(
        manager: AppManager,
        chatID: String,
        timeout: TimeInterval
    ) async throws -> String {
        try await waitFor(label: "current chat \(chatID)", timeout: timeout) {
            if let current = manager.state.currentChat, self.sameIdentifier(current.chatId, chatID) {
                return current.chatId
            }
            return nil
        }
    }

    private func waitForCreatedChat(
        manager: AppManager,
        dataDir: URL,
        peerInput: String,
        previousActiveChatID: String,
        previousThreadCount: Int,
        timeout: TimeInterval
    ) async throws -> String {
        let debugPath = dataDir.appendingPathComponent(debugSnapshotFilename)
        let deadline = Date().addingTimeInterval(timeout)
        var lastObservation = "no observation"

        while Date() < deadline {
            if let toast = manager.state.toast, !toast.isEmpty {
                throw HarnessError.unexpected("create_chat toast: \(toast)")
            }

            if let current = manager.state.currentChat,
               chatMatchesPeerReference(chatId: current.chatId, peerLabel: current.subtitle, peerInput: peerInput) {
                return current.chatId
            }

            if let thread = manager.state.chatList.first(where: {
                chatMatchesPeerReference(chatId: $0.chatId, peerLabel: $0.subtitle, peerInput: peerInput)
            }) {
                return thread.chatId
            }

            let meta = readJsonObject(at: dataDir.appendingPathComponent("core/meta.json"))
            let debug = readJsonObject(at: debugPath)
            let persistedActiveChatID = stringValue(meta?["active_chat_id"])
            let persistedThreadCount = splitPersistenceThreadFiles(dataDir: dataDir).count
            let debugActiveChatID = stringValue(debug?["active_chat_id"])
            let currentChatList = joinValues(arrayValue(debug?["current_chat_list"]))

            lastObservation = [
                "state.current=\(summarizeCurrentChat(manager.state.currentChat))",
                "state.chatList=\(summarizeChatList(manager.state.chatList))",
                "persisted.active=\(persistedActiveChatID)",
                "persisted.threads=\(persistedThreadCount)",
                "debug.active=\(debugActiveChatID)",
                "debug.current_chat_list=\(currentChatList)",
            ].joined(separator: " ")

            if !persistedActiveChatID.isEmpty &&
                (!sameIdentifier(persistedActiveChatID, previousActiveChatID) || persistedThreadCount > previousThreadCount) {
                return persistedActiveChatID
            }

            if !debugActiveChatID.isEmpty &&
                (!sameIdentifier(debugActiveChatID, previousActiveChatID) || !currentChatList.isEmpty) {
                return debugActiveChatID
            }

            try await Task.sleep(nanoseconds: 200_000_000)
        }

        throw HarnessError.unexpected("timed out waiting for chat \(peerInput); \(lastObservation)")
    }

    private func waitFor<T>(
        label: String,
        timeout: TimeInterval,
        pollIntervalNanoseconds: UInt64 = 200_000_000,
        _ body: @escaping () -> T?
    ) async throws -> T {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let value = body() {
                return value
            }
            try await Task.sleep(nanoseconds: pollIntervalNanoseconds)
        }
        throw HarnessError.timeout(label)
    }

    private func waitForNoVisibleDeliveredNotifications(timeout: TimeInterval) async throws -> [UNNotification] {
        let deadline = Date().addingTimeInterval(timeout)
        var lastDelivered: [UNNotification] = []
        while Date() < deadline {
            let delivered = await deliveredNotifications()
            lastDelivered = delivered
            let visible = delivered.filter { notification in
                let content = notification.request.content
                return !content.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    !content.subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    !content.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
            if !visible.isEmpty {
                throw HarnessError.unexpected("visible delivered notifications: \(summarizeDeliveredNotifications(visible))")
            }
            try await Task.sleep(nanoseconds: 500_000_000)
        }
        return lastDelivered
    }

    private func waitForVisibleDeliveredNotification(
        expectedBody: String,
        timeout: TimeInterval
    ) async throws -> UNNotification {
        let deadline = Date().addingTimeInterval(timeout)
        var lastDelivered: [UNNotification] = []
        while Date() < deadline {
            let delivered = await deliveredNotifications()
            lastDelivered = delivered
            let visible = delivered.filter { notification in
                let content = notification.request.content
                return !content.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    !content.subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    !content.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
            if let expected = visible.first(where: { notification in
                expectedBody.isEmpty || notification.request.content.body == expectedBody
            }) {
                return expected
            }
            try await Task.sleep(nanoseconds: 500_000_000)
        }
        throw HarnessError.unexpected("no visible delivered notification matching body `\(expectedBody)`; delivered=\(summarizeDeliveredNotifications(lastDelivered))")
    }

    private func deliveredNotifications() async -> [UNNotification] {
        await withCheckedContinuation { continuation in
            UNUserNotificationCenter.current().getDeliveredNotifications { notifications in
                continuation.resume(returning: notifications)
            }
        }
    }

    private func summarizeDeliveredNotifications(_ notifications: [UNNotification]) -> String {
        notifications
            .map { notification in
                let content = notification.request.content
                return [
                    "id=\(notification.request.identifier)",
                    "title=\(content.title)",
                    "subtitle=\(content.subtitle)",
                    "body=\(content.body)",
                ].joined(separator: ";")
            }
            .joined(separator: " | ")
    }

    private func requiredEnv(_ key: String, env: [String: String], fallback: String? = nil) throws -> String {
        if let fallback, !fallback.isEmpty {
            return fallback
        }
        guard let value = env[key], !value.isEmpty else {
            throw HarnessError.missingEnv(key)
        }
        return value
    }

    private func parseList(_ raw: String) -> [String] {
        raw
            .split(whereSeparator: { $0 == "," || $0 == "\n" || $0 == "|" })
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private func harnessRootDir(env: [String: String]) -> URL {
        if let explicit = env["IRIS_IOS_HARNESS_DATA_ROOT"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !explicit.isEmpty,
           isWritableHarnessRoot(explicit) {
            return URL(fileURLWithPath: explicit, isDirectory: true)
        }
        let sandboxRoot = FileManager.default.temporaryDirectory
            .appendingPathComponent("ndr-ios-harness", isDirectory: true)
        if isWritableHarnessRoot(sandboxRoot.path) {
            return sandboxRoot
        }
        return URL(fileURLWithPath: "/tmp/ndr-ios-harness", isDirectory: true)
    }

    private func isolatedHarnessDataDir(runID: String, env: [String: String]) -> URL {
#if os(iOS)
        return AppPaths.dataDir(
            fileManager: .default,
            environment: ["IRIS_UI_TEST_RUN_ID": "harness-\(runID)"]
        )
#else
        return harnessRootDir(env: env).appendingPathComponent(runID, isDirectory: true)
#endif
    }

    private func isWritableHarnessRoot(_ path: String) -> Bool {
        let url = URL(fileURLWithPath: path, isDirectory: true)
        let probe = url.appendingPathComponent(".write-probe-\(UUID().uuidString)")
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            try Data().write(to: probe)
            try? FileManager.default.removeItem(at: probe)
            return true
        } catch {
            return false
        }
    }

    private func resolvePeerOwnerHex(manager: AppManager, peerInput: String) -> String {
        let normalizedPeer = normalizePeerInput(input: peerInput)
        let peerHex = peerInputToHex(input: peerInput)
        if let existing = manager.state.chatList.first(where: {
            (!peerHex.isEmpty && sameIdentifier($0.chatId, peerHex)) ||
            sameIdentifier($0.chatId, normalizedPeer) ||
            sameIdentifier($0.subtitle ?? "", peerInput) ||
            sameIdentifier($0.subtitle ?? "", normalizedPeer)
        }) {
            return existing.chatId
        }
        return peerHex.isEmpty ? normalizedPeer : peerHex
    }

    private func directionMatches(isOutgoing: Bool, direction: String) -> Bool {
        switch direction {
        case "incoming":
            return !isOutgoing
        case "outgoing":
            return isOutgoing
        default:
            return true
        }
    }

    private func chatMatchesExpectedChat(chatId: String, peerInput: String?, expectedChatID: String?) -> Bool {
        if let expectedChatID, !expectedChatID.isEmpty {
            return sameIdentifier(chatId, expectedChatID)
        }
        guard let peerInput, !peerInput.isEmpty else {
            return true
        }
        let peerHex = peerInputToHex(input: peerInput)
        return (!peerHex.isEmpty && sameIdentifier(chatId, peerHex)) ||
            sameIdentifier(chatId, normalizePeerInput(input: peerInput))
    }

    private func chatMatchesPeerReference(chatId: String, peerLabel: String?, peerInput: String) -> Bool {
        let normalizedPeer = normalizePeerInput(input: peerInput)
        let peerHex = peerInputToHex(input: peerInput)
        return (!peerHex.isEmpty && sameIdentifier(chatId, peerHex)) ||
            sameIdentifier(chatId, normalizedPeer) ||
            sameIdentifier(peerLabel ?? "", peerInput) ||
            sameIdentifier(peerLabel ?? "", normalizedPeer)
    }

    private func splitPersistenceThreadFiles(dataDir: URL) -> [URL] {
        let threadsDir = dataDir.appendingPathComponent("core/threads", isDirectory: true)
        return (try? FileManager.default.contentsOfDirectory(
            at: threadsDir,
            includingPropertiesForKeys: nil
        ))?.filter { $0.pathExtension == "json" } ?? []
    }

    private func readSplitThread(dataDir: URL, chatID: String) -> JsonObject? {
        for url in splitPersistenceThreadFiles(dataDir: dataDir) {
            guard let thread = readJsonObject(at: url),
                  sameIdentifier(stringValue(thread["chat_id"]), chatID) else {
                continue
            }
            return thread
        }
        return nil
    }

    private func splitPersistenceThreadWithMessage(
        dataDir: URL,
        chatID: String?,
        expectedMessage: String,
        direction: String,
        peerInput: String?
    ) -> String? {
        for url in splitPersistenceThreadFiles(dataDir: dataDir) {
            guard let thread = readJsonObject(at: url) else { continue }
            let threadChatID = stringValue(thread["chat_id"])
            if !chatMatchesExpectedChat(chatId: threadChatID, peerInput: peerInput, expectedChatID: chatID) {
                continue
            }
            let messages = arrayValue(thread["messages"])
            let found = messages.contains { messageEntry in
                guard let message = dictValue(messageEntry) else { return false }
                return stringValue(message["body"]) == expectedMessage &&
                    directionMatches(isOutgoing: boolValue(message["is_outgoing"]), direction: direction)
            }
            if found {
                return threadChatID
            }
        }
        return nil
    }

    private func messageExists(
        manager: AppManager,
        dataDir: URL,
        chatID: String,
        message: String,
        direction: String,
        peerInput: String?
    ) -> Bool {
        if let current = manager.state.currentChat,
           chatMatchesExpectedChat(chatId: current.chatId, peerInput: peerInput, expectedChatID: chatID),
           current.messages.contains(where: {
               $0.body == message && directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
           }) {
            return true
        }
        return splitPersistenceThreadWithMessage(
            dataDir: dataDir,
            chatID: chatID,
            expectedMessage: message,
            direction: direction,
            peerInput: peerInput
        ) != nil
    }

    private func splitPersistenceMessageDelivery(
        dataDir: URL,
        chatID: String,
        message: String,
        direction: String
    ) -> String? {
        guard let thread = readSplitThread(dataDir: dataDir, chatID: chatID) else {
            return nil
        }
        for messageEntry in arrayValue(thread["messages"]) {
            guard let persistedMessage = dictValue(messageEntry) else { continue }
            guard stringValue(persistedMessage["body"]) == message else { continue }
            guard directionMatches(isOutgoing: boolValue(persistedMessage["is_outgoing"]), direction: direction) else {
                continue
            }
            let delivery = stringValue(persistedMessage["delivery"])
            if !delivery.isEmpty, delivery.caseInsensitiveCompare("Pending") != .orderedSame {
                return delivery
            }
        }
        return nil
    }

    private func splitPersistenceHasPeerRoster(dataDir: URL, peerOwnerHex: String) -> Bool {
        let appKeys = readJsonArray(at: dataDir.appendingPathComponent("core/app_keys.json"))
        return arrayValue(appKeys).contains { entry in
            guard let known = dictValue(entry) else { return false }
            return sameIdentifier(stringValue(known["owner_pubkey_hex"]), peerOwnerHex) &&
                !arrayValue(known["devices"]).isEmpty
        }
    }

    private func summarizeSplitPersistedPeer(dataDir: URL, manager: AppManager, peerOwnerHex: String) -> String {
        guard let account = manager.state.account else { return "" }
        let user = ndrKvUser(
            dataDir: dataDir,
            ownerPubkeyHex: account.publicKeyHex,
            devicePubkeyHex: account.devicePublicKeyHex,
            peerOwnerHex: peerOwnerHex
        )
        let rosterDevices = ndrKvAppKeysDeviceCount(
            dataDir: dataDir,
            ownerPubkeyHex: account.publicKeyHex,
            devicePubkeyHex: account.devicePublicKeyHex,
            peerOwnerHex: peerOwnerHex
        )
        let devices = arrayValue(user?["devices"])
        let activeSessions = devices.reduce(into: 0) { count, entry in
            guard let device = dictValue(entry) else { return }
            if dictValue(device["active_session"]) != nil {
                count += 1
            }
        }
        let inactiveSessions = devices.reduce(into: 0) { count, entry in
            guard let device = dictValue(entry) else { return }
            count += arrayValue(device["inactive_sessions"]).count
        }
        return [
            peerOwnerHex,
            "roster=\(rosterDevices > 0)",
            "rosterDevices=\(rosterDevices)",
            "devices=\(devices.count)",
            "active=\(activeSessions)",
            "inactive=\(inactiveSessions)",
        ].joined(separator: ",")
    }

    private func splitPersistenceHasPeerSession(dataDir: URL, manager: AppManager, peerOwnerHex: String) -> Bool {
        guard let account = manager.state.account else { return false }
        guard let user = ndrKvUser(
            dataDir: dataDir,
            ownerPubkeyHex: account.publicKeyHex,
            devicePubkeyHex: account.devicePublicKeyHex,
            peerOwnerHex: peerOwnerHex
        ) else {
            return false
        }
        return arrayValue(user["devices"]).contains { entry in
            guard let device = dictValue(entry) else { return false }
            return dictValue(device["active_session"]) != nil ||
                !arrayValue(device["inactive_sessions"]).isEmpty
        }
    }

    /// Read a `user/{peer}` value out of the SQLite-backed `ndr_kv` store.
    /// The pre-SQLite harness read JSON files at
    /// `{dataDir}/ndr_runtime/{owner}/{device}/user_{peer}.json`; that
    /// tree no longer exists.
    private func ndrKvUser(
        dataDir: URL,
        ownerPubkeyHex: String,
        devicePubkeyHex: String,
        peerOwnerHex: String
    ) -> JsonObject? {
        ndrKvJson(
            dataDir: dataDir,
            ownerPubkeyHex: ownerPubkeyHex,
            devicePubkeyHex: devicePubkeyHex,
            key: "user/\(peerOwnerHex)"
        ) as? JsonObject
    }

    private func ndrKvAppKeysDeviceCount(
        dataDir: URL,
        ownerPubkeyHex: String,
        devicePubkeyHex: String,
        peerOwnerHex: String
    ) -> Int {
        // Pre-SQLite harness counted devices in `core/app_keys.json`
        // entries with matching `owner_pubkey_hex`. App-keys live in
        // `app_keys` table now keyed by owner; the per-peer device
        // count is whatever the user record knows about.
        guard let user = ndrKvUser(
            dataDir: dataDir,
            ownerPubkeyHex: ownerPubkeyHex,
            devicePubkeyHex: devicePubkeyHex,
            peerOwnerHex: peerOwnerHex
        ) else {
            return 0
        }
        return arrayValue(user["known_device_identities"]).count
    }

    private func ndrKvJson(
        dataDir: URL,
        ownerPubkeyHex: String,
        devicePubkeyHex: String,
        key: String
    ) -> Any? {
        let dbPath = dataDir.appendingPathComponent("core.sqlite3").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return nil
        }
        defer { sqlite3_close(db) }
        var stmt: OpaquePointer?
        let sql = "SELECT value FROM ndr_kv WHERE owner_pubkey_hex = ? AND device_pubkey_hex = ? AND key = ?"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        sqlite3_bind_text(stmt, 1, ownerPubkeyHex, -1, transient)
        sqlite3_bind_text(stmt, 2, devicePubkeyHex, -1, transient)
        sqlite3_bind_text(stmt, 3, key, -1, transient)
        guard sqlite3_step(stmt) == SQLITE_ROW else {
            return nil
        }
        guard let cString = sqlite3_column_text(stmt, 0) else {
            return nil
        }
        let raw = String(cString: cString)
        guard let data = raw.data(using: .utf8) else {
            return nil
        }
        return try? JSONSerialization.jsonObject(with: data, options: [])
    }

    private func reportIdentity(_ snapshot: AccountSnapshot) {
        status("npub", snapshot.npub)
        status("public_key_hex", snapshot.publicKeyHex)
        status("device_npub", snapshot.deviceNpub)
        status("device_public_key_hex", snapshot.devicePublicKeyHex)
        status("authorization_state", String(describing: snapshot.authorizationState))
    }

    private func reportNearbySnapshot(manager: AppManager) {
        let peers = manager.nearbyIris.peers.map { peer in
            [
                "id": peer.id,
                "name": peer.name,
                "owner_pubkey_hex": peer.ownerPubkeyHex ?? "",
                "profile_event_id": peer.profileEventID ?? "",
            ]
        }
        let peersData = try? JSONSerialization.data(withJSONObject: peers)
        let peersJson = peersData.flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        status("nearby_visible", String(manager.nearbyIris.isVisible))
        status("nearby_status", manager.nearbyIris.status)
        status("nearby_peer_count", String(manager.nearbyIris.peers.count))
        status("nearby_peers", peersJson)
    }

    private func reportRuntimeDebugSnapshot(manager: AppManager, dataDir: URL) {
        let state = manager.state
        let debug = readJsonObject(at: dataDir.appendingPathComponent(debugSnapshotFilename))
        let plan = dictValue(debug?["current_protocol_plan"])

        status("data_dir", dataDir.path)
        status("rev", String(state.rev))
        status("default_screen", String(describing: state.router.defaultScreen))
        status("screen_stack", state.router.screenStack.map { String(describing: $0) }.joined(separator: "|"))
        status("current_chat", summarizeCurrentChat(state.currentChat))
        status("chat_list", summarizeChatList(state.chatList))
        status("toast", state.toast ?? "")
        status("runtime_file_present", debug == nil ? "false" : "true")
        status("generated_at_secs", stringValue(debug?["generated_at_secs"]))
        status("local_owner_pubkey_hex", stringValue(debug?["local_owner_pubkey_hex"]))
        status("local_device_pubkey_hex", stringValue(debug?["local_device_pubkey_hex"]))
        status("authorization_state", stringValue(debug?["authorization_state"]))
        status("tracked_owner_hexes", joinValues(arrayValue(debug?["tracked_owner_hexes"])))
        status("plan_roster_authors", joinValues(arrayValue(plan?["roster_authors"])))
        status("plan_invite_authors", joinValues(arrayValue(plan?["invite_authors"])))
        status("plan_message_authors", joinValues(arrayValue(plan?["message_authors"])))
        status("plan_invite_response_recipient", stringValue(plan?["invite_response_recipient"]))
        status("known_users", summarizeRuntimeKnownUsers(arrayValue(debug?["known_users"])))
        status("pending_outbound", summarizeRuntimePendingOutbound(arrayValue(debug?["pending_outbound"])))
        status("pending_group_controls", summarizeRuntimePendingGroupControls(arrayValue(debug?["pending_group_controls"])))
        status("recent_handshake_peers", summarizeRecentHandshakePeers(arrayValue(debug?["recent_handshake_peers"])))
        status("event_counts", summarizeEventCounts(dictValue(debug?["event_counts"])))
        status("recent_log", summarizeRecentLog(arrayValue(debug?["recent_log"])))
    }

    private func reportPersistedProtocolSnapshot(dataDir: URL) {
        let meta = readJsonObject(at: dataDir.appendingPathComponent("core/meta.json"))
        let appKeys = readJsonArray(at: dataDir.appendingPathComponent("core/app_keys.json"))
        let groups = readJsonArray(at: dataDir.appendingPathComponent("core/groups.json"))
        let seenEvents = readJsonObject(at: dataDir.appendingPathComponent("core/seen_events.json"))
        let threads = splitPersistenceThreadFiles(dataDir: dataDir).compactMap { readJsonObject(at: $0) }

        status("data_dir", dataDir.path)
        status("persisted_file_present", meta == nil ? "false" : "true")
        status("version", stringValue(meta?["version"]))
        status("active_chat_id", stringValue(meta?["active_chat_id"]))
        status("authorization_state", stringValue(meta?["authorization_state"]))
        status("app_keys", summarizePersistedAppKeys(appKeys))
        status("groups", summarizePersistedGroups(groups))
        status("seen_event_ids_count", String(arrayValue(seenEvents?["seen_event_ids"]).count))
        status("threads", summarizePersistedThreads(threads))
    }

    private func reportMobilePushSnapshot(manager: AppManager) {
        let snapshot = manager.state.mobilePush
        status("owner_pubkey_hex", snapshot.ownerPubkeyHex ?? "")
        status("message_author_pubkeys", snapshot.messageAuthorPubkeys.joined(separator: ","))
        status("sessions", snapshot.sessions.map { session in
            [
                session.recipientPubkeyHex,
                session.displayName,
                session.trackedSenderPubkeys.joined(separator: "+"),
                "receiving=\(session.hasReceivingCapability)",
            ].joined(separator: ",")
        }.joined(separator: "|"))
    }

    private func decryptNotificationPayloadFromArgs(
        secretStore: AccountSecretStore,
        dataDir: URL,
        env: [String: String]
    ) throws {
        let outerEventJson = try requiredEnv("IRIS_IOS_HARNESS_OUTER_EVENT_JSON", env: env)
        let expectedBody = env["IRIS_IOS_HARNESS_EXPECTED_BODY"] ?? ""
        let expectedTitle = env["IRIS_IOS_HARNESS_EXPECTED_TITLE"] ?? ""
        let eventObject = try jsonObjectFromString(outerEventJson)
        let payloadObject: JsonObject = [
            "aps": [
                "alert": [
                    "title": "Iris Chat",
                    "body": "New message",
                ],
                "mutable-content": 1,
            ],
            "event": eventObject,
            "title": "New message",
            "body": "New message",
        ]
        let payloadData = try JSONSerialization.data(withJSONObject: payloadObject)
        guard let payloadJson = String(data: payloadData, encoding: .utf8) else {
            throw HarnessError.unexpected("could not encode notification payload")
        }
        guard let bundle = secretStore.load() else {
            throw HarnessError.unexpected("stored account bundle unavailable")
        }

        let resolution = decryptMobilePushNotificationPayload(
            dataDir: dataDir.path,
            ownerPubkeyHex: bundle.ownerPubkeyHex,
            deviceNsec: bundle.deviceNsec,
            rawPayloadJson: payloadJson
        )
        status("notification_should_show", String(resolution.shouldShow))
        status("notification_title", resolution.title)
        status("notification_body", resolution.body)
        status("notification_payload_json", resolution.payloadJson)
        guard resolution.shouldShow else {
            throw HarnessError.unexpected("decrypted notification was suppressed")
        }
        if !expectedBody.isEmpty && resolution.body != expectedBody {
            throw HarnessError.unexpected("notification body `\(resolution.body)` != `\(expectedBody)`")
        }
        if !expectedTitle.isEmpty && resolution.title != expectedTitle {
            throw HarnessError.unexpected("notification title `\(resolution.title)` != `\(expectedTitle)`")
        }
    }

    private func reportMobilePushServerSnapshot(manager: AppManager) async throws {
        guard let ownerNsec = manager.exportOwnerNsec() else {
            throw HarnessError.unexpected("owner nsec unavailable")
        }
        let request = buildMobilePushListSubscriptionsRequest(
            ownerNsec: ownerNsec,
            platformKey: "ios",
            isRelease: false,
            serverUrlOverride: nil
        )
        guard let request else {
            throw HarnessError.unexpected("could not build mobile push list request")
        }
        guard let url = URL(string: request.url) else {
            throw HarnessError.unexpected("invalid mobile push url")
        }
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method
        urlRequest.setValue("application/json", forHTTPHeaderField: "accept")
        urlRequest.setValue(request.authorizationHeader, forHTTPHeaderField: "authorization")
        let (data, response) = try await URLSession.shared.data(for: urlRequest)
        let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
        status("status_code", String(statusCode))
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            status("raw", String(data: data, encoding: .utf8) ?? "")
            return
        }
        status("subscription_count", String(object.count))
        status("subscriptions", summarizeMobilePushServerSubscriptions(object))
    }

    private func summarizeCurrentChat(_ chat: CurrentChatSnapshot?) -> String {
        guard let chat else { return "" }
        return [
            chat.chatId,
            chat.displayName,
            chat.groupId ?? "",
            String(chat.memberCount),
            String(chat.messages.count),
        ].joined(separator: ",")
    }

    private func summarizeChatList(_ threads: [ChatThreadSnapshot]) -> String {
        threads.map { thread in
            [
                thread.chatId,
                String(describing: thread.kind),
                thread.displayName,
                String(thread.memberCount),
                thread.lastMessagePreview ?? "",
                String(thread.unreadCount),
            ].joined(separator: ",")
        }.joined(separator: "|")
    }

    private func summarizeMobilePushServerSubscriptions(_ subscriptions: JsonObject) -> String {
        subscriptions.map { id, value in
            let subscription = dictValue(value)
            let filter = dictValue(subscription?["filter"])
            let authors = arrayValue(filter?["authors"])
            let fcmTokens = arrayValue(subscription?["fcm_tokens"])
            let apnsTokens = arrayValue(subscription?["apns_tokens"])
            return [
                id,
                "authors=\(authors.count)",
                "fcm=\(fcmTokens.count)",
                "apns=\(apnsTokens.count)",
            ].joined(separator: ",")
        }
        .sorted()
        .joined(separator: "|")
    }

    private func summarizeRuntimeKnownUsers(_ users: JsonArray) -> String {
        joinObjects(users) { user in
            [
                stringValue(user["owner_pubkey_hex"]),
                "roster=\(boolValue(user["has_roster"]))",
                "rosterDevices=\(intValue(user["roster_device_count"]))",
                "devices=\(intValue(user["device_count"]))",
                "authorized=\(intValue(user["authorized_device_count"]))",
                "active=\(intValue(user["active_session_device_count"]))",
                "inactive=\(intValue(user["inactive_session_count"]))",
            ].joined(separator: ",")
        }
    }

    private func summarizeRuntimePendingOutbound(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["message_id"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["publish_mode"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    private func summarizeRuntimePendingGroupControls(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["operation_id"]),
                stringValue(entry["group_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["kind"]),
                "targets=\(joinValues(arrayValue(entry["target_owner_hexes"])))",
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    private func summarizeRecentHandshakePeers(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["owner_hex"]),
                stringValue(entry["device_hex"]),
                stringValue(entry["observed_at_secs"]),
            ].joined(separator: ",")
        }
    }

    private func summarizeEventCounts(_ eventCounts: JsonObject?) -> String {
        guard let eventCounts else { return "" }
        return [
            "roster=\(intValue(eventCounts["roster_events"]))",
            "invite=\(intValue(eventCounts["invite_events"]))",
            "inviteResponse=\(intValue(eventCounts["invite_response_events"]))",
            "message=\(intValue(eventCounts["message_events"]))",
            "other=\(intValue(eventCounts["other_events"]))",
        ].joined(separator: ",")
    }

    private func summarizeRecentLog(_ entries: JsonArray) -> String {
        joinObjects(entries, limit: 20) { entry in
            [
                stringValue(entry["timestamp_secs"]),
                stringValue(entry["category"]),
                stringValue(entry["detail"]),
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedUsers(_ users: JsonArray) -> String {
        joinObjects(users) { user in
            let devices = arrayValue(user["devices"])
            let activeSessions = devices.reduce(into: 0) { count, entry in
                guard let device = dictValue(entry) else { return }
                if dictValue(device["active_session"]) != nil {
                    count += 1
                }
            }
            let inactiveSessions = devices.reduce(into: 0) { count, entry in
                guard let device = dictValue(entry) else { return }
                count += arrayValue(device["inactive_sessions"]).count
            }
            return [
                stringValue(user["owner_pubkey"]),
                "roster=\(dictValue(user["roster"]) != nil)",
                "devices=\(devices.count)",
                "active=\(activeSessions)",
                "inactive=\(inactiveSessions)",
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedGroups(_ groups: JsonArray) -> String {
        joinObjects(groups) { group in
            [
                stringValue(group["group_id"]),
                stringValue(group["name"]),
                "revision=\(intValue(group["revision"]))",
                "members=\(arrayValue(group["members"]).count)",
                "admins=\(arrayValue(group["admins"]).count)",
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedAppKeys(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["owner_pubkey_hex"]),
                "devices=\(arrayValue(entry["devices"]).count)",
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedPendingOutbound(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["message_id"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["publish_mode"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedPendingGroupControls(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["operation_id"]),
                stringValue(entry["group_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["kind"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    private func summarizePersistedThreads(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["chat_id"]),
                "messages=\(arrayValue(entry["messages"]).count)",
                "unread=\(intValue(entry["unread_count"]))",
            ].joined(separator: ",")
        }
    }

    private func readJsonObject(at url: URL) -> JsonObject? {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? JsonObject else {
            return nil
        }
        return object
    }

    private func jsonObjectFromString(_ raw: String) throws -> JsonObject {
        guard let data = raw.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: data) as? JsonObject else {
            throw HarnessError.unexpected("invalid json object")
        }
        return object
    }

    private func readJsonArray(at url: URL) -> JsonArray {
        guard let data = try? Data(contentsOf: url),
              let array = try? JSONSerialization.jsonObject(with: data) as? JsonArray else {
            return []
        }
        return array
    }

    private func dictValue(_ value: Any?) -> JsonObject? {
        value as? JsonObject
    }

    private func arrayValue(_ value: Any?) -> JsonArray {
        value as? JsonArray ?? []
    }

    private func stringValue(_ value: Any?) -> String {
        switch value {
        case let value as String:
            return value
        case let value as NSNumber:
            return value.stringValue
        case _ as NSNull:
            return ""
        case .none:
            return ""
        default:
            return String(describing: value!)
        }
    }

    private func boolValue(_ value: Any?) -> Bool {
        switch value {
        case let value as Bool:
            return value
        case let value as NSNumber:
            return value.boolValue
        case let value as String:
            return ["1", "true", "TRUE", "True"].contains(value)
        default:
            return false
        }
    }

    private func intValue(_ value: Any?) -> Int {
        switch value {
        case let value as Int:
            return value
        case let value as UInt64:
            return Int(value)
        case let value as NSNumber:
            return value.intValue
        case let value as String:
            return Int(value) ?? 0
        default:
            return 0
        }
    }

    private func joinObjects(_ entries: JsonArray, limit: Int = Int.max, block: (JsonObject) -> String) -> String {
        entries.prefix(limit).compactMap { dictValue($0).map(block) }.joined(separator: "|")
    }

    private func joinValues(_ entries: JsonArray, limit: Int = Int.max) -> String {
        entries.prefix(limit).map(stringValue).joined(separator: "|")
    }

    private func sameIdentifier(_ lhs: String, _ rhs: String) -> Bool {
        lhs.caseInsensitiveCompare(rhs) == .orderedSame
    }

    private func nearbyProfileTimeout(env: [String: String]) -> TimeInterval {
        let requested = Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ?? 20
        return min(max(requested, 1), 20)
    }

    private func status(_ key: String, _ value: String) {
        print("HARNESS_STATUS: \(key)=\(value)")
        fflush(stdout)
    }
}
