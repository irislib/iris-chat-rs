fn queued_group_history(count: usize) -> (ProtocolEngine, Keys, Keys) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let sender_device = Keys::generate();
    let author = ndr_device(Keys::generate().public_key());
    let mut engine = test_engine(&owner, &device);
    engine
        .ingest_app_keys_event(&signed_app_keys(&sender, &[sender_device.public_key()], 1))
        .unwrap();
    let group = group_snapshot_for_test(
        "queued-group",
        "History",
        1,
        &sender,
        &[owner.public_key(), sender.public_key()],
    );
    let initial = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
    engine.group_manager = GroupEventManager::from_snapshot(GroupManagerSnapshot {
        local_owner_pubkey: engine.local_owner,
        groups: vec![group.clone()],
        sender_keys: vec![nostr_double_ratchet::GroupSenderKeyRecordSnapshot {
            group_id: group.group_id.clone(),
            sender_owner: ndr_owner(sender.public_key()),
            sender_device: ndr_device(sender_device.public_key()),
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
    let mut encryptor = initial;
    for index in 0..count {
        let plaintext = JsonGroupPayloadCodecV1
            .encode_sender_key_plaintext(
                nostr_double_ratchet::GroupPayloadEncodeContext {
                    local_device_pubkey: ndr_device(sender_device.public_key()),
                    created_at: NdrUnixSeconds(20),
                },
                &nostr_double_ratchet::GroupSenderKeyPlaintext {
                    group_id: group.group_id.clone(),
                    revision: 1,
                    body: format!("history-{index}").into_bytes(),
                },
            )
            .unwrap();
        let (message_number, ciphertext) = encryptor.encrypt_to_bytes(&plaintext).unwrap();
        engine.pending_group_sender_key_messages.push(
            nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
                sender_event_pubkey: author,
                key_id: 1,
                message_number,
                encrypted_header: Some("hidden position".into()),
                created_at: NdrUnixSeconds(20),
                ciphertext,
            },
        );
    }
    (engine, sender, sender_device)
}

#[test]
fn group_retry_budget_preserves_backlog_and_yields_between_batches() {
    let (mut engine, _, _) = queued_group_history(20);
    let first = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert!(
        first.group_result.events.len() <= 8,
        "one turn must not decrypt the whole history queue"
    );
    assert_eq!(
        first.group_result.events.len() + engine.pending_group_sender_key_messages.len(),
        20
    );
    assert!(
        engine.has_due_pending_retry_work(NdrUnixSeconds(30)),
        "the retained tail must schedule continuation"
    );
    let mut delivered = first.group_result.events.len();
    for _ in 0..20 {
        let next = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        assert!(next.group_result.events.len() <= 8);
        delivered += next.group_result.events.len();
        if engine.pending_group_sender_key_messages.is_empty() {
            break;
        }
    }
    assert_eq!(delivered, 20);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_does_not_run_for_an_ordinary_direct_message() {
    let (mut engine, sender, sender_device) = queued_group_history(20);
    let result = engine
        .process_group_pairwise_payload(
            b"ordinary direct message",
            sender.public_key(),
            Some(sender_device.public_key()),
        )
        .unwrap();
    assert!(!result.consumed);
    assert!(
        result.events.is_empty(),
        "a direct message must not synchronously decrypt unrelated group history"
    );
    assert_eq!(engine.pending_group_sender_key_messages.len(), 20);
}

struct GroupRetryCountingStorage {
    inner: Arc<dyn StorageAdapter>,
    puts: std::sync::atomic::AtomicUsize,
    fail_next: std::sync::atomic::AtomicBool,
}

impl StorageAdapter for GroupRetryCountingStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }
    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        self.puts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self
            .fail_next
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(StorageError::new("injected group retry save failure"));
        }
        self.inner.put(key, value)
    }
    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
}

#[test]
fn group_retry_unchanged_metadata_does_not_repeat_blind_search_or_checkpoint() {
    let (mut engine, sender, sender_device) = queued_group_history(1);
    engine.pending_group_sender_key_messages[0].ciphertext =
        nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 0)
            .encrypt_to_bytes(b"synthetic unknown ciphertext")
            .unwrap()
            .1;
    let mut group_state = engine.group_manager.snapshot();
    let distribution = nostr_double_ratchet::SenderKeyDistribution {
        group_id: "queued-group".into(),
        key_id: 1,
        sender_event_pubkey: group_state.sender_keys[0].sender_event_pubkey,
        chain_key: [9; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(20),
    };
    group_state.sender_keys[0]
        .distribution_history
        .push(distribution.clone());
    engine.group_manager = GroupEventManager::from_snapshot(group_state).unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    engine.storage = storage.clone();
    let now = NdrUnixSeconds(unix_now().get());
    let started = std::time::Instant::now();
    drain_group_retry(&mut engine, now);
    println!("one failed blind candidate: {:?}", started.elapsed());
    assert_eq!(engine.group_sender_key_retry.borrow().total_attempts, 1);
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
    assert!(!engine.has_ready_group_sender_key_retry_work());
    let mut presentation_only = engine.group_manager.snapshot();
    presentation_only.groups[0].name = "Renamed locally".into();
    presentation_only.groups[0].picture = Some("https://example.invalid/picture".into());
    presentation_only.groups[0].updated_at = now;
    engine.group_manager = GroupEventManager::from_snapshot(presentation_only).unwrap();
    assert!(
        !engine.has_ready_group_sender_key_retry_work(),
        "display metadata alone cannot unlock ciphertext"
    );
    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: now,
            },
            &GroupPairwiseCommand::MetadataSnapshot {
                snapshot: engine.group_manager.group("queued-group").unwrap(),
            },
        )
        .unwrap();
    let distribution_payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: now,
            },
            &GroupPairwiseCommand::SenderKeyDistribution { distribution },
        )
        .unwrap();
    let writes = storage.puts.load(std::sync::atomic::Ordering::Relaxed);
    for payload in [&payload, &distribution_payload]
        .into_iter()
        .cycle()
        .take(6)
    {
        engine
            .process_group_pairwise_payload(
                payload,
                sender.public_key(),
                Some(sender_device.public_key()),
            )
            .unwrap();
        engine.retry_pending_protocol(now).unwrap();
    }
    assert_eq!(
        engine.group_sender_key_retry.borrow().total_attempts,
        1,
        "identical metadata cannot unlock different key material"
    );
    assert_eq!(
        storage.puts.load(std::sync::atomic::Ordering::Relaxed),
        writes,
        "idle retry bookkeeping must not rewrite the durable backlog"
    );
}

#[test]
fn group_retry_budget_is_shared_by_a_whole_outer_batch_and_survives_restart() {
    let (mut engine, _, _) = queued_group_history(20);
    let owner = engine.owner_pubkey;
    let keys = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    let storage = engine.storage.clone();
    engine.enter_batch();
    let mut delivered = 0;
    for _ in 0..16 {
        delivered += engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len();
    }
    assert!(
        delivered <= 8,
        "sixteen background events still share one retry budget"
    );
    engine.exit_batch().unwrap();
    drop(engine);
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner, &keys).unwrap();
    assert_eq!(
        restored.pending_group_sender_key_messages.len(),
        20 - delivered
    );
    assert!(restored.has_due_pending_retry_work(NdrUnixSeconds(30)));
    for _ in 0..20 {
        delivered += restored
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len();
        if restored.pending_group_sender_key_messages.is_empty() {
            break;
        }
    }
    assert_eq!(delivered, 20);
    assert!(!restored.has_ready_group_sender_key_retry_work());
}

#[test]
fn group_retry_error_retains_the_candidate_and_unprocessed_tail() {
    let (mut engine, _, _) = queued_group_history(3);
    // The known-position path surfaces malformed ciphertext as an error.
    engine.pending_group_sender_key_messages[0].encrypted_header = None;
    engine.pending_group_sender_key_messages[0]
        .ciphertext
        .clear();
    let malformed = engine.pending_group_sender_key_messages[0].clone();
    let recovered = drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert_eq!(recovered.len(), 2);
    assert_eq!(&*engine.pending_group_sender_key_messages, &[malformed]);
    // Another success changes the ratchet once; after that input is attempted,
    // malformed ciphertext cannot spin indefinitely.
    drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert!(!engine.has_ready_group_sender_key_retry_work());
}

#[test]
fn group_retry_hot_stream_does_not_starve_another_stream_at_the_tail() {
    let (mut engine, _, _) = queued_group_history(20);
    let (other, _, other_device) = queued_group_history(1);
    let mut state = engine.group_manager.snapshot();
    let other_state = other.group_manager.snapshot();
    state.groups[0]
        .members
        .push(other_state.sender_keys[0].sender_owner);
    state.sender_keys.extend(other_state.sender_keys);
    engine.group_manager = GroupEventManager::from_snapshot(state).unwrap();
    for pending in other.pending_group_sender_key_messages.iter().cloned() {
        engine.pending_group_sender_key_messages.push(pending);
    }
    let mut found_tail = false;
    for _ in 0..21 {
        let batch = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        found_tail |= batch.group_result.events.iter().any(|event| matches!(event,
            GroupIncomingEvent::Message(message) if message.sender_device == Some(ndr_device(other_device.public_key()))));
        // Advancing the hot stream re-admits its old failures behind the tail.
        engine.has_ready_group_sender_key_retry_work();
        if found_tail {
            break;
        }
    }
    assert!(
        found_tail,
        "new work on the first stream cannot reset FIFO progress"
    );
}

#[test]
fn group_retry_new_revision_resumes_a_pending_ciphertext_after_sibling_sync() {
    let (mut engine, _, _) = queued_group_history(1);
    let mut state = engine.group_manager.snapshot();
    state.groups[0].revision = 0;
    engine.group_manager = GroupEventManager::from_snapshot(state).unwrap();
    engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
    assert!(!engine.has_ready_group_sender_key_retry_work());
    let mut group = engine.group_manager.group("queued-group").unwrap();
    group.revision = 1;
    assert!(engine.install_device_sync_group(group).unwrap());
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(30)));
    let recovered = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(recovered.group_result.events.len(), 1);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_ratchet_progress_unlocks_an_earlier_out_of_window_candidate() {
    let (mut engine, _, _) = queued_group_history(1);
    let template = engine.pending_group_sender_key_messages[0].clone();
    let mut bridge = None;
    let mut future = None;
    // Keep the production 10,000-key search window. A later queued message
    // advances the chain enough to unlock the previously failed ciphertext.
    let decoded = JsonGroupPayloadCodecV1
        .encode_sender_key_plaintext(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: engine.group_manager.snapshot().sender_keys[0].sender_device,
                created_at: NdrUnixSeconds(20),
            },
            &nostr_double_ratchet::GroupSenderKeyPlaintext {
                group_id: "queued-group".into(),
                revision: 1,
                body: b"recovered".to_vec(),
            },
        )
        .unwrap();
    let mut encryptor = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
    for number in 0..=10_001 {
        let (_, ciphertext) = encryptor.encrypt_to_bytes(&decoded).unwrap();
        if number == 5_000 {
            bridge = Some(ciphertext.clone());
        }
        if number == 10_001 {
            future = Some(ciphertext);
        }
    }
    engine.pending_group_sender_key_messages = vec![
        nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
            ciphertext: future.unwrap(),
            ..template.clone()
        },
        nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
            ciphertext: bridge.unwrap(),
            ..template
        },
    ]
    .into();
    let mut delivered = 0;
    for _ in 0..256 {
        delivered += engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len();
        if engine.pending_group_sender_key_messages.is_empty() {
            break;
        }
    }
    assert_eq!(delivered, 2);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn group_retry_storage_failure_restores_ratchet_and_delivers_once_after_retry() {
    let (mut engine, _, _) = queued_group_history(3);
    engine.persist().unwrap();
    let before = engine.pending_group_sender_key_messages.clone();
    let group_before = engine.group_manager.snapshot();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    engine.storage = storage.clone();
    let error = engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert_eq!(engine.pending_group_sender_key_messages, before);
    assert_eq!(engine.group_manager.snapshot(), group_before);
    assert!(engine.has_ready_group_sender_key_retry_work());
    let mut delivered = 0;
    for _ in 0..3 {
        delivered += engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len();
        if engine.pending_group_sender_key_messages.is_empty() {
            break;
        }
    }
    assert_eq!(delivered, 3);
    assert!(engine.pending_group_sender_key_messages.is_empty());
    assert!(engine
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
    let keys = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, engine.owner_pubkey, &keys)
            .unwrap();
    assert!(restored.pending_group_sender_key_messages.is_empty());
    assert!(restored
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
}

#[test]
fn group_retry_new_candidate_on_unchanged_stream_is_admitted_once() {
    let (mut engine, _, _) = queued_group_history(2);
    let next = engine.pending_group_sender_key_messages.pop().unwrap();
    engine.pending_group_sender_key_messages[0].ciphertext =
        nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 0)
            .encrypt_to_bytes(b"synthetic unknown ciphertext")
            .unwrap()
            .1;
    drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert!(!engine.has_ready_group_sender_key_retry_work());
    engine
        .queue_pending_group_sender_key_message(next.clone())
        .unwrap();
    assert!(engine.has_ready_group_sender_key_retry_work());
    let recovered = drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert_eq!(recovered.len(), 1);
    engine.queue_pending_group_sender_key_message(next).unwrap();
    drain_group_retry(&mut engine, NdrUnixSeconds(30));
    assert!(!engine.has_ready_group_sender_key_retry_work());
}

#[test]
fn group_retry_large_valid_history_advances_in_bounded_passes() {
    let (mut engine, _, _) = queued_group_history(512);
    let started = std::time::Instant::now();
    let mut delivered = 0;
    let mut passes = 0;
    let mut longest = std::time::Duration::ZERO;
    while engine.has_ready_group_sender_key_retry_work() {
        let pass_started = std::time::Instant::now();
        engine.enter_batch();
        let batch = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
        assert!(batch.group_result.events.len() <= GROUP_SENDER_KEY_RETRY_LIMIT);
        delivered += batch.group_result.events.len();
        engine.exit_batch().unwrap();
        longest = longest.max(pass_started.elapsed());
        passes += 1;
        assert!(passes <= 512, "every ready pass must progress");
    }
    assert_eq!(delivered, 512);
    println!(
        "valid history: messages={delivered} passes={passes} total_ms={:.3} max_pass_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0,
        longest.as_secs_f64() * 1000.0
    );
}

#[test]
fn group_retry_checkpoint_restores_eligibility_without_restarting_forever() {
    let (mut engine, _, _) = queued_group_history(1);
    assert!(engine.has_ready_group_sender_key_retry_work());
    let before = engine.state_checkpoint();
    assert_eq!(
        engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len(),
        1
    );
    assert!(!engine.has_ready_group_sender_key_retry_work());
    engine.restore_checkpoint(before);
    assert!(engine.has_ready_group_sender_key_retry_work());
    assert_eq!(
        engine
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .group_result
            .events
            .len(),
        1
    );
    assert!(!engine.has_ready_group_sender_key_retry_work());
}
