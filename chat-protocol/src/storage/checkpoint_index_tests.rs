use super::*;

fn remove_indexes(conn: &Connection) {
    let objects = conn.prepare("SELECT type,name FROM sqlite_schema WHERE name GLOB 'ndr_kv_checkpoint_shape_*' OR name IN ('ndr_kv_checkpoint_chunk_shapes','ndr_kv_checkpoint_node_namespace') ORDER BY type DESC").unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    for (kind, name) in objects {
        conn.execute_batch(&format!("DROP {kind} {name}")).unwrap();
    }
}

fn indexed(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='ndr_kv_checkpoint_chunk_shapes')",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn legacy_read_and_migration_rollback_preserve_original_checkpoint() {
    let (dir, conn, storage) = database();
    let before = checkpoint(&[body(1)], &[1], "before");
    let after = checkpoint(&[body(1), body(2)], &[2, 3], "after");
    storage.put_checkpoint("state", &before).unwrap();
    remove_indexes(&conn.lock().unwrap());
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(before.materialize().unwrap())
    );
    assert!(
        !indexed(&conn.lock().unwrap()),
        "read-only legacy loads must not migrate"
    );
    conn.lock().unwrap().execute_batch("BEGIN").unwrap();
    storage.put_checkpoint("state", &after).unwrap();
    assert!(indexed(&conn.lock().unwrap()));
    conn.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert!(!indexed(&conn.lock().unwrap()));
    assert_eq!(
        storage.get("state").unwrap(),
        Some(before.materialize().unwrap())
    );
    storage.put_checkpoint("state", &after).unwrap();
    assert!(indexed(&conn.lock().unwrap()));
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(after.materialize().unwrap())
    );
}

#[test]
fn invalid_legacy_payload_rolls_back_index_migration() {
    let (_dir, conn, storage) = database();
    let value = checkpoint(&[body(1)], &[1], "core");
    storage.put_checkpoint("state", &value).unwrap();
    remove_indexes(&conn.lock().unwrap());
    conn.lock().unwrap().execute_batch("UPDATE ndr_kv_checkpoint_chunks SET data=CAST(data AS TEXT) WHERE substr(payload_key,1,1)=x'01'").unwrap();
    let before = checkpoint_rows(&conn);
    assert!(storage.put_checkpoint("state", &value).is_err());
    assert!(!indexed(&conn.lock().unwrap()));
    assert!(checkpoint_rows(&conn) == before);
}

#[test]
fn migration_commit_failure_and_anchor_delete_keep_indexes_atomic() {
    let (dir, conn, storage) = database();
    let before = checkpoint(&[body(1)], &[1], "before");
    let after = checkpoint(&[body(2)], &[2], "after");
    storage.put_checkpoint("state", &before).unwrap();
    remove_indexes(&conn.lock().unwrap());
    unsafe {
        rusqlite::ffi::sqlite3_commit_hook(
            conn.lock().unwrap().handle(),
            Some(reject_commit),
            std::ptr::null_mut(),
        );
    }
    assert!(storage.put_checkpoint("state", &after).is_err());
    unsafe {
        rusqlite::ffi::sqlite3_commit_hook(
            conn.lock().unwrap().handle(),
            None,
            std::ptr::null_mut(),
        );
    }
    assert!(!indexed(&conn.lock().unwrap()));
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(before.materialize().unwrap())
    );
    storage.put_checkpoint("state", &after).unwrap();
    storage.del("state").unwrap();
    let remaining: i64 = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM ndr_kv_checkpoint_chunk_shapes",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "anchor deletion must drain derived metadata too"
    );
}

#[test]
fn derived_schema_and_direct_tampering_fail_closed() {
    for sql in [
        "DROP TRIGGER ndr_kv_checkpoint_shape_insert",
        "DROP TRIGGER ndr_kv_checkpoint_shape_insert; CREATE TRIGGER ndr_kv_checkpoint_shape_insert AFTER INSERT ON ndr_kv_checkpoint_chunks BEGIN SELECT 1; END",
        "DROP TRIGGER ndr_kv_checkpoint_shape_guard_insert; CREATE TABLE ndr_kv_checkpoint_shape_guard_insert(x)",
    ] {
        let (_dir, conn, storage) = database();
        let value = checkpoint(&[body(1)], &[1], "core");
        storage.put_checkpoint("state", &value).unwrap();
        conn.lock().unwrap().execute_batch(sql).unwrap();
        assert!(storage.put_checkpoint("state", &value).is_err());
        assert!(storage.get("state").is_err());
    }
    let (_dir, conn, storage) = database();
    let value = checkpoint(&[body(1)], &[1], "core");
    storage.put_checkpoint("state", &value).unwrap();
    let conn = conn.lock().unwrap();
    assert_eq!(
        conn.query_row("PRAGMA recursive_triggers", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    for sql in [
        "DELETE FROM ndr_kv_checkpoint_chunk_shapes",
        "UPDATE ndr_kv_checkpoint_chunk_shapes SET byte_length=byte_length+1",
        "UPDATE ndr_kv_checkpoint_chunk_shapes SET data_type='text'",
        "UPDATE ndr_kv_checkpoint_chunk_shapes SET key='other'",
        "INSERT INTO ndr_kv_checkpoint_chunk_shapes VALUES('owner','device','state',x'00',0,1,'blob')",
    ] {
        assert!(conn.execute_batch(sql).is_err(), "unguarded edit: {sql}");
    }
    assert!(conn
        .blob_open(
            rusqlite::DatabaseName::Main,
            "ndr_kv_checkpoint_chunk_shapes",
            "data_type",
            1,
            false
        )
        .is_err());
}

#[test]
fn old_writer_updates_moves_and_incremental_blobs_remain_compatible() {
    let (dir, shared, storage) = database();
    let value = checkpoint(&[body(1)], &[1], "before");
    storage.put_checkpoint("state", &value).unwrap();
    let other = Connection::open(dir.path().join("checkpoint.sqlite3")).unwrap();
    let rowid: i64 = other
        .query_row(
            "SELECT rowid FROM ndr_kv_checkpoint_chunks WHERE payload_key=x'00636f7265'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    other.execute_batch("BEGIN").unwrap();
    {
        let mut blob = other
            .blob_open(
                rusqlite::DatabaseName::Main,
                "ndr_kv_checkpoint_chunks",
                "data",
                rowid,
                false,
            )
            .unwrap();
        blob.write_at(b"X", 0).unwrap();
        blob.close().unwrap();
    }
    other.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        storage.get("state").unwrap(),
        Some(value.materialize().unwrap())
    );
    let bytes: Vec<u8> = other
        .query_row(
            "SELECT data FROM ndr_kv_checkpoint_chunks WHERE rowid=?1",
            [rowid],
            |r| r.get(0),
        )
        .unwrap();
    let offset = bytes.windows(6).position(|s| s == b"before").unwrap();
    {
        let mut blob = other
            .blob_open(
                rusqlite::DatabaseName::Main,
                "ndr_kv_checkpoint_chunks",
                "data",
                rowid,
                false,
            )
            .unwrap();
        blob.write_at(b"after!", offset).unwrap();
        blob.close().unwrap();
    }
    assert_eq!(
        storage.get("state").unwrap(),
        Some(value.materialize().unwrap().replace("before", "after!"))
    );
    storage.put_checkpoint("state", &value).unwrap();
    assert_eq!(
        storage.get("state").unwrap(),
        Some(value.materialize().unwrap())
    );
    // The older client's exact UPSERT, plus composite-key moves in all fields.
    other.execute_batch("INSERT INTO ndr_kv_checkpoint_chunks SELECT * FROM ndr_kv_checkpoint_chunks WHERE rowid IS NOT NULL ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index) DO UPDATE SET data=excluded.data;
        UPDATE ndr_kv_checkpoint_chunks SET owner_pubkey_hex='moved',device_pubkey_hex='moved',key='moved',payload_key=x'006d6f766564',chunk_index=7 WHERE rowid=(SELECT min(rowid) FROM ndr_kv_checkpoint_chunks);").unwrap();
    let mismatches: i64 = other.query_row("SELECT count(*) FROM (SELECT owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index,length(data),typeof(data) FROM ndr_kv_checkpoint_chunks EXCEPT SELECT * FROM ndr_kv_checkpoint_chunk_shapes)", [], |r| r.get(0)).unwrap();
    assert_eq!(mismatches, 0);
    let extras: i64 = other.query_row("SELECT count(*) FROM (SELECT * FROM ndr_kv_checkpoint_chunk_shapes EXCEPT SELECT owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index,length(data),typeof(data) FROM ndr_kv_checkpoint_chunks)", [], |r| r.get(0)).unwrap();
    assert_eq!(extras, 0);
    assert!(
        storage.put_checkpoint("state", &value).is_err(),
        "moved-away chunks must still be detected"
    );
    drop(other);
    drop(shared);
}

#[test]
fn immutable_blob_corruption_is_still_detected_by_full_read() {
    let (dir, conn, storage) = database();
    let value = checkpoint(&[body(1)], &[1], "core");
    storage.put_checkpoint("state", &value).unwrap();
    let conn = conn.lock().unwrap();
    let rowid: i64 = conn.query_row("SELECT rowid FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01' LIMIT 1", [], |r| r.get(0)).unwrap();
    let mut blob = conn
        .blob_open(
            rusqlite::DatabaseName::Main,
            "ndr_kv_checkpoint_chunks",
            "data",
            rowid,
            false,
        )
        .unwrap();
    blob.write_at(b"X", 0).unwrap();
    blob.close().unwrap();
    drop(conn);
    assert!(reopen(&dir).get("state").is_err());
}
