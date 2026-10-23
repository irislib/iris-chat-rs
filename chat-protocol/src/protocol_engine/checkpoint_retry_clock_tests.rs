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
