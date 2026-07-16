#[test]
fn direct_send_readiness_advances_from_appkeys_and_invite_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);

    assert_eq!(
        engine.direct_send_readiness(peer_owner.public_key()),
        DirectSendReadiness::MissingLocalAppKeys
    );

    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    assert_eq!(
        engine.direct_send_readiness(peer_owner.public_key()),
        DirectSendReadiness::MissingPeerAppKeys
    );

    observe_peer_appkeys_for_test(
        &mut engine,
        &peer_owner,
        &[peer_device.public_key()],
        1,
    );
    assert_eq!(
        engine.direct_send_readiness(peer_owner.public_key()),
        DirectSendReadiness::MissingPeerInviteOrSession
    );

    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 2);
    assert_eq!(
        engine.direct_send_readiness(peer_owner.public_key()),
        DirectSendReadiness::Ready
    );
}

#[test]
fn appcore_direct_text_queues_until_subscription_state_makes_peer_ready() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-init-queue-drain", &owner, &device);

    {
        let engine = core.protocol_engine.as_mut().expect("protocol engine");
        observe_current_device_appkeys_for_test(engine, &owner, &device);
    }

    let chat_id = peer_owner.public_key().to_hex();
    core.send_direct_message(&chat_id, "queued hello", UnixSeconds(10), None);

    let queued_message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.first())
        .expect("queued message")
        .clone();
    assert_eq!(queued_message.delivery, DeliveryState::Queued);
    assert!(queued_message.delivery_trace.outer_event_ids.is_empty());
    assert!(core.pending_relay_publishes.is_empty());

    let plan = core
        .compute_protocol_subscription_plan()
        .expect("protocol subscription plan");
    assert!(
        plan.roster_authors.contains(&chat_id),
        "queued direct message should keep peer AppKeys in subscription interest"
    );

    let peer_app_keys_event = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 11)])
        .get_event_at(peer_owner.public_key(), 11)
        .sign_with_keys(&peer_owner)
        .expect("signed peer appkeys");
    let app_keys_batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_event(&peer_app_keys_event)
        .expect("peer appkeys event");
    core.process_protocol_engine_retry_batch("test_peer_appkeys", app_keys_batch);
    let still_queued = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.first())
        .expect("still queued message");
    assert_eq!(still_queued.delivery, DeliveryState::Queued);
    assert!(core.pending_relay_publishes.is_empty());

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(12), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");
    let invite_batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .observe_invite_event(&invite_event)
        .expect("observe invite");
    core.process_protocol_engine_retry_batch("test_peer_invite", invite_batch);

    let thread = core.threads.get(&chat_id).expect("thread after drain");
    assert_eq!(thread.messages.len(), 1);
    let drained = &thread.messages[0];
    assert_ne!(
        drained.id, queued_message.id,
        "drain should replace the temporary local id with the final rumor id"
    );
    assert_eq!(drained.body, "queued hello");
    assert_eq!(drained.created_at_secs, queued_message.created_at_secs);
    assert!(!drained.delivery_trace.outer_event_ids.is_empty());
    assert!(!core.pending_relay_publishes.is_empty());
}

#[test]
fn appcore_ready_direct_text_uses_same_queue_then_drain_path() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("direct-ready-queue-drain", &owner, &device);

    {
        let engine = core.protocol_engine.as_mut().expect("protocol engine");
        observe_current_device_appkeys_for_test(engine, &owner, &device);
        engine
            .ingest_app_keys_snapshot(
                peer_owner.public_key(),
                AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 11)]),
                11,
            )
            .expect("peer appkeys");
        observe_peer_device_invite_for_test(engine, &peer_owner, &peer_device, 12);
    }

    let chat_id = peer_owner.public_key().to_hex();
    core.send_direct_message(&chat_id, "ready hello", UnixSeconds(13), None);

    let thread = core.threads.get(&chat_id).expect("thread after send");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_ne!(message.id, "1", "temporary local id should be replaced");
    assert_eq!(message.body, "ready hello");
    assert!(!message.delivery_trace.outer_event_ids.is_empty());
    assert!(!core.pending_relay_publishes.is_empty());
}

#[test]
fn group_fanout_retry_missing_roster_does_not_rewrite_persisted_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let storage = Arc::new(CountingStorage::new());
    let mut engine =
        test_protocol_engine_with_storage(&owner, &device, storage.clone() as Arc<dyn StorageAdapter>);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let create = engine
        .create_group(
            "Queued group".to_string(),
            vec![peer_owner.public_key()],
            UnixSeconds(3),
        )
        .expect("create group");
    assert!(create.effects.is_empty());
    assert!(
        engine.debug_snapshot().pending_group_fanout_count > 0,
        "test must exercise durable pending group fanout retry"
    );

    let retry_now = unix_now().get();
    let due_retry_at = retry_now.saturating_add(180);

    let quiet_before = storage.put_count();
    assert!(
        !engine.has_due_pending_retry_work(NdrUnixSeconds(retry_now)),
        "future-due group fanouts must not wake the protocol retry loop"
    );
    let quiet_batch = engine
        .retry_pending_protocol(NdrUnixSeconds(retry_now))
        .expect("retry before group fanout is due");
    assert!(
        quiet_batch.is_empty(),
        "future-due group fanouts must not refresh protocol subscriptions"
    );
    assert_eq!(
        storage.put_count(),
        quiet_before,
        "future-due group fanouts must not serialize unchanged ratchet state"
    );

    let generation_before_due = engine.debug_snapshot().subscription_generation;
    let before = storage.put_count();
    assert!(
        engine.has_due_pending_retry_work(NdrUnixSeconds(due_retry_at)),
        "due group fanouts should wake the protocol retry loop"
    );
    let batch = engine
        .retry_pending_protocol(NdrUnixSeconds(due_retry_at))
        .expect("retry missing group roster");

    assert!(
        batch.is_empty(),
        "missing-roster group retries should remain pending without fetch/backfill effects"
    );
    assert_eq!(
        storage.put_count(),
        before,
        "missing-roster group retries must not serialize unchanged ratchet state"
    );

    let generation_after_due = engine.debug_snapshot().subscription_generation;
    assert_eq!(
        generation_after_due, generation_before_due,
        "missing-roster retries should not advance subscription generation without protocol output"
    );
    let quiet_after_due = storage.put_count();
    let quiet_batch = engine
        .retry_pending_protocol(NdrUnixSeconds(due_retry_at.saturating_add(1)))
        .expect("retry before requeued group fanout is due");
    assert!(
        quiet_batch.is_empty(),
        "requeued group fanouts must stay quiet until their next retry time"
    );
    assert_eq!(
        engine.debug_snapshot().subscription_generation,
        generation_after_due,
        "quiet group retry checks must not churn subscription generations"
    );
    assert_eq!(
        storage.put_count(),
        quiet_after_due,
        "quiet group retry checks must not serialize unchanged ratchet state"
    );

    let quiet_later = storage.put_count();
    let quiet_batch = engine
        .retry_pending_protocol(NdrUnixSeconds(due_retry_at.saturating_add(30)))
        .expect("retry before stale group fanout backoff expires");
    assert!(
        quiet_batch.is_empty(),
        "stale group fanouts must back off instead of polling every two seconds"
    );
    assert_eq!(
        engine.debug_snapshot().subscription_generation,
        generation_after_due,
        "stale group fanout backoff must not churn subscription generations"
    );
    assert_eq!(
        storage.put_count(),
        quiet_later,
        "stale group fanout backoff must not serialize unchanged ratchet state"
    );
}

#[test]
fn appcore_direct_send_storage_failure_rolls_back_protocol_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut engine = test_protocol_engine_with_storage(
        &owner,
        &device,
        storage.clone() as Arc<dyn StorageAdapter>,
    );
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_appkeys_for_test(
        &mut engine,
        &peer_owner,
        &[peer_device.public_key()],
        1,
    );
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(2), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");
    engine
        .observe_invite_event(&invite_event)
        .expect("observe invite");
    let before = engine.session_manager_snapshot_for_test();

    storage.set_fail_puts(true);
    let result = engine.send_direct_text(
        peer_owner.public_key(),
        &peer_owner.public_key().to_hex(),
        "rollback",
        None,
        UnixSeconds(3),
    );

    assert!(result.is_err());
    assert_eq!(
        engine.session_manager_snapshot_for_test(),
        before,
        "failed persistence must roll back in-memory ratchet state"
    );
}

#[test]
fn appcore_group_create_storage_failure_rolls_back_protocol_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut engine = test_protocol_engine_with_storage(
        &owner,
        &device,
        storage.clone() as Arc<dyn StorageAdapter>,
    );
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let before_sessions = engine.session_manager_snapshot_for_test();
    let before_groups = engine.group_manager_snapshot_for_test();

    storage.set_fail_puts(true);
    let result = engine.create_group(
        "rollback group".to_string(),
        vec![peer_owner.public_key()],
        UnixSeconds(3),
    );

    assert!(result.is_err());
    assert_eq!(
        engine.session_manager_snapshot_for_test(),
        before_sessions,
        "failed group persistence must roll back session fanout preparation"
    );
    assert_eq!(
        engine.group_manager_snapshot_for_test(),
        before_groups,
        "failed group persistence must roll back group manager state"
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_fanout_count,
        0,
        "failed group persistence must not leave pending group fanouts in memory"
    );
}
