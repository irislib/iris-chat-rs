import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

extension InteropHarnessTests {
    /// An exact DB existence query prevents a filtered or omitted chat-list
    /// entry from being mistaken for evidence that no direct chat exists.
    func reportOfflineLanContext(manager: AppManager, dataDir: URL, env: [String: String], label: String) throws {
        let peer = try requiredEnv("IRIS_IOS_HARNESS_PEER_OWNER_HEX", env: env)
        let device = try requiredEnv("IRIS_IOS_HARNESS_PEER_DEVICE_NPUB", env: env)
        guard peer.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil,
              let account = manager.state.account, let chat = manager.state.currentChat else {
            throw HarnessError.unexpected("offline fixture account or chat unavailable")
        }
        let members = chat.participants.map(\.ownerPubkeyHex).sorted()
        guard members.count <= 256, Set(members).count == members.count,
              chat.groupId == nil || UInt64(members.count) == chat.memberCount else {
            throw HarnessError.unexpected("complete bounded group membership unavailable")
        }
        let persisted = try readOfflineLanPersistedAcceptance(
            database: dataDir.appendingPathComponent("core.sqlite3"), peer: peer
        )
        let projected = manager.state.chatList.first { $0.chatId == peer }
        let direct: JsonObject = [
            "source": "exact_sqlite_exists_and_production_projection",
            "thread_exists": persisted.threadExists,
            "outgoing_message_exists": persisted.outgoingMessageExists,
            "accepted_preference_persisted": persisted.accepted,
            "blocked_preference_persisted": persisted.blocked,
            "accepted_preference_live": manager.state.preferences.acceptedOwnerPubkeys.contains(peer),
            "blocked_preference_live": manager.state.preferences.blockedOwnerPubkeys.contains(peer),
            // This is the actual production projection, including durable
            // request-decision precedence over historical outgoing messages.
            "projected_is_request": projected.map { $0.isRequest as Any } ?? NSNull(),
        ]
        let context: JsonObject = [
            "schema": 1, "observed_at_ms": Int64(Date().timeIntervalSince1970 * 1_000),
            "owner_pubkey_hex": account.publicKeyHex, "device_npub": account.deviceNpub,
            "device_public_key_hex": account.devicePublicKeyHex,
            "chat_id": chat.chatId, "group_id": chat.groupId.map { $0 as Any } ?? NSNull(),
            "group_accepted": !chat.isRequest, "member_owner_hexes": members,
            "member_count": chat.memberCount, "peer_owner_hex": peer, "peer_device_npub": device,
            "direct_peer": direct,
        ]
        let data = try JSONSerialization.data(withJSONObject: context, options: [.sortedKeys])
        status("offline_lan_\(label)", String(decoding: data, as: UTF8.self))
    }
}
