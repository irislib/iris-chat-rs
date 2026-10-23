use super::{
    SharedConnection, SqliteStorageAdapter, StorageAdapter, StorageCheckpoint,
    StorageCheckpointBody,
};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

fn database() -> (tempfile::TempDir, SharedConnection, SqliteStorageAdapter) {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("checkpoint.sqlite3")).unwrap();
    conn.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE ndr_kv(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key));").unwrap();
    let shared = Arc::new(Mutex::new(conn));
    let storage = SqliteStorageAdapter::new(shared.clone(), "owner".into(), "device".into());
    (dir, shared, storage)
}

fn pages(conn: &SharedConnection) -> u64 {
    let conn = conn.lock().unwrap();
    let (mut current, mut high) = (0, 0);
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            conn.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,
            &mut current,
            &mut high,
            0,
        )
    };
    assert_eq!(result, rusqlite::ffi::SQLITE_OK);
    current as u64 * 4096
}

fn body(index: usize) -> Arc<StorageCheckpointBody> {
    let raw = serde_json::value::to_raw_value(
        &serde_json::json!({"index":index,"payload":"é🦀\\\u{0}".repeat(900)}),
    )
    .unwrap();
    Arc::new(StorageCheckpointBody::new(raw.into()))
}

fn checkpoint(
    records: &[Arc<StorageCheckpointBody>],
    retries: &[u64],
    core: &str,
) -> StorageCheckpoint {
    let mut value = StorageCheckpoint::new();
    value.push_inline("prefix", "{\"records\":[".into(), 0);
    for (index, record) in records.iter().enumerate() {
        value.push_record(
            0,
            record.clone(),
            format!(
                "{}{{\"retry\":{},\"body\":",
                if index == 0 { "" } else { "," },
                retries[index]
            ),
            "}".into(),
            0,
        );
    }
    value.push_inline(
        "core",
        format!("],\"core\":{}}}", serde_json::to_string(core).unwrap()),
        0,
    );
    value.pad_records(4096, 2048);
    value.push_inline("reserve", String::new(), 65536);
    value
}

fn reopen(dir: &tempfile::TempDir) -> SqliteStorageAdapter {
    let conn = Connection::open_with_flags(
        dir.path().join("checkpoint.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    SqliteStorageAdapter::new(Arc::new(Mutex::new(conn)), "owner".into(), "device".into())
}

#[test]
fn checkpoint_records_append_retire_and_metadata_bound_physical_writes() {
    let (dir, conn, storage) = database();
    let mut bodies = (0..512).map(body).collect::<Vec<_>>();
    let mut retries = vec![1; bodies.len()];
    storage
        .put_checkpoint("state", &checkpoint(&bodies, &retries, "before"))
        .unwrap();
    for operation in 0..3 {
        match operation {
            0 => {
                bodies.push(body(512));
                retries.push(1);
            }
            1 => {
                bodies.remove(0);
                retries.remove(0);
            }
            _ => {
                retries[5] = 99;
            }
        }
        let value = checkpoint(&bodies, &retries, "after");
        let before = pages(&conn);
        let io = super::checkpoint_io_bytes();
        storage.put_checkpoint("state", &value).unwrap();
        let written = pages(&conn) - before;
        let read = super::checkpoint_io_bytes().0 - io.0;
        assert!(
            written > 0 && written <= 1024 * 1024 && written < value.len() as u64 / 2,
            "operation {operation}: {written} page bytes"
        );
        assert!(
            read < value.len() / 4,
            "immutable payloads must not be read: {read}"
        );
        let expected = value.materialize().unwrap();
        assert_eq!(
            storage.get("state").unwrap().as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            reopen(&dir).get("state").unwrap().as_deref(),
            Some(expected.as_str())
        );
        let before = pages(&conn);
        storage.put_checkpoint("state", &value).unwrap();
        assert_eq!(
            pages(&conn),
            before,
            "no-op checkpoint must not dirty pages"
        );
    }
}

#[test]
fn checkpoint_records_duplicate_bodies_keep_independent_metadata_and_order() {
    let (_, conn, storage) = database();
    let shared = body(1);
    let records = vec![shared.clone(), body(2), shared.clone()];
    let before = checkpoint(&records, &[11, 22, 33], "first");
    storage.put_checkpoint("state", &before).unwrap();
    let after = checkpoint(&[shared.clone(), shared], &[33, 11], "second");
    storage.put_checkpoint("state", &after).unwrap();
    assert_eq!(
        storage.get("state").unwrap(),
        Some(after.materialize().unwrap())
    );
    let payloads: i64 = conn.lock().unwrap().query_row("SELECT count(DISTINCT payload_key) FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01'",[],|r|r.get(0)).unwrap();
    assert_eq!(payloads, 1, "retirement removes only unreferenced bodies");
    let nodes: i64 = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM ndr_kv_checkpoint_nodes WHERE substr(node_key,1,1)=x'01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nodes, 2, "logical duplicates must remain distinct");
}

#[test]
fn checkpoint_records_migrate_generic_and_return_to_small_values_atomically() {
    let (dir, conn, storage) = database();
    let bodies = vec![body(1), body(2)];
    let value = checkpoint(&bodies, &[1, 2], "migration");
    let expected = value.materialize().unwrap();
    storage.put("state", expected.clone()).unwrap();
    storage.put_checkpoint("state", &value).unwrap();
    assert_eq!(reopen(&dir).get("state").unwrap(), Some(expected));
    let generic: i64 = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM ndr_kv_segmented WHERE key='state'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(generic, 0);
    let small = "arbitrary\0text 🦀".to_string();
    storage.put("state", small.clone()).unwrap();
    assert_eq!(reopen(&dir).get("state").unwrap(), Some(small));
    for table in [
        "ndr_kv_checkpoint",
        "ndr_kv_checkpoint_nodes",
        "ndr_kv_checkpoint_chunks",
    ] {
        let remaining: i64 = conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE key='state'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }
}

#[test]
fn checkpoint_records_default_adapter_and_sqlite_return_identical_bytes() {
    let (_, _, storage) = database();
    let memory = super::InMemoryStorage::new();
    let value = checkpoint(&[body(0), body(1)], &[0, 1], "Unicode 🦀 and NUL \0");
    storage.put_checkpoint("state", &value).unwrap();
    memory.put_checkpoint("state", &value).unwrap();
    assert_eq!(storage.get("state").unwrap(), memory.get("state").unwrap());
    assert_eq!(value.len(), value.materialize().unwrap().len());
}

#[test]
fn checkpoint_records_namespace_delete_and_outer_rollback_keep_ownership() {
    let (dir, conn, storage) = database();
    let other = SqliteStorageAdapter::new(conn.clone(), "other".into(), "device".into());
    let old = checkpoint(&[body(1)], &[1], "old");
    let new = checkpoint(&[body(2)], &[2], "new");
    storage.put_checkpoint("state", &old).unwrap();
    other.put_checkpoint("state", &new).unwrap();
    conn.lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    storage.put_checkpoint("state", &new).unwrap();
    assert_eq!(
        storage.get("state").unwrap(),
        Some(new.materialize().unwrap())
    );
    conn.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(old.materialize().unwrap())
    );
    conn.lock()
        .unwrap()
        .execute("DELETE FROM ndr_kv WHERE owner_pubkey_hex='owner'", [])
        .unwrap();
    assert!(storage.get("state").unwrap().is_none());
    assert_eq!(
        other.get("state").unwrap(),
        Some(new.materialize().unwrap())
    );
    for table in [
        "ndr_kv_checkpoint",
        "ndr_kv_checkpoint_nodes",
        "ndr_kv_checkpoint_chunks",
    ] {
        let remaining: i64 = conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE owner_pubkey_hex='owner'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }
}

unsafe extern "C" fn reject_commit(_: *mut std::ffi::c_void) -> std::ffi::c_int {
    1
}

#[test]
fn checkpoint_records_mid_write_and_commit_failures_preserve_whole_generation() {
    let (dir, conn, storage) = database();
    let old = checkpoint(&[body(1), body(2)], &[1, 2], "old");
    let new = checkpoint(&[body(3), body(4)], &[3, 4], "new");
    storage.put_checkpoint("state", &old).unwrap();
    conn.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_node BEFORE INSERT ON ndr_kv_checkpoint_nodes WHEN substr(NEW.node_key,1,1)=x'01' BEGIN SELECT RAISE(ABORT,'injected node failure'); END; BEGIN IMMEDIATE;").unwrap();
    storage.put("earlier", "preserved".into()).unwrap();
    assert!(storage.put_checkpoint("state", &new).is_err());
    assert_eq!(
        storage.get("state").unwrap(),
        Some(old.materialize().unwrap())
    );
    assert_eq!(
        storage.get("earlier").unwrap().as_deref(),
        Some("preserved")
    );
    conn.lock()
        .unwrap()
        .execute_batch("COMMIT; DROP TRIGGER fail_node")
        .unwrap();
    {
        let conn = conn.lock().unwrap();
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(
                conn.handle(),
                Some(reject_commit),
                std::ptr::null_mut(),
            );
        }
    }
    assert!(storage.put_checkpoint("state", &new).is_err());
    {
        let conn = conn.lock().unwrap();
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(conn.handle(), None, std::ptr::null_mut());
        }
    }
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(old.materialize().unwrap())
    );
    storage.put_checkpoint("state", &new).unwrap();
    assert_eq!(
        reopen(&dir).get("state").unwrap(),
        Some(new.materialize().unwrap())
    );
}

#[test]
fn checkpoint_records_read_snapshot_survives_another_writer_commit() {
    let (dir, conn, storage) = database();
    let old = checkpoint(&[body(1)], &[1], "old");
    let new = checkpoint(&[body(2)], &[2], "new");
    storage.put_checkpoint("state", &old).unwrap();
    let reader_conn = Connection::open_with_flags(
        dir.path().join("checkpoint.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    reader_conn
        .execute_batch("BEGIN; SELECT byte_length FROM ndr_kv_checkpoint;")
        .unwrap();
    let reader_conn = Arc::new(Mutex::new(reader_conn));
    let reader = SqliteStorageAdapter::new(reader_conn.clone(), "owner".into(), "device".into());
    storage.put_checkpoint("state", &new).unwrap();
    assert_eq!(
        reader.get("state").unwrap(),
        Some(old.materialize().unwrap())
    );
    reader_conn.lock().unwrap().execute_batch("COMMIT").unwrap();
    assert_eq!(
        reader.get("state").unwrap(),
        Some(new.materialize().unwrap())
    );
    assert!(conn.lock().unwrap().is_autocommit());
}

#[test]
fn checkpoint_records_corruption_and_conflicting_formats_are_rejected() {
    for corruption in [
        "DELETE FROM ndr_kv_checkpoint_chunks WHERE rowid=(SELECT min(rowid) FROM ndr_kv_checkpoint_chunks WHERE length(data)>1000)",
        "UPDATE ndr_kv_checkpoint SET format_version=99",
        "UPDATE ndr_kv_checkpoint_nodes SET next_key=node_key WHERE next_key IS NOT NULL",
        "INSERT INTO ndr_kv_segmented VALUES('owner','device','state',1,65536,65536)",
        "DROP TABLE ndr_kv_checkpoint_chunks",
    ] {
        let (_,conn,storage) = database();
        storage.put("schema","prepare generic schema".into()).unwrap();
        let value = checkpoint(&[body(1)],&[1],"core");
        storage.put_checkpoint("state",&value).unwrap();
        conn.lock().unwrap().execute_batch(corruption).unwrap();
        assert!(storage.get("state").is_err(),"corrupt read must fail: {corruption}");
        assert!(storage.put_checkpoint("state",&value).is_err(),"corrupt state must not be overwritten: {corruption}");
    }
}

fn shared_chunked_checkpoint() -> StorageCheckpoint {
    let raw = serde_json::value::to_raw_value(&"é🦀\0".repeat(12_000)).unwrap();
    let shared = Arc::new(StorageCheckpointBody::new(raw.into()));
    assert!(shared.json().len() > 2 * 65536 && shared.json().len() < 3 * 65536);
    checkpoint(&[shared.clone(), shared], &[11, 22], "shared")
}

#[derive(PartialEq)]
enum CheckpointCell {
    Null,
    Integer(i64),
    Real(u64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

fn checkpoint_rows(conn: &SharedConnection) -> Vec<Vec<Vec<CheckpointCell>>> {
    let conn = conn.lock().unwrap();
    [
        "ndr_kv",
        "ndr_kv_checkpoint",
        "ndr_kv_checkpoint_nodes",
        "ndr_kv_checkpoint_chunks",
    ]
    .into_iter()
    .map(|table| {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..columns)
                    .map(|column| {
                        use rusqlite::types::ValueRef;
                        Ok(match row.get_ref(column)? {
                            ValueRef::Null => CheckpointCell::Null,
                            ValueRef::Integer(n) => CheckpointCell::Integer(n),
                            ValueRef::Real(n) => CheckpointCell::Real(n.to_bits()),
                            ValueRef::Text(bytes) => CheckpointCell::Text(bytes.to_vec()),
                            ValueRef::Blob(bytes) => CheckpointCell::Blob(bytes.to_vec()),
                        })
                    })
                    .collect()
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    })
    .collect()
}

#[test]
fn checkpoint_records_shared_empty_payloads_and_chunk_order_are_valid() {
    let (dir, conn, storage) = database();
    let value = shared_chunked_checkpoint();
    storage.put_checkpoint("state", &value).unwrap();
    conn.lock().unwrap().execute_batch(
        "CREATE TEMP TABLE reordered AS SELECT * FROM ndr_kv_checkpoint_chunks;
         DELETE FROM ndr_kv_checkpoint_chunks;
         INSERT INTO ndr_kv_checkpoint_chunks SELECT * FROM reordered ORDER BY payload_key DESC,chunk_index DESC;
         DROP TABLE reordered;"
    ).unwrap();
    {
        let conn = conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT chunk_index FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01' ORDER BY rowid").unwrap();
        let indices = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            indices,
            [2, 1, 0],
            "shared body has one physically reversed chunk set"
        );
        let empty_chunks: i64 = conn.query_row("SELECT count(*) FROM ndr_kv_checkpoint_chunks WHERE payload_key IN (SELECT payload_key FROM ndr_kv_checkpoint_nodes WHERE byte_length=0)", [], |r| r.get(0)).unwrap();
        assert_eq!(empty_chunks, 0);
    }
    let expected = value.materialize().unwrap();
    assert_eq!(storage.get("state").unwrap(), Some(expected.clone()));
    assert_eq!(reopen(&dir).get("state").unwrap(), Some(expected));
    let before = pages(&conn);
    storage.put_checkpoint("state", &value).unwrap();
    assert_eq!(
        pages(&conn),
        before,
        "valid shared/empty bodies remain a no-op"
    );
}

#[test]
fn checkpoint_records_payload_shape_corruption_preserves_existing_rows() {
    for (case, corruption) in [
        // Preserve each node's logical size so the shared-length conflict reaches payload validation.
        ("inconsistent shared lengths", "UPDATE ndr_kv_checkpoint_nodes SET byte_length=byte_length+1,padding=padding-1 WHERE rowid=(SELECT min(rowid) FROM ndr_kv_checkpoint_nodes WHERE substr(payload_key,1,1)=x'01')"),
        ("empty payload with a chunk", "INSERT INTO ndr_kv_checkpoint_chunks SELECT owner_pubkey_hex,device_pubkey_hex,key,payload_key,0,x'' FROM ndr_kv_checkpoint_nodes WHERE byte_length=0"),
        ("missing middle chunk", "DELETE FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01' AND chunk_index=1"),
        ("missing final chunk", "DELETE FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01' AND chunk_index=2"),
        ("negative index", "UPDATE ndr_kv_checkpoint_chunks SET chunk_index=-1 WHERE substr(payload_key,1,1)=x'01' AND chunk_index=0"),
        ("noninteger index", "UPDATE ndr_kv_checkpoint_chunks SET chunk_index=0.5 WHERE substr(payload_key,1,1)=x'01' AND chunk_index=0"),
        ("extra final chunk", "INSERT INTO ndr_kv_checkpoint_chunks SELECT owner_pubkey_hex,device_pubkey_hex,key,payload_key,3,x'' FROM ndr_kv_checkpoint_chunks WHERE substr(payload_key,1,1)=x'01' AND chunk_index=2"),
        ("short full chunk", "UPDATE ndr_kv_checkpoint_chunks SET data=substr(data,1,length(data)-1) WHERE substr(payload_key,1,1)=x'01' AND chunk_index=0"),
        ("long full chunk", "UPDATE ndr_kv_checkpoint_chunks SET data=CAST(data||x'00' AS BLOB) WHERE substr(payload_key,1,1)=x'01' AND chunk_index=0"),
        ("short final chunk", "UPDATE ndr_kv_checkpoint_chunks SET data=substr(data,1,length(data)-1) WHERE substr(payload_key,1,1)=x'01' AND chunk_index=2"),
        ("text data", "UPDATE ndr_kv_checkpoint_chunks SET data=hex(zeroblob(length(data)/2)) WHERE substr(payload_key,1,1)=x'01' AND chunk_index=0"),
        ("text payload key", "UPDATE ndr_kv_checkpoint_chunks SET payload_key='text-key' WHERE substr(payload_key,1,1)=x'01'"),
        ("orphan payload", "INSERT INTO ndr_kv_checkpoint_chunks VALUES('owner','device','state',x'006f727068616e',0,x'00')"),
    ] {
        let (dir, conn, storage) = database();
        let value = shared_chunked_checkpoint();
        storage.put_checkpoint("state", &value).unwrap();
        conn.lock().unwrap().execute_batch(corruption).unwrap();
        let before = checkpoint_rows(&conn);
        let written = pages(&conn);
        assert!(storage.get("state").is_err(), "corrupt read: {case}");
        assert!(storage.put_checkpoint("state", &value).is_err(), "corrupt replacement: {case}");
        assert!(checkpoint_rows(&conn) == before, "failed replacement changed rows: {case}");
        assert_eq!(pages(&conn), written, "failed replacement dirtied pages: {case}");
        assert!(reopen(&dir).get("state").is_err(), "corruption must survive rejection: {case}");
    }
}

fn descriptor_checkpoint(
    records: &[Arc<StorageCheckpointBody>],
    labels: &[(&str, &str)],
) -> StorageCheckpoint {
    assert_eq!(records.len(), labels.len());
    let mut value = StorageCheckpoint::new();
    value.push_inline("open", "[".into(), 0);
    for (index, (record, (before, after))) in records.iter().zip(labels).enumerate() {
        value.push_record(
            0,
            record.clone(),
            format!(
                "{}{{\"before\":{},\"body\":",
                if index == 0 { "" } else { "," },
                serde_json::to_string(before).unwrap()
            ),
            format!(",\"after\":{}}}", serde_json::to_string(after).unwrap()),
            0,
        );
    }
    value.push_inline("close", "]".into(), 65536);
    value
}

#[test]
fn checkpoint_records_warmed_put_allocation_does_not_scale_with_descriptors() {
    use crate::protocol_engine::tests::remote_roster_tests::allocated_bytes;

    fn measure(label_bytes: usize) -> usize {
        let (dir, conn, storage) = database();
        let shared = body(1);
        let records = vec![shared; 64];
        let label = "a".repeat(label_bytes);
        let labels = vec![(label.as_str(), label.as_str()); records.len()];
        let value = descriptor_checkpoint(&records, &labels);
        storage.put_checkpoint("state", &value).unwrap();
        storage.put_checkpoint("state", &value).unwrap();
        let before = pages(&conn);
        let (result, allocated) = allocated_bytes(|| storage.put_checkpoint("state", &value));
        result.unwrap();
        assert_eq!(pages(&conn), before, "warmed no-op must not dirty pages");
        let expected = value.materialize().unwrap();
        assert_eq!(storage.get("state").unwrap(), Some(expected.clone()));
        assert_eq!(reopen(&dir).get("state").unwrap(), Some(expected));
        allocated
    }

    let small = measure(16);
    let large = measure(4096);
    eprintln!("checkpoint_descriptor_allocations nodes=64 small={small} large={large} maximum_growth=65536");
    // The same keys, bodies and links have 522,240 additional descriptor bytes.
    // Reading those bytes must not allocate an owned copy on every save.
    assert!(
        large <= small + 65536,
        "warmed put allocation scales with stored descriptors: small={small}, large={large}"
    );
}

#[test]
fn checkpoint_records_descriptor_updates_order_and_external_writes_stay_fresh() {
    let (dir, conn, storage) = database();
    let a = body(1);
    let b = body(2);
    let values = [
        descriptor_checkpoint(
            &[a.clone(), b.clone(), a.clone()],
            &[("a0a0", "0000"), ("b0b0", "1111"), ("a1a1", "2222")],
        ),
        // Independently change prefix and suffix without changing their byte lengths.
        descriptor_checkpoint(
            &[a.clone(), b.clone(), a.clone()],
            &[("x0x0", "0000"), ("b0b0", "9999"), ("a1a1", "2222")],
        ),
        descriptor_checkpoint(
            &[b, a.clone(), a.clone()],
            &[("b0b0", "9999"), ("x0x0", "0000"), ("a1a1", "2222")],
        ),
        descriptor_checkpoint(&[a.clone(), a], &[("a0a0", "0000"), ("a1a1", "2222")]),
    ];
    for value in &values {
        storage.put_checkpoint("state", value).unwrap();
        let expected = value.materialize().unwrap();
        assert_eq!(storage.get("state").unwrap(), Some(expected.clone()));
        assert_eq!(reopen(&dir).get("state").unwrap(), Some(expected));
        let before = pages(&conn);
        storage.put_checkpoint("state", value).unwrap();
        assert_eq!(
            pages(&conn),
            before,
            "unchanged descriptors must remain a no-op"
        );
    }
    let value = values.last().unwrap();
    let expected = value.materialize().unwrap();
    let writer = Connection::open(dir.path().join("checkpoint.sqlite3")).unwrap();
    assert_eq!(writer.execute("UPDATE ndr_kv_checkpoint_nodes SET prefix=replace(prefix,'a0a0','z9z9') WHERE owner_pubkey_hex='owner' AND device_pubkey_hex='device' AND key='state' AND instr(prefix,'a0a0')>0", []).unwrap(), 1);
    assert_eq!(
        storage.get("state").unwrap(),
        Some(expected.replace("a0a0", "z9z9"))
    );
    let before = pages(&conn);
    storage.put_checkpoint("state", value).unwrap();
    assert!(
        pages(&conn) > before,
        "fresh external descriptor changes must be restored"
    );
    assert_eq!(reopen(&dir).get("state").unwrap(), Some(expected));
    let before = pages(&conn);
    storage.put_checkpoint("state", value).unwrap();
    assert_eq!(pages(&conn), before);
}

#[test]
fn checkpoint_records_invalid_descriptors_in_retained_or_retired_rows_are_rejected() {
    for column in ["prefix", "suffix"] {
        for invalid_utf8 in [false, true] {
            for retire in [false, true] {
                let (dir, conn, storage) = database();
                let records = [body(1), body(2)];
                let value =
                    descriptor_checkpoint(&records, &[("first", "tail1"), ("other", "tail2")]);
                let replacement = descriptor_checkpoint(&records[..1], &[("first", "tail1")]);
                storage.put_checkpoint("state", &value).unwrap();
                let expression = if invalid_utf8 {
                    format!("CAST(x'ff'||substr(CAST({column} AS BLOB),2) AS TEXT)")
                } else {
                    format!("CAST({column} AS BLOB)")
                };
                // The second record is absent from replacement, but its old descriptor
                // still belongs to the stored generation and must be validated.
                conn.lock().unwrap().execute_batch(&format!("UPDATE ndr_kv_checkpoint_nodes SET {column}={expression} WHERE rowid=(SELECT max(rowid) FROM ndr_kv_checkpoint_nodes WHERE substr(node_key,1,1)=x'01')")).unwrap();
                let before = checkpoint_rows(&conn);
                let written = pages(&conn);
                assert!(
                    storage.get("state").is_err(),
                    "{column}, invalid_utf8={invalid_utf8}"
                );
                let proposed = if retire { &replacement } else { &value };
                assert!(
                    storage.put_checkpoint("state", proposed).is_err(),
                    "{column}, invalid_utf8={invalid_utf8}, retire={retire}"
                );
                assert!(
                    checkpoint_rows(&conn) == before,
                    "rejected descriptor changed stored rows"
                );
                assert_eq!(pages(&conn), written, "rejected descriptor dirtied pages");
                assert!(reopen(&dir).get("state").is_err());
            }
        }
    }
}
