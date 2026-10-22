// Synthetic packets only. The counts and deadline mix come from aggregate
// checkpoint diagnostics; no account payloads or identities enter this fixture.
pub(super) fn synthetic_pending_inbound(
    receiver: &mut ProtocolEngine,
    count: usize,
    due: usize,
    now: u64,
) -> Vec<ProtocolPendingInbound> {
    let sender = Keys::generate();
    receiver
        .ingest_app_keys_event(&signed_app_keys(&sender, &[sender.public_key()], 1))
        .unwrap();
    let (mut session, response) = receiver
        .local_invite()
        .unwrap()
        .accept_with_owner(
            sender.public_key(),
            sender.secret_key().to_secret_bytes(),
            Some(sender.public_key().to_hex()),
            Some(sender.public_key()),
        )
        .unwrap();
    receiver
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    (0..count)
        .map(|index| {
            let plan = session
                .plan_send(
                    format!("deferred-{index}:{}", "x".repeat(1024)).as_bytes(),
                    NdrUnixSeconds(1),
                )
                .unwrap();
            let mut envelope = session.apply_send(plan).envelope;
            // The sender is known across restarts, but these damaged packets
            // cannot advance its ratchet. Each signed event remains distinct.
            envelope.ciphertext = format!("invalid ciphertext {index}:{}", "x".repeat(1024));
            let event = nostr_double_ratchet::message_event(&envelope).unwrap();
            let metadata =
                receiver.pending_inbound_metadata_for_event(&event, Some(&envelope), None);
            let mut pending = ProtocolPendingInbound {
                event,
                created_at_secs: 1,
                next_retry_at_secs: if index < due { now } else { now + 52 },
                event_id: String::new(),
                envelope: None,
                sender_message_pubkey_hex: None,
                resolved_owner_pubkey_hex: None,
                claimed_owner_pubkey_hex: None,
                metadata_verified: false,
            };
            apply_pending_inbound_metadata(&mut pending, metadata);
            pending
        })
        .collect()
}

#[test]
fn deferred_inbound_retry_allocation_is_bounded_by_due_packets() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let now = NdrUnixSeconds(20_000_000);
    engine.pending_inbound = synthetic_pending_inbound(&mut engine, 4_214, 102, now.get());
    let mut expected = engine.pending_inbound.clone();
    assert_eq!(
        expected
            .iter()
            .map(|item| item.event.id)
            .collect::<HashSet<_>>()
            .len(),
        4_214,
        "fixture must contain distinct packets, not repeated references"
    );
    for item in &mut expected[..102] {
        item.next_retry_at_secs = now.get() + 60;
    }
    let ((), allocated) = remote_roster_tests::allocated_bytes(|| {
        // The first pass has expired, unresolved packets. Following ready-group
        // continuations revisit the same now-deferred queue during this second.
        for _ in 0..32 {
            assert!(engine.retry_pending_protocol(now).unwrap().is_empty());
        }
    });
    eprintln!("inbound_retry packets=4214 initially_due=102 turns=32 allocated_bytes={allocated}");
    assert!(
        allocated < 2 * 1024 * 1024,
        "allocated {allocated} bytes copying deferred packets"
    );
    assert_eq!(engine.pending_inbound, expected);
    assert!(engine.pending_decrypted_deliveries.is_empty());
}

#[cfg(target_os = "linux")]
fn retry_process_cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields = stat[stat.rfind(')').unwrap() + 2..]
        .split_whitespace()
        .collect::<Vec<_>>();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "60-second hosted CPU qualification with a synthetic 350 MB checkpoint; run alone"]
fn restored_backlog_whole_retry_at_continuation_cadence() {
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::{Duration, Instant};

    let now_secs = unix_now().get();
    let clock_started = Instant::now();
    const INBOUND: usize = 4_214;
    const FANOUTS: usize = 24_990;
    const CIPHERTEXTS: usize = 23_437;
    const CHECKPOINT_BYTES: usize = 350_224_384;
    let mut f = checkpoint_backlog_fixture(FANOUTS, 0);
    let (mut group, sender, sender_device) = queued_group_history(1);
    long_group_candidate(&mut group, &sender_device, 700);
    let mut group_state = group.group_manager.snapshot();
    group_state.local_owner_pubkey = f.sender.local_owner;
    group_state.groups[0]
        .members
        .retain(|owner| *owner != group.local_owner);
    group_state.groups[0].members.push(f.sender.local_owner);
    f.sender.group_manager = GroupEventManager::from_snapshot(group_state).unwrap();
    f.sender
        .ingest_app_keys_event(&signed_app_keys(&sender, &[sender_device.public_key()], 1))
        .unwrap();

    let valid = group.pending_group_sender_key_messages[0].clone();
    let mut ciphertexts = vec![valid.clone()];
    let mut unavailable = nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 0);
    for index in 1..CIPHERTEXTS {
        let mut parsed = valid.clone();
        let (number, ciphertext) = unavailable
            .encrypt_to_bytes(format!("unresolved-{index}:{}", "x".repeat(768)).as_bytes())
            .unwrap();
        parsed.message_number = number;
        parsed.ciphertext = ciphertext;
        ciphertexts.push(parsed);
    }
    f.sender.pending_inbound = synthetic_pending_inbound(&mut f.sender, INBOUND, 102, now_secs);
    f.sender.pending_group_sender_key_messages = ciphertexts.into();
    for (index, pending) in f.sender.pending_group_fanouts.iter_mut().enumerate() {
        pending.created_at_secs = now_secs - 4_933 - (index as u64 % 34_274);
        pending.next_retry_at_secs = if index < 10_462 {
            now_secs - (index as u64 % 43)
        } else {
            now_secs + 1 + (index as u64 % 53)
        };
    }
    // These counts model a restored durable snapshot, not exact live deadlines.
    // Direct retry backoff itself is deliberately only an in-memory mutation.
    for (index, pending) in f.sender.pending_inbound.iter_mut().enumerate() {
        pending.created_at_secs = now_secs - 6_463 - (index as u64 % 13_572_070);
        if index < 102 {
            pending.next_retry_at_secs = now_secs - index as u64;
        }
    }
    let missing_owner = ndr_owner(Keys::generate().public_key());
    f.sender.pending_remote_sends = (0..44)
        .map(|index| ProtocolPendingRemoteSend {
            recipient_owner: missing_owner,
            eligible_devices: BTreeSet::from([ndr_device(Keys::generate().public_key())]),
            completed_devices: BTreeSet::new(),
            chat_id: missing_owner.to_hex(),
            payload: format!("queued-outgoing-{index}").into_bytes(),
            inner_event_id: None,
            message_id: format!("outgoing-{index}"),
            expires_at_secs: None,
            created_at_secs: now_secs - 6_463,
            next_retry_at_secs: if index < 7 { now_secs } else { now_secs + 53 },
        })
        .collect();

    let directory = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE ndr_kv (
        owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL,
        key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(owner_pubkey_hex, device_pubkey_hex, key));").unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: Arc::new(SqliteStorageAdapter::new(
            Arc::new(Mutex::new(connection)),
            f.owner.public_key().to_hex(),
            f.device.public_key().to_hex(),
        )),
        puts: 0.into(),
        fail_next: false.into(),
    });
    f.sender.storage = storage.clone();
    f.sender.persist().unwrap();
    f.sender.checkpoint_layout.borrow_mut().bytes = CHECKPOINT_BYTES;
    f.sender.persist().unwrap();
    // Exercise the real restore path before timing, with no test-only outer batch.
    drop(f.sender);
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    assert_eq!(engine.checkpoint_layout.borrow().bytes, CHECKPOINT_BYTES);
    assert_eq!(engine.pending_inbound.len(), INBOUND);
    assert_eq!(engine.pending_group_fanouts.len(), FANOUTS);
    assert_eq!(engine.pending_group_sender_key_messages.len(), CIPHERTEXTS);
    let inbound_before = engine.pending_inbound.clone();
    let fanouts_before = engine.pending_group_fanouts.clone();
    let ciphertexts_before = engine.pending_group_sender_key_messages.clone();
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
    let puts_before = storage.puts.load(Relaxed);
    let io_before = crate::storage::checkpoint_io_bytes();
    let cpu_before = retry_process_cpu_ticks();
    let start = Instant::now();
    let mut turns = 0;
    let mut continuations = 0;
    let mut group_deliveries = 0;
    let mut work = Duration::ZERO;
    let mut max_turn = Duration::ZERO;
    while start.elapsed() < Duration::from_secs(60) {
        let now = NdrUnixSeconds(now_secs + clock_started.elapsed().as_secs());
        let turn = Instant::now();
        if engine.has_due_pending_retry_work(now) {
            let batch = engine.retry_pending_protocol(now).unwrap();
            group_deliveries += batch.group_result.events.len();
            // No missing recipient may become a publicly identified fallback.
            assert!(batch.effects.is_empty());
            assert!(batch.group_result.effects.is_empty());
            if !batch.direct_messages.is_empty() {
                engine.ack_pending_decrypted_deliveries().unwrap();
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
        std::thread::sleep(delay.min(Duration::from_secs(60).saturating_sub(start.elapsed())));
    }
    let cpu_seconds = (retry_process_cpu_ticks() - cpu_before) as f64 / ticks_per_second;
    let elapsed_seconds = start.elapsed().as_secs_f64();
    let io_after = crate::storage::checkpoint_io_bytes();
    let cpu_percent = 100.0 * cpu_seconds / elapsed_seconds;
    let receipt = serde_json::json!({
        "fixture": "restored-mixed-backlog-v1", "synthetic": true,
        "pending_inbound": INBOUND, "initially_due_inbound": 102,
        "fanouts": FANOUTS, "initially_due_fanouts": 10462,
        "ciphertexts": CIPHERTEXTS, "checkpoint_bytes": CHECKPOINT_BYTES,
        "remote_sends": 44, "all_ciphertext_authors_have_group_state": true,
        "continuation_delay_ms": 25, "liveness_delay_ms": 2000,
        "outer_batch": false, "turns": turns, "continuations": continuations,
        "group_deliveries": group_deliveries, "elapsed_seconds": elapsed_seconds,
        "work_seconds": work.as_secs_f64(), "max_turn_seconds": max_turn.as_secs_f64(),
        "cpu_seconds": cpu_seconds, "cpu_percent_one_core": cpu_percent,
        "puts": storage.puts.load(Relaxed) - puts_before,
        "read_bytes": io_after.0 - io_before.0, "written_bytes": io_after.1 - io_before.1,
        "remaining_inbound": engine.pending_inbound.len(),
        "remaining_fanouts": engine.pending_group_fanouts.len(),
        "remaining_ciphertexts": engine.pending_group_sender_key_messages.len(),
    });
    eprintln!("whole_retry_cpu {receipt}");
    assert!(
        continuations >= 8,
        "fixture must exercise repeated ready continuations"
    );
    assert_eq!(
        group_deliveries, 1,
        "the valid message must survive failed candidates"
    );
    assert_eq!(
        engine.pending_group_sender_key_messages.len(),
        CIPHERTEXTS - 1
    );
    assert!(engine
        .pending_group_sender_key_messages
        .iter()
        .eq(ciphertexts_before.iter().skip(1)));
    assert_eq!(engine.pending_remote_sends.len(), 44);
    assert_eq!(engine.pending_inbound.len(), INBOUND);
    for (actual, original) in engine.pending_inbound.iter().zip(&inbound_before) {
        let mut expected = original.clone();
        expected.next_retry_at_secs = actual.next_retry_at_secs;
        assert_eq!(actual, &expected);
    }
    assert!(engine.pending_group_fanouts.len() >= FANOUTS);
    for (actual, original) in engine.pending_group_fanouts.iter().zip(&fanouts_before) {
        let mut expected = original.clone();
        expected.next_retry_at_secs = actual.next_retry_at_secs;
        assert_eq!(actual, &expected);
    }
    assert!(
        storage.puts.load(Relaxed) > puts_before,
        "exercise real durable writes"
    );
    engine.persist().unwrap();
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, f.owner.public_key(), &f.device)
            .unwrap();
    assert_eq!(restored.pending_inbound, engine.pending_inbound);
    assert_eq!(restored.pending_group_fanouts, engine.pending_group_fanouts);
    assert_eq!(
        restored.pending_group_sender_key_messages,
        engine.pending_group_sender_key_messages
    );
    assert_eq!(
        serde_json::to_value(&restored.pending_remote_sends).unwrap(),
        serde_json::to_value(&engine.pending_remote_sends).unwrap()
    );
    // This is a qualification gate, not an ignored performance printout. A
    // passing microbenchmark cannot override a still-busy whole retry loop.
    assert!(
        cpu_percent <= 5.0,
        "whole retry loop still uses {cpu_percent:.2}% of one CPU"
    );
}
