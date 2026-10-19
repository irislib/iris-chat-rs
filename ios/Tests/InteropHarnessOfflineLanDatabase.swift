import Foundation
import SQLite3

struct OfflineLanPersistedAcceptance: Equatable {
    let threadExists: Bool
    let outgoingMessageExists: Bool
    let accepted: Bool
    let blocked: Bool
}

struct OfflineLanDatabaseError: Error, CustomStringConvertible {
    let stage: String
    let code: Int32
    let extendedCode: Int32
    let preferenceRows: Int32?

    var description: String {
        "offline fixture acceptance \(stage) failed (SQLite \(code)/\(extendedCode), preferences rows: \(preferenceRows.map(String.init) ?? "unobserved"))"
    }
}

/// Observe exact persisted state without writing or treating a failed read as absence.
func readOfflineLanPersistedAcceptance(
    database: URL, peer: String, busyTimeoutMilliseconds: Int32 = 5_000
) throws -> OfflineLanPersistedAcceptance {
    guard (1...5_000).contains(busyTimeoutMilliseconds) else {
        throw OfflineLanDatabaseError(stage: "timeout bound", code: SQLITE_MISUSE,
                                      extendedCode: SQLITE_MISUSE, preferenceRows: nil)
    }
    var db: OpaquePointer?
    let opened = sqlite3_open_v2(database.path, &db, SQLITE_OPEN_READONLY, nil)
    defer { sqlite3_close(db) }
    func failure(_ stage: String, _ code: Int32, rows: Int32? = nil) -> OfflineLanDatabaseError {
        OfflineLanDatabaseError(stage: stage, code: code,
                                extendedCode: sqlite3_extended_errcode(db), preferenceRows: rows)
    }
    guard opened == SQLITE_OK else { throw failure("open", opened) }
    // Mobile uses DELETE journaling to avoid suspension-time WAL locks. Its
    // foreground writer can briefly exclude this separate diagnostic reader.
    // Match the core's bounded wait; never change journaling or acceptance.
    let timeout = sqlite3_busy_timeout(db, busyTimeoutMilliseconds)
    guard timeout == SQLITE_OK else { throw failure("busy timeout", timeout) }
    let sql = """
        SELECT EXISTS(SELECT 1 FROM threads WHERE chat_id=?1),
               EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND is_outgoing=1),
               EXISTS(SELECT 1 FROM preferences p, json_each(p.accepted_owner_pubkeys_json) WHERE p.id=1 AND value=?1),
               EXISTS(SELECT 1 FROM preferences p, json_each(p.blocked_owner_pubkeys_json) WHERE p.id=1 AND value=?1),
               (SELECT COUNT(*) FROM preferences WHERE id=1)
        """
    var statement: OpaquePointer?
    defer { sqlite3_finalize(statement) }
    let prepared = sqlite3_prepare_v2(db, sql, -1, &statement, nil)
    guard prepared == SQLITE_OK else { throw failure("prepare", prepared) }
    let bound = peer.withCString {
        sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
    }
    guard bound == SQLITE_OK else { throw failure("bind", bound) }
    let stepped = sqlite3_step(statement)
    guard stepped == SQLITE_ROW else { throw failure("step", stepped) }
    let preferenceRows = sqlite3_column_int(statement, 4)
    guard preferenceRows == 1 else { throw failure("preferences row count", stepped, rows: preferenceRows) }
    let result = OfflineLanPersistedAcceptance(
        threadExists: sqlite3_column_int(statement, 0) != 0,
        outgoingMessageExists: sqlite3_column_int(statement, 1) != 0,
        accepted: sqlite3_column_int(statement, 2) != 0,
        blocked: sqlite3_column_int(statement, 3) != 0
    )
    let finished = sqlite3_step(statement)
    guard finished == SQLITE_DONE else { throw failure("bounded result", finished) }
    return result
}
