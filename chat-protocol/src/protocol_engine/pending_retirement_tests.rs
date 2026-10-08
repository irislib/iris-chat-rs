#[test]
fn pending_retirement_preserves_sealed_deliveries_and_other_sibling_intents() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let ready_device = Keys::generate();
    let late_device = Keys::generate();
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
    let old =
        nostr::EventBuilder::new(Kind::from(10451), "obsolete intent").build(owner.public_key());
    let sealed = sender
        .send_local_sibling_unsigned_event(
            owner.public_key(),
            &owner.public_key().to_hex(),
            old,
            UnixSeconds(10),
        )
        .unwrap()
        .effects;
    assert!(!sealed.is_empty());
    sender
        .send_local_sibling_unsigned_event(
            owner.public_key(),
            &owner.public_key().to_hex(),
            own_seen_rumor(&owner),
            UnixSeconds(11),
        )
        .unwrap();
    sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    assert_eq!(
        sender
            .retire_pending_local_sibling_events(|_, event| { event.kind.as_u16() == 10451 })
            .unwrap(),
        1
    );
    sender = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    let old_delivery = decrypt_own_sync_effects(&mut ready, sealed);
    assert_eq!(
        old_delivery.len(),
        1,
        "already sealed ciphertext remains deliverable"
    );
    let retry = observe_sibling_invite(&mut sender, &late, &late_device);
    let delivered = decrypt_own_sync_effects(&mut late, retry.effects);
    assert_eq!(
        delivered.len(),
        1,
        "only unrelated queued receipt reaches late sibling"
    );
    let rumor: UnsignedEvent = serde_json::from_str(&delivered[0].content).unwrap();
    assert_ne!(rumor.kind.as_u16(), 10451);
    assert!(!sender.has_pending_retry_work());
}
#[test]
fn pending_direct_retirement_is_durable_and_preserves_other_chat_and_group_work() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(peer, &peer.to_hex(), "cancel this", None, UnixSeconds(10))
        .unwrap();
    // The same recipient still needs group controls, which have their own chat ID.
    f.sender
        .send_direct_text(
            peer,
            "group:shared",
            "keep group control",
            None,
            UnixSeconds(11),
        )
        .unwrap();
    let other_owner = Keys::generate();
    let other_ready_key = Keys::generate();
    let other_late_key = Keys::generate();
    let other_ready = test_engine(&other_owner, &other_ready_key);
    let mut other_late = test_engine(&other_owner, &other_late_key);
    f.sender
        .ingest_app_keys_event(&signed_app_keys(
            &other_owner,
            &[other_ready_key.public_key(), other_late_key.public_key()],
            1,
        ))
        .unwrap();
    observe_sibling_invite(&mut f.sender, &other_ready, &other_ready_key);
    f.sender
        .send_direct_text(
            other_owner.public_key(),
            &other_owner.public_key().to_hex(),
            "keep other conversation",
            None,
            UnixSeconds(12),
        )
        .unwrap();
    assert_eq!(f.sender.pending_remote_sends.len(), 3);
    // Include a copy waiting for one of our own devices.
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
            peer,
            &peer.to_hex(),
            own_seen_rumor(&f.owner),
            UnixSeconds(13),
        )
        .unwrap();
    assert_eq!(f.sender.pending_local_sibling_sends.len(), 1);
    assert_eq!(f.sender.retire_pending_direct_sends(peer, None).unwrap(), 2);
    f.sender =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(f.sender.pending_local_sibling_sends.is_empty());
    assert_eq!(f.sender.pending_remote_sends.len(), 2);
    let retry = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    let delivered = decrypt_own_sync_effects(&mut f.late, retry.effects);
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].content.contains("keep group control"));
    let retry = observe_sibling_invite(&mut f.sender, &other_late, &other_late_key);
    let delivered = decrypt_own_sync_effects(&mut other_late, retry.effects);
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].content.contains("keep other conversation"));
}
#[test]
fn pending_direct_hold_waits_for_closed_interval_without_losing_newer_intent() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    for (body, authored) in [("before", 50), ("during", 150), ("after", 250)] {
        f.sender
            .send_direct_text_created_at(
                peer,
                &peer.to_hex(),
                body,
                None,
                UnixSeconds(authored),
                UnixSeconds(300),
            )
            .unwrap();
    }
    f.sender.hold_pending_direct_sends(peer, true);
    let ready_while_held = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    assert!(
        ready_while_held.effects.is_empty(),
        "an available device must not consume held intents"
    );
    assert_eq!(
        f.sender
            .retire_pending_direct_sends(peer, Some(199))
            .unwrap(),
        2
    );
    f.sender.hold_pending_direct_sends(peer, false);
    let retry = f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    let delivered = decrypt_own_sync_effects(&mut f.late, retry.effects);
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].content.contains("after"));
    assert!(f.sender.pending_remote_sends.is_empty());
}
