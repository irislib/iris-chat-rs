import Foundation
import SQLite3
import XCTest

final class OfflineLanDatabaseTests: XCTestCase {
    private let peer = String(repeating: "a", count: 64)

    private func database() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("test.sqlite3")
        var db: OpaquePointer?
        XCTAssertEqual(sqlite3_open(url.path, &db), SQLITE_OK)
        defer { sqlite3_close(db) }
        let sql = """
            PRAGMA journal_mode=DELETE;
            CREATE TABLE threads(chat_id TEXT);
            CREATE TABLE messages(chat_id TEXT,is_outgoing INTEGER);
            CREATE TABLE preferences(id INTEGER PRIMARY KEY,accepted_owner_pubkeys_json TEXT,blocked_owner_pubkeys_json TEXT);
            INSERT INTO preferences VALUES(1,'["\(peer)"]','[]');
            INSERT INTO threads VALUES('\(peer)');
            INSERT INTO messages VALUES('\(peer)',1);
            """
        XCTAssertEqual(sqlite3_exec(db, sql, nil, nil, nil), SQLITE_OK)
        return url
    }

    func testWaitsForBriefRollbackJournalWriterAndPreservesExactState() throws {
        let url = try database()
        let writer = try OfflineLanLockedTestWriter(database: url)
        let finished = DispatchSemaphore(value: 0)
        DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(100)) {
            writer.release()
            finished.signal()
        }
        defer { XCTAssertEqual(finished.wait(timeout: .now() + 5), .success) }
        let observed = try readOfflineLanPersistedAcceptance(database: url, peer: peer)
        XCTAssertEqual(observed, OfflineLanPersistedAcceptance(
            threadExists: true, outgoingMessageExists: true, accepted: true, blocked: false
        ))
    }

    func testHeldWriterFailsWithinBoundAndReportsSQLiteCode() throws {
        let url = try database()
        let writer = try OfflineLanLockedTestWriter(database: url)
        defer { writer.release() }
        let started = ProcessInfo.processInfo.systemUptime
        XCTAssertThrowsError(try readOfflineLanPersistedAcceptance(
            database: url, peer: peer, busyTimeoutMilliseconds: 50
        )) { error in
            guard let failure = error as? OfflineLanDatabaseError else {
                return XCTFail("Expected SQLite diagnostic, got \(error)")
            }
            XCTAssertEqual(failure.code & 0xff, SQLITE_BUSY)
            XCTAssertTrue(["prepare", "step"].contains(failure.stage))
        }
        let elapsed = ProcessInfo.processInfo.systemUptime - started
        XCTAssertGreaterThanOrEqual(elapsed, 0.03, "The reader must wait for the held writer instead of immediately failing")
        XCTAssertLessThan(elapsed, 2)
    }

    func testMissingPreferencesAreNotReportedAsAbsentAcceptance() throws {
        let url = try database()
        var db: OpaquePointer?
        XCTAssertEqual(sqlite3_open(url.path, &db), SQLITE_OK)
        XCTAssertEqual(sqlite3_exec(db, "DELETE FROM preferences", nil, nil, nil), SQLITE_OK)
        sqlite3_close(db)
        XCTAssertThrowsError(try readOfflineLanPersistedAcceptance(database: url, peer: peer)) { error in
            let failure = error as? OfflineLanDatabaseError
            XCTAssertEqual(failure?.stage, "preferences row count")
            XCTAssertEqual(failure?.preferenceRows, 0)
        }
    }
}

/// SQLite's serialized connection can be released by the scheduled test worker.
private final class OfflineLanLockedTestWriter: @unchecked Sendable {
    private let db: OpaquePointer

    init(database: URL) throws {
        var handle: OpaquePointer?
        let opened = sqlite3_open_v2(database.path, &handle, SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX, nil)
        guard opened == SQLITE_OK, let handle else {
            sqlite3_close(handle)
            throw NSError(domain: "OfflineLanDatabaseTests", code: Int(opened))
        }
        db = handle
        let locked = sqlite3_exec(db, "BEGIN EXCLUSIVE", nil, nil, nil)
        guard locked == SQLITE_OK else {
            sqlite3_close(db)
            throw NSError(domain: "OfflineLanDatabaseTests", code: Int(locked))
        }
    }

    func release() { sqlite3_exec(db, "ROLLBACK", nil, nil, nil) }
    deinit { sqlite3_close(db) }
}
