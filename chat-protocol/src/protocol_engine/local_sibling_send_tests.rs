fn own_seen_rumor(owner: &Keys) -> UnsignedEvent {
    pairwise_codec::receipt_event(
        owner.public_key(),
        pairwise_codec::ReceiptType::Seen,
        vec!["a".repeat(64)],
        pairwise_codec::EncodeOptions::new(10, 10_000),
    )
    .unwrap()
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
