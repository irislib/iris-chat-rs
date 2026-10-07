import CryptoKit
import Foundation
import SQLite3
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

/// Reads one explicitly selected, already stopped BLE UI-test account without
/// opening its secret store or bootstrapping a network client.
final class BleSavedStateDiagnosticsTests: XCTestCase {
    func testExportSavedBleState() throws {
        let env = ProcessInfo.processInfo.environment
        guard let sourceRun = env["IRIS_BLE_DIAGNOSTIC_SOURCE_RUN"] else {
            throw XCTSkip("An explicit saved Bluetooth UI-test run is required")
        }
        let readerRun = try XCTUnwrap(AppPaths.testRunId(environment: env))
        try requireBleRun(sourceRun)
        XCTAssertNotEqual(readerRun, sourceRun, "The test host must not reopen the account being inspected")
        guard readerRun.hasPrefix("ble-diagnostic-reader-"), readerRun != sourceRun,
              env["IRIS_UI_TEST_RESET"] != "1", env["IRIS_UI_TEST_DATA_DIR"] == nil else {
            throw BleDiagnosticError.invalidInput
        }
        let probe = try XCTUnwrap(env["IRIS_BLE_DIAGNOSTIC_PROBE"])
        guard probe.range(of: "^idle-probe-[a-f0-9]{32}$", options: .regularExpression) != nil else {
            throw BleDiagnosticError.invalidInput
        }
        // Resolve the app's supported storage container without invoking the
        // normal data-directory helper, which can migrate legacy files.
        #if os(iOS)
        let base = try XCTUnwrap(FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: AppPaths.appGroupIdentifier
        ))
        #else
        let base = try XCTUnwrap(FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first)
        #endif
        let directory = base.appendingPathComponent(sourceRun, isDirectory: true)
        let database = directory.appendingPathComponent("core.sqlite3")
        guard FileManager.default.fileExists(atPath: database.path) else {
            throw BleDiagnosticError.missingDatabase
        }
        let report = try bleStoredDiagnostics(database: database, probe: probe)
        let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys, .prettyPrinted])
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.json")
        attachment.name = "saved-ble-public-routing.json"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testOrdinaryAndUnvalidatedStorageIsRejected() throws {
        for run in ["iris-chat", "harness-example", "ble-idle-../iris-chat", "ble-idle-", "ble-idle-" + String(repeating: "g", count: 32)] {
            XCTAssertThrowsError(try requireBleRun(run))
        }
        XCTAssertNoThrow(try requireBleRun("ble-idle-" + String(repeating: "a", count: 32)))
    }

    func testReaderPreservesDatabaseAndExcludesSecretsAndMessageContents() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("core.sqlite3")
        var db: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &db), SQLITE_OK)
        let owner = String(repeating: "a", count: 64), device = String(repeating: "b", count: 64)
        let schema = """
        CREATE TABLE app_meta(key TEXT,value TEXT);
        INSERT INTO app_meta VALUES('account_owner_pubkey_hex','\(owner)'),('private_key','secret-value');
        CREATE TABLE ndr_kv(owner_pubkey_hex TEXT,device_pubkey_hex TEXT,value TEXT);
        INSERT INTO ndr_kv VALUES('\(owner)','\(device)','secret-session');
        CREATE TABLE app_keys(owner_pubkey_hex TEXT,devices_json TEXT);
        INSERT INTO app_keys VALUES('\(owner)','[{"identity_pubkey_hex":"\(device)","secret":"hidden"}]');
        CREATE TABLE preferences(nearby_enabled INTEGER,nearby_bluetooth_enabled INTEGER,nearby_lan_enabled INTEGER,nearby_mailbag_enabled INTEGER);
        INSERT INTO preferences VALUES(1,1,0,1);
        CREATE TABLE messages(id TEXT,chat_id TEXT,body TEXT,is_outgoing INTEGER,delivery TEXT,source_event_id TEXT,outgoing_event_json TEXT,delivery_trace_json TEXT);
        INSERT INTO messages VALUES('test','\(owner)','probe',1,'sent',NULL,'private-body','{}');
        CREATE TABLE pending_relay_publishes(event_id TEXT,chat_id TEXT,attempt_count INTEGER,event_json TEXT);
        INSERT INTO pending_relay_publishes VALUES('event','\(owner)',1,'private-payload');
        """
        XCTAssertEqual(sqlite3_exec(db, schema, nil, nil, nil), SQLITE_OK)
        sqlite3_close(db)
        let before = Data(SHA256.hash(data: try Data(contentsOf: database)))
        let report = try bleStoredDiagnostics(database: database, probe: "probe")
        let output = String(decoding: try JSONSerialization.data(withJSONObject: report), as: UTF8.self)
        for excluded in ["secret-value", "secret-session", "hidden", "private-body", "private-payload"] {
            XCTAssertFalse(output.contains(excluded))
        }
        XCTAssertEqual((report["probe_messages"] as? [[String: String]])?.first?["delivery"], "sent")
        XCTAssertEqual(Data(SHA256.hash(data: try Data(contentsOf: database))), before)
    }
}

private enum BleDiagnosticError: Error { case invalidInput, missingDatabase, unreadableDatabase, queryFailed }

private func requireBleRun(_ run: String) throws {
    guard run.range(of: "^ble-idle-[a-f0-9]{32}$", options: .regularExpression) != nil else {
        throw BleDiagnosticError.invalidInput
    }
}

private func bleStoredDiagnostics(database: URL, probe: String) throws -> [String: Any] {
    var db: OpaquePointer?
    guard sqlite3_open_v2(database.path, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
        sqlite3_close(db)
        throw BleDiagnosticError.unreadableDatabase
    }
    defer { sqlite3_close(db) }
    func rows(_ sql: String, arguments: [String] = []) throws -> [[String: String]] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { throw BleDiagnosticError.queryFailed }
        defer { sqlite3_finalize(statement) }
        for (index, argument) in arguments.enumerated() {
            let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
            guard sqlite3_bind_text(statement, Int32(index + 1), argument, -1, transient) == SQLITE_OK else {
                throw BleDiagnosticError.queryFailed
            }
        }
        var result = [[String: String]]()
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { return result }
            guard status == SQLITE_ROW, result.count < 256 else { throw BleDiagnosticError.queryFailed }
            var row = [String: String]()
            for column in 0..<sqlite3_column_count(statement) {
                if let value = sqlite3_column_text(statement, column) {
                    row[String(cString: sqlite3_column_name(statement, column))] = String(cString: value)
                }
            }
            result.append(row)
        }
    }
    let identity = try rows("SELECT value AS owner FROM app_meta WHERE key='account_owner_pubkey_hex'")
    guard identity.count == 1, let owner = identity.first?["owner"],
          owner.range(of: "^[a-f0-9]{64}$", options: .regularExpression) != nil else { throw BleDiagnosticError.invalidInput }
    let messages = try rows("SELECT id,chat_id,delivery,source_event_id,length(outgoing_event_json) AS encrypted_event_bytes,delivery_trace_json FROM messages WHERE body=? AND is_outgoing=1", arguments: [probe])
    let peers = Set(messages.compactMap { $0["chat_id"] }).union([owner])
    var rosters = [[String: Any]]()
    for peer in peers.sorted() {
        for row in try rows("SELECT devices_json FROM app_keys WHERE owner_pubkey_hex=?", arguments: [peer]) {
            let data = Data((row["devices_json"] ?? "[]").utf8)
            let devices = try JSONSerialization.jsonObject(with: data) as? [[String: Any]] ?? []
            rosters.append(["owner": peer, "devices": devices.compactMap { $0["identity_pubkey_hex"] as? String }])
        }
    }
    return ["identity": identity, "rosters": rosters, "probe_messages": messages,
            "local_devices": try rows("SELECT DISTINCT device_pubkey_hex FROM ndr_kv WHERE owner_pubkey_hex=?", arguments: [owner]),
            "nearby": try rows("SELECT nearby_enabled,nearby_bluetooth_enabled,nearby_lan_enabled,nearby_mailbag_enabled FROM preferences"),
            "pending_probe_publishes": try rows("SELECT event_id,chat_id,attempt_count FROM pending_relay_publishes WHERE chat_id IN (SELECT chat_id FROM messages WHERE body=? AND is_outgoing=1)", arguments: [probe])]
}
