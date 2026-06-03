import Foundation
import SQLite3
import UserNotifications
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

extension InteropHarnessTests {
    func ensureLoggedIn(manager: AppManager, env: [String: String]) async throws -> AccountSnapshot {
        if let account = manager.state.account {
            return account
        }

        return try await waitFor(label: "restored logged in account", timeout: 30) {
            manager.state.account
        }
    }

    func createOrLoadAccount(manager: AppManager, env: [String: String]) async throws -> AccountSnapshot {
        if let account = manager.state.account {
            return account
        }

        manager.dispatch(.createAccount(name: env["IRIS_IOS_HARNESS_DISPLAY_NAME"] ?? ""))
        return try await waitFor(label: "logged in account", timeout: 90) {
            manager.state.account
        }
    }

    func harnessManagerEnvironment(runID: String) -> [String: String] {
        [
            "IRIS_UI_TEST_RUN_ID": "harness-\(runID)"
        ]
    }

    func maybeDisableRelays(manager: AppManager, env: [String: String]) async throws {
        if env["IRIS_IOS_HARNESS_DISABLE_RELAYS"] != "0" {
            _ = try await ensureLoggedIn(manager: manager, env: env)
            try await disableRelays(manager: manager)
        }
    }

    func disableRelays(manager: AppManager) async throws {
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

    func ensureChatOpen(
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

    func waitForCurrentChat(
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

    func waitForGroupDetails(
        manager: AppManager,
        groupID: String,
        timeout: TimeInterval,
        predicate: @escaping (GroupDetailsSnapshot) -> Bool
    ) async throws -> GroupDetailsSnapshot {
        manager.dispatch(.pushScreen(screen: .groupDetails(groupId: groupID)))
        return try await waitFor(label: "group details \(groupID)", timeout: timeout) {
            guard let details = manager.state.groupDetails,
                  self.sameIdentifier(details.groupId, groupID),
                  predicate(details) else {
                return nil
            }
            return details
        }
    }

    func waitForChatSettings(
        manager: AppManager,
        chatID: String,
        env: [String: String],
        timeout: TimeInterval
    ) async throws -> (muted: Bool, pinned: Bool, ttl: UInt64?) {
        let expectedMuted = optionalBoolEnv("IRIS_IOS_HARNESS_MUTED", env: env)
        let expectedPinned = optionalBoolEnv("IRIS_IOS_HARNESS_PINNED", env: env)
        let ttlRaw = env["IRIS_IOS_HARNESS_TTL_SECONDS"]?.trimmingCharacters(in: .whitespacesAndNewlines)
        let expectedTtl = ttlRaw?.isEmpty == false ? UInt64(ttlRaw!) : nil
        return try await waitFor(label: "chat settings \(chatID)", timeout: timeout) {
            let current = manager.state.currentChat
            let thread = manager.state.chatList.first(where: { self.sameIdentifier($0.chatId, chatID) })
            let muted = current?.isMuted ?? thread?.isMuted ?? false
            let pinned = thread?.isPinned ?? false
            let ttl = current?.messageTtlSeconds
            if let expectedMuted, muted != expectedMuted {
                return nil
            }
            if let expectedPinned, pinned != expectedPinned {
                return nil
            }
            if let expectedTtl, ttl != expectedTtl {
                return nil
            }
            return (muted: muted, pinned: pinned, ttl: ttl)
        }
    }

    func waitForCreatedChat(
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
            let persistedPeerChatID = splitPersistenceThreadFiles(dataDir: dataDir)
                .compactMap { self.readJsonObject(at: $0) }
                .map { self.stringValue($0["chat_id"]) }
                .first {
                    !self.stringValue($0).isEmpty &&
                        self.chatMatchesExpectedChat(chatId: self.stringValue($0), peerInput: peerInput, expectedChatID: nil)
                } ?? ""

            lastObservation = [
                "state.current=\(summarizeCurrentChat(manager.state.currentChat))",
                "state.chatList=\(summarizeChatList(manager.state.chatList))",
                "persisted.active=\(persistedActiveChatID)",
                "persisted.peer=\(persistedPeerChatID)",
                "persisted.threads=\(persistedThreadCount)",
                "debug.active=\(debugActiveChatID)",
                "debug.current_chat_list=\(currentChatList)",
            ].joined(separator: " ")

            if !persistedPeerChatID.isEmpty {
                return persistedPeerChatID
            }

            if !persistedActiveChatID.isEmpty &&
                !sameIdentifier(persistedActiveChatID, previousActiveChatID) &&
                chatMatchesExpectedChat(chatId: persistedActiveChatID, peerInput: peerInput, expectedChatID: nil) {
                return persistedActiveChatID
            }

            if !debugActiveChatID.isEmpty &&
                !sameIdentifier(debugActiveChatID, previousActiveChatID) &&
                chatMatchesExpectedChat(chatId: debugActiveChatID, peerInput: peerInput, expectedChatID: nil) {
                return debugActiveChatID
            }

            try await Task.sleep(nanoseconds: 200_000_000)
        }

        throw HarnessError.unexpected("timed out waiting for chat \(peerInput); \(lastObservation)")
    }

    func waitFor<T>(
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

    func waitForRelayDrainIfRequested(manager: AppManager, dataDir: URL, env: [String: String]) async throws {
        let raw = (env["IRIS_IOS_HARNESS_WAIT_FOR_RELAY_DRAIN"] ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard ["1", "true", "yes"].contains(raw) else {
            return
        }

        let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_RELAY_DRAIN_TIMEOUT_SECS"] ?? "") ?? 60)
        let runtimeOnly = ["1", "true", "yes"].contains(
            (env["IRIS_IOS_HARNESS_RELAY_DRAIN_RUNTIME_ONLY"] ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
        )
        manager.appForegrounded()
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let status = manager.state.networkStatus {
                let pendingDurablePublishCount = self.sqlitePendingRelayPublishCount(dataDir: dataDir)
                if (status.relayUrls.isEmpty || status.connectedRelayCount > 0) &&
                    !status.syncing &&
                    pendingDurablePublishCount == 0 &&
                    (runtimeOnly || status.pendingOutboundCount == 0) &&
                    status.pendingGroupControlCount == 0 {
                    self.status("pending_outbound_count", String(status.pendingOutboundCount))
                    self.status("pending_runtime_outbound_count", String(pendingDurablePublishCount ?? 0))
                    self.status("pending_group_control_count", String(status.pendingGroupControlCount))
                    self.status("network_syncing", String(status.syncing))
                    return
                }
            }

            try await Task.sleep(nanoseconds: 200_000_000)
        }

        if let status = manager.state.networkStatus {
            let pendingDurablePublishCount = self.sqlitePendingRelayPublishCount(dataDir: dataDir)
            self.status("pending_outbound_count", String(status.pendingOutboundCount))
            self.status("pending_runtime_outbound_count", pendingDurablePublishCount.map(String.init) ?? "unavailable")
            self.status("pending_group_control_count", String(status.pendingGroupControlCount))
            self.status("network_syncing", String(status.syncing))
            if (status.relayUrls.isEmpty || status.connectedRelayCount > 0) &&
                !status.syncing &&
                pendingDurablePublishCount == 0 &&
                (runtimeOnly || status.pendingOutboundCount == 0) &&
                status.pendingGroupControlCount == 0 {
                return
            }
        }

        throw HarnessError.timeout("relay publish drain")
    }

    func waitForRuntimeSnapshotIdleIfRequested(manager: AppManager, env: [String: String]) async throws -> JsonObject? {
        let waitForRelayDrain = ["1", "true", "yes"].contains(
            (env["IRIS_IOS_HARNESS_WAIT_FOR_RELAY_DRAIN"] ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
        )
        let waitForRuntimeIdle = ["1", "true", "yes"].contains(
            (env["IRIS_IOS_HARNESS_WAIT_FOR_RUNTIME_IDLE"] ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
        )
        let runtimeOnly = ["1", "true", "yes"].contains(
            (env["IRIS_IOS_HARNESS_RELAY_DRAIN_RUNTIME_ONLY"] ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
        )
        guard (waitForRelayDrain || waitForRuntimeIdle) && !runtimeOnly else {
            return nil
        }

        let timeout = TimeInterval(Double(env["IRIS_IOS_HARNESS_RUNTIME_IDLE_TIMEOUT_SECS"]
            ?? env["IRIS_IOS_HARNESS_RELAY_DRAIN_TIMEOUT_SECS"]
            ?? "") ?? 60)
        let deadline = Date().addingTimeInterval(timeout)
        var lastDebug: JsonObject?
        while Date() < deadline {
            manager.appForegrounded()
            if let debug = await liveRuntimeDebugSnapshot(manager: manager) {
                lastDebug = debug
                if runtimeSnapshotIsIdle(debug) {
                    status("runtime_settled", "true")
                    status("runtime_pending_summary", runtimePendingSummary(debug))
                    return debug
                }
            }
            try await Task.sleep(nanoseconds: 200_000_000)
        }

        status("runtime_settled", "false")
        status("runtime_pending_summary", lastDebug.map(runtimePendingSummary) ?? "snapshot=unavailable")
        throw HarnessError.timeout("runtime snapshot idle")
    }

    func waitForNoVisibleDeliveredNotifications(timeout: TimeInterval) async throws -> [UNNotification] {
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

    func waitForVisibleDeliveredNotification(
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

    func deliveredNotifications() async -> [UNNotification] {
        await withCheckedContinuation { continuation in
            UNUserNotificationCenter.current().getDeliveredNotifications { notifications in
                continuation.resume(returning: notifications)
            }
        }
    }

    func summarizeDeliveredNotifications(_ notifications: [UNNotification]) -> String {
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

    func requiredEnv(_ key: String, env: [String: String], fallback: String? = nil) throws -> String {
        if let fallback, !fallback.isEmpty {
            return fallback
        }
        guard let value = env[key], !value.isEmpty else {
            throw HarnessError.missingEnv(key)
        }
        return value
    }

    func parseList(_ raw: String) -> [String] {
        raw
            .split(whereSeparator: { $0 == "," || $0 == "\n" || $0 == "|" })
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    func harnessRootDir(env: [String: String]) -> URL {
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

    func isolatedHarnessDataDir(runID: String, env: [String: String]) -> URL {
    #if os(iOS)
        return AppPaths.dataDir(
            fileManager: .default,
            environment: ["IRIS_UI_TEST_RUN_ID": "harness-\(runID)"]
        )
    #else
        return harnessRootDir(env: env).appendingPathComponent(runID, isDirectory: true)
    #endif
    }

    func isWritableHarnessRoot(_ path: String) -> Bool {
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

    func resolvePeerOwnerHex(manager: AppManager, peerInput: String) -> String {
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

    func directionMatches(isOutgoing: Bool, direction: String) -> Bool {
        switch direction {
        case "incoming":
            return !isOutgoing
        case "outgoing":
            return isOutgoing
        default:
            return true
        }
    }

    func chatMatchesExpectedChat(chatId: String, peerInput: String?, expectedChatID: String?) -> Bool {
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

    func chatMatchesPeerReference(chatId: String, peerLabel: String?, peerInput: String) -> Bool {
        let normalizedPeer = normalizePeerInput(input: peerInput)
        let peerHex = peerInputToHex(input: peerInput)
        return (!peerHex.isEmpty && sameIdentifier(chatId, peerHex)) ||
            sameIdentifier(chatId, normalizedPeer) ||
            sameIdentifier(peerLabel ?? "", peerInput) ||
            sameIdentifier(peerLabel ?? "", normalizedPeer)
    }
}
