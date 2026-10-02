use super::{SqliteStorageAdapter, StorageResult};
use rusqlite::{blob::Blob, Connection, DatabaseName, OptionalExtension};

const PAGE_BYTES: usize = 4096;
const MIN_INCREMENTAL_BYTES: usize = 64 * 1024;

// SQLite's ordinary TEXT replacement frees and rewrites all overflow pages,
// including unchanged bytes. Incremental I/O preserves those pages. A savepoint
// makes every changed region one atomic durable update, also inside outer txns.
pub(super) fn try_put(
    conn: &mut Connection,
    owner: &str,
    device: &str,
    key: &str,
    value: &str,
) -> StorageResult<bool> {
    try_put_with(conn, owner, device, key, value, |blob, bytes, offset| {
        blob.write_at(bytes, offset)
    })
}

fn try_put_with(
    conn: &mut Connection,
    owner: &str,
    device: &str,
    key: &str,
    value: &str,
    mut write: impl FnMut(&mut Blob<'_>, &[u8], usize) -> rusqlite::Result<()>,
) -> StorageResult<bool> {
    if value.len() < MIN_INCREMENTAL_BYTES {
        return Ok(false);
    }
    let transaction = conn.savepoint().map_err(SqliteStorageAdapter::map_err)?;
    let previous: Option<(i64, String)> = transaction.query_row(
        "SELECT rowid, value FROM ndr_kv WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key = ?3",
        (owner, device, key), |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(SqliteStorageAdapter::map_err)?;
    let Some((rowid, previous)) = previous.filter(|(_, old)| old.len() == value.len()) else {
        transaction
            .commit()
            .map_err(SqliteStorageAdapter::map_err)?;
        return Ok(false);
    };
    if previous != value {
        let mut blob = transaction
            .blob_open(DatabaseName::Main, "ndr_kv", "value", rowid, false)
            .map_err(SqliteStorageAdapter::map_err)?;
        for (index, (old, new)) in previous
            .as_bytes()
            .chunks(PAGE_BYTES)
            .zip(value.as_bytes().chunks(PAGE_BYTES))
            .enumerate()
        {
            if old != new {
                write(&mut blob, new, index * PAGE_BYTES).map_err(SqliteStorageAdapter::map_err)?;
            }
        }
        // Propagate close failures before committing; Drop cannot report them.
        blob.close().map_err(SqliteStorageAdapter::map_err)?;
    }
    transaction
        .commit()
        .map_err(SqliteStorageAdapter::map_err)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SharedConnection, StorageAdapter};
    use std::sync::{Arc, Mutex};

    fn database() -> (tempfile::TempDir, SharedConnection, SqliteStorageAdapter) {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("state.sqlite3")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; CREATE TABLE ndr_kv (
            owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
            key TEXT NOT NULL, value TEXT NOT NULL,
            PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));",
        )
        .unwrap();
        let shared = Arc::new(Mutex::new(conn));
        let adapter = SqliteStorageAdapter::new(shared.clone(), "owner".into(), "device".into());
        (dir, shared, adapter)
    }

    fn pages_written(conn: &Connection) -> i32 {
        let mut current = 0;
        let mut highwater = 0;
        // The caller holds the connection and these output integers live until
        // sqlite3_db_status returns. This only reads SQLite's page-write counter.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_db_status(
                conn.handle(),
                rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,
                &mut current,
                &mut highwater,
                0,
            )
        };
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
        current
    }

    #[test]
    fn text_updates_write_changed_pages_and_survive_reopen() {
        let (dir, shared, adapter) = database();
        let mut value = "a".repeat(1024 * 1024);
        adapter.put("state", value.clone()).unwrap();
        let before = pages_written(&shared.lock().unwrap());
        value.replace_range(4096..4100, "🦀");
        adapter.put("state", value.clone()).unwrap();
        let pages = pages_written(&shared.lock().unwrap()) - before;
        assert!(pages <= 4, "one changed region wrote {pages} pages");
        assert_eq!(adapter.get("state").unwrap(), Some(value.clone()));
        let before = pages_written(&shared.lock().unwrap());
        adapter.put("state", value.clone()).unwrap();
        assert_eq!(pages_written(&shared.lock().unwrap()), before);
        drop(adapter);
        drop(shared);
        let conn = Connection::open(dir.path().join("state.sqlite3")).unwrap();
        let restored: String = conn
            .query_row("SELECT value FROM ndr_kv", [], |r| r.get(0))
            .unwrap();
        assert_eq!(restored, value);
        let kind: String = conn
            .query_row("SELECT typeof(value) FROM ndr_kv", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            kind, "text",
            "older clients must still read an ordinary TEXT value"
        );
    }

    #[test]
    fn small_growing_and_shrinking_values_keep_exact_text() {
        let (_dir, _shared, adapter) = database();
        for value in [
            "old JSON".to_owned(),
            "ä".repeat(40000),
            "🦀".repeat(40000),
            "long".repeat(20000),
            "small again".to_owned(),
        ] {
            adapter.put("state", value.clone()).unwrap();
            assert_eq!(adapter.get("state").unwrap(), Some(value));
        }
    }

    #[test]
    fn failed_second_region_rolls_back_first_region_on_actual_text_column() {
        let (_dir, shared, adapter) = database();
        let original = "a".repeat(128 * 1024);
        adapter.put("state", original.clone()).unwrap();
        let mut changed = original.clone();
        changed.replace_range(0..1, "b");
        changed.replace_range(65536..65537, "c");
        let mut writes = 0;
        let result = try_put_with(
            &mut shared.lock().unwrap(),
            "owner",
            "device",
            "state",
            &changed,
            |blob, bytes, offset| {
                writes += 1;
                if writes == 2 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                blob.write_at(bytes, offset)
            },
        );
        assert!(result.is_err());
        assert_eq!(writes, 2);
        assert_eq!(adapter.get("state").unwrap(), Some(original));
        assert!(shared.lock().unwrap().is_autocommit());
    }
    #[test]
    fn incremental_updates_respect_accounts_and_outer_transaction_rollback() {
        let (_dir, shared, adapter) = database();
        let other = SqliteStorageAdapter::new(shared.clone(), "other".into(), "device".into());
        let another_device =
            SqliteStorageAdapter::new(shared.clone(), "owner".into(), "other-device".into());
        let original = "a".repeat(128 * 1024);
        for item in [&adapter, &other, &another_device] {
            item.put("state", original.clone()).unwrap();
        }
        shared
            .lock()
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        let mut changed = original.clone();
        changed.replace_range(4096..4100, "🦀");
        adapter.put("state", changed.clone()).unwrap();
        assert_eq!(adapter.get("state").unwrap(), Some(changed));
        assert_eq!(other.get("state").unwrap(), Some(original.clone()));
        assert_eq!(another_device.get("state").unwrap(), Some(original.clone()));
        assert!(
            !shared.lock().unwrap().is_autocommit(),
            "adapter must not commit its caller's transaction"
        );
        shared.lock().unwrap().execute_batch("ROLLBACK").unwrap();
        assert_eq!(adapter.get("state").unwrap(), Some(original));
    }
}
