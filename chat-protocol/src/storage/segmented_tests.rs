use super::{SharedConnection, SqliteStorageAdapter, StorageAdapter};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

const CHUNK: usize = 64 * 1024;

fn database() -> (tempfile::TempDir, SharedConnection, SqliteStorageAdapter) {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("state.sqlite3")).unwrap();
    conn.execute_batch(
        "PRAGMA page_size=4096; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
         CREATE TABLE ndr_kv(owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
         key TEXT NOT NULL, value TEXT NOT NULL,
         PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));",
    )
    .unwrap();
    let shared = Arc::new(Mutex::new(conn));
    let adapter = SqliteStorageAdapter::new(shared.clone(), "owner".into(), "device".into());
    (dir, shared, adapter)
}

fn page_writes(shared: &SharedConnection) -> u64 {
    let conn = shared.lock().unwrap();
    let mut current = 0;
    let mut highwater = 0;
    // Read SQLite's actual page writes while the connection mutex is held.
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
    current as u64
}

fn wal_bytes(dir: &tempfile::TempDir) -> u64 {
    std::fs::metadata(dir.path().join("state.sqlite3-wal"))
        .unwrap()
        .len()
}

fn assert_reopened(dir: &tempfile::TempDir, expected: &str) {
    let conn = Connection::open_with_flags(
        dir.path().join("state.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let reader =
        SqliteStorageAdapter::new(Arc::new(Mutex::new(conn)), "owner".into(), "device".into());
    assert_eq!(reader.get("state").unwrap().as_deref(), Some(expected));
}

#[test]
fn segmented_one_byte_growth_bounds_actual_wal_writes_and_noop() {
    let (dir, shared, adapter) = database();
    let mut value = format!("{{\"payload\":\"{}\"}}", "a".repeat(4 * 1024 * 1024));
    adapter.put("state", value.clone()).unwrap();
    let pages = page_writes(&shared);
    let wal = wal_bytes(&dir);
    value.insert(value.len() - 2, 'b');
    adapter.put("state", value.clone()).unwrap();
    assert_eq!(
        adapter.get("state").unwrap().as_deref(),
        Some(value.as_str())
    );
    assert_reopened(&dir, &value);
    let written = (page_writes(&shared) - pages) * 4096;
    let wal_growth = wal_bytes(&dir) - wal;
    assert!(written <= 512 * 1024 && wal_growth <= 512 * 1024,
        "one byte growth must not rewrite the whole checkpoint: page_bytes={written}, wal_bytes={wal_growth}");
    let pages = page_writes(&shared);
    let wal = wal_bytes(&dir);
    adapter.put("state", value).unwrap();
    assert_eq!(
        page_writes(&shared),
        pages,
        "identical value must not dirty pages"
    );
    assert_eq!(
        wal_bytes(&dir),
        wal,
        "identical value must not append WAL frames"
    );
}

#[test]
fn segmented_migrates_legacy_text_once_then_bounds_partial_mutation() {
    let (dir, shared, adapter) = database();
    let mut value = "a".repeat(4 * 1024 * 1024);
    shared
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO ndr_kv VALUES('owner','device','state',?1)",
            [&value],
        )
        .unwrap();
    assert_reopened(&dir, &value);
    let before_io = super::checkpoint_io_bytes();
    adapter.put("state", value.clone()).unwrap();
    let after_io = super::checkpoint_io_bytes();
    assert!(
        after_io.1 - before_io.1 >= value.len(),
        "migration must account for all chunk writes"
    );
    let anchor: String = shared
        .lock()
        .unwrap()
        .query_row("SELECT value FROM ndr_kv", [], |r| r.get(0))
        .unwrap();
    assert!(anchor.is_empty(), "no full compatibility mirror");
    let pages = page_writes(&shared);
    let written_before = super::checkpoint_io_bytes().1;
    value.replace_range(CHUNK + 17..CHUNK + 18, "b");
    adapter.put("state", value.clone()).unwrap();
    assert_eq!(super::checkpoint_io_bytes().1 - written_before, 4096);
    let written = (page_writes(&shared) - pages) * 4096;
    assert!(
        written <= 512 * 1024,
        "partial mutation page bytes={written}"
    );
    assert_reopened(&dir, &value);
}

#[test]
fn segmented_one_byte_shrink_keeps_unchanged_chunks() {
    let (dir, shared, adapter) = database();
    let mut value = "a".repeat(4 * 1024 * 1024 + 17);
    adapter.put("state", value.clone()).unwrap();
    let pages = page_writes(&shared);
    let wal = wal_bytes(&dir);
    value.pop();
    adapter.put("state", value.clone()).unwrap();
    assert_reopened(&dir, &value);
    let written = (page_writes(&shared) - pages) * 4096;
    let wal_growth = wal_bytes(&dir) - wal;
    assert!(written <= 512 * 1024 && wal_growth <= 512 * 1024,
        "one byte shrink must preserve unrelated chunks: page_bytes={written}, wal_bytes={wal_growth}");
}

#[test]
fn segmented_utf8_nul_partial_tail_growth_and_shrink_are_exact() {
    let (dir, shared, adapter) = database();
    let mut value = format!("{}🦀\0tail", "ä".repeat(CHUNK - 1));
    for next in [
        value.clone(),
        format!("{value}🐬"),
        "ö\0".repeat(CHUNK / 2),
        "small\0🦀".into(),
        String::new(),
        value.clone(),
    ] {
        adapter.put("state", next.clone()).unwrap();
        assert_eq!(adapter.get("state").unwrap(), Some(next.clone()));
        assert_reopened(&dir, &next);
        value = next;
    }
    assert!(!value.is_empty());
    adapter.put("state", "small".into()).unwrap();
    for table in ["ndr_kv_segmented", "ndr_kv_chunks"] {
        let count: i64 = shared
            .lock()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "small replacement retires backing");
    }
}

#[test]
fn segmented_namespaces_lists_and_direct_delete_are_isolated() {
    let (_dir, shared, adapter) = database();
    let other_owner = SqliteStorageAdapter::new(shared.clone(), "other".into(), "device".into());
    let other_device = SqliteStorageAdapter::new(shared.clone(), "owner".into(), "other".into());
    let value = "x".repeat(CHUNK + 1);
    for store in [&adapter, &other_owner, &other_device] {
        store.put("prefix/state", value.clone()).unwrap();
    }
    adapter
        .put("literal", "ndr-kv-segmented-v1\0arbitrary text".into())
        .unwrap();
    assert_eq!(adapter.list("prefix/").unwrap(), vec!["prefix/state"]);
    shared
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM ndr_kv WHERE owner_pubkey_hex='owner' AND device_pubkey_hex='device'",
            [],
        )
        .unwrap();
    assert!(adapter.list("").unwrap().is_empty());
    for store in [&other_owner, &other_device] {
        assert_eq!(store.get("prefix/state").unwrap(), Some(value.clone()));
    }
    for table in ["ndr_kv_segmented", "ndr_kv_chunks"] {
        let count: i64 = shared.lock().unwrap().query_row(&format!("SELECT count(*) FROM {table} WHERE owner_pubkey_hex='owner' AND device_pubkey_hex='device'"), [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "direct namespace delete must retire backing");
    }
    other_owner.del("prefix/state").unwrap();
    assert_eq!(other_owner.get("prefix/state").unwrap(), None);
}

#[test]
fn segmented_outer_rollback_restores_value_and_schema_creation() {
    let (_dir, shared, adapter) = database();
    shared
        .lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    adapter.put("state", "a".repeat(CHUNK * 3)).unwrap();
    shared.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(adapter.get("state").unwrap(), None);
    let original = "b".repeat(CHUNK * 3);
    adapter.put("state", original.clone()).unwrap();
    shared
        .lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    adapter.put("state", format!("{original}c")).unwrap();
    adapter.put("other", "caller's write".into()).unwrap();
    shared.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(adapter.get("state").unwrap(), Some(original));
    assert_eq!(adapter.get("other").unwrap(), None);
}

#[test]
fn segmented_reader_keeps_one_generation_during_another_connections_commit() {
    let (dir, shared, adapter) = database();
    let original = "a".repeat(CHUNK * 3);
    adapter.put("state", original.clone()).unwrap();
    let writer = SqliteStorageAdapter::new(
        Arc::new(Mutex::new(
            Connection::open(dir.path().join("state.sqlite3")).unwrap(),
        )),
        "owner".into(),
        "device".into(),
    );
    let changed = "b".repeat(CHUNK * 3 + 1);
    let mut committed = false;
    let observed = super::segmented::get_with(
        &mut shared.lock().unwrap(),
        ("owner", "device", "state"),
        || {
            writer.put("state", changed.clone())?;
            committed = true;
            Ok(())
        },
    )
    .unwrap();
    assert!(
        committed,
        "second connection must commit between metadata and chunks"
    );
    assert_eq!(observed, Some(original));
    assert_eq!(adapter.get("state").unwrap(), Some(changed.clone()));
    assert_reopened(&dir, &changed);
}

#[test]
fn segmented_second_chunk_failure_rolls_back_without_losing_outer_work() {
    let (dir, shared, adapter) = database();
    let original = "a".repeat(CHUNK + 1);
    adapter.put("state", original.clone()).unwrap();
    shared.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_second BEFORE UPDATE ON ndr_kv_chunks WHEN NEW.chunk_index=1 BEGIN SELECT RAISE(ABORT, 'injected second chunk'); END; BEGIN IMMEDIATE;").unwrap();
    adapter.put("earlier", "caller's write".into()).unwrap();
    let changed = "b".repeat(CHUNK + 2);
    assert!(adapter.put("state", changed).is_err());
    assert_eq!(adapter.get("state").unwrap(), Some(original.clone()));
    assert_eq!(
        adapter.get("earlier").unwrap().as_deref(),
        Some("caller's write")
    );
    assert!(!shared.lock().unwrap().is_autocommit());
    shared
        .lock()
        .unwrap()
        .execute_batch("COMMIT; DROP TRIGGER fail_second;")
        .unwrap();
    assert_reopened(&dir, &original);
}

#[test]
fn segmented_blob_write_and_close_failures_restore_all_pages() {
    let (dir, shared, adapter) = database();
    let original = "a".repeat(CHUNK * 3);
    adapter.put("state", original.clone()).unwrap();
    let mut changed = original.clone();
    changed.replace_range(0..1, "b");
    changed.replace_range(CHUNK..CHUNK + 1, "c");
    for fail_close in [false, true] {
        let mut writes = 0;
        let mut closes = 0;
        let result = super::segmented::put_with(
            &mut shared.lock().unwrap(),
            ("owner", "device", "state"),
            &changed,
            |blob, bytes, offset| {
                writes += 1;
                if !fail_close && writes == 2 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                blob.write_at(bytes, offset)
            },
            |blob| {
                closes += 1;
                blob.close()?;
                if fail_close {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!((writes, closes), if fail_close { (1, 1) } else { (2, 1) });
        assert!(shared.lock().unwrap().is_autocommit());
        assert_reopened(&dir, &original);
    }
}

#[test]
fn segmented_commit_failure_and_migration_failure_are_atomic() {
    let (dir, shared, adapter) = database();
    // Establish the schema, then seed an authentic legacy TEXT value.
    adapter.put("unrelated", "keep".into()).unwrap();
    let original = "a".repeat(CHUNK * 3);
    shared
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO ndr_kv VALUES('owner','device','state',?1)",
            [&original],
        )
        .unwrap();
    shared.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_migration BEFORE INSERT ON ndr_kv_chunks WHEN NEW.chunk_index=1 BEGIN SELECT RAISE(ABORT,'injected migration'); END;").unwrap();
    assert!(adapter.put("state", format!("{original}b")).is_err());
    assert_reopened(&dir, &original);
    shared
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_migration")
        .unwrap();
    adapter.put("state", original.clone()).unwrap();
    unsafe extern "C" fn reject_commit(_: *mut std::ffi::c_void) -> std::ffi::c_int {
        1
    }
    {
        let conn = shared.lock().unwrap();
        // The callback has no captured state and is removed before releasing
        // this connection to any subsequent operation after the tested put.
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(
                conn.handle(),
                Some(reject_commit),
                std::ptr::null_mut(),
            );
        }
    }
    let result = adapter.put("state", format!("{original}b"));
    {
        let conn = shared.lock().unwrap();
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(conn.handle(), None, std::ptr::null_mut());
        }
    }
    assert!(result.is_err());
    assert!(shared.lock().unwrap().is_autocommit());
    assert_reopened(&dir, &original);
}

#[test]
fn segmented_late_read_corruption_is_rejected_before_any_write() {
    let (_dir, shared, adapter) = database();
    adapter.put("state", "a".repeat(CHUNK * 3)).unwrap();
    shared
        .lock()
        .unwrap()
        .execute("DELETE FROM ndr_kv_chunks WHERE chunk_index=2", [])
        .unwrap();
    let pages = page_writes(&shared);
    assert!(adapter.put("state", "b".repeat(CHUNK * 3)).is_err());
    let first: Vec<u8> = shared
        .lock()
        .unwrap()
        .query_row(
            "SELECT data FROM ndr_kv_chunks WHERE chunk_index=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(first, vec![b'a'; CHUNK]);
    assert_eq!(page_writes(&shared), pages);
}

#[test]
fn segmented_incomplete_or_unknown_format_is_rejected() {
    for corruption in [
        "DELETE FROM ndr_kv_chunks WHERE chunk_index=1",
        "UPDATE ndr_kv_chunks SET data=x'00' WHERE chunk_index=1",
        "UPDATE ndr_kv_segmented SET format_version=99",
        "UPDATE ndr_kv_chunks SET data=zeroblob(65536) WHERE chunk_index=0; UPDATE ndr_kv_chunks SET data=x'ff' WHERE chunk_index=2",
        "DELETE FROM ndr_kv_segmented",
        "DROP TABLE ndr_kv_chunks",
    ] {
        let (_dir, shared, adapter) = database();
        adapter.put("state", "a".repeat(CHUNK * 2 + 1)).unwrap();
        shared.lock().unwrap().execute_batch(corruption).unwrap();
        assert!(adapter.get("state").is_err(), "corruption must fail closed: {corruption}");
    }
}
