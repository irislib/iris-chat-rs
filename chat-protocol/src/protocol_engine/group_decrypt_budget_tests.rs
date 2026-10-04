fn drain_group_retry(engine: &mut ProtocolEngine, now: NdrUnixSeconds) -> Vec<GroupIncomingEvent> {
    let mut events = Vec::new();
    for _ in 0..512 {
        if !engine.has_ready_group_sender_key_retry_work() {
            return events;
        }
        events.extend(
            engine
                .retry_pending_protocol(now)
                .unwrap()
                .group_result
                .events,
        );
    }
    panic!("fixed candidate set must finish without spinning");
}

fn long_group_candidate(engine: &mut ProtocolEngine, sender_device: &Keys, number: u32) {
    long_group_candidate_at_revision(engine, sender_device, number, 1);
}

#[test]
fn group_receive_checkpoints_only_ready_mutations_not_replays_or_search_slices() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 700);
    let parsed = engine.pending_group_sender_key_messages[0].clone();
    let message = engine.group_sender_key_message_from_parsed(&parsed).unwrap();
    assert!(engine.handle_group_sender_key_message(message.clone()).unwrap().pending);
    assert_eq!(engine.group_sender_key_retry.borrow().receive_checkpoints, 0,
        "Pure bounded search must not clone the complete receive state");
    engine.group_sender_key_retry.borrow_mut().reset_budget();
    assert!(engine.handle_group_sender_key_message(message.clone()).unwrap().pending);
    assert_eq!(engine.group_sender_key_retry.borrow().receive_checkpoints, 0);
    engine.group_sender_key_retry.borrow_mut().reset_budget();
    assert_eq!(engine.handle_group_sender_key_message(message.clone()).unwrap().events.len(), 1);
    assert_eq!(engine.group_sender_key_retry.borrow().receive_checkpoints, 1);
    for _ in 0..32 {
        assert!(engine.handle_group_sender_key_message(message.clone()).unwrap().events.is_empty());
    }
    assert_eq!(engine.group_sender_key_retry.borrow().receive_checkpoints, 1,
        "Already processed events must remain borrowed fast paths");
}

fn long_group_candidate_at_revision(
    engine: &mut ProtocolEngine,
    sender_device: &Keys,
    number: u32,
    revision: u64,
) {
    let plaintext = JsonGroupPayloadCodecV1
        .encode_sender_key_plaintext(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(20),
            },
            &nostr_double_ratchet::GroupSenderKeyPlaintext {
                group_id: "queued-group".into(),
                revision,
                body: b"last recoverable position".to_vec(),
            },
        )
        .unwrap();
    let mut sender = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
    for _ in 0..=number {
        engine.pending_group_sender_key_messages[0].ciphertext =
            sender.encrypt_to_bytes(&plaintext).unwrap().1;
    }
}

fn add_other_group_stream(
    engine: &mut ProtocolEngine,
) -> nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
    let (other, _, _) = queued_group_history(1);
    let mut state = engine.group_manager.snapshot();
    let other_state = other.group_manager.snapshot();
    state.groups[0]
        .members
        .push(other_state.sender_keys[0].sender_owner);
    state.sender_keys.extend(other_state.sender_keys);
    engine.group_manager = GroupEventManager::from_snapshot(state).unwrap();
    other.pending_group_sender_key_messages[0].clone()
}

#[test]
fn group_retry_removed_active_candidate_does_not_block_outer_batch() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 700);
    let removed = engine.pending_group_sender_key_messages[0].clone();
    let other = add_other_group_stream(&mut engine);
    engine.enter_batch();
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .is_empty());
    engine.exit_batch().unwrap();
    assert!(engine.group_sender_key_retry.borrow().active.is_some());
    assert!(engine.clear_pending_group_sender_key_candidate(&removed));
    engine
        .queue_pending_group_sender_key_message(other)
        .unwrap();
    engine.enter_batch();
    assert_eq!(
        engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len(),
        1
    );
    engine.exit_batch().unwrap();
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_outer_batch_checks_live_membership_between_slices() {
    let (mut engine, sender, device) = queued_group_history(1);
    long_group_candidate_at_revision(&mut engine, &device, 700, 2);
    engine.enter_batch();
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .is_empty());
    engine.exit_batch().unwrap();
    let mut group = engine.group_manager.group("queued-group").unwrap();
    group.revision += 1;
    group.members.retain(|owner| *owner != engine.local_owner);
    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(device.public_key()),
                created_at: NdrUnixSeconds(31),
            },
            &GroupPairwiseCommand::MetadataSnapshot { snapshot: group },
        )
        .unwrap();
    // Apply authenticated current membership directly to expose any stale scheduling cache.
    engine
        .group_manager
        .handle_pairwise_payload(
            ndr_owner(sender.public_key()),
            ndr_device(device.public_key()),
            &payload,
        )
        .unwrap();
    let after_revocation = engine.group_manager.snapshot();
    engine.enter_batch();
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(32))
        .unwrap()
        .group_result
        .events
        .is_empty());
    engine.exit_batch().unwrap();
    assert_eq!(engine.group_manager.snapshot(), after_revocation);
    assert!(engine.group_sender_key_retry.borrow().active.is_none());
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_new_arrival_preserves_active_progress_and_eventually_delivers_both() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 700);
    let other = add_other_group_stream(&mut engine);
    let active = engine.pending_group_sender_key_messages[0].clone();
    let message = engine
        .group_sender_key_message_from_parsed(&active)
        .unwrap();
    engine.enter_batch();
    assert!(
        engine
            .handle_group_sender_key_message(message)
            .unwrap()
            .pending
    );
    let first_cursor = engine
        .group_sender_key_retry
        .borrow()
        .active
        .as_ref()
        .unwrap()
        .fingerprint
        .clone();
    let other_message = engine.group_sender_key_message_from_parsed(&other).unwrap();
    assert!(
        engine
            .handle_group_sender_key_message(other_message)
            .unwrap()
            .pending
    );
    assert_eq!(
        engine
            .group_sender_key_retry
            .borrow()
            .active
            .as_ref()
            .unwrap()
            .fingerprint,
        first_cursor
    );
    engine
        .queue_pending_group_sender_key_message(other)
        .unwrap();
    engine.exit_batch().unwrap();
    let events = drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert_eq!(events.len(), 2);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_continuations_do_not_checkpoint_and_failed_final_save_can_replan() {
    let (mut engine, _, device) = queued_group_history(1);
    let mut early = engine
        .group_sender_key_message_from_parsed(&engine.pending_group_sender_key_messages[0])
        .unwrap();
    early.encrypted_header = None;
    long_group_candidate(&mut engine, &device, 700);
    let initial = engine.group_manager.snapshot();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    engine.storage = storage.clone();
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .is_empty());
    let writes = storage.puts.load(std::sync::atomic::Ordering::Relaxed);
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .is_empty());
    assert_eq!(
        storage.puts.load(std::sync::atomic::Ordering::Relaxed),
        writes
    );
    assert_eq!(engine.group_manager.snapshot(), initial);
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(engine.retry_pending_protocol(NdrUnixSeconds(30)).is_err());
    assert_eq!(engine.group_manager.snapshot(), initial);
    assert!(engine.group_sender_key_retry.borrow().prepared.is_none());
    assert!(matches!(
        engine
            .group_manager
            .handle_sender_key_message(early)
            .unwrap(),
        GroupSenderKeyHandleResult::Event(_)
    ));
    assert_eq!(drain_group_retry(&mut engine, NdrUnixSeconds(30)).len(), 1);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_one_blind_candidate_yields_before_scanning_the_full_window() {
    let (mut engine, _, sender_device) = queued_group_history(1);
    let plaintext = JsonGroupPayloadCodecV1
        .encode_sender_key_plaintext(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(20),
            },
            &nostr_double_ratchet::GroupSenderKeyPlaintext {
                group_id: "queued-group".into(),
                revision: 1,
                body: b"last recoverable position".to_vec(),
            },
        )
        .unwrap();
    let mut sender = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
    for _ in 0..=nostr_double_ratchet::SENDER_KEY_MAX_SKIP {
        engine.pending_group_sender_key_messages[0].ciphertext =
            sender.encrypt_to_bytes(&plaintext).unwrap().1;
    }
    let started = std::time::Instant::now();
    let first = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    eprintln!(
        "one hidden-position candidate at end of search window: {:.3} ms; foreground actor work cannot run until this call returns",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert!(
        first.group_result.events.is_empty(),
        "one retry turn must yield inside a long blind search, retaining continuation"
    );
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
    assert!(engine.has_ready_group_sender_key_retry_work());
    let mut delivered = 0;
    let mut max_turn = started.elapsed();
    let mut turns = 1;
    for turn in 2..=129 {
        let turn_started = std::time::Instant::now();
        delivered += engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len();
        max_turn = max_turn.max(turn_started.elapsed());
        turns = turn;
        if engine.pending_group_sender_key_messages.is_empty() {
            break;
        }
    }
    eprintln!(
        "full Chat key-10,000 recovery: turns={turns} total_ms={:.3} max_actor_turn_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0,
        max_turn.as_secs_f64() * 1000.0
    );
    assert_eq!(
        delivered, 1,
        "the full original recovery window is retained"
    );
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_cached_continuation_touches_only_its_candidate_in_a_large_backlog() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 2_000);
    let candidate = engine.pending_group_sender_key_messages[0].clone();
    // Unmapped synthetic entries represent 16 MiB of unrelated durable traffic.
    let mut unrelated = candidate.clone();
    unrelated.sender_event_pubkey = ndr_device(Keys::generate().public_key());
    unrelated.ciphertext = vec![99; 128 * 1024];
    for _ in 0..128 {
        engine
            .pending_group_sender_key_messages
            .insert(0, unrelated.clone());
    }
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(
        engine
            .group_sender_key_retry
            .borrow()
            .active
            .as_ref()
            .unwrap()
            .pending_index,
        128
    );
    let before_writes = engine.group_manager.snapshot();
    let started = std::time::Instant::now();
    for _ in 0..3 {
        assert!(engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .is_empty());
        assert_eq!(
            engine
                .group_sender_key_retry
                .borrow()
                .active
                .as_ref()
                .unwrap()
                .pending_index,
            128
        );
    }
    eprintln!(
        "three continuation slices with 16 MiB unrelated backlog: {:.3} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(engine.group_manager.snapshot(), before_writes);
    // A queue mutation invalidates the index; the next slice repairs it safely.
    engine.pending_group_sender_key_messages.remove(0);
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(
        engine
            .group_sender_key_retry
            .borrow()
            .active
            .as_ref()
            .unwrap()
            .pending_index,
        127
    );
    assert_eq!(drain_group_retry(&mut engine, NdrUnixSeconds(30)).len(), 1);
}

#[test]
fn group_retry_completion_does_not_reset_the_shared_trial_budget() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 510);
    let other = add_other_group_stream(&mut engine);
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .is_empty());
    engine
        .queue_pending_group_sender_key_message(other)
        .unwrap();
    let delivered = engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .group_result
        .events
        .len();
    assert_eq!(delivered, 2);
    assert_eq!(
        engine.group_sender_key_retry.borrow().key_trials_remaining,
        0,
        "255 remaining trials plus one other message consumes the same turn's 256 budget"
    );
}
