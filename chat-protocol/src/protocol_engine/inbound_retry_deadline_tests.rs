thread_local! {
    static INBOUND_RETRY_DEADLINE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_inbound_retry_deadline_scan() {
    INBOUND_RETRY_DEADLINE_SCANS.with(|count| count.set(count.get() + 1));
}

pub(super) fn take_inbound_retry_deadline_scans() -> usize {
    INBOUND_RETRY_DEADLINE_SCANS.with(|count| count.replace(0))
}

fn pending_retry_owner(pending: &ProtocolPendingInbound) -> NdrOwnerPubkey {
    ndr_owner(PublicKey::from_hex(pending.resolved_owner_pubkey_hex.as_ref().unwrap()).unwrap())
}

#[test]
fn deferred_inbound_deadline_scans_are_bounded_across_32_continuations() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let now = NdrUnixSeconds(20_000_000);
    engine.pending_inbound = synthetic_pending_inbound(&mut engine, 4_214, 102, now.get());
    let mut expected = engine.pending_inbound.clone();
    for pending in &mut expected[..102] {
        pending.next_retry_at_secs = now.get() + 60;
    }
    let sessions = serde_json::to_value(engine.session_manager.snapshot()).unwrap();
    take_inbound_retry_deadline_scans();
    for turn in 0..32 {
        assert_eq!(engine.has_due_pending_retry_work(now), turn == 0);
        // Group continuations call retry even while all direct packets are
        // deferred. Both the scheduler query and the retry must avoid rescans.
        assert!(engine.retry_pending_protocol(now).unwrap().is_empty());
    }
    let scans = take_inbound_retry_deadline_scans();
    assert!(
        (4_214..=3 * 4_214).contains(&scans),
        "32 continuations inspected {scans} inbound retry deadlines"
    );
    assert!(engine.pending_inbound == expected);
    assert!(engine.pending_decrypted_deliveries.is_empty());
    assert!(serde_json::to_value(engine.session_manager.snapshot()).unwrap() == sessions);
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(now.get() + 51)));
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(now.get() + 52)));
}

#[test]
fn inbound_deadline_query_notices_earlier_enqueues_after_empty_or_deferred_queue() {
    for initial_count in [0, 2] {
        let mut engine = test_engine(&Keys::generate(), &Keys::generate());
        let now = NdrUnixSeconds(1_000);
        let mut packets = synthetic_pending_inbound(&mut engine, initial_count + 1, 0, now.get());
        let earlier = packets.pop().unwrap();
        engine.pending_inbound = packets;
        let untouched = engine.pending_inbound.clone();
        assert!(!engine.has_due_pending_retry_work(now));
        engine
            .queue_pending_inbound_direct_event(
                earlier.event.clone(),
                now.get(),
                earlier.envelope.as_ref(),
                None,
            )
            .unwrap();
        assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(1_001)));
        assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(1_002)));
        assert!(engine
            .retry_pending_inbound_direct_events(NdrUnixSeconds(1_002))
            .unwrap()
            .is_empty());
        assert!(engine.pending_inbound[..initial_count] == untouched);
        let retried = &engine.pending_inbound[initial_count];
        assert_eq!(retried.event.id, earlier.event.id);
        assert_eq!(retried.created_at_secs, 1_000);
        assert_eq!(retried.next_retry_at_secs, 1_004);
        assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(1_003)));
        assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(1_004)));
    }
}

#[test]
fn inbound_deadline_owner_wake_retries_only_matching_owner() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let now = NdrUnixSeconds(1_000);
    let first = synthetic_pending_inbound(&mut engine, 2, 0, now.get());
    let second = synthetic_pending_inbound(&mut engine, 2, 0, now.get());
    let owner = pending_retry_owner(&first[0]);
    assert_ne!(owner, pending_retry_owner(&second[0]));
    engine.pending_inbound = first;
    engine.pending_inbound.extend(second.clone());
    let mut expected = engine.pending_inbound.clone();
    assert!(!engine.has_due_pending_retry_work(now));
    engine.wake_pending_protocol_for_owner(owner);
    assert!(engine.has_due_pending_retry_work(now));
    for pending in &mut expected[..2] {
        pending.next_retry_at_secs = 0;
    }
    assert!(engine.pending_inbound == expected);
    assert!(engine
        .retry_pending_inbound_direct_events(now)
        .unwrap()
        .is_empty());
    for pending in &mut expected[..2] {
        pending.next_retry_at_secs = 1_060;
    }
    assert!(engine.pending_inbound == expected);
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(1_051)));
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(1_052)));
}

#[test]
fn inbound_deadline_checkpoint_rollback_and_reload_rederive_cutoff() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let now = NdrUnixSeconds(1_000);
    engine.pending_inbound = synthetic_pending_inbound(&mut engine, 2, 0, now.get());
    let expected = engine.pending_inbound.clone();
    let peer = pending_retry_owner(&expected[0]);
    engine.persist().unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    engine.storage = storage.clone();
    assert!(!engine.has_due_pending_retry_work(now));
    assert!(engine
        .with_state_checkpoint(|engine| {
            engine.wake_pending_protocol_for_owner(peer);
            assert!(engine.has_due_pending_retry_work(now));
            engine.persist()
        })
        .is_err());
    assert!(engine.pending_inbound == expected);
    take_inbound_retry_deadline_scans();
    assert!(!engine.has_due_pending_retry_work(now));
    assert!(
        take_inbound_retry_deadline_scans() > 0,
        "rollback must rederive the cutoff"
    );
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
            .unwrap();
    assert!(restored.pending_inbound == expected);
    take_inbound_retry_deadline_scans();
    assert!(!restored.has_due_pending_retry_work(now));
    assert!(
        take_inbound_retry_deadline_scans() > 0,
        "reload must start with an unknown cutoff"
    );
    assert!(restored.has_due_pending_retry_work(NdrUnixSeconds(1_052)));
}

#[test]
fn inbound_deadline_failed_enqueue_and_invite_save_rederive_cutoff() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let now = NdrUnixSeconds(1_000);
    let mut packets = synthetic_pending_inbound(&mut engine, 3, 0, now.get());
    let earlier = packets.pop().unwrap();
    engine.pending_inbound = packets;
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    engine.storage = storage.clone();
    assert!(!engine.has_due_pending_retry_work(now));
    assert!(engine
        .queue_pending_inbound_direct_event(
            earlier.event.clone(),
            now.get(),
            earlier.envelope.as_ref(),
            None,
        )
        .is_err());
    // Enqueue retains the packet even if its save fails. It must remain due.
    assert_eq!(engine.pending_inbound.len(), 3);
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(1_002)));
    let before = engine.pending_inbound.clone();
    let invite_keys = Keys::generate();
    let invite_engine = test_engine(&invite_keys, &invite_keys);
    let invite = invite_unsigned_event(&invite_engine.local_invite().unwrap())
        .unwrap()
        .sign_with_keys(&invite_keys)
        .unwrap();
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(engine.observe_invite_event(&invite).is_err());
    assert!(engine.pending_inbound == before);
    take_inbound_retry_deadline_scans();
    assert!(!engine.has_due_pending_retry_work(now));
    assert!(
        take_inbound_retry_deadline_scans() > 0,
        "invite rollback must rederive the cutoff"
    );
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(1_002)));
}
