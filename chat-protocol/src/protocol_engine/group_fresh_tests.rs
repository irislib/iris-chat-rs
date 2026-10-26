fn fresh_frontier_events(engine: &mut ProtocolEngine, count: usize) -> Vec<Event> {
    let (other, _, device) = queued_group_history(0);
    let author = Keys::generate();
    let mut other_snapshot = other.group_manager.snapshot();
    let record = &mut other_snapshot.sender_keys[0];
    record.sender_event_pubkey = ndr_device(author.public_key());
    // The next live message is behind 2000 retained skipped keys in the old
    // history-first algorithm, and is in the latest of three real key epochs.
    let mut receiver = nostr_double_ratchet::SenderKeyState::new(3, [33; 32], 0);
    let mut sender = receiver.clone();
    let mut last = Vec::new();
    for _ in 0..=2000 {
        last = sender
            .encrypt_to_bytes(b"retained earlier history")
            .unwrap()
            .1;
    }
    receiver.decrypt_from_bytes(2000, &last).unwrap();
    assert_eq!(receiver.skipped_len(), 2000);
    record.latest_key_id = Some(3);
    record.states = vec![
        nostr_double_ratchet::SenderKeyState::new(1, [11; 32], 0),
        nostr_double_ratchet::SenderKeyState::new(2, [22; 32], 0),
        receiver,
    ];
    let mut snapshot = engine.group_manager.snapshot();
    snapshot.groups[0].members.push(record.sender_owner);
    snapshot.sender_keys.extend(other_snapshot.sender_keys);
    engine.group_manager = GroupEventManager::from_snapshot(snapshot).unwrap();
    (0..count)
        .map(|index| {
            let plaintext = JsonGroupPayloadCodecV1
                .encode_sender_key_plaintext(
                    nostr_double_ratchet::GroupPayloadEncodeContext {
                        local_device_pubkey: ndr_device(device.public_key()),
                        created_at: NdrUnixSeconds(20),
                    },
                    &nostr_double_ratchet::GroupSenderKeyPlaintext {
                        group_id: "queued-group".into(),
                        revision: 1,
                        body: format!("fresh-{index}").into_bytes(),
                    },
                )
                .unwrap();
            let (message_number, ciphertext) = sender.encrypt_to_bytes(&plaintext).unwrap();
            nostr_double_ratchet::wire::group_sender_key_message_event(
                &nostr_double_ratchet::GroupSenderKeyMessageEnvelope {
                    group_id: "queued-group".into(),
                    sender_event_pubkey: ndr_device(author.public_key()),
                    signer_secret_key: author.secret_key().to_secret_bytes(),
                    key_id: 3,
                    message_number,
                    encrypted_header: Some("unchanged authenticated message cover".into()),
                    created_at: NdrUnixSeconds(20),
                    ciphertext,
                },
            )
            .unwrap()
        })
        .collect()
}

fn advance_fresh_pacing_clock(engine: &ProtocolEngine) {
    let mut pacing = engine.group_retry_pacing.borrow_mut();
    pacing.clock = Some(
        pacing
            .fresh_not_before
            .unwrap_or_else(|| pacing.now())
            .max(pacing.now()),
    );
}

#[test]
fn fresh_public_receive_skips_old_history_without_waiting_or_losing_its_cursor() {
    for direct_probe_first in [false, true] {
        let mut engine = expensive_unknown_history();
        let events = fresh_frontier_events(&mut engine, 2);
        enable_virtual_history_pacing(&engine);
        engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        let old_fingerprint = engine
            .group_sender_key_retry
            .borrow()
            .active
            .as_ref()
            .unwrap()
            .fingerprint
            .clone();
        let old_trials = engine.group_retry_pacing.borrow().key_trials;
        let old_deadline = engine.group_retry_pacing.borrow().not_before.unwrap();
        let arrived = engine.group_retry_pacing.borrow().now();
        let parsed = parse_group_sender_key_message_event_unchecked(&events[0]).unwrap();
        assert!(parsed.encrypted_header.is_some());
        assert_ne!(
            parsed.message_number, 2001,
            "the wire must not reveal the ratchet position"
        );
        if direct_probe_first {
            engine
                .queue_header_group_sender_key_candidate(&events[0])
                .unwrap();
        }
        let first = engine.process_group_outer_event(&events[0]).unwrap();
        let latency = engine
            .group_retry_pacing
            .borrow()
            .now()
            .duration_since(arrived);
        assert_eq!(first.source_event_count, 1);
        assert_eq!(first.events.len(), 1);
        assert!(
            latency < std::time::Duration::from_secs(1),
            "fresh message waited {latency:?}"
        );
        assert_eq!(engine.group_retry_pacing.borrow().fresh_key_trials, 1);
        assert_eq!(engine.group_retry_pacing.borrow().key_trials, old_trials);
        assert!(engine.group_retry_pacing.borrow().not_before.unwrap() >= old_deadline);
        assert_eq!(
            engine
                .group_sender_key_retry
                .borrow()
                .active
                .as_ref()
                .unwrap()
                .fingerprint,
            old_fingerprint
        );
        // The second live message arrives before the reserved allowance refills.
        // Duplicates must not bypass it, re-admit a spent marker, or redo history.
        assert!(
            engine
                .process_group_outer_event(&events[1])
                .unwrap()
                .pending
        );
        for _ in 0..16 {
            assert!(engine
                .process_group_outer_event(&events[1])
                .unwrap()
                .events
                .is_empty());
            engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        }
        assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 1);
        assert_eq!(engine.group_sender_key_retry.borrow().fresh.len(), 1);
        let second_arrived = engine.group_retry_pacing.borrow().now();
        advance_fresh_pacing_clock(&engine);
        let second = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        let second_latency = engine
            .group_retry_pacing
            .borrow()
            .now()
            .duration_since(second_arrived);
        assert_eq!(second.group_result.events.len(), 1);
        assert!(
            second_latency < std::time::Duration::from_secs(1),
            "second message waited {second_latency:?}"
        );
        assert_eq!(engine.group_retry_pacing.borrow().fresh_key_trials, 2);
        assert_eq!(engine.group_retry_pacing.borrow().key_trials, old_trials);
        assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
        assert!(engine.group_sender_key_retry.borrow().fresh.is_empty());
        // Once the combined debt is paid the SAME older search advances again.
        advance_history_pacing_clock(&engine);
        engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        assert!(engine.group_retry_pacing.borrow().key_trials > old_trials);
        eprintln!("fresh_frontier_latency direct_probe_first={direct_probe_first} first_ms={} second_ms={} key_trials=2 skipped_keys=2000 epochs=3", latency.as_secs_f64()*1000.0, second_latency.as_secs_f64()*1000.0);
    }
}

#[test]
fn fresh_queue_is_capped_and_misses_and_duplicates_share_one_work_allowance() {
    let mut engine = expensive_unknown_history();
    let original = engine.pending_group_sender_key_messages[0].clone();
    enable_virtual_history_pacing(&engine);
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    let initial_trials = engine.group_retry_pacing.borrow().key_trials;
    let mut arrivals = Vec::new();
    for index in 0..40 {
        let mut arrival = original.clone();
        arrival.created_at = NdrUnixSeconds(40 + index);
        engine
            .queue_pending_group_sender_key_message(arrival.clone())
            .unwrap();
        arrivals.push(arrival);
    }
    assert_eq!(
        engine.group_sender_key_retry.borrow().fresh.len(),
        GROUP_FRESH_QUEUE_LIMIT
    );
    assert_eq!(engine.pending_group_sender_key_messages.len(), 41);
    for _ in 0..16 {
        for arrival in &arrivals {
            engine
                .queue_pending_group_sender_key_message(arrival.clone())
                .unwrap();
        }
        engine.retry_pending_protocol(NdrUnixSeconds(50)).unwrap();
    }
    assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 1);
    assert_eq!(engine.group_retry_pacing.borrow().fresh_key_trials, 3);
    assert_eq!(
        engine.group_retry_pacing.borrow().key_trials,
        initial_trials
    );
    assert_eq!(
        engine.group_sender_key_retry.borrow().fresh.len(),
        GROUP_FRESH_QUEUE_LIMIT - 1
    );
    assert_eq!(
        engine.pending_group_sender_key_messages.len(),
        41,
        "overflow and misses stay durable"
    );
    advance_fresh_pacing_clock(&engine);
    engine.retry_pending_protocol(NdrUnixSeconds(50)).unwrap();
    assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 2);
    assert_eq!(engine.group_retry_pacing.borrow().fresh_key_trials, 6);
    assert_eq!(
        engine.group_retry_pacing.borrow().key_trials,
        initial_trials
    );
}

#[test]
fn fresh_receive_failed_save_preserves_old_cursor_and_charges_work() {
    let mut engine = expensive_unknown_history();
    let event = fresh_frontier_events(&mut engine, 1).remove(0);
    enable_virtual_history_pacing(&engine);
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    engine
        .queue_header_group_sender_key_candidate(&event)
        .unwrap();
    let old_fingerprint = engine
        .group_sender_key_retry
        .borrow()
        .active
        .as_ref()
        .unwrap()
        .fingerprint
        .clone();
    let original = engine.group_manager.snapshot();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    engine.storage = storage;
    assert!(engine.retry_pending_protocol(NdrUnixSeconds(30)).is_err());
    assert_eq!(engine.group_manager.snapshot(), original);
    assert_eq!(
        engine
            .group_sender_key_retry
            .borrow()
            .active
            .as_ref()
            .unwrap()
            .fingerprint,
        old_fingerprint
    );
    assert_eq!(engine.group_sender_key_retry.borrow().fresh.len(), 1);
    assert_eq!(engine.pending_group_sender_key_messages.len(), 2);
    assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 1);
    assert!(!engine.group_retry_pacing.borrow().fresh_delay().is_zero());
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 1);
    advance_fresh_pacing_clock(&engine);
    assert_eq!(
        engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len(),
        1
    );
    assert_eq!(engine.group_retry_pacing.borrow().fresh_work_calls, 2);
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
}

#[test]
fn fresh_work_extends_global_debt_instead_of_adding_an_independent_cpu_budget() {
    let start = std::time::Instant::now();
    let mut pacing = GroupRetryPacing {
        enabled: true,
        clock: Some(start),
        ..Default::default()
    };
    pacing.record_work(std::time::Duration::from_millis(100));
    let initial_deadline = pacing.not_before.unwrap();
    for _ in 0..100 {
        pacing.clock = Some(
            pacing
                .fresh_not_before
                .unwrap_or_else(|| pacing.now())
                .max(pacing.now()),
        );
        pacing.record_fresh_work(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        pacing.not_before.unwrap().duration_since(initial_deadline),
        std::time::Duration::from_millis(4900)
    );
    assert!(
        !pacing.delay().is_zero(),
        "sustained fresh traffic spends the shared history allowance"
    );
}

#[test]
fn fresh_allowance_is_not_readmitted_by_restored_ciphertext_replays() {
    let mut engine = expensive_unknown_history();
    let event = fresh_frontier_events(&mut engine, 1).remove(0);
    enable_virtual_history_pacing(&engine);
    engine
        .queue_header_group_sender_key_candidate(&event)
        .unwrap();
    assert_eq!(engine.group_sender_key_retry.borrow().fresh.len(), 1);
    let keys = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        engine.storage.clone(),
        engine.owner_pubkey,
        &keys,
    )
    .unwrap();
    enable_virtual_history_pacing(&restored);
    assert!(restored.group_sender_key_retry.borrow().fresh.is_empty());
    restored.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    let trials = restored.group_retry_pacing.borrow().key_trials;
    for _ in 0..16 {
        restored
            .queue_header_group_sender_key_candidate(&event)
            .unwrap();
        assert!(restored
            .process_group_outer_event(&event)
            .unwrap()
            .events
            .is_empty());
    }
    assert!(restored.group_sender_key_retry.borrow().fresh.is_empty());
    assert_eq!(restored.group_retry_pacing.borrow().fresh_work_calls, 0);
    assert_eq!(restored.group_retry_pacing.borrow().key_trials, trials);
    assert_eq!(restored.pending_group_sender_key_messages.len(), 2);
}
