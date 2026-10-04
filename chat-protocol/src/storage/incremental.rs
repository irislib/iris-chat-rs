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
    try_put_with(
        conn,
        (owner, device, key),
        value,
        |blob, bytes, offset| blob.read_at_exact(bytes, offset),
        |blob, bytes, offset| blob.write_at(bytes, offset),
        |blob| blob.close(),
    )
}

fn try_put_with(
    conn: &mut Connection,
    namespace: (&str, &str, &str),
    value: &str,
    mut read: impl FnMut(&Blob<'_>, &mut [u8], usize) -> rusqlite::Result<()>,
    mut write: impl FnMut(&mut Blob<'_>, &[u8], usize) -> rusqlite::Result<()>,
    close: impl FnOnce(Blob<'_>) -> rusqlite::Result<()>,
) -> StorageResult<bool> {
    if value.len() < MIN_INCREMENTAL_BYTES {
        return Ok(false);
    }
    let (owner, device, key) = namespace;
    let transaction = conn.savepoint().map_err(SqliteStorageAdapter::map_err)?;
    let rowid: Option<i64> = transaction.query_row(
        "SELECT rowid FROM ndr_kv WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key = ?3",
        (owner, device, key), |row| row.get(0),
    ).optional().map_err(SqliteStorageAdapter::map_err)?;
    let Some(rowid) = rowid else {
        transaction
            .commit()
            .map_err(SqliteStorageAdapter::map_err)?;
        return Ok(false);
    };
    let mut blob = transaction
        .blob_open(DatabaseName::Main, "ndr_kv", "value", rowid, false)
        .map_err(SqliteStorageAdapter::map_err)?;
    // Blob metadata gives the byte length of TEXT, including UTF-8 and NULs,
    // without materializing another full checkpoint in either SQLite or Rust.
    if blob.len() != value.len() {
        close(blob).map_err(SqliteStorageAdapter::map_err)?;
        transaction
            .commit()
            .map_err(SqliteStorageAdapter::map_err)?;
        return Ok(false);
    }
    let mut previous = [0; PAGE_BYTES];
    for (index, new) in value.as_bytes().chunks(PAGE_BYTES).enumerate() {
        let offset = index * PAGE_BYTES;
        let old = &mut previous[..new.len()];
        read(&blob, old, offset).map_err(SqliteStorageAdapter::map_err)?;
        if old != new {
            write(&mut blob, new, offset).map_err(SqliteStorageAdapter::map_err)?;
        }
    }
    // Propagate close failures before committing; Drop cannot report them.
    close(blob).map_err(SqliteStorageAdapter::map_err)?;
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
    fn unicode_and_embedded_nul_updates_compare_bytes_through_the_partial_last_page() {
        let (_dir, shared, adapter) = database();
        let original = format!("{}🦀", "ä\0".repeat(24 * 1024));
        adapter.put("state", original.clone()).unwrap();
        let mut changed = original;
        changed.replace_range(0..2, "ö");
        let last = changed.len() - 4;
        changed.replace_range(last..last + 4, "🐬");
        let mut writes = Vec::new();
        assert!(try_put_with(
            &mut shared.lock().unwrap(),
            ("owner", "device", "state"),
            &changed,
            |blob, bytes, offset| blob.read_at_exact(bytes, offset),
            |blob, bytes, offset| {
                writes.push((offset, bytes.len()));
                blob.write_at(bytes, offset)
            },
            |blob| blob.close(),
        )
        .unwrap());
        let last_page = (changed.len() - 1) / PAGE_BYTES * PAGE_BYTES;
        assert_eq!(
            writes,
            [(0, PAGE_BYTES), (last_page, changed.len() - last_page)]
        );
        assert_eq!(adapter.get("state").unwrap(), Some(changed.clone()));
        writes.clear();
        assert!(try_put_with(
            &mut shared.lock().unwrap(),
            ("owner", "device", "state"),
            &changed,
            |blob, bytes, offset| blob.read_at_exact(bytes, offset),
            |blob, bytes, offset| {
                writes.push((offset, bytes.len()));
                blob.write_at(bytes, offset)
            },
            |blob| blob.close(),
        )
        .unwrap());
        assert!(writes.is_empty(), "equal bytes must not be written");
    }

    #[test]
    fn absent_rows_and_different_byte_lengths_fall_back_without_changing_text() {
        let (_dir, shared, adapter) = database();
        let original = "ä".repeat(40 * 1024);
        adapter.put("state", original.clone()).unwrap();
        for (owner, device, key, value) in [
            ("other", "device", "state", original.clone()),
            ("owner", "other", "state", original.clone()),
            ("owner", "device", "missing", original.clone()),
            ("owner", "device", "state", "a".repeat(original.len() - 1)),
            ("owner", "device", "state", "a".repeat(original.len() + 1)),
        ] {
            assert!(!try_put(&mut shared.lock().unwrap(), owner, device, key, &value).unwrap());
            assert_eq!(adapter.get("state").unwrap(), Some(original.clone()));
            assert!(shared.lock().unwrap().is_autocommit());
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
            ("owner", "device", "state"),
            &changed,
            |blob, bytes, offset| blob.read_at_exact(bytes, offset),
            |blob, bytes, offset| {
                writes += 1;
                if writes == 2 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                blob.write_at(bytes, offset)
            },
            |blob| blob.close(),
        );
        assert!(result.is_err());
        assert_eq!(writes, 2);
        assert_eq!(adapter.get("state").unwrap(), Some(original));
        assert!(shared.lock().unwrap().is_autocommit());
    }

    #[test]
    fn failed_read_after_a_write_rolls_back_only_its_savepoint() {
        let (_dir, shared, adapter) = database();
        let original = "a".repeat(128 * 1024);
        adapter.put("state", original.clone()).unwrap();
        shared
            .lock()
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        adapter.put("earlier", "caller's update".into()).unwrap();
        let mut changed = original.clone();
        changed.replace_range(0..1, "b");
        let mut reads = 0;
        let mut writes = 0;
        let result = try_put_with(
            &mut shared.lock().unwrap(),
            ("owner", "device", "state"),
            &changed,
            |blob, bytes, offset| {
                reads += 1;
                if reads == 2 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                blob.read_at_exact(bytes, offset)
            },
            |blob, bytes, offset| {
                writes += 1;
                blob.write_at(bytes, offset)
            },
            |blob| blob.close(),
        );
        assert!(result.is_err());
        assert_eq!((reads, writes), (2, 1));
        assert_eq!(adapter.get("state").unwrap(), Some(original));
        assert_eq!(
            adapter.get("earlier").unwrap(),
            Some("caller's update".into())
        );
        assert!(!shared.lock().unwrap().is_autocommit());
        shared.lock().unwrap().execute_batch("ROLLBACK").unwrap();
        assert_eq!(adapter.get("earlier").unwrap(), None);
    }

    #[test]
    fn close_failure_is_propagated_before_commit_or_length_fallback() {
        let (_dir, shared, adapter) = database();
        let original = "a".repeat(128 * 1024);
        adapter.put("state", original.clone()).unwrap();
        let mut changed = original.clone();
        changed.replace_range(0..1, "b");
        for (value, expected_writes) in [
            (original.clone(), 0),
            (changed, 1),
            (format!("{original}a"), 0),
        ] {
            let mut writes = 0;
            let mut closes = 0;
            let result = try_put_with(
                &mut shared.lock().unwrap(),
                ("owner", "device", "state"),
                &value,
                |blob, bytes, offset| blob.read_at_exact(bytes, offset),
                |blob, bytes, offset| {
                    writes += 1;
                    blob.write_at(bytes, offset)
                },
                |blob| {
                    closes += 1;
                    blob.close()?;
                    Err(rusqlite::Error::InvalidQuery)
                },
            );
            assert_eq!(
                result.unwrap_err().to_string(),
                rusqlite::Error::InvalidQuery.to_string()
            );
            assert_eq!((writes, closes), (expected_writes, 1));
            assert_eq!(adapter.get("state").unwrap(), Some(original.clone()));
            assert!(shared.lock().unwrap().is_autocommit());
        }
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
