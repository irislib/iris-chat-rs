#[test]
fn checkpoint_fanout_cache_preserves_legacy_json_and_retry_metadata() {
    let mut pending = pending_local_group_fanout("escaped 🦊", Some("event"), b"[{}] \" fanout", 5);
    let payload = pending.fanout.serialized().unwrap();
    assert_eq!(
        payload.get(),
        serde_json::to_string(pending.fanout.as_ref()).unwrap()
    );
    let original = pending.clone();
    pending.next_retry_at_secs = 100;
    pending.remaining_devices = Some(BTreeSet::from([ndr_device(Keys::generate().public_key())]));
    assert!(Arc::ptr_eq(&payload, &pending.fanout.serialized().unwrap()));
    assert!(Arc::ptr_eq(&original.fanout.value, &pending.fanout.value));
    let restored: ProtocolPendingGroupFanout =
        serde_json::from_str(&serde_json::to_string(&pending).unwrap()).unwrap();
    assert_eq!(restored, pending);
    assert_ne!(restored, original);
    let weak_payload = Arc::downgrade(&payload);
    drop(payload);
    drop(pending);
    drop(original);
    assert!(
        weak_payload.upgrade().is_none(),
        "retired payload JSON must not outlive its queue/rollback generation"
    );
}

#[test]
fn checkpoint_cached_fanout_layout_matches_full_scan_and_mismatch_fallback() {
    let pending = (0..32)
        .map(|index| {
            pending_local_group_fanout(
                "g",
                Some("event"),
                format!("{index}🦊\\\"}}[{}", "x".repeat(2048)).as_bytes(),
                5,
            )
        })
        .collect::<Vec<_>>();
    let cached = pending
        .iter()
        .map(|p| p.fanout.serialized().unwrap())
        .collect::<Vec<_>>();
    let json = serde_json::json!({"fake": {"fanout": "ignored"}, "pending_group_fanouts":pending, "tail":[{"id":1}]}).to_string();
    let baseline = layout_protocol_checkpoint(json.clone(), &ProtocolCheckpointLayout::default()).0;
    for values in [
        cached.clone(),
        cached.into_iter().rev().collect(),
        Vec::new(),
    ] {
        assert_eq!(
            checkpoint_records_with_cached_fields(&json, 0, None, &values),
            checkpoint_records(&json)
        );
        assert_eq!(
            layout_protocol_checkpoint_with_cached_fields(
                json.clone(),
                &ProtocolCheckpointLayout::default(),
                None,
                &values
            )
            .0,
            baseline
        );
        let previous = ProtocolCheckpointLayout::from_json(&baseline);
        assert_eq!(
            layout_protocol_checkpoint_with_cached_fields(json.clone(), &previous, None, &values).0,
            baseline
        );
    }
}

fn checkpoint_backlog_fixture(fanout_count: usize, ciphertext_count: usize) -> RemoteSendFixture {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let hello = f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "establish sender",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, hello.effects).len(),
        1
    );
    let owner = f.owner.public_key();
    let reply = f
        .ready
        .send_direct_text(
            owner,
            &owner.to_hex(),
            "establish receiver",
            None,
            UnixSeconds(11),
        )
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.sender, reply.effects).len(),
        1
    );
    let context = nostr_double_ratchet::GroupPayloadEncodeContext {
        local_device_pubkey: ndr_device(f.device.public_key()),
        created_at: NdrUnixSeconds(20),
    };
    let stream = ndr_device(Keys::generate().public_key());
    f.sender.pending_group_fanouts = (0..fanout_count)
        .map(|index| {
            let request = SenderKeyRepairRequest {
                group_id: "backlog-group".into(),
                sender_event_pubkey: stream,
                key_id: None,
                message_number: None,
                required_revision: Some(1),
                created_at: NdrUnixSeconds(index as u64 + 20),
            };
            let payload = JsonGroupPayloadCodecV1
                .encode_pairwise_command(
                    context,
                    &GroupPairwiseCommand::SenderKeyRepairRequest { request },
                )
                .unwrap();
            ProtocolPendingGroupFanout {
                group_id: "backlog-group".into(),
                fanout: GroupPendingFanout::Remote {
                    recipient_owner: ndr_owner(peer),
                    payload,
                }
                .into(),
                remaining_devices: Some(BTreeSet::from([ndr_device(f.late_device.public_key())])),
                inner_event_id: None,
                created_at_secs: 20,
                next_retry_at_secs: u64::MAX,
            }
        })
        .collect();
    let mut encryptor = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
    let prototype = queued_group_history(1).0.pending_group_sender_key_messages[0].clone();
    let mut ciphertexts = Vec::with_capacity(ciphertext_count);
    for index in 0..ciphertext_count {
        let mut message = prototype.clone();
        let body = format!("history-{index}:{}", "x".repeat(768));
        let (number, ciphertext) = encryptor.encrypt_to_bytes(body.as_bytes()).unwrap();
        message.message_number = number;
        message.ciphertext = ciphertext;
        ciphertexts.push(message);
    }
    f.sender.pending_group_sender_key_messages = ciphertexts.into();
    f
}

fn checkpoint_pager_bytes(connection: &Arc<Mutex<rusqlite::Connection>>) -> u64 {
    let connection = connection.lock().unwrap();
    let (mut current, mut high) = (0, 0);
    let status = unsafe { rusqlite::ffi::sqlite3_db_status(connection.handle(), rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS, &mut current, &mut high, 0) };
    assert_eq!(status, rusqlite::ffi::SQLITE_OK);
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
    current as u64 * page_size
}

fn checkpoint_backlog_send_roundtrip(fanouts: usize, ciphertexts: usize) {
    use std::sync::atomic::Ordering::Relaxed;
    let fixture_started = std::time::Instant::now();
    let mut f = checkpoint_backlog_fixture(fanouts, ciphertexts);
    let directory = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE ndr_kv (
        owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
        key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));").unwrap();
    let connection = Arc::new(Mutex::new(connection));
    let sqlite = Arc::new(SqliteStorageAdapter::new(
        connection.clone(),
        f.owner.public_key().to_hex(),
        f.device.public_key().to_hex(),
    ));
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: sqlite,
        puts: 0.into(),
        fail_next: false.into(),
    });
    f.sender.storage = storage.clone();
    let started = std::time::Instant::now();
    f.sender.persist().unwrap();
    if fanouts == 22_702 {
        // A checkpoint retains space after records shrink. Reproduce the
        // observed aggregate allocation without using any real account data.
        f.sender.checkpoint_layout.borrow_mut().bytes = 254_672_896;
        f.sender.persist().unwrap();
    }
    let cold_ms = started.elapsed().as_millis();
    let bytes = f.sender.checkpoint_layout.borrow().bytes;
    let encoded_fanouts = CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(std::cell::Cell::get);
    let encoded_ciphertexts = CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get);
    let before_fanouts = f.sender.pending_group_fanouts.clone();
    let before_ciphertexts = f.sender.pending_group_sender_key_messages.clone();
    let peer = f.peer_owner.public_key();
    let owner = f.owner.public_key();
    let mut effects = Vec::new();
    for index in 0..3 {
        effects.extend(
            f.ready
                .send_direct_text(
                    owner,
                    &owner.to_hex(),
                    &format!("durable incoming {index}"),
                    None,
                    UnixSeconds(100 + index),
                )
                .unwrap()
                .effects,
        );
    }
    let before_puts = storage.puts.load(Relaxed);
    // Real AppCore writes app state before protocol checkpointing. A cold pager
    // makes this catch visits to immutable chunk pages even when SQL returns
    // only their small length/type metadata and logical I/O counters look low.
    connection.lock().unwrap().execute_batch("CREATE TABLE app_state(value INTEGER); INSERT INTO app_state VALUES(0); PRAGMA cache_size=-2048; PRAGMA shrink_memory;").unwrap();
    let before_pager = checkpoint_pager_bytes(&connection);
    let before_io = crate::storage::checkpoint_io_bytes();
    let before_scan = CHECKPOINT_LAYOUT_SCANNED_BYTES.with(std::cell::Cell::get);
    let started = std::time::Instant::now();
    f.sender.enter_batch();
    assert_eq!(decrypt_own_sync_effects(&mut f.sender, effects).len(), 3);
    f.sender.exit_batch().unwrap();
    let warm_ms = started.elapsed().as_millis();
    let pager_bytes = checkpoint_pager_bytes(&connection) - before_pager;
    let pager_limit = ((fanouts + ciphertexts) * 1024 + 2 * 1024 * 1024) as u64;
    assert!(pager_bytes <= pager_limit, "checkpoint traversed payload pages: {pager_bytes} > {pager_limit}");
    let after_io = crate::storage::checkpoint_io_bytes();
    let read_bytes = after_io.0 - before_io.0;
    let written_bytes = after_io.1 - before_io.1;
    let scanned_bytes = CHECKPOINT_LAYOUT_SCANNED_BYTES.with(std::cell::Cell::get) - before_scan;
    assert_eq!(f.sender.checkpoint_layout.borrow().bytes, bytes);
    if fanouts == 22_702 {
        assert_eq!(
            bytes, 254_672_896,
            "exercise the retained production-sized allocation"
        );
    }
    assert!(
        read_bytes <= (fanouts + ciphertexts) * 512 + 2 * 1024 * 1024 && read_bytes < bytes / 2,
        "checkpoint reads must remain limited to metadata and changed core records: {read_bytes}"
    );
    assert!(
        written_bytes < (4 * 1024 * 1024).min(bytes / 2),
        "a small dirty batch must not rewrite its backlog: {written_bytes}"
    );
    assert!(
        scanned_bytes <= fanouts * 512 + 2 * 1024 * 1024,
        "layout must scan metadata, not the cached payloads: {scanned_bytes}"
    );
    assert_eq!(
        storage.puts.load(Relaxed),
        before_puts + 1,
        "one atomic checkpoint per dirty message batch"
    );
    assert_eq!(
        CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(std::cell::Cell::get),
        encoded_fanouts,
        "ordinary messages must not re-encode the queued fanout payloads"
    );
    assert_eq!(
        CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get),
        encoded_ciphertexts
    );
    connection.lock().unwrap().execute_batch("UPDATE app_state SET value=value+1; PRAGMA shrink_memory;").unwrap();
    let outgoing_pager = checkpoint_pager_bytes(&connection);
    let outgoing_io = crate::storage::checkpoint_io_bytes();
    let outgoing_scan = CHECKPOINT_LAYOUT_SCANNED_BYTES.with(std::cell::Cell::get);
    let outgoing_started = std::time::Instant::now();
    f.sender.enter_batch();
    let mut outgoing = Vec::new();
    for index in 0..3 {
        outgoing.extend(
            f.sender
                .send_direct_text(
                    peer,
                    &peer.to_hex(),
                    &format!("durable outgoing {index}"),
                    None,
                    UnixSeconds(150 + index),
                )
                .unwrap()
                .effects,
        );
    }
    f.sender.exit_batch().unwrap();
    let outgoing_ms = outgoing_started.elapsed().as_millis();
    let outgoing_pager_bytes = checkpoint_pager_bytes(&connection) - outgoing_pager;
    assert!(outgoing_pager_bytes <= pager_limit, "direct-send checkpoint traversed payload pages: {outgoing_pager_bytes} > {pager_limit}");
    let outgoing_after_io = crate::storage::checkpoint_io_bytes();
    let outgoing_read_bytes = outgoing_after_io.0 - outgoing_io.0;
    let outgoing_written_bytes = outgoing_after_io.1 - outgoing_io.1;
    let outgoing_scanned_bytes =
        CHECKPOINT_LAYOUT_SCANNED_BYTES.with(std::cell::Cell::get) - outgoing_scan;
    assert_eq!(storage.puts.load(Relaxed), before_puts + 2);
    assert_eq!(f.sender.checkpoint_layout.borrow().bytes, bytes);
    assert!(
        outgoing_read_bytes <= (fanouts + ciphertexts) * 512 + 2 * 1024 * 1024
            && outgoing_read_bytes < bytes / 2,
        "direct-send checkpoint reads must not scan immutable payloads: {outgoing_read_bytes}"
    );
    assert!(
        outgoing_written_bytes < (4 * 1024 * 1024).min(bytes / 2),
        "new pending direct sends must not shift the large backlogs: {outgoing_written_bytes}"
    );
    assert!(outgoing_scanned_bytes <= fanouts * 512 + 2 * 1024 * 1024);
    assert_eq!(
        CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(std::cell::Cell::get),
        encoded_fanouts
    );
    assert_eq!(
        CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get),
        encoded_ciphertexts
    );
    assert_eq!(decrypt_own_sync_effects(&mut f.ready, outgoing).len(), 3);
    let saved_sessions = serde_json::to_value(f.sender.session_manager_snapshot()).unwrap();
    for _ in 0..3 {
        f.sender.enter_batch();
        f.sender.exit_batch().unwrap();
    }
    assert_eq!(
        storage.puts.load(Relaxed),
        before_puts + 2,
        "unchanged batches must not checkpoint"
    );
    storage.fail_next.store(true, Relaxed);
    assert!(f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "failed checkpoint",
            None,
            UnixSeconds(199)
        )
        .is_err());
    assert_eq!(
        serde_json::to_value(f.sender.session_manager_snapshot()).unwrap(),
        saved_sessions,
        "failed persistence must restore the prior ratchet alongside its pending intents"
    );
    assert_eq!(f.sender.pending_group_fanouts, before_fanouts);
    drop(f.sender);
    drop(storage);
    let reopened = Arc::new(SqliteStorageAdapter::new(
        Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        )),
        f.owner.public_key().to_hex(),
        f.device.public_key().to_hex(),
    ));
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(reopened, f.owner.public_key(), &f.device)
            .unwrap();
    assert_eq!(restored.pending_group_fanouts, before_fanouts);
    assert_eq!(
        restored.pending_group_sender_key_messages,
        before_ciphertexts
    );
    assert_eq!(
        serde_json::to_value(restored.session_manager_snapshot()).unwrap(),
        saved_sessions
    );
    let sent = restored
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "after restart",
            None,
            UnixSeconds(200),
        )
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
        1,
        "the exact committed ratchet must resume after reopening"
    );
    eprintln!("checkpoint_backlog fanouts={fanouts} ciphertexts={ciphertexts} checkpoint_bytes={bytes} cold_ms={cold_ms} warm_three_receive_batch_ms={warm_ms} scanned_bytes={scanned_bytes} read_bytes={read_bytes} written_bytes={written_bytes} warm_three_send_batch_ms={outgoing_ms} outgoing_scanned_bytes={outgoing_scanned_bytes} outgoing_read_bytes={outgoing_read_bytes} outgoing_written_bytes={outgoing_written_bytes}");
    eprintln!("checkpoint_pager receive_bytes={pager_bytes} send_bytes={outgoing_pager_bytes} limit={pager_limit}");
    assert!(
        fixture_started.elapsed() < std::time::Duration::from_secs(180),
        "bounded backlog checkpoint fixture exceeded three minutes"
    );
}

#[test]
fn checkpoint_backlog_incoming_group_control_preserves_unrelated_pending_work() {
    let mut f = checkpoint_backlog_fixture(64, 32);
    let peer = f.peer_owner.public_key();
    let hello = f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "establish both directions",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, hello.effects).len(),
        1
    );
    let fanouts = f.sender.pending_group_fanouts.clone();
    let ciphertexts = f.sender.pending_group_sender_key_messages.clone();
    let group = f
        .ready
        .create_group(
            "Incoming control".into(),
            vec![f.owner.public_key()],
            UnixSeconds(30),
        )
        .unwrap();
    let group_id = group.snapshot.unwrap().group_id;
    let encoded = CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(std::cell::Cell::get);
    f.sender.enter_batch();
    let messages = decrypt_own_sync_effects(&mut f.sender, group.effects);
    assert!(!messages.is_empty());
    for message in messages {
        let result = f
            .sender
            .process_group_pairwise_payload(
                message.content.as_bytes(),
                message.sender,
                message.sender_device,
            )
            .unwrap();
        assert!(result.consumed);
    }
    f.sender.exit_batch().unwrap();
    assert!(f.sender.group_manager.group(&group_id).is_some());
    assert_eq!(f.sender.pending_group_fanouts, fanouts);
    assert_eq!(f.sender.pending_group_sender_key_messages, ciphertexts);
    assert_eq!(
        CHECKPOINT_FANOUT_SERIALIZED_BYTES.with(std::cell::Cell::get),
        encoded
    );
    let restored =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(restored.group_manager.group(&group_id).is_some());
    assert_eq!(restored.pending_group_fanouts, fanouts);
    assert_eq!(restored.pending_group_sender_key_messages, ciphertexts);
}

#[test]
fn checkpoint_backlog_receive_batch_is_durable_and_restores_exact_ratchets() {
    checkpoint_backlog_send_roundtrip(64, 32);
}

#[test]
#[ignore = "bounded production-count SQLite checkpoint benchmark; run explicitly"]
fn checkpoint_backlog_production_counts_are_durable() {
    checkpoint_backlog_send_roundtrip(22_702, 19_231);
}

#[test]
fn checkpoint_fanout_failed_write_restores_pending_intent_and_cached_generation() {
    let mut f = checkpoint_backlog_fixture(64, 32);
    f.sender.persist().unwrap();
    let before = f.sender.pending_group_fanouts.clone();
    let cached = before[0].fanout.serialized().unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    f.sender.storage = storage.clone();
    let failure: anyhow::Result<()> = f.sender.with_state_checkpoint(|engine| {
        engine.pending_group_fanouts.remove(0);
        engine.persist()
    });
    assert!(failure.is_err());
    assert_eq!(f.sender.pending_group_fanouts, before);
    assert!(Arc::ptr_eq(
        &cached,
        &f.sender.pending_group_fanouts[0]
            .fanout
            .serialized()
            .unwrap()
    ));
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, f.owner.public_key(), &f.device)
            .unwrap();
    assert_eq!(restored.pending_group_fanouts, before);
}
