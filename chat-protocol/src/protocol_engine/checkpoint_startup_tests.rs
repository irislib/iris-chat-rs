#[test]
fn checkpoint_cold_layout_recovers_core_without_matching_quoted_or_reordered_fields() {
    let prefix = format!("{{\"version\":{},\"pending_group_sender_key_messages\":[],\"pending_group_fanouts\":[],\"pending_inbound\":[]", PROTOCOL_ENGINE_STATE_VERSION);
    let sessions = serde_json::json!({"users": (0..40).map(|i|
        serde_json::json!({"i":i,"state":"x".repeat(2300)})).collect::<Vec<_>>()});
    let core = format!("{prefix},\"session_manager\":{sessions}}}");
    let (core, layout) = layout_protocol_checkpoint(core, &ProtocolCheckpointLayout::default());
    let quoted = serde_json::to_string("\"pending_inbound\":[],\"session_manager\":{} 🦊").unwrap();
    let json = format!("{{\"version\":{},\"pending_group_sender_key_messages\":[{{\"text\":{quoted}}}],\"pending_group_fanouts\":[],\"pending_inbound\":[]{}   ", PROTOCOL_ENGINE_STATE_VERSION, &core[prefix.len()..]);
    let restored = ProtocolCheckpointLayout::from_json(&json);
    let restored_core = restored.core.unwrap();
    assert_eq!(restored_core.record_ends, layout.record_ends);
    assert_eq!(restored_core.bytes, core.trim_end().len());
    // Old/reordered JSON is still readable, but its unrelated whitespace must
    // not be mistaken for this serializer's separately laid-out core.
    let reordered = format!("{{\"session_manager\":{sessions},\"version\":1,\"pending_group_sender_key_messages\":[],\"pending_group_fanouts\":[],\"pending_inbound\":[]}}");
    assert!(ProtocolCheckpointLayout::from_json(&reordered)
        .core
        .is_none());
    let nested =
        serde_json::json!({"nested": serde_json::from_str::<serde_json::Value>(&json).unwrap()})
            .to_string();
    assert!(ProtocolCheckpointLayout::from_json(&nested).core.is_none());
}

#[test]
fn checkpoint_cold_reopen_retains_core_boundaries_after_real_expiry() {
    let (mut fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let peer = fixture.peer_owner.public_key();
    fixture.sender.enter_batch();
    for index in 0..33 {
        let text = format!(
            "cold-start-{index}:{}",
            "x".repeat(if index == 0 { 8_000 } else { 1_200 })
        );
        let sent = fixture
            .sender
            .send_direct_text(
                peer,
                &peer.to_hex(),
                &text,
                (index == 0).then_some(200),
                UnixSeconds(100),
            )
            .unwrap();
        assert_eq!(
            decrypt_own_sync_effects(&mut fixture.ready, sent.effects).len(),
            1
        );
    }
    fixture.sender.exit_batch().unwrap();
    let before = fixture.sender.pending_remote_sends.len();
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(200))
        .unwrap()
        .is_empty());
    assert_eq!(fixture.sender.pending_remote_sends.len(), before - 1);
    // Real expiry leaves the following smaller records in the saved larger
    // slots. A cold load must retain those boundaries instead of repacking.
    assert!(
        fixture
            .sender
            .checkpoint_layout
            .borrow()
            .core
            .as_ref()
            .unwrap()
            .bytes
            > 128 * 1024
    );
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let expected: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let owner = fixture.owner.public_key();
    let device = fixture.device.clone();
    drop(saved);
    drop(fixture);
    drop(storage);
    drop(connection);

    let mut writes = Vec::new();
    for _ in 0..2 {
        let conn = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
        conn.execute_batch("PRAGMA wal_autocheckpoint=0;
            CREATE TEMP TABLE startup_payload_writes(immutable INTEGER NOT NULL, bytes INTEGER NOT NULL);
            CREATE TEMP TRIGGER startup_payload_insert AFTER INSERT ON main.ndr_kv_checkpoint_chunks BEGIN
                INSERT INTO startup_payload_writes VALUES(substr(NEW.payload_key,1,1)=x'01',length(NEW.data));
            END;
            CREATE TEMP TRIGGER startup_payload_update AFTER UPDATE ON main.ndr_kv_checkpoint_chunks BEGIN
                INSERT INTO startup_payload_writes VALUES(substr(NEW.payload_key,1,1)=x'01',length(NEW.data));
            END;").unwrap();
        let connection = Arc::new(Mutex::new(conn));
        let storage = Arc::new(SqliteStorageAdapter::new(
            connection.clone(),
            owner.to_hex(),
            device.public_key().to_hex(),
        ));
        let before = checkpoint_growth_written_bytes(&connection);
        let restored =
            ProtocolEngine::load_or_create_for_local_device(storage.clone(), owner, &device)
                .unwrap();
        let written = checkpoint_growth_written_bytes(&connection) - before;
        let (immutable, inline): (u64,u64) = connection.lock().unwrap().query_row(
            "SELECT coalesce(sum(CASE WHEN immutable THEN bytes ELSE 0 END),0),coalesce(sum(CASE WHEN NOT immutable THEN bytes ELSE 0 END),0) FROM startup_payload_writes",[],|row|Ok((row.get(0)?,row.get(1)?))
        ).unwrap();
        assert_eq!(
            immutable, 0,
            "cold caches must not reinsert immutable bodies"
        );
        let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
        assert_eq!(saved.len(), restored.checkpoint_layout.borrow().bytes);
        assert!(serde_json::from_str::<serde_json::Value>(&saved).unwrap() == expected,
            "startup must preserve the complete queued state, ratchets, journal and authorization evidence");
        eprintln!("checkpoint_cold_reopen physical_write_bytes={written} immutable_insert_update_bytes={immutable} inline_insert_update_bytes={inline}");
        writes.push(written);
        drop(restored);
        drop(storage);
        drop(connection);
    }
    // The materialized JSON does not distinguish core trailing whitespace
    // from the final reserve. A first restore may normalize its last chunk.
    assert!(
        writes[0] <= 128 * 1024,
        "cold restore repacked the mutable core: {}",
        writes[0]
    );
    assert_eq!(
        writes[1], 0,
        "a second unchanged cold restore must not write data"
    );
}

#[test]
fn checkpoint_cold_reopen_migrates_legacy_once_without_losing_state() {
    let (fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let expected: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let owner = fixture.owner.public_key();
    let device = fixture.device.clone();
    // The ordinary adapter path represents an older whole-value checkpoint;
    // no logical-record metadata survives this replacement.
    storage.put(PROTOCOL_ENGINE_STATE_KEY, saved).unwrap();
    let count: usize = connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM ndr_kv_checkpoint", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    drop(fixture);
    drop(storage);
    drop(connection);
    let mut writes = Vec::new();
    for _ in 0..2 {
        let connection = Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        ));
        let storage = Arc::new(SqliteStorageAdapter::new(
            connection.clone(),
            owner.to_hex(),
            device.public_key().to_hex(),
        ));
        let before = checkpoint_growth_written_bytes(&connection);
        let restored =
            ProtocolEngine::load_or_create_for_local_device(storage.clone(), owner, &device)
                .unwrap();
        writes.push(checkpoint_growth_written_bytes(&connection) - before);
        let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&saved).unwrap() == expected);
        let count: usize = connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM ndr_kv_checkpoint", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        drop(restored);
        drop(storage);
        drop(connection);
    }
    assert!(
        writes[0] > 0,
        "first migration must actually store the payloads"
    );
    assert_eq!(
        writes[1], 0,
        "completed migration must not recur on cold reopen"
    );
}
