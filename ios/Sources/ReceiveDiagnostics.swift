#if DEBUG && os(iOS)
import Foundation
import SQLite3
import UserNotifications

@MainActor
func captureReceiveDiagnosticsIfRequested(
    manager: AppManager, dataDir: URL, environment: [String: String]
) {
    guard environment["IRIS_DEBUG_CAPTURE_RECEIVE"] == "1" else { return }
    Task { @MainActor [weak manager] in
        try? await Task.sleep(nanoseconds: 5_000_000_000)
        guard let manager else { return }
        let notifications = await UNUserNotificationCenter.current().deliveredNotifications()
        let support = await manager.supportBundleJsonAsync()
        let notificationPayloads = notifications.map { notification -> [String: Any] in
            var payload: [String: Any] = [:]
            for (key, value) in notification.request.content.userInfo {
                if let key = key as? String { payload[key] = value }
            }
            return payload
        }
        let payloadData = (try? JSONSerialization.data(withJSONObject: notificationPayloads)) ?? Data("[]".utf8)
        await Task.detached(priority: .utility) {
            var diagnostic = receiveDatabaseDiagnostics(dataDir: dataDir)
            diagnostic["support"] = (try? JSONSerialization.jsonObject(with: Data(support.utf8))) ?? [:]
            diagnostic["notification_payloads"] = (try? JSONSerialization.jsonObject(with: payloadData)) ?? []
            let target = FileManager.default.temporaryDirectory.appendingPathComponent("iris-receive-diagnostic.json")
            if let data = try? JSONSerialization.data(withJSONObject: diagnostic, options: [.sortedKeys]) {
                try? data.write(to: target, options: .atomic)
            }
        }.value
    }
}

// Read only selected metadata. Secret keys, ratchet state and message bodies never leave the phone.
private func receiveDatabaseDiagnostics(dataDir: URL) -> [String: Any] {
    var output: [String: Any] = [:]
    var db: OpaquePointer?
    guard sqlite3_open_v2(dataDir.appendingPathComponent("core.sqlite3").path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
        if let db { sqlite3_close(db) }
        return ["database_read_error": true]
    }
    defer { sqlite3_close(db) }
    sqlite3_busy_timeout(db, 1000)
    func rows(_ sql: String) -> [[String: Any]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(statement) }
        var result: [[String: Any]] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            var row: [String: Any] = [:]
            for i in 0..<sqlite3_column_count(statement) {
                let key = String(cString: sqlite3_column_name(statement, i))
                if sqlite3_column_type(statement, i) == SQLITE_INTEGER {
                    row[key] = sqlite3_column_int64(statement, i)
                } else if let value = sqlite3_column_text(statement, i) {
                    row[key] = String(cString: value)
                }
            }
            result.append(row)
        }
        return result
    }
    let records = rows("SELECT value FROM ndr_kv WHERE key = 'appcore/protocol-engine-state-v1'")
    output["protocol_records"] = records.compactMap { row -> [String: Any]? in
        guard let value = row["value"] as? String,
              let state = try? JSONSerialization.jsonObject(with: Data(value.utf8)) as? [String: Any] else { return nil }
        let pending = (state["pending_inbound"] as? [[String: Any]] ?? []).map { item -> [String: Any] in
            var result: [String: Any] = [:]
            for key in ["event", "event_id", "created_at_secs", "next_retry_at_secs", "sender_message_pubkey_hex", "resolved_owner_pubkey_hex", "claimed_owner_pubkey_hex", "metadata_verified"] {
                result[key] = item[key]
            }
            return result
        }
        let delivery = (state["pending_decrypted_deliveries"] as? [[String: Any]] ?? []).map { item -> [String: Any] in
            var result: [String: Any] = [:]
            for key in ["event_id", "created_at_secs", "sender", "sender_device", "conversation_owner"] { result[key] = item[key] }
            return result
        }
        let sessions = state["session_manager"] as? [String: Any] ?? [:]
        let users = (sessions["users"] as? [[String: Any]] ?? []).map { user -> [String: Any] in
            let devices = (user["devices"] as? [[String: Any]] ?? []).map { device -> [String: Any] in
                var result: [String: Any] = [:]
                for key in ["device_pubkey", "claimed_owner_pubkey", "authorized", "is_stale"] { result[key] = device[key] }
                return result
            }
            return ["owner": user["owner_pubkey"] ?? "", "devices": devices]
        }
        return ["pending_inbound": pending, "pending_delivery": delivery, "users": users,
                "signed_evidence": state["invite_owner_app_keys_evidence"] ?? [:],
                "ratchet_signed_evidence": sessions["verified_peer_app_keys_events"] ?? []]
    }
    output["recent_messages"] = rows("SELECT chat_id, id, source_event_id, is_outgoing, created_at_secs, length(body) AS body_length FROM messages ORDER BY created_at_secs DESC LIMIT 100")
    output["recent_seen_events"] = rows("SELECT event_id FROM seen_events ORDER BY sequence DESC LIMIT 500")
    output["chat_deletions"] = rows("SELECT key, value FROM app_meta WHERE key LIKE 'chat_deleted_at:%'")
    let input = FileManager.default.temporaryDirectory.appendingPathComponent("iris-receive-input.json")
    if let data = try? Data(contentsOf: input),
       let events = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]],
       let bundle = KeychainSecretStore().load() {
        output["event_previews"] = events.prefix(50).compactMap { event -> [String: Any]? in
            guard let data = try? JSONSerialization.data(withJSONObject: event), let eventJson = String(data: data, encoding: .utf8) else { return nil }
            let diagnostic = diagnoseMobilePushReceive(dataDir: dataDir.path, ownerPubkeyHex: bundle.ownerPubkeyHex, deviceNsec: bundle.deviceNsec, rawEventJson: eventJson)
            return (try? JSONSerialization.jsonObject(with: Data(diagnostic.utf8))) as? [String: Any]
        }
    }
    return output
}
#endif
