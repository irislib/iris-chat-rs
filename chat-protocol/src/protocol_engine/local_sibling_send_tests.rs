fn own_seen_rumor(owner: &Keys) -> UnsignedEvent {
    pairwise_codec::receipt_event(
        owner.public_key(),
        pairwise_codec::ReceiptType::Seen,
        vec!["a".repeat(64)],
        pairwise_codec::EncodeOptions::new(10, 10_000),
    )
    .unwrap()
}

#[test]
fn sibling_retry_backoff_stays_quiet_and_delivery_progress_survives_restart() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut f = established_readiness_fixture();
    let sibling = Keys::generate();
    f.sender
        .ingest_app_keys_event(&signed_app_keys(
            &f.owner,
            &[f.device.public_key(), sibling.public_key()],
            2,
        ))
        .unwrap();
    f.sender
        .send_local_sibling_unsigned_event(
            f.peer_owner.public_key(),
            &f.peer_owner.public_key().to_hex(),
            own_seen_rumor(&f.owner),
            UnixSeconds(10),
        )
        .unwrap();
    assert_eq!(f.sender.pending_local_sibling_sends.len(), 1);
    let mut sessions = f.sender.session_manager.snapshot();
    let mut receive_only = sessions
        .users
        .iter()
        .find(|user| user.owner_pubkey == ndr_owner(f.peer_owner.public_key()))
        .unwrap()
        .devices
        .iter()
        .find(|device| device.device_pubkey == ndr_device(f.ready_device.public_key()))
        .unwrap()
        .active_session
        .clone()
        .unwrap();
    receive_only.our_current_nostr_key = None;
    assert!(!nostr_double_ratchet::Session::from_state(receive_only.clone()).can_send());
    let record = sessions
        .users
        .iter_mut()
        .find(|user| user.owner_pubkey == f.sender.local_owner)
        .unwrap()
        .devices
        .iter_mut()
        .find(|device| device.device_pubkey == ndr_device(sibling.public_key()))
        .unwrap();
    record.active_session = None;
    record.inactive_sessions = vec![receive_only];
    record.last_activity = Some(NdrUnixSeconds(100));
    f.sender.session_manager =
        SessionManager::from_snapshot(sessions, f.device.secret_key().to_secret_bytes()).unwrap();
    f.sender.persist().unwrap();
    let before = f.sender.session_manager.snapshot();
    let pending = f.sender.pending_local_sibling_sends.clone();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    f.sender.storage = storage.clone();
    assert!(f
        .sender
        .retry_pending_local_sibling_sends(NdrUnixSeconds(1_000))
        .unwrap()
        .is_empty());
    assert_eq!(f.sender.session_manager.snapshot(), before);
    assert_eq!(storage.puts.load(Relaxed), 0);
    let mut ready = test_engine(&f.owner, &sibling);
    let _queued = std::mem::take(&mut f.sender.pending_local_sibling_sends);
    observe_sibling_invite(&mut f.sender, &ready, &sibling);
    f.sender.pending_local_sibling_sends = pending;
    let before = f.sender.session_manager.snapshot();
    let pending = serde_json::to_value(&f.sender.pending_local_sibling_sends).unwrap();
    let writes = storage.puts.load(Relaxed);
    storage.fail_next.store(true, Relaxed);
    assert!(f
        .sender
        .retry_pending_local_sibling_sends(NdrUnixSeconds(1_060))
        .is_err());
    assert_eq!(f.sender.session_manager.snapshot(), before);
    assert_eq!(
        serde_json::to_value(&f.sender.pending_local_sibling_sends).unwrap(),
        pending
    );
    let effects = f
        .sender
        .retry_pending_local_sibling_sends(NdrUnixSeconds(1_060))
        .unwrap();
    assert_eq!(decrypt_own_sync_effects(&mut ready, effects).len(), 1);
    let progressed = f.sender.session_manager.snapshot();
    assert_ne!(progressed, before);
    assert_eq!(storage.puts.load(Relaxed), writes + 2);
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    assert_eq!(restored.session_manager.snapshot(), progressed);
    let writes = storage.puts.load(Relaxed);
    assert!(restored
        .retry_pending_local_sibling_sends(NdrUnixSeconds(1_120))
        .unwrap()
        .is_empty());
    assert_eq!(restored.session_manager.snapshot(), progressed);
    assert_eq!(
        storage.puts.load(Relaxed),
        writes,
        "unchanged retry after restart must not save again"
    );
    assert!(restored.pending_local_sibling_sends.is_empty());
}

fn observe_sibling_invite(
    sender: &mut ProtocolEngine,
    sibling: &ProtocolEngine,
    keys: &Keys,
) -> ProtocolRetryBatch {
    let invite = invite_unsigned_event(&sibling.local_invite().unwrap())
        .unwrap()
        .sign_with_keys(keys)
        .unwrap();
    sender.observe_invite_event(&invite).unwrap()
}

fn decrypt_own_sync_effects(
    receiver: &mut ProtocolEngine,
    effects: Vec<ProtocolEffect>,
) -> Vec<ProtocolDecryptedMessage> {
    let mut messages = Vec::new();
    for ProtocolEffect::Publish(publish) in effects {
        match publish.event.kind.as_u16() as u32 {
            INVITE_RESPONSE_KIND => {
                messages.extend(
                    receiver
                        .observe_invite_response_event(&publish.event)
                        .unwrap()
                        .direct_messages,
                );
            }
            MESSAGE_EVENT_KIND => {
                messages.extend(
                    receiver
                        .process_direct_message_event(&publish.event)
                        .unwrap(),
                );
            }
            _ => panic!("unexpected own-sync event"),
        }
    }
    messages
}

#[test]
fn own_seen_sync_delivers_ready_sibling_and_retries_missing_sibling_after_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let ready_device = Keys::generate();
    let late_device = Keys::generate();
    let peer = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut ready = test_engine(&owner, &ready_device);
    let mut late = test_engine(&owner, &late_device);
    let roster = signed_app_keys(
        &owner,
        &[
            device.public_key(),
            ready_device.public_key(),
            late_device.public_key(),
        ],
        1,
    );
    for engine in [&mut sender, &mut ready, &mut late] {
        engine.ingest_app_keys_event(&roster).unwrap();
    }
    observe_sibling_invite(&mut sender, &ready, &ready_device);
    let sent = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            own_seen_rumor(&owner),
            UnixSeconds(10),
        )
        .expect("a missing sibling must not reject the ready sibling's receipt");
    let messages = decrypt_own_sync_effects(&mut ready, sent.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, owner.public_key());
    assert_eq!(messages[0].conversation_owner, Some(peer.public_key()));
    assert!(sender.has_pending_retry_work());
    let retry = sender.retry_pending_protocol(NdrUnixSeconds(20)).unwrap();
    assert!(
        retry.effects.is_empty(),
        "a still missing sibling must not resend to ready siblings"
    );
    sender = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    let retry = observe_sibling_invite(&mut sender, &late, &late_device);
    assert_eq!(retry.effects.iter().filter(|effect| matches!(effect, ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)).count(), 1, "already delivered sibling must not be re-encrypted on every retry");
    let messages = decrypt_own_sync_effects(&mut late, retry.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, owner.public_key());
    assert_eq!(messages[0].conversation_owner, Some(peer.public_key()));
    assert!(!sender.has_pending_retry_work());
}

#[test]
fn own_seen_sync_waits_for_local_roster_and_recovers_after_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut sibling = test_engine(&owner, &sibling_device);
    let sent = sender
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            own_seen_rumor(&owner),
            UnixSeconds(10),
        )
        .unwrap();
    assert!(sent.effects.is_empty());
    assert!(
        sender.has_pending_retry_work(),
        "unknown own roster must retain the receipt instead of treating zero targets as success"
    );
    sender = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    let roster = signed_app_keys(
        &owner,
        &[device.public_key(), sibling_device.public_key()],
        1,
    );
    sender.ingest_app_keys_event(&roster).unwrap();
    sibling.ingest_app_keys_event(&roster).unwrap();
    let retry = observe_sibling_invite(&mut sender, &sibling, &sibling_device);
    assert_eq!(
        decrypt_own_sync_effects(&mut sibling, retry.effects).len(),
        1
    );
    assert!(!sender.has_pending_retry_work());
}

#[test]
fn own_seen_sync_with_known_single_device_needs_no_retry() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    engine
        .ingest_app_keys_event(&signed_app_keys(&owner, &[device.public_key()], 1))
        .unwrap();
    let sent = engine
        .send_local_sibling_unsigned_event(
            peer.public_key(),
            &peer.public_key().to_hex(),
            own_seen_rumor(&owner),
            UnixSeconds(10),
        )
        .unwrap();
    assert!(sent.effects.is_empty());
    assert!(!engine.has_pending_retry_work());
}

#[test]
fn direct_send_does_not_copy_history_to_later_siblings_after_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut sibling = test_engine(&owner, &sibling_device);
    let mut peer = test_engine(&peer_owner, &peer_device);
    let own_roster = signed_app_keys(
        &owner,
        &[device.public_key(), sibling_device.public_key()],
        1,
    );
    let peer_roster = signed_app_keys(&peer_owner, &[peer_device.public_key()], 1);
    for engine in [&mut sender, &mut sibling, &mut peer] {
        engine.ingest_app_keys_event(&own_roster).unwrap();
        engine.ingest_app_keys_event(&peer_roster).unwrap();
    }
    observe_sibling_invite(&mut sender, &peer, &peer_device);
    assert_eq!(
        sender.direct_send_readiness(peer_owner.public_key()),
        DirectSendReadiness::Ready
    );
    let sent = sender
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "peer first, sibling later",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let messages = decrypt_own_sync_effects(&mut peer, sent.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, owner.public_key());
    assert_eq!(sender.pending_local_sibling_sends.len(), 1);
    assert!(sender
        .retry_pending_protocol(NdrUnixSeconds(20))
        .unwrap()
        .effects
        .is_empty());
    // A newly linked device should receive the chat list, never this older
    // queued message body, even when retry discovers its invite first.
    let new_device = Keys::generate();
    let mut newly_added = test_engine(&owner, &new_device);
    let expanded_roster = signed_app_keys(
        &owner,
        &[
            device.public_key(),
            sibling_device.public_key(),
            new_device.public_key(),
        ],
        30,
    );
    for engine in [&mut sender, &mut sibling, &mut newly_added] {
        assert!(engine
            .ingest_app_keys_event(&expanded_roster)
            .unwrap()
            .effects
            .is_empty());
    }
    sender = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    let retry = observe_sibling_invite(&mut sender, &newly_added, &new_device);
    assert!(
        retry.effects.is_empty(),
        "newly authorized sibling must not receive an old queued body"
    );

    assert_eq!(sender.pending_local_sibling_sends.len(), 1);
    let retry = observe_sibling_invite(&mut sender, &sibling, &sibling_device);
    let messages = decrypt_own_sync_effects(&mut sibling, retry.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, owner.public_key());
    assert_eq!(
        messages[0].conversation_owner,
        Some(peer_owner.public_key())
    );
    assert!(sender.pending_local_sibling_sends.is_empty());
    assert!(sender
        .retry_pending_protocol(NdrUnixSeconds(40))
        .unwrap()
        .effects
        .is_empty());
}
