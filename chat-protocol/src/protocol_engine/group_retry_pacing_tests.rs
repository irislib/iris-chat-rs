fn enable_virtual_history_pacing(engine: &ProtocolEngine) {
    engine.enable_background_history_recovery_pacing();
    engine.group_retry_pacing.borrow_mut().clock = Some(std::time::Instant::now());
}

fn advance_history_pacing_clock(engine: &ProtocolEngine) {
    let mut pacing = engine.group_retry_pacing.borrow_mut();
    pacing.clock = Some(pacing.not_before.unwrap_or_else(|| pacing.now()));
}

fn expensive_unknown_history() -> ProtocolEngine {
    let (mut engine, _, _) = queued_group_history(1);
    let mut snapshot = engine.group_manager.snapshot();
    for id in 2..=3 {
        snapshot.sender_keys[0]
            .states
            .push(nostr_double_ratchet::SenderKeyState::new(
                id,
                [id as u8; 32],
                0,
            ));
    }
    engine.group_manager = GroupEventManager::from_snapshot(snapshot).unwrap();
    engine.pending_group_sender_key_messages[0].ciphertext =
        nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 0)
            .encrypt_to_bytes(&[42; 4096])
            .unwrap()
            .1;
    engine
}

#[test]
fn history_pacing_stops_other_events_from_bypassing_the_work_allowance() {
    let mut manual = expensive_unknown_history();
    let mut paced = expensive_unknown_history();
    enable_virtual_history_pacing(&paced);
    let now = NdrUnixSeconds(30);
    for engine in [&mut manual, &mut paced] {
        engine.retry_pending_protocol(now).unwrap();
    }
    let first_trials = paced.group_retry_pacing.borrow().key_trials;
    assert!(first_trials > 0);
    assert!(paced.group_sender_key_retry_delay() > std::time::Duration::ZERO);
    let message = paced
        .group_sender_key_message_from_parsed(&paced.pending_group_sender_key_messages[0])
        .unwrap();
    // Both timer turns and unrelated event-driven receives reach the real
    // protocol methods. A per-timer sleep alone would fail this regression.
    for _ in 0..16 {
        for engine in [&mut manual, &mut paced] {
            engine.enter_batch();
            engine.retry_pending_protocol(now).unwrap();
            engine.exit_batch().unwrap();
        }
        assert!(
            paced
                .handle_group_sender_key_message(message.clone())
                .unwrap()
                .pending
        );
    }
    assert_eq!(paced.group_retry_pacing.borrow().key_trials, first_trials);
    assert_eq!(paced.group_retry_pacing.borrow().work_calls, 1);
    assert!(manual.group_retry_pacing.borrow().key_trials >= first_trials * 8);
    assert!(paced.has_ready_group_sender_key_retry_work());
    advance_history_pacing_clock(&paced);
    paced.retry_pending_protocol(now).unwrap();
    assert!(paced.group_retry_pacing.borrow().key_trials > first_trials);
    assert_eq!(paced.pending_group_sender_key_messages.len(), 1);
}

#[test]
fn paced_history_delivers_late_other_authors_without_repeating_exhausted_work() {
    let mut engine = expensive_unknown_history();
    enable_virtual_history_pacing(&engine);
    let unresolved = engine.pending_group_sender_key_messages[0].clone();
    let later = [
        add_other_group_stream(&mut engine),
        add_other_group_stream(&mut engine),
    ];
    let now = NdrUnixSeconds(30);
    engine.retry_pending_protocol(now).unwrap();
    assert!(engine.group_sender_key_retry.borrow().active.is_some());
    // Arrive after the expensive three-epoch search has started. These are
    // distinct authenticated sender records, not copies of one author's queue.
    for other in later {
        engine
            .queue_pending_group_sender_key_message(other)
            .unwrap();
    }
    let mut delivered = Vec::new();
    let mut turns = 0;
    for _ in 0..256 {
        advance_history_pacing_clock(&engine);
        engine.enter_batch();
        let result = engine.retry_pending_protocol(now).unwrap();
        engine.exit_batch().unwrap();
        delivered.extend(result.group_result.events);
        turns += 1;
        if !engine.has_ready_group_sender_key_retry_work() {
            break;
        }
    }
    assert!(
        turns < 256,
        "frozen work must finish and release other authors"
    );
    assert_eq!(delivered.len(), 2);
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
    assert_eq!(engine.pending_group_sender_key_messages[0], unresolved);
    assert!(!engine.has_ready_group_sender_key_retry_work());
    let trials = engine.group_retry_pacing.borrow().key_trials;
    for _ in 0..32 {
        advance_history_pacing_clock(&engine);
        engine.retry_pending_protocol(now).unwrap();
    }
    assert_eq!(engine.group_retry_pacing.borrow().key_trials, trials);
    assert_eq!(engine.pending_decrypted_deliveries.len(), 2);
}

#[test]
fn history_pacing_debt_survives_protocol_rollback() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 700);
    enable_virtual_history_pacing(&engine);
    let checkpoint = engine.state_checkpoint();
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    let delay = engine.group_sender_key_retry_delay();
    assert!(!delay.is_zero());
    // Resource accounting is deliberately outside transactional protocol state.
    engine.restore_checkpoint(checkpoint);
    assert_eq!(engine.group_sender_key_retry_delay(), delay);
}

#[test]
fn history_pacing_applies_a_prepared_result_without_waiting_or_more_key_trials() {
    let (mut engine, _, device) = queued_group_history(1);
    long_group_candidate(&mut engine, &device, 700);
    enable_virtual_history_pacing(&engine);
    let now = NdrUnixSeconds(30);
    for _ in 0..2 {
        advance_history_pacing_clock(&engine);
        assert!(engine
            .retry_pending_protocol(now)
            .unwrap()
            .group_result
            .events
            .is_empty());
    }
    advance_history_pacing_clock(&engine);
    engine.group_sender_key_retry.borrow_mut().reset_budget();
    assert!(!engine.advance_group_sender_key_continuation().unwrap());
    assert!(engine.group_sender_key_retry.borrow().prepared.is_some());
    assert!(!engine.group_retry_pacing.borrow().delay().is_zero());
    assert!(engine.group_sender_key_retry_delay().is_zero());
    let trials = engine.group_retry_pacing.borrow().key_trials;
    assert_eq!(
        engine
            .retry_pending_protocol(now)
            .unwrap()
            .group_result
            .events
            .len(),
        1
    );
    assert_eq!(engine.group_retry_pacing.borrow().key_trials, trials);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn history_pacing_keeps_known_position_receive_immediate() {
    let mut engine = expensive_unknown_history();
    let mut other = add_other_group_stream(&mut engine);
    other.encrypted_header = None;
    enable_virtual_history_pacing(&engine);
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert!(!engine.group_sender_key_retry_delay().is_zero());
    let trials = engine.group_retry_pacing.borrow().key_trials;
    let message = engine.group_sender_key_message_from_parsed(&other).unwrap();
    let received = engine.handle_group_sender_key_message(message).unwrap();
    assert_eq!(received.events.len(), 1);
    assert_eq!(engine.group_retry_pacing.borrow().key_trials, trials);
    assert!(!engine.group_sender_key_retry_delay().is_zero());
}
