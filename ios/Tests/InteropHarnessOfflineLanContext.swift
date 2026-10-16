import Foundation
import SQLite3
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
        var db: OpaquePointer?
        guard sqlite3_open_v2(dataDir.appendingPathComponent("core.sqlite3").path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            throw HarnessError.unexpected("offline fixture acceptance database unavailable")
        }
        defer { sqlite3_close(db) }
        let sql = """
            SELECT EXISTS(SELECT 1 FROM threads WHERE chat_id=?1),
                   EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND is_outgoing=1),
                   EXISTS(SELECT 1 FROM preferences p, json_each(p.accepted_owner_pubkeys_json) WHERE p.id=1 AND value=?1),
                   EXISTS(SELECT 1 FROM preferences p, json_each(p.blocked_owner_pubkeys_json) WHERE p.id=1 AND value=?1),
                   (SELECT COUNT(*) FROM preferences WHERE id=1)
            """
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else {
            throw HarnessError.unexpected("offline fixture acceptance query unavailable")
        }
        defer { sqlite3_finalize(statement) }
        let bound = peer.withCString {
            sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
        }
        guard bound == SQLITE_OK, sqlite3_step(statement) == SQLITE_ROW,
              sqlite3_column_int(statement, 4) == 1 else {
            throw HarnessError.unexpected("offline fixture acceptance query failed")
        }
        let projected = manager.state.chatList.first { $0.chatId == peer }
        let direct: JsonObject = [
            "source": "exact_sqlite_exists_and_production_projection",
            "thread_exists": sqlite3_column_int(statement, 0) != 0,
            "outgoing_message_exists": sqlite3_column_int(statement, 1) != 0,
            "accepted_preference_persisted": sqlite3_column_int(statement, 2) != 0,
            "blocked_preference_persisted": sqlite3_column_int(statement, 3) != 0,
            "accepted_preference_live": manager.state.preferences.acceptedOwnerPubkeys.contains(peer),
            "blocked_preference_live": manager.state.preferences.blockedOwnerPubkeys.contains(peer),
            // This is the actual production projection, including durable
            // request-decision precedence over historical outgoing messages.
            "projected_is_request": projected.map { $0.isRequest as Any } ?? NSNull(),
        ]
        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw HarnessError.unexpected("offline fixture acceptance query was not bounded")
        }
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
