#[test]
fn checkpoint_remote_expiry_flushes_retry_clocks_without_rewriting_inbound_bodies() {
    const INBOUND: usize = 256;
    const NOW: u64 = 1_800_000_000;
    let mut fixture = checkpoint_backlog_fixture(512, 512);
    fixture.sender.pending_inbound = synthetic_pending_inbound_with_ciphertext(
        &mut fixture.sender,
        INBOUND,
        INBOUND,
        NOW,
        SyntheticInboundCiphertext::CorruptMac,
    );
    assert_corrupt_mac_inbound_format(&fixture.sender.pending_inbound);
    for pending in &mut fixture.sender.pending_group_fanouts {
        pending.created_at_secs = NOW - 1_000;
        pending.next_retry_at_secs = NOW;
    }
    fixture.sender.invalidate_pending_group_fanout_retry_deadline();
    fixture.sender.pending_remote_sends = missing_roster_remote_retries(2, NOW);
    for (index, pending) in fixture.sender.pending_remote_sends.iter_mut().enumerate() {
        let expires = NOW + 60 * (index as u64 + 1);
        let mut rumor = pairwise_codec::typing_event(
            fixture.sender.owner_pubkey,
            pairwise_codec::EncodeOptions::new(NOW, NOW * 1_000).with_expiration(expires),
        )
        .unwrap();
        rumor.ensure_id();
        pending.payload = serde_json::to_vec(&rumor).unwrap();
        pending.inner_event_id = rumor.id.map(|id| id.to_hex());
        pending.expires_at_secs = Some(expires);
        pending.next_retry_at_secs = expires;
    }
    let (mut fixture, directory, connection, storage) = checkpoint_growth_install_storage(fixture);
    let initial_bytes = fixture.sender.checkpoint_layout.borrow().bytes;
    let inbound_before = fixture.sender.pending_inbound.clone();
    let fanouts_before = fixture.sender.pending_group_fanouts.clone();
    let sessions_before = serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap();
    let pending_before = fixture.sender.pending_group_sender_key_messages.clone();
    let journal_before = fixture.sender.pending_decrypted_deliveries.clone();

    // Isolate removal/alignment from the accumulated in-memory retry clocks.
    let before = checkpoint_growth_written_bytes(&connection);
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(NOW + 60))
        .unwrap()
        .is_empty());
    let expiry_only_written = checkpoint_growth_written_bytes(&connection) - before;
    assert_eq!(fixture.sender.pending_remote_sends.len(), 1);
    assert_eq!(fixture.sender.pending_inbound, inbound_before);
    assert!(fixture.sender.pending_group_fanouts == fanouts_before);

    // These are real failed-decrypt and unavailable-device retry operations.
    // Their volatile backoff changes are flushed by the next genuine expiry.
    let before = checkpoint_growth_written_bytes(&connection);
    assert!(fixture
        .sender
        .retry_pending_inbound_direct_events(NdrUnixSeconds(NOW + 120))
        .unwrap()
        .is_empty());
    let result = fixture
        .sender
        .retry_pending_group_fanouts(NdrUnixSeconds(NOW + 120))
        .unwrap();
    assert!(result.effects.is_empty());
    assert_eq!(checkpoint_growth_written_bytes(&connection), before);
    let mut inbound_expected = inbound_before;
    for pending in &mut inbound_expected {
        pending.next_retry_at_secs = NOW + 180;
    }
    assert_eq!(fixture.sender.pending_inbound, inbound_expected);
    let mut fanouts_expected = fanouts_before;
    for pending in fanouts_expected
        .iter_mut()
        .take(PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE)
    {
        pending.next_retry_at_secs = NOW + 180;
    }
    assert!(fixture.sender.pending_group_fanouts == fanouts_expected);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap(),
        sessions_before
    );
    assert!(fixture.sender.pending_group_sender_key_messages == pending_before);
    assert_eq!(fixture.sender.pending_decrypted_deliveries, journal_before);

    let io_before = crate::storage::checkpoint_io_bytes();
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(NOW + 120))
        .unwrap()
        .is_empty());
    let clocks_written = checkpoint_growth_written_bytes(&connection) - before;
    let io_after = crate::storage::checkpoint_io_bytes();
    assert!(fixture.sender.pending_remote_sends.is_empty());
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let persisted: ProtocolEnginePersistedState = serde_json::from_str(&saved).unwrap();
    assert_eq!(persisted.pending_inbound, inbound_expected);
    assert!(persisted.pending_group_fanouts == fanouts_expected);
    assert!(persisted.pending_remote_sends.is_empty());
    assert!(persisted.pending_group_sender_key_messages == pending_before);
    assert_eq!(
        serde_json::to_value(persisted.session_manager).unwrap(),
        sessions_before
    );
    assert_eq!(persisted.pending_decrypted_deliveries, journal_before);

    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    let unchanged_written = checkpoint_growth_written_bytes(&connection) - before;
    let reopened = Arc::new(SqliteStorageAdapter::new(
        Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        )),
        fixture.owner.public_key().to_hex(),
        fixture.device.public_key().to_hex(),
    ));
    assert!(reopened.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().as_deref() == Some(saved.as_str()));
    let restored = ProtocolEngine::load_or_create_for_local_device(
        reopened,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert_eq!(restored.pending_inbound, inbound_expected);
    assert!(restored.pending_remote_sends.is_empty());
    assert!(restored.pending_group_fanouts == fanouts_expected);
    drop(restored);
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
    eprintln!("checkpoint_retry_clocks inbound={INBOUND} initial_bytes={initial_bytes} expiry_only_written_bytes={expiry_only_written} clocks_written_bytes={clocks_written} logical_read_bytes={} logical_written_bytes={} unchanged_written_bytes={unchanged_written}", io_after.0 - io_before.0, io_after.1 - io_before.1);
    assert_eq!(unchanged_written, 0);
    assert!(
        expiry_only_written <= 512 * 1024,
        "expiry without other mutations rewrote the core: {expiry_only_written}"
    );
    assert!(
        clocks_written <= 512 * 1024 && clocks_written < initial_bytes as u64 / 4,
        "durable retry clocks rewrote immutable inbound ciphertext: {clocks_written}"
    );
}

#[test]
fn checkpoint_inbound_cache_tracks_all_body_mutations_and_keeps_duplicate_clocks() {
    let mut fixture = remote_send_fixture();
    let original = synthetic_pending_inbound_with_ciphertext(
        &mut fixture.sender,
        1,
        1,
        1_800_000_000,
        SyntheticInboundCiphertext::CorruptMac,
    )
    .remove(0);
    let original_json = serde_json::to_value(&original).unwrap();
    let cached = original.body.checkpoint_bodies().unwrap();
    let mut clocks = original.clone();
    clocks.created_at_secs += 1;
    clocks.next_retry_at_secs += 2;
    let reused = clocks.body.checkpoint_bodies().unwrap();
    assert!(Arc::ptr_eq(&cached.event, &reused.event));
    assert!(Arc::ptr_eq(&cached.envelope, &reused.envelope));
    let mutations: [fn(&mut ProtocolPendingInbound); 7] = [
        |p| p.event.content.push('x'),
        |p| p.envelope.as_mut().unwrap().ciphertext.push('x'),
        |p| p.event_id.push('x'),
        |p| p.sender_message_pubkey_hex = None,
        |p| p.resolved_owner_pubkey_hex = Some("changed owner".into()),
        |p| p.claimed_owner_pubkey_hex = Some("changed claim".into()),
        |p| p.metadata_verified = !p.metadata_verified,
    ];
    let mut entries = vec![original.clone(), clocks];
    for mutate in mutations {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert_ne!(changed, original);
        let fresh = changed.body.checkpoint_bodies().unwrap();
        assert!(!Arc::ptr_eq(&cached.event, &fresh.event));
        assert!(!Arc::ptr_eq(&cached.envelope, &fresh.envelope));
        assert_eq!(
            serde_json::from_str::<Event>(fresh.event.json()).unwrap(),
            changed.event
        );
        assert_eq!(
            serde_json::from_str::<Option<MessageEnvelope>>(fresh.envelope.json()).unwrap(),
            changed.envelope
        );
        assert_eq!(serde_json::to_value(&original).unwrap(), original_json);
        entries.push(changed);
    }
    let mut checkpoint = crate::StorageCheckpoint::new();
    checkpoint.push_inline("start", "[".into(), 0);
    append_checkpoint_inbound(&mut checkpoint, &entries).unwrap();
    checkpoint.push_inline("end", "]".into(), 0);
    let materialized = checkpoint.materialize().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&materialized).unwrap(),
        serde_json::to_value(&entries).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<Vec<ProtocolPendingInbound>>(&materialized).unwrap(),
        entries
    );
}

#[test]
fn checkpoint_inbound_clock_flush_commit_failure_keeps_live_and_durable_generations() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static COMMIT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn reject_commit(_: *mut std::ffi::c_void) -> std::ffi::c_int {
        COMMIT_ATTEMPTS.fetch_add(1, Relaxed);
        1
    }
    const NOW: u64 = 1_800_000_000;
    let mut fixture = checkpoint_backlog_fixture(512, 512);
    fixture.sender.pending_inbound = synthetic_pending_inbound_with_ciphertext(
        &mut fixture.sender,
        16,
        16,
        NOW,
        SyntheticInboundCiphertext::CorruptMac,
    );
    fixture.sender.pending_remote_sends = missing_roster_remote_retries(1, NOW);
    fixture.sender.pending_remote_sends[0].expires_at_secs = Some(NOW);
    let (mut fixture, directory, connection, storage) = checkpoint_growth_install_storage(fixture);
    let durable_inbound = fixture.sender.pending_inbound.clone();
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    assert!(fixture
        .sender
        .retry_pending_inbound_direct_events(NdrUnixSeconds(NOW))
        .unwrap()
        .is_empty());
    let live_before = fixture.sender.state_checkpoint();
    assert_ne!(fixture.sender.pending_inbound, durable_inbound);
    COMMIT_ATTEMPTS.store(0, Relaxed);
    unsafe {
        rusqlite::ffi::sqlite3_commit_hook(
            connection.lock().unwrap().handle(),
            Some(reject_commit),
            std::ptr::null_mut(),
        );
    }
    let result = fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(NOW));
    unsafe {
        rusqlite::ffi::sqlite3_commit_hook(
            connection.lock().unwrap().handle(),
            None,
            std::ptr::null_mut(),
        );
    }
    assert_eq!(COMMIT_ATTEMPTS.load(Relaxed), 1);
    assert!(result.unwrap_err().downcast_ref::<StorageError>().is_some());
    assert!(connection.lock().unwrap().is_autocommit());
    assert!(storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().as_deref() == Some(saved.as_str()));
    assert_eq!(fixture.sender.pending_inbound, live_before.pending_inbound);
    assert_eq!(
        serde_json::to_value(&fixture.sender.pending_remote_sends).unwrap(),
        serde_json::to_value(&live_before.pending_remote_sends).unwrap()
    );
    assert!(fixture.sender.pending_group_fanouts == live_before.pending_group_fanouts);
    assert!(
        fixture.sender.pending_group_sender_key_messages
            == live_before.pending_group_sender_key_messages
    );
    assert_eq!(
        fixture.sender.pending_decrypted_deliveries,
        live_before.pending_decrypted_deliveries
    );
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap(),
        serde_json::to_value(live_before.session_manager.snapshot()).unwrap()
    );
    assert_eq!(
        fixture.sender.group_manager.snapshot(),
        live_before.group_manager.snapshot()
    );
    let reopened = Arc::new(SqliteStorageAdapter::new(
        Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        )),
        fixture.owner.public_key().to_hex(),
        fixture.device.public_key().to_hex(),
    ));
    let restored = ProtocolEngine::load_or_create_for_local_device(
        reopened,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert_eq!(restored.pending_inbound, durable_inbound);
    assert_eq!(restored.pending_remote_sends.len(), 1);
    drop(restored);
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(NOW))
        .unwrap()
        .is_empty());
    let persisted: ProtocolEnginePersistedState =
        serde_json::from_str(&storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(persisted.pending_inbound, live_before.pending_inbound);
    assert!(persisted.pending_remote_sends.is_empty());
    let before = checkpoint_growth_written_bytes(&connection);
    fixture.sender.persist().unwrap();
    assert_eq!(checkpoint_growth_written_bytes(&connection), before);
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
}

struct CheckpointNodeSqlAudit {
    connection: Arc<Mutex<rusqlite::Connection>>,
    counts: Box<[std::sync::atomic::AtomicUsize; 2]>,
}

impl CheckpointNodeSqlAudit {
    fn start(connection: Arc<Mutex<rusqlite::Connection>>) -> Self {
        use std::ffi::{c_char, c_int, c_void, CStr};
        unsafe fn count(context: *mut c_void, table: *const c_char, index: usize) {
            if !table.is_null()
                && unsafe { CStr::from_ptr(table) }.to_bytes() == b"ndr_kv_checkpoint_nodes"
            {
                let counts = unsafe { &*(context as *const [std::sync::atomic::AtomicUsize; 2]) };
                counts[index].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        unsafe extern "C" fn authorize(
            context: *mut c_void,
            action: c_int,
            table: *const c_char,
            _: *const c_char,
            _: *const c_char,
            _: *const c_char,
        ) -> c_int {
            if action == rusqlite::ffi::SQLITE_INSERT {
                unsafe { count(context, table, 0) };
            }
            rusqlite::ffi::SQLITE_OK
        }
        unsafe extern "C" fn update(
            context: *mut c_void,
            action: c_int,
            _: *const c_char,
            table: *const c_char,
            _: i64,
        ) {
            if action == rusqlite::ffi::SQLITE_UPDATE {
                unsafe { count(context, table, 1) };
            }
        }
        let mut audit = Self {
            connection,
            counts: Box::new([0.into(), 0.into()]),
        };
        let context = audit.counts.as_mut() as *mut _ as *mut c_void;
        {
            let conn = audit.connection.lock().unwrap();
            unsafe {
                assert_eq!(
                    rusqlite::ffi::sqlite3_set_authorizer(conn.handle(), Some(authorize), context),
                    rusqlite::ffi::SQLITE_OK
                );
                rusqlite::ffi::sqlite3_update_hook(conn.handle(), Some(update), context);
            }
        }
        audit
    }
}

impl Drop for CheckpointNodeSqlAudit {
    fn drop(&mut self) {
        let conn = self.connection.lock().unwrap();
        // Clear both hooks before the boxed callback context is released,
        // including if an assertion unwinds this test.
        unsafe {
            rusqlite::ffi::sqlite3_set_authorizer(conn.handle(), None, std::ptr::null_mut());
            rusqlite::ffi::sqlite3_update_hook(conn.handle(), None, std::ptr::null_mut());
        }
    }
}

#[test]
fn checkpoint_retry_clock_flush_prepares_node_update_once() {
    use std::sync::atomic::Ordering::Relaxed;
    const FANOUTS: usize = 512;
    const INBOUND: usize = 64;
    const NOW: u64 = 1_800_000_000;
    let mut fixture = checkpoint_backlog_fixture(FANOUTS, 512);
    fixture.sender.pending_inbound = synthetic_pending_inbound_with_ciphertext(
        &mut fixture.sender,
        INBOUND,
        INBOUND,
        NOW,
        SyntheticInboundCiphertext::CorruptMac,
    );
    for pending in &mut fixture.sender.pending_group_fanouts {
        pending.created_at_secs = NOW - 1_000;
        pending.next_retry_at_secs = NOW;
    }
    fixture.sender.invalidate_pending_group_fanout_retry_deadline();
    fixture.sender.pending_remote_sends = missing_roster_remote_retries(1, NOW);
    fixture.sender.pending_remote_sends[0].expires_at_secs = Some(NOW);
    let (mut fixture, directory, connection, storage) = checkpoint_growth_install_storage(fixture);
    let inbound_before = fixture.sender.pending_inbound.clone();
    let fanouts_before = fixture.sender.pending_group_fanouts.clone();
    let sessions_before = serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap();
    let before = checkpoint_growth_written_bytes(&connection);
    assert!(fixture
        .sender
        .retry_pending_inbound_direct_events(NdrUnixSeconds(NOW))
        .unwrap()
        .is_empty());
    for _ in 0..FANOUTS / PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE {
        assert!(fixture
            .sender
            .retry_pending_group_fanouts(NdrUnixSeconds(NOW))
            .unwrap()
            .effects
            .is_empty());
    }
    assert_eq!(checkpoint_growth_written_bytes(&connection), before);
    let mut expected_inbound = inbound_before;
    for pending in &mut expected_inbound {
        pending.next_retry_at_secs = NOW + 60;
    }
    let mut expected_fanouts = fanouts_before;
    for pending in &mut expected_fanouts {
        pending.next_retry_at_secs = NOW + 60;
    }
    assert_eq!(fixture.sender.pending_inbound, expected_inbound);
    assert!(fixture.sender.pending_group_fanouts == expected_fanouts);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap(),
        sessions_before
    );

    // Authorizer calls count SQLite compilation, not execution; the update
    // hook independently confirms every dirty metadata row was actually saved.
    let audit = CheckpointNodeSqlAudit::start(connection.clone());
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(NOW))
        .unwrap()
        .is_empty());
    let preparations = audit.counts[0].load(Relaxed);
    let updated_nodes = audit.counts[1].load(Relaxed);
    drop(audit);
    let written = checkpoint_growth_written_bytes(&connection) - before;
    assert!(fixture.sender.pending_remote_sends.is_empty());
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let persisted: ProtocolEnginePersistedState = serde_json::from_str(&saved).unwrap();
    assert_eq!(persisted.pending_inbound, expected_inbound);
    assert!(persisted.pending_group_fanouts == expected_fanouts);
    assert!(persisted.pending_remote_sends.is_empty());
    assert_eq!(
        serde_json::to_value(persisted.session_manager).unwrap(),
        sessions_before
    );
    checkpoint_growth_assert_reopen(fixture, &directory, connection, storage);
    eprintln!("checkpoint_node_sql preparations={preparations} updated_nodes={updated_nodes} updated_fanout_nodes={FANOUTS} updated_inbound_nodes={INBOUND} sqlite_written_bytes={written}");
    // The exact saved vectors above prove every clock; core/reserve
    // descriptors may also change when the expiring send is retired.
    assert!(updated_nodes >= FANOUTS + INBOUND);
    assert!(written > 0);
    assert_eq!(
        preparations, 1,
        "one expiry flush must not recompile the same node update per dirty record"
    );
}
