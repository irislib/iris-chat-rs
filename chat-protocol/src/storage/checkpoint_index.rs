//! Transactional, disposable indexes for checkpoint metadata. The raw chunks
//! remain authoritative; full reads still validate their shape and content hash.
use super::{SqliteStorageAdapter, StorageError, StorageResult};
use rusqlite::Connection;

const SHAPES: &str = "ndr_kv_checkpoint_chunk_shapes";
// The implicit rowid suffix visits node-table pages in storage order. Indexing
// descriptors themselves would amplify every retry-clock update instead.
const NODES: &str = "CREATE INDEX ndr_kv_checkpoint_node_namespace ON ndr_kv_checkpoint_nodes(owner_pubkey_hex,device_pubkey_hex,key)";
const TABLE: &str = "CREATE TABLE ndr_kv_checkpoint_chunk_shapes(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,payload_key BLOB NOT NULL,chunk_index INTEGER NOT NULL,byte_length INTEGER NOT NULL,data_type TEXT NOT NULL,PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index)) WITHOUT ROWID";

fn identity(left: &str, right: &str) -> String {
    [
        "owner_pubkey_hex",
        "device_pubkey_hex",
        "key",
        "payload_key",
        "chunk_index",
    ]
    .map(|column| format!("{left}.{column} IS {right}.{column}"))
    .join(" AND ")
}

fn definitions() -> Vec<(&'static str, &'static str, String)> {
    let columns =
        "owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index,byte_length,data_type";
    let new_values = "NEW.owner_pubkey_hex,NEW.device_pubkey_hex,NEW.key,NEW.payload_key,NEW.chunk_index,length(NEW.data),typeof(NEW.data)";
    let upsert = format!("INSERT INTO {SHAPES}({columns}) VALUES({new_values}) ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index) DO UPDATE SET byte_length=excluded.byte_length,data_type=excluded.data_type;");
    let old = identity(SHAPES, "OLD");
    let same = identity("OLD", "NEW");
    let matches_raw = format!("EXISTS(SELECT 1 FROM ndr_kv_checkpoint_chunks AS raw WHERE {} AND length(raw.data) IS NEW.byte_length AND typeof(raw.data) IS NEW.data_type)", identity("raw", "NEW"));
    let mut result = vec![
        (SHAPES, "table", TABLE.to_owned()),
        (
            "ndr_kv_checkpoint_node_namespace",
            "index",
            NODES.to_owned(),
        ),
    ];
    for (name, event, body) in [
        ("ndr_kv_checkpoint_shape_insert", "INSERT", upsert.clone()),
        (
            "ndr_kv_checkpoint_shape_update",
            "UPDATE",
            format!("DELETE FROM {SHAPES} WHERE {old} AND NOT ({same}); {upsert}"),
        ),
        (
            "ndr_kv_checkpoint_shape_delete",
            "DELETE",
            format!("DELETE FROM {SHAPES} WHERE {old};"),
        ),
    ] {
        result.push((
            name,
            "trigger",
            format!(
                "CREATE TRIGGER {name} AFTER {event} ON ndr_kv_checkpoint_chunks BEGIN {body} END"
            ),
        ));
    }
    // These guards reject direct edits that could make the derived table lie.
    // Incremental BLOB writes do not run triggers, but cannot change length or
    // storage class. In particular, do not index expressions on chunks.data:
    // that would prevent older clients from opening their writable BLOBs.
    for (name, event, condition) in [
        (
            "ndr_kv_checkpoint_shape_guard_insert",
            "INSERT",
            format!("NOT ({matches_raw})"),
        ),
        (
            "ndr_kv_checkpoint_shape_guard_update",
            "UPDATE",
            format!("NOT ({same}) OR NOT ({matches_raw})"),
        ),
        (
            "ndr_kv_checkpoint_shape_guard_delete",
            "DELETE",
            format!(
                "EXISTS(SELECT 1 FROM ndr_kv_checkpoint_chunks AS raw WHERE {})",
                identity("raw", "OLD")
            ),
        ),
    ] {
        result.push((name, "trigger", format!("CREATE TRIGGER {name} BEFORE {event} ON {SHAPES} WHEN {condition} BEGIN SELECT RAISE(ABORT,'invalid checkpoint chunk shape'); END")));
    }
    result
}

/// Caller supplies a read snapshot or the same savepoint as the checkpoint
/// write. Never remember schema success across a transaction rollback.
pub(super) fn schema(conn: &Connection, create: bool) -> StorageResult<bool> {
    let definitions = definitions();
    let mut found = 0;
    for (name, kind, sql) in &definitions {
        let mut stmt = conn
            .prepare("SELECT type,sql FROM sqlite_schema WHERE name=?1")
            .map_err(SqliteStorageAdapter::map_err)?;
        let mut rows = stmt.query([name]).map_err(SqliteStorageAdapter::map_err)?;
        if let Some(row) = rows.next().map_err(SqliteStorageAdapter::map_err)? {
            let actual_kind: String = row.get(0).map_err(SqliteStorageAdapter::map_err)?;
            let actual_sql: String = row.get(1).map_err(SqliteStorageAdapter::map_err)?;
            if actual_kind != *kind || actual_sql != *sql {
                return Err(StorageError::new("invalid checkpoint metadata indexes"));
            }
            found += 1;
        }
    }
    if found == definitions.len() {
        return Ok(true);
    }
    if found != 0 {
        return Err(StorageError::new("incomplete checkpoint metadata indexes"));
    }
    if !create {
        return Ok(false);
    }
    conn.execute_batch(TABLE)
        .map_err(SqliteStorageAdapter::map_err)?;
    // No casts: malformed legacy lengths/types must remain visible to the
    // checkpoint validator, including rows retired by the attempted save.
    conn.execute_batch(&format!("INSERT INTO {SHAPES} SELECT owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index,length(data),typeof(data) FROM ndr_kv_checkpoint_chunks"))
        .map_err(SqliteStorageAdapter::map_err)?;
    for (_, _, sql) in definitions.iter().skip(1) {
        conn.execute_batch(sql)
            .map_err(SqliteStorageAdapter::map_err)?;
    }
    Ok(true)
}
