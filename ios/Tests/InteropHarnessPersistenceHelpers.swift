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
    func splitPersistenceThreadFiles(dataDir: URL) -> [URL] {
        let threadsDir = dataDir.appendingPathComponent("core/threads", isDirectory: true)
        return (try? FileManager.default.contentsOfDirectory(
            at: threadsDir,
            includingPropertiesForKeys: nil
        ))?.filter { $0.pathExtension == "json" } ?? []
    }

    func readSplitThread(dataDir: URL, chatID: String) -> JsonObject? {
        for url in splitPersistenceThreadFiles(dataDir: dataDir) {
            guard let thread = readJsonObject(at: url),
                  sameIdentifier(stringValue(thread["chat_id"]), chatID) else {
                continue
            }
            return thread
        }
        return nil
    }

    func splitPersistenceThreadWithMessage(
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

    func messageExists(
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
        ) != nil || sqliteThreadWithMessage(
            dataDir: dataDir,
            chatID: chatID,
            expectedMessage: message,
            direction: direction,
            peerInput: peerInput
        ) != nil
    }

    func countMessages(
        manager: AppManager,
        dataDir: URL,
        chatID: String,
        message: String,
        direction: String,
        peerInput: String?
    ) -> Int {
        let stateCount = manager.state.currentChat
            .flatMap { current -> Int? in
                guard chatMatchesExpectedChat(chatId: current.chatId, peerInput: peerInput, expectedChatID: chatID) else {
                    return nil
                }
                return current.messages.count {
                    $0.body == message && directionMatches(isOutgoing: $0.isOutgoing, direction: direction)
                }
            } ?? 0
        let splitCount = countSplitPersistenceMessages(
            dataDir: dataDir,
            chatID: chatID,
            expectedMessage: message,
            direction: direction,
            peerInput: peerInput
        )
        let sqliteCount = countSqliteMessages(
            dataDir: dataDir,
            chatID: chatID,
            expectedMessage: message,
            direction: direction,
            peerInput: peerInput
        )
        return max(stateCount, splitCount, sqliteCount)
    }

    func countSplitPersistenceMessages(
        dataDir: URL,
        chatID: String,
        expectedMessage: String,
        direction: String,
        peerInput: String?
    ) -> Int {
        for url in splitPersistenceThreadFiles(dataDir: dataDir) {
            guard let thread = readJsonObject(at: url) else { continue }
            let threadChatID = stringValue(thread["chat_id"])
            guard chatMatchesExpectedChat(chatId: threadChatID, peerInput: peerInput, expectedChatID: chatID) else {
                continue
            }
            return arrayValue(thread["messages"]).filter { messageEntry in
                guard let message = dictValue(messageEntry) else { return false }
                return stringValue(message["body"]) == expectedMessage &&
                    directionMatches(isOutgoing: boolValue(message["is_outgoing"]), direction: direction)
            }.count
        }
        return 0
    }

    func countSqliteMessages(
        dataDir: URL,
        chatID: String,
        expectedMessage: String,
        direction: String,
        peerInput: String?
    ) -> Int {
        let dbPath = dataDir.appendingPathComponent("core.sqlite3").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return 0
        }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = """
            SELECT chat_id, is_outgoing
            FROM messages
            WHERE body = ?
            LIMIT 200
        """
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            return 0
        }
        defer { sqlite3_finalize(stmt) }

        var count = 0
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        sqlite3_bind_text(stmt, 1, expectedMessage, -1, transient)
        while sqlite3_step(stmt) == SQLITE_ROW {
            let rowChatID = sqliteColumnString(stmt, 0)
            let isOutgoing = sqlite3_column_int64(stmt, 1) != 0
            if directionMatches(isOutgoing: isOutgoing, direction: direction) &&
                chatMatchesExpectedChat(chatId: rowChatID, peerInput: peerInput, expectedChatID: chatID) {
                count += 1
            }
        }
        return count
    }

    func sqliteThreadWithMessage(
        dataDir: URL,
        chatID: String?,
        expectedMessage: String,
        direction: String,
        peerInput: String?
    ) -> String? {
        let dbPath = dataDir.appendingPathComponent("core.sqlite3").path
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return nil
        }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = """
            SELECT chat_id, is_outgoing
            FROM messages
            WHERE body = ?
            ORDER BY created_at_secs DESC, id DESC
            LIMIT 50
        """
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }

        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        sqlite3_bind_text(stmt, 1, expectedMessage, -1, transient)
        while sqlite3_step(stmt) == SQLITE_ROW {
            let rowChatID = sqliteColumnString(stmt, 0)
            let isOutgoing = sqlite3_column_int64(stmt, 1) != 0
            if directionMatches(isOutgoing: isOutgoing, direction: direction) &&
                chatMatchesExpectedChat(chatId: rowChatID, peerInput: peerInput, expectedChatID: chatID) {
                return rowChatID
            }
        }
        return nil
    }

    func splitPersistenceMessageDelivery(
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

    func splitPersistenceHasPeerRoster(dataDir: URL, peerOwnerHex: String) -> Bool {
        let appKeys = readJsonArray(at: dataDir.appendingPathComponent("core/app_keys.json"))
        return arrayValue(appKeys).contains { entry in
            guard let known = dictValue(entry) else { return false }
            return sameIdentifier(stringValue(known["owner_pubkey_hex"]), peerOwnerHex) &&
                !arrayValue(known["devices"]).isEmpty
        }
    }

    func summarizeSplitPersistedPeer(dataDir: URL, manager: AppManager, peerOwnerHex: String) -> String {
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

    func splitPersistenceHasPeerSession(dataDir: URL, manager: AppManager, peerOwnerHex: String) -> Bool {
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

    func readRuntimeDebugSnapshot(dataDir: URL) -> JsonObject? {
        readJsonObject(at: dataDir.appendingPathComponent(debugSnapshotFilename))
    }

    func runtimeDebugHasPeerRoster(_ debug: JsonObject, peerOwnerHex: String) -> Bool {
        runtimeDebugKnownPeer(debug, peerOwnerHex: peerOwnerHex) { user in
            boolValue(user["has_roster"]) &&
                intValue(user["roster_device_count"]) > 0
        }
    }

    func runtimeDebugHasPeerSession(_ debug: JsonObject, peerOwnerHex: String) -> Bool {
        runtimeDebugKnownPeer(debug, peerOwnerHex: peerOwnerHex) { user in
            intValue(user["active_session_device_count"]) > 0 ||
                intValue(user["inactive_session_count"]) > 0
        }
    }

    func runtimeDebugHasPeerTransportReady(_ debug: JsonObject, peerOwnerHex: String) -> Bool {
        runtimeDebugKnownPeer(debug, peerOwnerHex: peerOwnerHex) { user in
            boolValue(user["has_roster"]) &&
                intValue(user["roster_device_count"]) > 0 &&
                intValue(user["device_count"]) > 0 &&
                intValue(user["authorized_device_count"]) > 0
        }
    }

    func runtimeDebugKnownPeer(
        _ debug: JsonObject,
        peerOwnerHex: String,
        predicate: (JsonObject) -> Bool
    ) -> Bool {
        arrayValue(debug["known_users"]).contains { entry in
            guard let user = dictValue(entry) else { return false }
            return sameIdentifier(stringValue(user["owner_pubkey_hex"]), peerOwnerHex) &&
                predicate(user)
        }
    }

    /// Read a `user/{peer}` value out of the SQLite-backed `ndr_kv` store.
    /// The pre-SQLite harness read JSON files at
    /// `{dataDir}/ndr_runtime/{owner}/{device}/user_{peer}.json`; that
    /// tree no longer exists.
    func ndrKvUser(
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

    func ndrKvAppKeysDeviceCount(
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

    func ndrKvJson(
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

    struct SqliteCoreSnapshot {
        var filePresent: Bool
        var appMeta: String = ""
        var appKeys: String = ""
        var groups: String = ""
        var threads: String = ""
        var messages: String = ""
        var pendingRelayPublishes: String = ""
    }

    func sqlitePendingRelayPublishCount(dataDir: URL) -> UInt64? {
        let dbPath = dataDir.appendingPathComponent("core.sqlite3").path
        guard FileManager.default.fileExists(atPath: dbPath) else {
            return nil
        }
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return nil
        }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT COUNT(*) FROM pending_relay_publishes", -1, &stmt, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }
        guard sqlite3_step(stmt) == SQLITE_ROW else {
            return nil
        }
        return UInt64(sqlite3_column_int64(stmt, 0))
    }

    func readSqliteCoreSnapshot(dataDir: URL) -> SqliteCoreSnapshot {
        let dbPath = dataDir.appendingPathComponent("core.sqlite3").path
        guard FileManager.default.fileExists(atPath: dbPath) else {
            return SqliteCoreSnapshot(filePresent: false)
        }
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(db)
            return SqliteCoreSnapshot(filePresent: true, appMeta: "open_error")
        }
        defer { sqlite3_close(db) }

        return SqliteCoreSnapshot(
            filePresent: true,
            appMeta: sqliteRows(db: db, sql: "SELECT key, value FROM app_meta ORDER BY key") { stmt in
                "\(sqliteColumnString(stmt, 0))=\(sqliteColumnString(stmt, 1))"
            },
            appKeys: sqliteRows(
                db: db,
                sql: """
                    SELECT owner_pubkey_hex, created_at_secs, devices_json
                    FROM app_keys
                    ORDER BY owner_pubkey_hex
                """
            ) { stmt in
                [
                    sqliteColumnString(stmt, 0),
                    String(sqlite3_column_int64(stmt, 1)),
                    String(sqliteColumnString(stmt, 2).prefix(160)),
                ].joined(separator: ",")
            },
            groups: sqliteRows(
                db: db,
                sql: """
                    SELECT group_id, name, updated_at_secs
                    FROM groups
                    ORDER BY updated_at_secs DESC, group_id
                """
            ) { stmt in
                [
                    sqliteColumnString(stmt, 0),
                    sqliteColumnString(stmt, 1),
                    String(sqlite3_column_int64(stmt, 2)),
                ].joined(separator: ",")
            },
            threads: sqliteRows(
                db: db,
                sql: """
                    SELECT chat_id, unread_count, updated_at_secs
                    FROM threads
                    ORDER BY updated_at_secs DESC, chat_id
                """
            ) { stmt in
                [
                    sqliteColumnString(stmt, 0),
                    String(sqlite3_column_int64(stmt, 1)),
                    String(sqlite3_column_int64(stmt, 2)),
                ].joined(separator: ",")
            },
            messages: sqliteRows(
                db: db,
                sql: """
                    SELECT chat_id, id, delivery, is_outgoing, body
                    FROM messages
                    ORDER BY created_at_secs DESC, id DESC
                    LIMIT 20
                """
            ) { stmt in
                [
                    sqliteColumnString(stmt, 0),
                    sqliteColumnString(stmt, 1),
                    sqliteColumnString(stmt, 2),
                    String(sqlite3_column_int64(stmt, 3)),
                    String(sqliteColumnString(stmt, 4).replacingOccurrences(of: "|", with: "/").prefix(120)),
                ].joined(separator: ",")
            },
            pendingRelayPublishes: sqliteRows(
                db: db,
                sql: """
                    SELECT label, chat_id, inner_event_id, attempt_count
                    FROM pending_relay_publishes
                    ORDER BY created_at_secs DESC
                    LIMIT 30
                """
            ) { stmt in
                [
                    sqliteColumnString(stmt, 0),
                    sqliteColumnString(stmt, 1),
                    sqliteColumnString(stmt, 2),
                    String(sqlite3_column_int64(stmt, 3)),
                ].joined(separator: ",")
            }
        )
    }

    func sqliteRows(
        db: OpaquePointer?,
        sql: String,
        row: (OpaquePointer?) -> String
    ) -> String {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            let message = db.flatMap { sqlite3_errmsg($0) }.map { String(cString: $0) } ?? "prepare_error"
            return "read_error=\(message)"
        }
        defer { sqlite3_finalize(stmt) }
        var rows: [String] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            rows.append(row(stmt))
        }
        return rows.joined(separator: "|")
    }

    func sqliteColumnString(_ stmt: OpaquePointer?, _ index: Int32) -> String {
        guard sqlite3_column_type(stmt, index) != SQLITE_NULL,
              let cString = sqlite3_column_text(stmt, index) else {
            return ""
        }
        return String(cString: cString)
    }
}
