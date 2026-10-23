const GROWTH_BACKLOG_RECORDS: usize = 512;

fn checkpoint_growth_fixture() -> (
    RemoteSendFixture,
    tempfile::TempDir,
    Arc<Mutex<rusqlite::Connection>>,
    Arc<SqliteStorageAdapter>,
) {
    checkpoint_growth_install_storage(checkpoint_backlog_fixture(
        GROWTH_BACKLOG_RECORDS,
        GROWTH_BACKLOG_RECORDS,
    ))
}

fn checkpoint_growth_install_storage(
    mut fixture: RemoteSendFixture,
) -> (
    RemoteSendFixture,
    tempfile::TempDir,
    Arc<Mutex<rusqlite::Connection>>,
    Arc<SqliteStorageAdapter>,
) {
    let directory = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
    connection
        .execute_batch(
            "PRAGMA page_size=4096; PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE ndr_kv (
                 owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
                 key TEXT NOT NULL, value TEXT NOT NULL,
                 PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));",
        )
        .unwrap();
    let connection = Arc::new(Mutex::new(connection));
    let storage = Arc::new(SqliteStorageAdapter::new(
        connection.clone(),
        fixture.owner.public_key().to_hex(),
        fixture.device.public_key().to_hex(),
    ));
    fixture.sender.storage = storage.clone();
    fixture.sender.persist().unwrap();
    let bytes = fixture.sender.checkpoint_layout.borrow().bytes;
    assert!(
        (2 * 1024 * 1024..=8 * 1024 * 1024).contains(&bytes),
        "natural checkpoint must be modest but multi-MiB, got {bytes}"
    );
    // No forced layout allocation: the first save chooses its normal padding.
    assert_eq!(bytes % CHECKPOINT_RESERVE, 0);
    assert_eq!(
        storage
            .get(PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap()
            .len(),
        bytes
    );
    (fixture, directory, connection, storage)
}

fn checkpoint_growth_written_bytes(connection: &Mutex<rusqlite::Connection>) -> u64 {
    let connection = connection.lock().unwrap();
    let mut current = 0;
    let mut highwater = 0;
    // Unlike checkpoint_io_bytes, this sees SQLite writes through both the
    // incremental path and ordinary INSERT/UPDATE fallback.
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            connection.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,
            &mut current,
            &mut highwater,
            0,
        )
    };
    assert_eq!(result, rusqlite::ffi::SQLITE_OK);
    let page_size: u64 = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    assert_eq!(page_size, 4096);
    u64::try_from(current).unwrap() * page_size
}

fn checkpoint_growth_next_fanout(
    engine: &ProtocolEngine,
    ordinal: usize,
) -> ProtocolPendingGroupFanout {
    let mut pending = engine.pending_group_fanouts[0].clone();
    let GroupPendingFanout::Remote {
        recipient_owner,
        payload,
    } = pending.fanout.as_ref()
    else {
        panic!("growth fixture needs a typed remote fanout");
    };
    let Some(GroupPairwiseCommand::SenderKeyRepairRequest { mut request }) =
        JsonGroupPayloadCodecV1
            .decode_pairwise_command(payload)
            .unwrap()
    else {
        panic!("growth fixture needs a genuine repair command");
    };
    let created_at = NdrUnixSeconds(20 + ordinal as u64);
    request.created_at = created_at;
    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: engine.local_device,
                created_at,
            },
            &GroupPairwiseCommand::SenderKeyRepairRequest { request },
        )
        .unwrap();
    pending.fanout = GroupPendingFanout::Remote {
        recipient_owner: *recipient_owner,
        payload,
    }
    .into();
    pending.created_at_secs = created_at.get();
    pending
}

fn checkpoint_growth_assert_reopen(
    fixture: RemoteSendFixture,
    directory: &tempfile::TempDir,
    connection: Arc<Mutex<rusqlite::Connection>>,
    storage: Arc<SqliteStorageAdapter>,
) {
    let pending = fixture.sender.pending_group_sender_key_messages.clone();
    let fanouts = fixture.sender.pending_group_fanouts.clone();
    let sessions = serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap();
    let groups = serde_json::to_value(fixture.sender.group_manager.snapshot()).unwrap();
    let deliveries = serde_json::to_value(&fixture.sender.pending_decrypted_deliveries).unwrap();
    let processed = fixture.sender.processed_group_sender_key_messages.clone();
    let owner = fixture.owner.public_key();
    let device = fixture.device.clone();
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    drop(fixture);
    drop(storage);
    drop(connection);
    let reopened = Arc::new(SqliteStorageAdapter::new(
        Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        )),
        owner.to_hex(),
        device.public_key().to_hex(),
    ));
    assert!(
        reopened.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().as_deref() == Some(saved.as_str()),
        "reopening SQLite changed the logical checkpoint text"
    );
    let restored =
        ProtocolEngine::load_or_create_for_local_device(reopened, owner, &device).unwrap();
    assert_eq!(
        restored.pending_group_sender_key_messages.len(),
        pending.len()
    );
    assert_eq!(restored.pending_group_fanouts.len(), fanouts.len());
    assert!(restored.pending_group_sender_key_messages == pending);
    assert!(restored.pending_group_fanouts == fanouts);
    assert_eq!(
        serde_json::to_value(restored.session_manager_snapshot()).unwrap(),
        sessions,
        "checkpoint growth must preserve exact direct ratchet state"
    );
    assert_eq!(
        serde_json::to_value(restored.group_manager.snapshot()).unwrap(),
        groups
    );
    assert_eq!(
        serde_json::to_value(&restored.pending_decrypted_deliveries).unwrap(),
        deliveries
    );
    assert_eq!(
        serde_json::to_value(&restored.processed_group_sender_key_messages).unwrap(),
        serde_json::to_value(&processed).unwrap()
    );
}

#[test]
fn checkpoint_natural_growth_does_not_rewrite_the_whole_sqlite_value() {
    let (mut fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let initial_bytes = fixture.sender.checkpoint_layout.borrow().bytes;
    let original_pending = fixture.sender.pending_group_sender_key_messages.clone();
    let original_sessions =
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap();
    let mut crossing = None;
    for appended in 1..=64 {
        let next =
            checkpoint_growth_next_fanout(&fixture.sender, GROWTH_BACKLOG_RECORDS + appended);
        fixture.sender.pending_group_fanouts.push(next);
        let before = checkpoint_growth_written_bytes(&connection);
        fixture.sender.persist().unwrap();
        let written = checkpoint_growth_written_bytes(&connection) - before;
        let bytes = fixture.sender.checkpoint_layout.borrow().bytes;
        if bytes > initial_bytes {
            crossing = Some((appended, bytes, written));
            break;
        }
    }
    let (appended, grown_bytes, written) =
        crossing.expect("64 real fanouts must exhaust tail slack");
    assert_eq!(grown_bytes - initial_bytes, CHECKPOINT_RESERVE);
    assert_eq!(
        fixture.sender.pending_group_fanouts.len(),
        GROWTH_BACKLOG_RECORDS + appended
    );
    assert!(fixture.sender.pending_group_sender_key_messages == original_pending);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap(),
        original_sessions
    );
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let unchanged_written = checkpoint_growth_written_bytes(&connection) - before;
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
    eprintln!(
        "checkpoint_growth initial_bytes={initial_bytes} grown_bytes={grown_bytes} appended={appended} crossing_written_bytes={written} unchanged_written_bytes={unchanged_written}"
    );
    assert_eq!(
        unchanged_written, 0,
        "an unchanged save must not dirty SQLite pages"
    );
    assert!(
        written <= 1024 * 1024 && written < initial_bytes as u64 / 2,
        "small checkpoint growth rewrote its backlog: {written} bytes for a {initial_bytes}-byte value"
    );
}

#[test]
fn checkpoint_front_ciphertext_mutation_reports_physical_write_cost_and_restores_state() {
    let (mut fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let initial_bytes = fixture.sender.checkpoint_layout.borrow().bytes;
    let original_pending = fixture.sender.pending_group_sender_key_messages.clone();
    let original_fanouts = fixture.sender.pending_group_fanouts.clone();
    let original_sessions =
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap();
    let mut encryptor = nostr_double_ratchet::SenderKeyState::new(1, [19; 32], 0);
    let mut inserted = original_pending[0].clone();
    let (number, ciphertext) = encryptor
        .encrypt_to_bytes(format!("front-control:{}", "x".repeat(768)).as_bytes())
        .unwrap();
    inserted.message_number = number;
    inserted.ciphertext = ciphertext;
    fixture
        .sender
        .pending_group_sender_key_messages
        .insert(0, inserted.clone());
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let insertion_written = checkpoint_growth_written_bytes(&connection) - before;
    let saved: ProtocolEnginePersistedState =
        serde_json::from_str(&storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(
        saved.pending_group_sender_key_messages.len(),
        GROWTH_BACKLOG_RECORDS + 1
    );
    assert!(saved.pending_group_sender_key_messages[0] == inserted);
    assert!(saved
        .pending_group_sender_key_messages
        .iter()
        .skip(1)
        .eq(original_pending.iter()));
    assert!(saved.pending_group_fanouts == original_fanouts);
    assert!(fixture.sender.pending_group_sender_key_messages.remove(0) == inserted);
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let removal_written = checkpoint_growth_written_bytes(&connection) - before;
    assert!(fixture.sender.pending_group_sender_key_messages == original_pending);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap(),
        original_sessions
    );
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let unchanged_written = checkpoint_growth_written_bytes(&connection) - before;
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
    eprintln!(
        "checkpoint_front_control initial_bytes={initial_bytes} insertion_written_bytes={insertion_written} removal_written_bytes={removal_written} unchanged_written_bytes={unchanged_written} small_write_bound_claimed=false"
    );
    // Ordinal record padding can shift unrelated records after a front edit.
    // Expose that separate cost instead of conflating it with storage resizing.
    assert!(insertion_written > 0 && removal_written > 0);
    assert_eq!(unchanged_written, 0);
}

#[test]
fn checkpoint_real_group_queue_append_and_retry_retirement_are_bounded() {
    let mut fixture = checkpoint_backlog_fixture(GROWTH_BACKLOG_RECORDS, GROWTH_BACKLOG_RECORDS);
    let group = group_snapshot_for_test(
        "checkpoint-growth-group",
        "Queued history",
        1,
        &fixture.peer_owner,
        &[fixture.owner.public_key(), fixture.peer_owner.public_key()],
    );
    let author = ndr_device(Keys::generate().public_key());
    let initial = nostr_double_ratchet::SenderKeyState::new(1, [31; 32], 0);
    let body = format!("authenticated queue retirement:{}", "x".repeat(768)).into_bytes();
    let plaintext = JsonGroupPayloadCodecV1
        .encode_sender_key_plaintext(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(fixture.ready_device.public_key()),
                created_at: NdrUnixSeconds(20),
            },
            &nostr_double_ratchet::GroupSenderKeyPlaintext {
                group_id: group.group_id.clone(),
                revision: 1,
                body: body.clone(),
            },
        )
        .unwrap();
    let (message_number, ciphertext) = initial.clone().encrypt_to_bytes(&plaintext).unwrap();
    let first = nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
        sender_event_pubkey: author,
        key_id: 1,
        message_number,
        encrypted_header: Some("hidden position".into()),
        created_at: NdrUnixSeconds(20),
        ciphertext,
    };
    fixture.sender.group_manager = GroupEventManager::from_snapshot(GroupManagerSnapshot {
        local_owner_pubkey: fixture.sender.local_owner,
        groups: vec![group],
        sender_keys: vec![nostr_double_ratchet::GroupSenderKeyRecordSnapshot {
            group_id: "checkpoint-growth-group".into(),
            sender_owner: ndr_owner(fixture.peer_owner.public_key()),
            sender_device: ndr_device(fixture.ready_device.public_key()),
            sender_event_pubkey: author,
            sender_event_secret_key: None,
            latest_key_id: Some(1),
            states: vec![initial.clone()],
            distribution_history: vec![],
            distributed_to: vec![],
            repair_snapshots: vec![],
        }],
    })
    .unwrap();
    // Set up one recoverable front candidate before creating the real database.
    // Every measured queue mutation below uses the production admission/retry path.
    fixture.sender.pending_group_sender_key_messages[0] = first.clone();
    let mut appended = first.clone();
    appended.sender_event_pubkey = ndr_device(Keys::generate().public_key());
    appended.ciphertext = nostr_double_ratchet::SenderKeyState::new(1, [32; 32], 0)
        .encrypt_to_bytes(&plaintext)
        .unwrap()
        .1;
    assert!(fixture
        .sender
        .group_sender_key_message_from_parsed(&appended)
        .is_none());
    let mut expected_groups = fixture.sender.group_manager.snapshot();
    let mut advanced = initial;
    assert_eq!(
        advanced
            .decrypt_from_bytes(first.message_number, &first.ciphertext)
            .unwrap(),
        plaintext
    );
    expected_groups.sender_keys[0].states = vec![advanced];
    let (mut fixture, directory, connection, storage) = checkpoint_growth_install_storage(fixture);
    let initial_bytes = fixture.sender.checkpoint_layout.borrow().bytes;
    let original_fanouts = fixture.sender.pending_group_fanouts.clone();
    let original_deliveries = fixture.sender.pending_decrypted_deliveries.clone();
    let original_sessions =
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap();
    let mut expected_pending = fixture.sender.pending_group_sender_key_messages.clone();
    expected_pending.push(appended.clone());
    let before = checkpoint_growth_written_bytes(&connection);
    fixture
        .sender
        .queue_pending_group_sender_key_message(appended.clone())
        .unwrap();
    let append_written = checkpoint_growth_written_bytes(&connection) - before;
    assert!(fixture.sender.pending_group_sender_key_messages == expected_pending);
    assert_eq!(expected_pending.len(), GROWTH_BACKLOG_RECORDS + 1);
    let appended_state: ProtocolEnginePersistedState =
        serde_json::from_str(&storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap()).unwrap();
    assert!(appended_state.pending_group_sender_key_messages == expected_pending);
    assert!(appended_state.pending_group_fanouts == original_fanouts);
    drop(appended_state);
    let before = checkpoint_growth_written_bytes(&connection);
    let result = fixture
        .sender
        .retry_eligible_group_sender_key_messages()
        .unwrap();
    let retirement_written = checkpoint_growth_written_bytes(&connection) - before;
    let [GroupIncomingEvent::Message(received)] = result.events.as_slice() else {
        panic!("the authenticated first candidate must be delivered exactly once");
    };
    assert!(result.effects.is_empty());
    assert_eq!(received.body, body);
    assert_eq!(
        received.sender_owner,
        ndr_owner(fixture.peer_owner.public_key())
    );
    assert_eq!(
        received.sender_device,
        Some(ndr_device(fixture.ready_device.public_key()))
    );
    assert!(expected_pending.remove(0) == first);
    assert_eq!(expected_pending.len(), GROWTH_BACKLOG_RECORDS);
    assert!(expected_pending.last() == Some(&appended));
    assert!(fixture.sender.pending_group_sender_key_messages == expected_pending);
    assert!(fixture.sender.pending_group_fanouts == original_fanouts);
    assert_eq!(fixture.sender.group_manager.snapshot(), expected_groups);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap(),
        original_sessions
    );
    assert_eq!(
        fixture.sender.pending_decrypted_deliveries.len(),
        original_deliveries.len() + 1
    );
    assert_eq!(
        &fixture.sender.pending_decrypted_deliveries[..original_deliveries.len()],
        original_deliveries.as_slice()
    );
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let unchanged_written = checkpoint_growth_written_bytes(&connection) - before;
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
    eprintln!(
        "checkpoint_real_queue initial_bytes={initial_bytes} appended=1 retired_front=1 remaining={GROWTH_BACKLOG_RECORDS} append_written_bytes={append_written} retirement_written_bytes={retirement_written} unchanged_written_bytes={unchanged_written}"
    );
    assert_eq!(
        unchanged_written, 0,
        "unchanged queue save must not dirty pages"
    );
    for (operation, written) in [
        ("append", append_written),
        ("retirement", retirement_written),
    ] {
        assert!(
            written > 0 && written <= 1024 * 1024 && written < initial_bytes as u64 / 2,
            "real queue {operation} rewrote its backlog: {written} bytes for a {initial_bytes}-byte value"
        );
    }
}

#[test]
fn checkpoint_queue_commit_failure_restores_live_and_reopened_engine() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static COMMIT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn reject_commit(_: *mut std::ffi::c_void) -> std::ffi::c_int {
        COMMIT_ATTEMPTS.fetch_add(1, Relaxed);
        1
    }
    let (mut fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let before = fixture.sender.state_checkpoint();
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
    let mut candidate = fixture.sender.pending_group_sender_key_messages[0].clone();
    candidate.created_at = NdrUnixSeconds(candidate.created_at.get() + 1);
    assert!(!fixture
        .sender
        .pending_group_sender_key_messages
        .contains(&candidate));
    COMMIT_ATTEMPTS.store(0, Relaxed);
    {
        let conn = connection.lock().unwrap();
        // Reject only the final SQLite commit, after the actual checkpoint
        // writes. The static callback remains valid even if this test panics.
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(
                conn.handle(),
                Some(reject_commit),
                std::ptr::null_mut(),
            );
        }
    }
    let result = fixture.sender.with_state_checkpoint(|engine| {
        engine.queue_pending_group_sender_key_message(candidate.clone())
    });
    {
        let conn = connection.lock().unwrap();
        unsafe {
            rusqlite::ffi::sqlite3_commit_hook(conn.handle(), None, std::ptr::null_mut());
        }
    }
    assert_eq!(COMMIT_ATTEMPTS.load(Relaxed), 1);
    assert!(result.unwrap_err().downcast_ref::<StorageError>().is_some());
    assert!(connection.lock().unwrap().is_autocommit());
    assert_eq!(storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(), saved);
    assert!(
        fixture.sender.pending_group_sender_key_messages
            == before.pending_group_sender_key_messages
    );
    assert!(fixture.sender.pending_group_fanouts == before.pending_group_fanouts);
    assert_eq!(
        fixture.sender.group_manager.snapshot(),
        before.group_manager.snapshot()
    );
    assert_eq!(
        fixture.sender.pending_decrypted_deliveries,
        before.pending_decrypted_deliveries
    );
    let restored = ProtocolEngine::load_or_create_for_local_device(
        Arc::new(SqliteStorageAdapter::new(
            Arc::new(Mutex::new(
                rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
            )),
            fixture.owner.public_key().to_hex(),
            fixture.device.public_key().to_hex(),
        )),
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert!(restored.pending_group_sender_key_messages == before.pending_group_sender_key_messages);
    assert!(restored.pending_group_fanouts == before.pending_group_fanouts);
    assert_eq!(
        restored.group_manager.snapshot(),
        before.group_manager.snapshot()
    );
    assert_eq!(
        restored.pending_decrypted_deliveries,
        before.pending_decrypted_deliveries
    );
    let expected_sessions = serde_json::to_value(before.session_manager.snapshot()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.session_manager_snapshot()).unwrap(),
        expected_sessions
    );
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager_snapshot()).unwrap(),
        expected_sessions
    );
    drop(restored);
    fixture
        .sender
        .queue_pending_group_sender_key_message(candidate.clone())
        .unwrap();
    let mut expected = before.pending_group_sender_key_messages;
    expected.push(candidate);
    assert!(fixture.sender.pending_group_sender_key_messages == expected);
    let writes = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    assert_eq!(checkpoint_growth_written_bytes(&connection), writes);
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
}

#[test]
fn checkpoint_invalid_or_unsupported_state_is_preserved_without_startup_writes() {
    use std::sync::atomic::Ordering::Relaxed;
    let owner = Keys::generate();
    let device = Keys::generate();
    let valid = test_engine(&owner, &device)
        .storage
        .get(PROTOCOL_ENGINE_STATE_KEY)
        .unwrap()
        .unwrap();
    let mut unsupported: serde_json::Value = serde_json::from_str(&valid).unwrap();
    unsupported["version"] = serde_json::json!(PROTOCOL_ENGINE_STATE_VERSION + 1);
    for raw in ["{\"version\":".to_owned(), unsupported.to_string()] {
        let inner = Arc::new(InMemoryStorage::new());
        inner.put(PROTOCOL_ENGINE_STATE_KEY, raw.clone()).unwrap();
        let counted = Arc::new(GroupRetryCountingStorage {
            inner: inner.clone(),
            puts: 0.into(),
            fail_next: false.into(),
        });
        assert!(ProtocolEngine::load_or_create_for_local_device(
            counted.clone(),
            owner.public_key(),
            &device,
        )
        .is_err());
        assert_eq!(
            counted.puts.load(Relaxed),
            0,
            "failed load must not create replacement state or keys"
        );
        assert_eq!(inner.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(), Some(raw));
        assert_eq!(
            inner.list("").unwrap(),
            vec![PROTOCOL_ENGINE_STATE_KEY.to_owned()]
        );
    }
}
