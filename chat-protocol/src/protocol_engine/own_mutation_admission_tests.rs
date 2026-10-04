struct MutationJournalStorage {
    inner: Arc<dyn StorageAdapter>,
    writes: std::sync::Mutex<Vec<String>>,
    fail_next: std::sync::atomic::AtomicBool,
}
impl StorageAdapter for MutationJournalStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }
    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        self.writes.lock().unwrap().push(value.clone());
        if self
            .fail_next
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(StorageError::new("injected mutation checkpoint failure"));
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
fn observe_mutation_journal(engine: &mut ProtocolEngine) -> Arc<MutationJournalStorage> {
    let storage = Arc::new(MutationJournalStorage {
        inner: engine.storage.clone(),
        writes: Default::default(),
        fail_next: false.into(),
    });
    engine.storage = storage.clone();
    storage
}
fn mutation_rumor(author: PublicKey, kind: u16, body: &str) -> UnsignedEvent {
    let mut event = UnsignedEvent::new(
        author,
        Timestamp::from_secs(20),
        Kind::Custom(kind),
        vec![nostr::Tag::parse(["e", "original-before-link"]).unwrap()],
        body,
    );
    event.ensure_id();
    event
}

#[test]
fn own_mutation_direct_transport_never_journals_replacement_text() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let peer = Keys::generate();
    let mut sender = test_engine(&owner, &a);
    let mut receiver = test_engine(&owner, &b);
    let roster = signed_app_keys(&owner, &[a.public_key(), b.public_key()], 1);
    for engine in [&mut sender, &mut receiver] {
        engine.ingest_app_keys_event(&roster).unwrap();
    }
    observe_sibling_invite(&mut sender, &receiver, &b);
    let storage = observe_mutation_journal(&mut receiver);
    for kind in [1009, 5, 14] {
        let body = if kind == 14 {
            "ordinary message"
        } else {
            "HISTORY_CHOICE_SECRET"
        };
        // Emulate an older sender's ordinary own-device fanout.
        let sent = sender
            .send_local_sibling_unsigned_event(
                peer.public_key(),
                &peer.public_key().to_hex(),
                mutation_rumor(owner.public_key(), kind, body),
                UnixSeconds(20),
            )
            .unwrap();
        let messages = decrypt_own_sync_effects(&mut receiver, sent.effects);
        assert_eq!(messages.len(), usize::from(kind == 14), "kind={kind}");
    }
    assert!(receiver.pending_inbound.is_empty());
    assert_eq!(receiver.pending_decrypted_deliveries.len(), 1);
    assert!(
        !storage
            .writes
            .lock()
            .unwrap()
            .iter()
            .any(|value| value.contains("HISTORY_CHOICE_SECRET")),
        "Even intermediate protocol checkpoints must not contain rejected text"
    );
    receiver.ack_pending_decrypted_deliveries().unwrap();
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &b).unwrap();
    assert!(restored
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
}

#[test]
fn own_mutation_filter_preserves_direct_participant_out_of_order_delivery() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut receiver = test_engine(&owner, &device);
    let mut sender = test_engine(&peer, &peer_device);
    let roster = signed_app_keys(&peer, &[peer_device.public_key()], 1);
    receiver.ingest_app_keys_event(&roster).unwrap();
    sender.ingest_app_keys_event(&roster).unwrap();
    sender
        .ingest_app_keys_event(&signed_app_keys(&owner, &[device.public_key()], 1))
        .unwrap();
    observe_sibling_invite(&mut sender, &receiver, &device);
    let storage = observe_mutation_journal(&mut receiver);
    let sent = sender
        .send_direct_unsigned_event_to_peer_only(
            owner.public_key(),
            &owner.public_key().to_hex(),
            mutation_rumor(peer.public_key(), 1009, "PARTICIPANT_EDIT"),
            UnixSeconds(20),
        )
        .unwrap();
    let messages = decrypt_own_sync_effects(&mut receiver, sent.effects);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].content.contains("PARTICIPANT_EDIT"));
    assert!(storage
        .writes
        .lock()
        .unwrap()
        .iter()
        .any(|value| value.contains("PARTICIPANT_EDIT")));
}

#[test]
fn own_mutation_group_transport_advances_ratchet_without_journaling_content() {
    for own in [false, true] {
        let (mut engine, sender, device) = queued_group_history(1);
        if own {
            let mut state = engine.group_manager.snapshot();
            state.sender_keys[0].sender_owner = engine.local_owner;
            engine.group_manager = GroupEventManager::from_snapshot(state).unwrap();
        }
        let author = if own {
            engine.owner_pubkey
        } else {
            sender.public_key()
        };
        let storage = observe_mutation_journal(&mut engine);
        let mut encryptor = nostr_double_ratchet::SenderKeyState::new(1, [9; 32], 0);
        for (index, kind) in [1009, 5, 14].into_iter().enumerate() {
            let body = if kind == 14 {
                "ordinary group message"
            } else {
                "GROUP_HISTORY_CHOICE_SECRET"
            };
            let plaintext = JsonGroupPayloadCodecV1
                .encode_sender_key_plaintext(
                    nostr_double_ratchet::GroupPayloadEncodeContext {
                        local_device_pubkey: ndr_device(device.public_key()),
                        created_at: NdrUnixSeconds(20),
                    },
                    &nostr_double_ratchet::GroupSenderKeyPlaintext {
                        group_id: "queued-group".into(),
                        revision: 1,
                        body: serde_json::to_vec(&mutation_rumor(author, kind, body)).unwrap(),
                    },
                )
                .unwrap();
            let mut candidate = engine.pending_group_sender_key_messages[0].clone();
            let (number, ciphertext) = encryptor.encrypt_to_bytes(&plaintext).unwrap();
            candidate.ciphertext = ciphertext;
            candidate.message_number = number;
            if index == 0 {
                engine.pending_group_sender_key_messages[0] = candidate;
            } else {
                engine.pending_group_sender_key_messages.push(candidate);
            }
        }
        let events = drain_group_retry(&mut engine, NdrUnixSeconds(30));
        assert_eq!(events.len(), if own { 1 } else { 3 });
        assert_eq!(
            engine.pending_decrypted_deliveries.len(),
            if own { 1 } else { 3 }
        );
        assert_eq!(
            storage
                .writes
                .lock()
                .unwrap()
                .iter()
                .any(|value| checkpoint_contains_group_secret(value)),
            !own,
            "Authentication, not claimed event author, distinguishes own-device transport"
        );
        assert!(engine.pending_group_sender_key_messages.is_empty());
    }
}

fn checkpoint_contains_group_secret(value: &str) -> bool {
    let state: ProtocolEnginePersistedState = serde_json::from_str(value).unwrap();
    state.pending_decrypted_deliveries.iter().any(|delivery| {
        matches!(JsonGroupPayloadCodecV1.decode_pairwise_command(delivery.content.as_bytes()),
            Ok(Some(GroupPairwiseCommand::GroupMessage { body, .. }))
                if String::from_utf8_lossy(&body).contains("GROUP_HISTORY_CHOICE_SECRET"))
    })
}

#[test]
fn own_mutation_direct_save_failure_retries_without_plaintext_or_duplicate_work() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let peer = Keys::generate();
    let mut sender = test_engine(&owner, &a);
    let mut receiver = test_engine(&owner, &b);
    let roster = signed_app_keys(&owner, &[a.public_key(), b.public_key()], 1);
    for engine in [&mut sender, &mut receiver] {
        engine.ingest_app_keys_event(&roster).unwrap();
    }
    // Establish a session before injecting the receive checkpoint failure.
    observe_sibling_invite(&mut sender, &receiver, &b);
    let ready = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            mutation_rumor(owner.public_key(), 14, "warmup"),
            UnixSeconds(19),
        )
        .unwrap();
    decrypt_own_sync_effects(&mut receiver, ready.effects);
    receiver.ack_pending_decrypted_deliveries().unwrap();
    let storage = observe_mutation_journal(&mut receiver);
    let sent = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            mutation_rumor(owner.public_key(), 1009, "FAILED_WRITE_SECRET"),
            UnixSeconds(20),
        )
        .unwrap();
    let event = sent
        .effects
        .into_iter()
        .find_map(|ProtocolEffect::Publish(publish)| {
            (publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND).then_some(publish.event)
        })
        .unwrap();
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(receiver
        .process_direct_message_event(&event)
        .unwrap_err()
        .downcast_ref::<StorageError>()
        .is_some());
    assert!(receiver
        .pending_decrypted_deliveries
        .iter()
        .all(|delivery| delivery.content.is_empty()));
    assert!(receiver
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
    assert!(receiver.pending_decrypted_deliveries.is_empty());
    assert!(receiver.pending_inbound.is_empty());
    assert!(!storage
        .writes
        .lock()
        .unwrap()
        .iter()
        .any(|value| value.contains("FAILED_WRITE_SECRET")));
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &b).unwrap();
    assert!(restored
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
}

#[test]
fn own_mutation_direct_batch_save_failure_keeps_consumed_marker_until_commit() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let peer = Keys::generate();
    let mut sender = test_engine(&owner, &a);
    let mut receiver = test_engine(&owner, &b);
    let roster = signed_app_keys(&owner, &[a.public_key(), b.public_key()], 1);
    for engine in [&mut sender, &mut receiver] {
        engine.ingest_app_keys_event(&roster).unwrap();
    }
    // Establish a session before injecting the receive checkpoint failure.
    observe_sibling_invite(&mut sender, &receiver, &b);
    let ready = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            mutation_rumor(owner.public_key(), 14, "warmup"),
            UnixSeconds(19),
        )
        .unwrap();
    decrypt_own_sync_effects(&mut receiver, ready.effects);
    receiver.ack_pending_decrypted_deliveries().unwrap();
    let storage = observe_mutation_journal(&mut receiver);
    let sent = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            mutation_rumor(owner.public_key(), 1009, "FAILED_WRITE_SECRET"),
            UnixSeconds(20),
        )
        .unwrap();
    let event = sent
        .effects
        .into_iter()
        .find_map(|ProtocolEffect::Publish(publish)| {
            (publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND).then_some(publish.event)
        })
        .unwrap();
    receiver.enter_batch();
    assert!(receiver
        .process_direct_message_event(&event)
        .unwrap()
        .is_none());
    assert!(
        receiver
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .is_empty(),
        "No content-free placeholder may reach the app"
    );
    assert!(receiver
        .pending_decrypted_deliveries
        .iter()
        .any(|delivery| delivery.discarded));
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(receiver
        .exit_batch()
        .unwrap_err()
        .downcast_ref::<StorageError>()
        .is_some());
    assert!(receiver
        .pending_decrypted_deliveries
        .iter()
        .any(|delivery| delivery.discarded));
    assert!(receiver
        .pending_decrypted_deliveries
        .iter()
        .all(|delivery| delivery.content.is_empty()));
    receiver.enter_batch();
    assert!(receiver
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
    assert!(receiver
        .pending_decrypted_deliveries
        .iter()
        .any(|delivery| delivery.discarded));
    receiver.exit_batch().unwrap();
    assert!(receiver.pending_decrypted_deliveries.is_empty());
    let writes = storage.writes.lock().unwrap().len();
    for _ in 0..3 {
        receiver.enter_batch();
        assert!(receiver
            .retry_pending_protocol(NdrUnixSeconds(30))
            .unwrap()
            .is_empty());
        receiver.exit_batch().unwrap();
    }
    assert_eq!(
        storage.writes.lock().unwrap().len(),
        writes,
        "No permanent retry/checkpoint loop"
    );
    assert!(receiver.pending_inbound.is_empty());
    assert!(!storage
        .writes
        .lock()
        .unwrap()
        .iter()
        .any(|value| value.contains("FAILED_WRITE_SECRET")));
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &b).unwrap();
    assert!(restored
        .retry_pending_protocol(NdrUnixSeconds(30))
        .unwrap()
        .is_empty());
}
