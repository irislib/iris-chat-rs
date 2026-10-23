include!("batched_retry_durable_work.rs");

fn batched_retry_progress(
    engine: &ProtocolEngine,
    audit: &RetrySearchAuditGuard,
) -> serde_json::Value {
    let retry = engine.group_sender_key_retry.borrow();
    let (starts, repeated, changes) = audit.counts();
    serde_json::json!({
        "search_starts": starts, "repeated_search_starts": repeated,
        "input_generation_changes": changes, "ready_candidates": retry.ready.len(),
        "active_cursor": retry.active.is_some(), "prepared_result": retry.prepared.is_some(),
        "engine_attempts": retry.total_attempts,
    })
}

#[test]
#[ignore = "300-second hosted batched CPU qualification with a synthetic 350 MB checkpoint; run optimized and alone"]
fn restored_backlog_batched_retry_stays_within_cpu_budget() {
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::{Duration, Instant};

    assert!(
        !cfg!(debug_assertions),
        "CPU qualification requires the optimized profile"
    );
    const INBOUND: usize = 4_214;
    const FANOUTS: usize = 24_990;
    const CIPHERTEXTS: usize = 23_437;
    const CHECKPOINT_BYTES: usize = 350_224_384;
    let (f, _directory, storage, now_secs, clock_started) =
        prepared_retry_backlog_fixture(INBOUND, FANOUTS, CIPHERTEXTS, CHECKPOINT_BYTES);
    assert_eq!(f.sender.pending_decrypted_deliveries.len(), 1);
    let setup_delivery: UnsignedEvent =
        serde_json::from_str(&f.sender.pending_decrypted_deliveries[0].content).unwrap();
    assert_eq!(setup_delivery.pubkey, f.peer_owner.public_key());
    assert_eq!(setup_delivery.content, "establish receiver");
    let setup_journal = f.sender.pending_decrypted_deliveries[0].clone();
    let setup_id = setup_journal.event_id.clone().unwrap();
    // Expected payloads are copied before timing, never once per retry turn.
    let inbound_before = f.sender.pending_inbound.clone();
    let fanouts_before = f.sender.pending_group_fanouts.clone();
    let ciphertexts_before = f.sender.pending_group_sender_key_messages.clone();
    let group_message = f
        .sender
        .group_sender_key_message_from_parsed(&ciphertexts_before[0])
        .unwrap();
    let group_id = format!(
        "group-delivery:{}",
        group_sender_key_fingerprint(&group_message)
    );
    let group_sender = f
        .sender
        .group_manager
        .snapshot()
        .sender_keys
        .into_iter()
        .find(|record| record.sender_event_pubkey == group_message.sender_event_pubkey)
        .unwrap();
    let remote_before = f.sender.pending_remote_sends.clone();
    let sessions_before = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let ticks_per_second: f64 = String::from_utf8(
        std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap();
    drop(f.sender);

    // Keep one audit epoch across restore and all five slices. Initial input
    // admission is observed too; only the intentional final reload resets it.
    let audit = RetrySearchAuditGuard::start();
    let mut cpu_before = retry_process_cpu_ticks();
    let mut puts_before = storage.puts.load(Relaxed);
    let mut io_before = crate::storage::checkpoint_io_bytes();
    let recovery_started = Instant::now();
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    let restore_seconds = recovery_started.elapsed().as_secs_f64();
    assert_eq!(engine.checkpoint_layout.borrow().bytes, CHECKPOINT_BYTES);
    assert_eq!(engine.pending_inbound.len(), INBOUND);
    assert_eq!(engine.pending_group_fanouts.len(), FANOUTS);
    assert_eq!(engine.pending_group_sender_key_messages.len(), CIPHERTEXTS);
    let control_now = now_secs + clock_started.elapsed().as_secs();
    let recovery_epoch = control_now - recovery_started.elapsed().as_secs();
    let mut expiring_controls =
        BatchedExpiringControls::seed(&mut engine, recovery_epoch, control_now);
    assert_eq!(engine.pending_remote_sends.len(), 46);
    assert_eq!(engine.checkpoint_layout.borrow().bytes, CHECKPOINT_BYTES);
    let initially_ready = engine.has_ready_group_sender_key_retry_work();
    let readiness_seconds = recovery_started.elapsed().as_secs_f64();
    let mut first_valid_delivery_seconds = None;
    let mut group_deliveries = 0usize;
    let mut acknowledged_deliveries = 0usize;
    let mut acknowledged_ids = HashSet::new();
    let mut acknowledged_setup_deliveries = 0usize;
    let mut acknowledged_group_deliveries = 0usize;
    let mut remote_deadlines = engine
        .pending_remote_sends
        .iter()
        .take(remote_before.len())
        .map(|pending| pending.next_retry_at_secs)
        .collect::<Vec<_>>();
    let mut slices = Vec::new();
    let mut phase_started = recovery_started;
    for index in 0..5 {
        let progress_before = batched_retry_progress(&engine, &audit);
        let now = now_secs + clock_started.elapsed().as_secs();
        let due_at_start = serde_json::json!({
            "inbound": engine.pending_inbound.iter().filter(|p| p.next_retry_at_secs <= now).count(),
            "fanouts": engine.pending_group_fanouts.iter().filter(|p| p.next_retry_at_secs <= now).count(),
            "remote_sends": engine.pending_remote_sends.iter().filter(|p| p.next_retry_at_secs <= now).count(),
        });
        let mut turns = 0usize;
        let mut continuations = 0usize;
        let mut work = if index == 0 {
            phase_started.elapsed()
        } else {
            Duration::ZERO
        };
        let mut max_turn = Duration::ZERO;
        let ack_before = acknowledged_deliveries;
        let retired_controls_before = expiring_controls.retired;
        while phase_started.elapsed() < Duration::from_secs(60) {
            assert!(
                clock_started.elapsed() < Duration::from_secs(420),
                "batched retry test exceeded its timebox"
            );
            let now = NdrUnixSeconds(now_secs + clock_started.elapsed().as_secs());
            let turn = Instant::now();
            if engine.has_due_pending_retry_work(now) {
                // Mirrors one lifecycle.handle_messages scope. ACK and the
                // outermost SQLite flush both belong to this measured turn.
                let turn_puts_before = storage.puts.load(Relaxed);
                engine.enter_batch();
                let batch = engine.retry_pending_protocol(now).unwrap();
                assert!(
                    batch.effects.is_empty() && batch.group_result.effects.is_empty(),
                    "missing recipients must not produce an identified fallback"
                );
                let delivered = batch.group_result.events.len();
                group_deliveries += delivered;
                if !batch.direct_messages.is_empty() {
                    // This journal contains both the inherited direct reply and
                    // the newly recovered group message. Verify each before ACK.
                    for delivery in &batch.direct_messages {
                        let id = delivery.event_id.as_ref().unwrap();
                        assert!(acknowledged_ids.insert(id.clone()), "duplicate delivery");
                        if id == &setup_id {
                            assert_eq!(delivery.sender, setup_journal.sender);
                            assert_eq!(delivery.sender_device, setup_journal.sender_device);
                            assert_eq!(
                                delivery.conversation_owner,
                                setup_journal.conversation_owner
                            );
                            assert_eq!(delivery.created_at_secs, setup_journal.created_at_secs);
                            assert_eq!(delivery.content, setup_journal.content);
                            acknowledged_setup_deliveries += 1;
                        } else {
                            assert_eq!(id, &group_id, "unexpected journal delivery");
                            let Some(GroupIncomingEvent::Message(received)) =
                                engine.pending_group_decrypted_delivery(id).unwrap()
                            else {
                                panic!("authenticated group journal required");
                            };
                            assert_eq!(received.group_id, "queued-group");
                            assert_eq!(received.revision, 1);
                            assert_eq!(received.sender_owner, group_sender.sender_owner);
                            assert_eq!(received.sender_device, Some(group_sender.sender_device));
                            assert_eq!(received.body, b"last recoverable position");
                            assert_eq!(delivery.created_at_secs, group_message.created_at.get());
                            acknowledged_group_deliveries += 1;
                        }
                    }
                    acknowledged_deliveries += batch.direct_messages.len();
                    engine.ack_pending_decrypted_deliveries().unwrap();
                }
                {
                    let _phase = RetryPhaseTimer::start("flush");
                    engine.exit_batch().unwrap();
                }
                assert_eq!(engine.batch_depth.get(), 0);
                expiring_controls.observe_turn(
                    &engine,
                    now.get(),
                    turn_puts_before,
                    storage.puts.load(Relaxed),
                );
                if delivered > 0 && first_valid_delivery_seconds.is_none() {
                    first_valid_delivery_seconds = Some(recovery_started.elapsed().as_secs_f64());
                }
                turns += 1;
            }
            let ready = engine.has_ready_group_sender_key_retry_work();
            let elapsed = turn.elapsed();
            work += elapsed;
            max_turn = max_turn.max(elapsed);
            let delay = if ready {
                continuations += 1;
                Duration::from_millis(25)
            } else {
                Duration::from_secs(2)
            };
            std::thread::sleep(
                delay.min(Duration::from_secs(60).saturating_sub(phase_started.elapsed())),
            );
        }
        let elapsed_seconds = phase_started.elapsed().as_secs_f64();
        let cpu_after = retry_process_cpu_ticks();
        let cpu_seconds = (cpu_after - cpu_before) as f64 / ticks_per_second;
        let io_after = crate::storage::checkpoint_io_bytes();
        let puts_after = storage.puts.load(Relaxed);
        let mut remote_deadline_updates = 0;
        for (old, current) in remote_deadlines
            .iter_mut()
            .zip(&engine.pending_remote_sends)
        {
            remote_deadline_updates += usize::from(*old != current.next_retry_at_secs);
            *old = current.next_retry_at_secs;
        }
        let slice = serde_json::json!({
            "stage": if index < 3 { "settle" } else { "measure" },
            "stage_index": if index < 3 { index } else { index - 3 },
            "elapsed_seconds": elapsed_seconds, "recovery_elapsed_seconds": recovery_started.elapsed().as_secs_f64(),
            "cpu_seconds": cpu_seconds, "cpu_percent_one_core": 100.0 * cpu_seconds / elapsed_seconds,
            "work_seconds": work.as_secs_f64(), "max_turn_seconds": max_turn.as_secs_f64(),
            "turns": turns, "continuations": continuations,
            "acknowledged_deliveries": acknowledged_deliveries - ack_before,
            "puts": puts_after - puts_before, "read_bytes": io_after.0 - io_before.0,
            "written_bytes": io_after.1 - io_before.1, "due_at_start": due_at_start,
            "remote_deadline_updates": remote_deadline_updates,
            "expired_control_retirements": expiring_controls.retired - retired_controls_before,
            "progress_before": progress_before, "progress_after": batched_retry_progress(&engine, &audit),
            "phase_work_seconds": audit.take_phase_seconds(),
        });
        eprintln!("batched_retry_slice {slice}");
        slices.push(slice);
        cpu_before = retry_process_cpu_ticks();
        puts_before = storage.puts.load(Relaxed);
        io_before = crate::storage::checkpoint_io_bytes();
        phase_started = Instant::now();
    }
    let (search_starts, repeated_search_starts, input_generation_changes) = audit.counts();
    drop(audit);

    assert_eq!(
        group_deliveries, 1,
        "valid recovery must deliver exactly once"
    );
    assert_eq!(
        acknowledged_deliveries, 2,
        "the measured turns must acknowledge the setup reply and recovered group message"
    );
    assert_eq!(acknowledged_setup_deliveries, 1);
    assert_eq!(acknowledged_group_deliveries, 1);
    assert_eq!(engine.pending_inbound.len(), INBOUND);
    assert_eq!(engine.pending_remote_sends.len(), 44);
    assert_eq!(expiring_controls.retired, 2);
    assert_eq!(
        engine.pending_group_sender_key_messages.len(),
        CIPHERTEXTS - 1
    );
    assert!(engine
        .pending_group_sender_key_messages
        .iter()
        .eq(ciphertexts_before.iter().skip(1)));
    for (index, (actual, original)) in engine
        .pending_inbound
        .iter()
        .zip(&inbound_before)
        .enumerate()
    {
        let mut expected = original.clone();
        expected.next_retry_at_secs = actual.next_retry_at_secs;
        assert!(
            actual == &expected,
            "inbound packet {index} changed beyond its retry deadline"
        );
    }
    assert!(engine.pending_group_fanouts.len() >= FANOUTS);
    for (index, (actual, original)) in engine
        .pending_group_fanouts
        .iter()
        .zip(&fanouts_before)
        .enumerate()
    {
        let mut expected = original.clone();
        expected.next_retry_at_secs = actual.next_retry_at_secs;
        assert!(
            actual == &expected,
            "fanout {index} changed beyond its retry deadline"
        );
    }
    for (index, (actual, original)) in engine
        .pending_remote_sends
        .iter()
        .zip(&remote_before)
        .enumerate()
    {
        let mut expected = original.clone();
        expected.next_retry_at_secs = actual.next_retry_at_secs;
        assert!(
            serde_json::to_value(actual).unwrap() == serde_json::to_value(expected).unwrap(),
            "remote send {index} changed beyond its retry deadline"
        );
    }
    assert!(
        serde_json::to_value(engine.session_manager.snapshot()).unwrap() == sessions_before,
        "unavailable traffic must not advance direct ratchets"
    );
    assert!(engine.pending_decrypted_deliveries.is_empty());
    engine.persist().unwrap();
    let restart_audit = RetrySearchAuditGuard::start();
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(
        restored.pending_inbound == engine.pending_inbound,
        "restart changed inbound packets"
    );
    assert!(
        restored.pending_group_fanouts == engine.pending_group_fanouts,
        "restart changed fanouts"
    );
    assert!(
        restored.pending_group_sender_key_messages == engine.pending_group_sender_key_messages,
        "restart changed unresolved ciphertexts"
    );
    assert!(
        serde_json::to_value(&restored.pending_remote_sends).unwrap()
            == serde_json::to_value(&engine.pending_remote_sends).unwrap(),
        "restart changed remote sends"
    );
    assert!(
        restored.group_manager.snapshot() == engine.group_manager.snapshot(),
        "restart changed group ratchets"
    );
    assert!(
        serde_json::to_value(restored.session_manager.snapshot()).unwrap() == sessions_before,
        "restart changed direct ratchets"
    );
    assert!(restored.pending_decrypted_deliveries.is_empty());
    let restart_ready = restored.has_ready_group_sender_key_retry_work();
    let restart_progress = batched_retry_progress(&restored, &restart_audit);
    assert!(
        restart_ready,
        "reload must retain the unresolved candidates"
    );
    assert_eq!(
        restart_audit.counts().0,
        0,
        "readiness alone must not run a key search"
    );
    // Keep each macro expansion bounded as the receipt gains independent checks.
    let mut receipt = serde_json::json!({
        "schema": "iris-batched-retry-cpu/v2", "fixture": "restored-mixed-backlog-with-expiring-controls-v2", "synthetic": true,
        "pending_inbound": INBOUND, "initially_due_inbound": 102, "fanouts": FANOUTS,
        "initially_due_fanouts": 10462, "ciphertexts": CIPHERTEXTS, "remote_sends": 44,
        "expiring_controls": 2, "initial_remote_sends": 46, "retired_expiring_controls": expiring_controls.retired,
        "checkpoint_bytes": CHECKPOINT_BYTES, "outer_batch": true, "ack_and_flush_in_turn": true,
        "continuation_delay_ms": 25, "liveness_delay_ms": 2000, "settle_seconds": 180,
        "measurement_windows": 2, "window_seconds": 60, "debug_assertions": cfg!(debug_assertions),
    });
    let serde_json::Value::Object(measurements) = serde_json::json!({
        "restore_seconds": restore_seconds, "initially_ready": initially_ready,
        "readiness_seconds": readiness_seconds, "first_valid_delivery_seconds": first_valid_delivery_seconds,
        "group_deliveries": group_deliveries, "acknowledged_deliveries": acknowledged_deliveries,
        "acknowledged_setup_deliveries": acknowledged_setup_deliveries,
        "acknowledged_group_deliveries": acknowledged_group_deliveries,
        "search_starts": search_starts, "repeated_search_starts": repeated_search_starts,
        "input_generation_changes": input_generation_changes, "wire_effect_count": 0,
        "remaining_inbound": engine.pending_inbound.len(), "remaining_fanouts": engine.pending_group_fanouts.len(),
        "remaining_ciphertexts": engine.pending_group_sender_key_messages.len(), "remaining_remote_sends": 44,
        "preservation_checked": true, "restart_checked": true, "restart_ready": restart_ready,
        "restart_progress": restart_progress, "slices": slices,
    }) else {
        unreachable!("receipt measurements are a JSON object");
    };
    receipt.as_object_mut().unwrap().extend(measurements);
    eprintln!("batched_retry_cpu {receipt}");
    assert!(
        restore_seconds <= 180.0 && initially_ready && readiness_seconds <= 180.0,
        "restoration and scheduler readiness exceeded the settle deadline"
    );
    assert!(
        first_valid_delivery_seconds.is_some_and(|seconds| seconds <= 180.0),
        "valid delivery exceeded the settle deadline"
    );
    assert!(
        search_starts > 0 && repeated_search_starts == 0,
        "unchanged input generation re-admitted an exhausted candidate"
    );
    for (index, slice) in slices.iter().enumerate() {
        assert_eq!(
            slice["expired_control_retirements"].as_u64().unwrap(),
            u64::from(index >= 3),
            "each measured minute must retire one real control, after settling"
        );
    }
    for slice in &slices[3..] {
        assert!(
            slice["turns"].as_u64().unwrap() > 0
                && slice["remote_deadline_updates"].as_u64().unwrap() > 0
                && slice["puts"].as_u64().unwrap() > 0,
            "measurement must exercise recurring due work, not only a quiet gap"
        );
        assert!(
            slice["cpu_percent_one_core"].as_f64().unwrap() <= 5.0,
            "batched steady retry loop still exceeds 5% of one CPU: {slice}"
        );
    }
    assert!(
        clock_started.elapsed() < Duration::from_secs(420),
        "batched retry test exceeded its timebox"
    );
}
