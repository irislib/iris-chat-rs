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
