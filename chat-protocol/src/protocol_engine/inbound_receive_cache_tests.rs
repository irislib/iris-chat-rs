thread_local! {
    static INBOUND_RECEIVE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_inbound_receive_call() {
    INBOUND_RECEIVE_CALLS.with(|count| count.set(count.get() + 1));
}

fn take_inbound_receive_calls() -> usize {
    INBOUND_RECEIVE_CALLS.with(|count| count.replace(0))
}

fn make_inbound_due(engine: &mut ProtocolEngine) {
    for pending in &mut engine.pending_inbound {
        pending.next_retry_at_secs = 0;
    }
    engine.invalidate_pending_inbound_retry_deadline();
}

#[test]
fn failed_receive_cache_unchanged_failures_bind_envelope_and_event_context() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let mut packets = synthetic_pending_inbound_with_ciphertext(
        &mut engine,
        nostr_double_ratchet::MAX_SKIP + 2,
        0,
        1_000,
        SyntheticInboundCiphertext::CorruptMac,
    );
    let far = packets.pop().unwrap();
    engine.pending_inbound = vec![packets.remove(0), far];
    let initial = engine.session_manager.snapshot();
    take_inbound_receive_calls();
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_000))
        .unwrap();
    let first = take_inbound_receive_calls();
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_001))
        .unwrap();
    let repeated = take_inbound_receive_calls();
    // The cached envelope is a separate restored field. The event ID alone is
    // insufficient to bind it, even if the signed event itself is unchanged.
    engine.pending_inbound[0]
        .envelope
        .as_mut()
        .unwrap()
        .encrypted_header
        .push('!');
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_002))
        .unwrap();
    let changed_envelope = take_inbound_receive_calls();
    engine.pending_inbound[1].event.created_at = nostr::Timestamp::from(2);
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_003))
        .unwrap();
    let changed_context = take_inbound_receive_calls();
    assert!(engine.session_manager.snapshot() == initial);
    assert!(engine.pending_decrypted_deliveries.is_empty());
    assert_eq!(engine.pending_inbound.len(), 2);
    assert_eq!(first, 2);
    // Invalid-header failures are deliberately not memoized.
    assert_eq!(changed_context, 2);
    assert_eq!((repeated, changed_envelope), (0, 1));
}

#[test]
fn failed_receive_cache_same_pass_progress_unlocks_exact_old_packet() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let mut packets = synthetic_pending_inbound_modified(
        &mut engine,
        nostr_double_ratchet::MAX_SKIP + 2,
        0,
        1_000,
        |_, _| {},
    );
    let future = packets.pop().unwrap();
    let bridge = packets.remove(0);
    let owner = future.resolved_owner_pubkey_hex.clone().unwrap();
    for packet in [&bridge, &future] {
        let wire = serde_json::to_string(&packet.event).unwrap();
        let parsed: Event = serde_json::from_str(&wire).unwrap();
        parsed.verify().unwrap();
        assert_ne!(parsed.pubkey.to_hex(), owner);
        assert!(parsed.tags.public_keys().next().is_none());
    }
    engine.pending_inbound = vec![future.clone()];
    make_inbound_due(&mut engine);
    take_inbound_receive_calls();
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_000))
        .unwrap();
    assert_eq!(take_inbound_receive_calls(), 1);
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_001))
        .unwrap();
    let repeated = take_inbound_receive_calls();
    engine.pending_inbound.insert(0, bridge);
    make_inbound_due(&mut engine);
    let delivered = engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(1_002))
        .unwrap();
    assert_eq!(delivered.len(), 2);
    assert_eq!(
        delivered[1].event_id.as_deref(),
        Some(future.event.id.to_hex().as_str())
    );
    assert!(delivered[1].content.starts_with("deferred-1001:"));
    assert!(engine.pending_inbound.is_empty());
    assert_eq!(take_inbound_receive_calls(), 2);
    assert_eq!(repeated, 0);
}

fn failed_receive_import_fixture() -> (ProtocolEngine, Keys, Keys, Keys, Keys, SessionState) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    engine
        .ingest_app_keys_event(&signed_app_keys(&peer, &[peer_device.public_key()], 10))
        .unwrap();
    let invite = engine.local_invite().unwrap();
    let (mut sending, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer.public_key()),
        )
        .unwrap();
    engine
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    let mut snapshot = engine.session_manager.snapshot();
    let record = snapshot
        .users
        .iter_mut()
        .find(|u| u.owner_pubkey == ndr_owner(peer.public_key()))
        .unwrap()
        .devices
        .iter_mut()
        .find(|d| d.device_pubkey == ndr_device(peer_device.public_key()))
        .unwrap();
    let good = record.active_session.clone().unwrap();
    record.active_session.as_mut().unwrap().root_key[0] ^= 1;
    engine.session_manager =
        SessionManager::from_snapshot(snapshot, device.secret_key().to_secret_bytes()).unwrap();
    let plan = sending
        .plan_send(b"old packet unlocked", NdrUnixSeconds(100))
        .unwrap();
    let envelope = sending.apply_send(plan).envelope;
    let event = nostr_double_ratchet::message_event(&envelope).unwrap();
    assert_ne!(event.pubkey, peer.public_key());
    assert_ne!(event.pubkey, peer_device.public_key());
    assert!(event.tags.public_keys().next().is_none());
    engine
        .queue_pending_inbound_direct_event(event, 100, Some(&envelope), None)
        .unwrap();
    (engine, owner, device, peer, peer_device, good)
}

#[test]
fn failed_receive_cache_import_unlocks_inactive_session_after_authorization_change() {
    let (mut engine, _, _, peer, peer_device, good) = failed_receive_import_fixture();
    let event_id = engine.pending_inbound[0].event.id;
    take_inbound_receive_calls();
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(200))
        .unwrap();
    assert_eq!(take_inbound_receive_calls(), 1);
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(201))
        .unwrap();
    let repeated = take_inbound_receive_calls();
    // A revoked owner/device must not be authenticated through a stale cache.
    let revoked = engine
        .ingest_app_keys_event(&signed_app_keys(&peer, &[], 20))
        .unwrap();
    assert!(revoked.direct_messages.is_empty());
    assert!(engine.pending_decrypted_deliveries.is_empty());
    // The newly imported good state has equal priority and remains inactive
    // behind the old wrong-key active state. Receive must still try it.
    engine.session_manager.import_session_state(
        ndr_owner(peer.public_key()),
        ndr_device(peer_device.public_key()),
        good,
        NdrUnixSeconds(202),
    );
    let batch = engine
        .ingest_app_keys_event(&signed_app_keys(&peer, &[peer_device.public_key()], 30))
        .unwrap();
    assert_eq!(batch.direct_messages.len(), 1);
    assert_eq!(batch.direct_messages[0].content, "old packet unlocked");
    assert_eq!(
        batch.direct_messages[0].event_id.as_deref(),
        Some(event_id.to_hex().as_str())
    );
    assert!(engine.pending_inbound.is_empty());
    assert_eq!(repeated, 0);
}

#[test]
fn failed_receive_cache_rollback_reload_and_save_failure_keep_journal_first() {
    use std::sync::atomic::Ordering::Relaxed;
    let (mut engine, owner, device, peer, peer_device, good) = failed_receive_import_fixture();
    let checkpoint = engine.state_checkpoint();
    make_inbound_due(&mut engine);
    take_inbound_receive_calls();
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(200))
        .unwrap();
    assert_eq!(take_inbound_receive_calls(), 1);
    make_inbound_due(&mut engine);
    engine
        .retry_pending_inbound_direct_events(NdrUnixSeconds(201))
        .unwrap();
    let repeated = take_inbound_receive_calls();
    engine.restore_checkpoint(checkpoint);
    engine.persist().unwrap();
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        engine.storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    make_inbound_due(&mut restored);
    take_inbound_receive_calls();
    restored
        .retry_pending_inbound_direct_events(NdrUnixSeconds(202))
        .unwrap();
    assert_eq!(
        take_inbound_receive_calls(),
        1,
        "reload must not restore memoized failures"
    );
    restored.session_manager.import_session_state(
        ndr_owner(peer.public_key()),
        ndr_device(peer_device.public_key()),
        good,
        NdrUnixSeconds(203),
    );
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: restored.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    restored.storage = storage.clone();
    make_inbound_due(&mut restored);
    assert!(restored
        .retry_pending_inbound_direct_events(NdrUnixSeconds(203))
        .is_err());
    assert_eq!(restored.pending_decrypted_deliveries.len(), 1);
    assert_eq!(take_inbound_receive_calls(), 1);
    let ratchets = restored.session_manager.snapshot();
    let delivered = restored
        .retry_pending_inbound_direct_events(NdrUnixSeconds(203))
        .unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].content, "old packet unlocked");
    assert_eq!(
        take_inbound_receive_calls(),
        0,
        "journal replay must precede the failure cache and crypto"
    );
    assert!(restored.session_manager.snapshot() == ratchets);
    assert!(storage.puts.load(Relaxed) > 0);
    assert_eq!(repeated, 0);
}
