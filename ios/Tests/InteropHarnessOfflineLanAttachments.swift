import Foundation
import Security
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

extension InteropHarnessTests {
    func runOfflineLanAttachment(
        action: String, fixture: OfflineLanFixture, manager: AppManager, dataDir: URL, env: [String: String]
    ) async throws {
        let chatID = try await offlineLanOpenExistingChat(manager: manager, env: env)
        try reportOfflineLanContext(manager: manager, dataDir: dataDir, env: env, label: "context")
        if action == "offline_lan_attachment_prepare" {
            try await disableRelays(manager: manager)
            manager.setNearbyBluetoothEnabled(false)
            manager.setNearbyLanEnabled(true)
            manager.setNearbyEnabled(true)
            _ = try await waitFor(label: "persisted LAN-only fixture settings", timeout: 30) {
                self.offlineLanPreferencesPersisted(dataDir: dataDir) ? true : nil
            }
            status("offline_lan_prepared", "true")
            status("offline_lan_restart_required", "true")
            return
        }
        guard manager.state.preferences.nostrRelayUrls.isEmpty,
              !manager.state.preferences.nearbyBluetoothEnabled,
              manager.state.preferences.nearbyLanEnabled, manager.state.preferences.nearbyEnabled else {
            throw HarnessError.unexpected("persisted offline settings did not survive bootstrap")
        }
        let peer = try requiredEnv("IRIS_IOS_HARNESS_PEER_DEVICE_NPUB", env: env)
        let wifi = try await offlineLanWiFiAddresses()
        let caption = try requiredEnv("IRIS_IOS_HARNESS_CAPTION", env: env)
        guard caption.utf8.count <= 120, !caption.contains("\n") else {
            throw HarnessError.unexpected("fixture caption must be a short unique marker")
        }
        let before = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "before")
        try before.verifyPeers(expected: peer, wifi: wifi, required: false)
        if action == "offline_lan_attachment_sender" {
            try await offlineLanSend(fixture: fixture, manager: manager, chatID: chatID, caption: caption, env: env)
            let after = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "after_send")
            try after.verifyPeers(expected: peer, wifi: wifi, required: false)
            try await offlineLanHold(fixture: fixture, manager: manager, peer: peer, wifi: wifi, env: env)
        } else {
            let warm = action == "offline_lan_attachment_reopen"
            try await offlineLanRead(
                fixture: fixture, manager: manager, chatID: chatID, caption: caption,
                peer: peer, wifi: wifi, warm: warm, env: env
            )
            // Any member that cached the attachment can be restarted as the
            // source, regardless of the original message's author/direction.
            if warm && env["IRIS_IOS_HARNESS_SERVE_AFTER_READ"] == "1" {
                try await offlineLanHold(fixture: fixture, manager: manager, peer: peer, wifi: wifi, env: env)
            }
        }
        try reportOfflineLanContext(manager: manager, dataDir: dataDir, env: env, label: "context_after")
        status("offline_lan_action_verified", "true")
        status("offline_lan_physical_gate_complete", "false")
    }

    func offlineLanSend(
        fixture: OfflineLanFixture, manager: AppManager, chatID: String, caption: String, env: [String: String]
    ) async throws {
        guard manager.state.currentChat?.messages.contains(where: { $0.body == caption }) == false else {
            throw HarnessError.unexpected("use a fresh caption; fixture refuses a duplicate send")
        }
        // The random file is generated after effective core isolation has been
        // checked. Distinct chunks prevent a repeated-block dedup shortcut.
        var bytes = Data(count: OfflineLanFixture.byteCount)
        let result = bytes.withUnsafeMutableBytes { buffer -> Int32 in
            guard let address = buffer.baseAddress else { return errSecAllocate }
            return SecRandomCopyBytes(kSecRandomDefault, buffer.count, address)
        }
        guard result == errSecSuccess else { throw HarnessError.unexpected("fresh fixture randomness failed") }
        let generatedAt = Int64(Date().timeIntervalSince1970 * 1_000)
        guard generatedAt >= fixture.configuredAtMS else { throw HarnessError.unexpected("fixture clock moved backwards") }
        let path = fixture.directory.appendingPathComponent("send-\(UUID().uuidString).bin")
        try bytes.write(to: path, options: .atomic)
        manager.dispatch(.sendAttachments(
            chatId: chatID,
            attachments: [OutgoingAttachment(filePath: path.path, filename: OfflineLanFixture.filename)],
            caption: caption
        ))
        let message: ChatMessageSnapshot = try await waitFor(label: "locally accepted attachment", timeout: 90) {
            guard !manager.state.busy.uploadingAttachment else { return nil }
            return manager.state.currentChat?.messages.first(where: {
                $0.isOutgoing && $0.body == caption && $0.attachments.count == 1 &&
                    $0.attachments[0].filename == OfflineLanFixture.filename
            })
        }
        guard message.delivery != .failed, message.attachments[0].uploadedUrls.isEmpty else {
            throw HarnessError.unexpected("attachment was rejected or unexpectedly uploaded over HTTP")
        }
        let receipt: JsonObject = [
            "message_id": message.id, "caption": caption,
            "attachment_sha256": sha256Hex(bytes), "attachment_size": bytes.count,
            "attachment_reference_sha256": sha256Hex(Data(message.attachments[0].nhash.utf8)),
            "generated_at_ms": generatedAt, "configured_at_ms": fixture.configuredAtMS,
        ]
        try JSONSerialization.data(withJSONObject: receipt, options: [.sortedKeys])
            .write(to: fixture.directory.appendingPathComponent("sender-\(UUID().uuidString).json"), options: .atomic)
        status("message_id", message.id)
        status("caption", caption)
        status("attachment_sha256", sha256Hex(bytes))
        status("attachment_size", String(bytes.count))
        status("attachment_reference_sha256", sha256Hex(Data(message.attachments[0].nhash.utf8)))
        status("offline_lan_file_generated_at_ms", String(generatedAt))
        status("offline_lan_sender_ready", "true")
    }

    func offlineLanRead(
        fixture: OfflineLanFixture, manager: AppManager, chatID: String, caption: String,
        peer: String, wifi: [OfflineLanWiFiAddress], warm: Bool, env: [String: String]
    ) async throws {
        let expected = try requiredEnv("IRIS_IOS_HARNESS_EXPECTED_SHA256", env: env)
        guard expected.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else {
            throw HarnessError.unexpected("use the exact SHA-256 reported by the fresh sender")
        }
        let message: ChatMessageSnapshot = try await waitFor(label: "attachment message", timeout: 120) {
            guard let current = manager.state.currentChat, self.sameIdentifier(current.chatId, chatID) else { return nil }
            return current.messages.first(where: {
                $0.body == caption && (warm || !$0.isOutgoing) && $0.attachments.count == 1 &&
                    $0.attachments[0].filename == OfflineLanFixture.filename
            })
        }
        let attachment = message.attachments[0]
        guard attachment.uploadedUrls.isEmpty else { throw HarnessError.unexpected("fixture has HTTP upload receipts") }
        let before = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "before_read")
        try before.verifyPeers(expected: peer, wifi: wifi, required: !warm)
        // Call the production Rust downloader directly. AppManager's decoded
        // media cache cannot satisfy either the cold or restart assertion.
        let reference = attachment.nhash
        let downloaded = await Task.detached(priority: .userInitiated) { () -> Data? in
            let result = downloadHashtreeAttachment(nhash: reference)
            return result.dataBase64.flatMap { Data(base64Encoded: $0) }
        }.value
        guard let downloaded, downloaded.count == OfflineLanFixture.byteCount,
              sha256Hex(downloaded) == expected else {
            throw HarnessError.unexpected("downloaded attachment does not match the sender's plaintext")
        }
        let after = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "after_read")
        try after.verifyPeers(expected: peer, wifi: wifi, required: !warm)
        try before.verifyRead(after: after, warm: warm)
        status("message_id", message.id)
        status("attachment_sha256", sha256Hex(downloaded))
        status("attachment_size", String(downloaded.count))
        status("attachment_reference_sha256", sha256Hex(Data(attachment.nhash.utf8)))
        status("offline_lan_read_kind", warm ? "durable-cache" : "cold-peer")
        status("download_verified", "true")
    }

    func offlineLanHold(
        fixture: OfflineLanFixture, manager: AppManager, peer: String,
        wifi: [OfflineLanWiFiAddress], env: [String: String]
    ) async throws {
        let seconds = min(max(Double(env["IRIS_IOS_HARNESS_HOLD_SECONDS"] ?? "") ?? 180, 1), 180)
        _ = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "before_serve")
        status("offline_lan_serving_seconds", String(Int(seconds)))
        try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        let after = try await offlineLanEvidence(manager: manager, fixture: fixture, label: "after_serve")
        try after.verifyPeers(expected: peer, wifi: wifi, required: false)
    }
}
