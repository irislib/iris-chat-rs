fn repair_retry_fixture() -> (ProtocolEngine, SenderKeyRepairRequest) {
    let (mut engine, _, _) = queued_group_history(1);
    let pending = &mut engine.pending_group_sender_key_messages[0];
    pending.encrypted_header = None;
    pending.key_id = 999;
    let request = SenderKeyRepairRequest {
        group_id: "queued-group".into(),
        sender_event_pubkey: pending.sender_event_pubkey,
        key_id: Some(pending.key_id),
        message_number: Some(pending.message_number),
        required_revision: None,
        created_at: NdrUnixSeconds(100),
    };
    (engine, request)
}

fn repair_retry_count_saves(engine: &mut ProtocolEngine) -> Arc<GroupRetryCountingStorage> {
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    engine.storage = storage.clone();
    storage
}

fn repair_retry_reload(engine: &ProtocolEngine) -> ProtocolEngine {
    let keys = Keys::new(SecretKey::from_slice(&engine.local_device_secret).unwrap());
    ProtocolEngine::load_or_create_for_local_device(
        engine.storage.clone(),
        engine.owner_pubkey,
        &keys,
    )
    .unwrap()
}

#[test]
fn repair_retry_backoff_only_keeps_exact_intent_without_checkpoint() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut observed = Vec::new();
    for already_dirty in [false, true] {
        let (mut engine, request) = repair_retry_fixture();
        assert!(engine
            .sender_key_repair_request_effects(request, NdrUnixSeconds(100))
            .unwrap()
            .is_empty());
        assert_eq!(engine.pending_group_sender_key_repairs.len(), 1);
        assert_eq!(engine.pending_group_fanouts.len(), 1);
        assert!(engine
            .session_manager
            .roster(engine.local_owner)
            .unwrap()
            .devices()
            .iter()
            .all(|device| device.device_pubkey == engine.local_device));
        engine.persist().unwrap();
        let before = engine.storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
        let repair_before = engine.pending_group_sender_key_repairs[0].clone();
        let fanouts = engine.pending_group_fanouts.clone();
        let ciphertexts = engine.pending_group_sender_key_messages.clone();
        let sessions = serde_json::to_value(engine.session_manager.snapshot()).unwrap();
        let groups = engine.group_manager.snapshot();
        let storage = repair_retry_count_saves(&mut engine);
        engine.enter_batch();
        if already_dirty {
            engine.subscription_generation += 1;
            engine.persist().unwrap();
        }
        let result = engine.retry_eligible_group_sender_key_messages().unwrap();
        engine.exit_batch().unwrap();
        let writes = storage.puts.load(Relaxed);
        assert!(result.effects.is_empty() && result.events.is_empty());
        assert_eq!(engine.pending_group_fanouts, fanouts);
        assert!(engine.pending_group_sender_key_messages == ciphertexts);
        assert_eq!(
            serde_json::to_value(engine.session_manager.snapshot()).unwrap(),
            sessions
        );
        assert!(engine.group_manager.snapshot() == groups);
        assert_eq!(engine.pending_group_sender_key_repairs.len(), 1);
        let repair = &engine.pending_group_sender_key_repairs[0];
        assert_eq!(repair.request_count, repair_before.request_count + 1);
        assert!(repair.last_requested_at_secs > repair_before.last_requested_at_secs);
        assert_eq!(
            repair.next_retry_at_secs,
            protocol_sender_key_repair_next_retry_at(
                NdrUnixSeconds(repair.last_requested_at_secs),
                repair.request_count
            )
            .get()
        );
        let mut expected = repair_before;
        expected.last_requested_at_secs = repair.last_requested_at_secs;
        expected.request_count = repair.request_count;
        expected.next_retry_at_secs = repair.next_retry_at_secs;
        assert_eq!(*repair, expected, "only retry bookkeeping may change");
        let unchanged_checkpoint = engine.storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap() == before;
        let restarted = repair_retry_reload(&engine);
        assert_eq!(restarted.pending_group_fanouts, fanouts);
        assert!(restarted.pending_group_sender_key_messages == ciphertexts);
        assert_eq!(
            serde_json::to_value(restarted.session_manager.snapshot()).unwrap(),
            sessions
        );
        assert_eq!(restarted.pending_group_sender_key_repairs.len(), 1);
        assert_eq!(
            restarted.pending_group_sender_key_repairs[0].key_id,
            Some(999)
        );
        if already_dirty {
            assert_eq!(
                restarted.pending_group_sender_key_repairs,
                engine.pending_group_sender_key_repairs
            );
            assert_eq!(
                restarted.subscription_generation,
                engine.subscription_generation
            );
        }
        observed.push((writes, unchanged_checkpoint));
    }
    assert_eq!(observed, vec![(0, true), (1, false)]);
}

#[test]
fn repair_retry_new_intent_and_removal_survive_save_failure_and_restart() {
    use std::sync::atomic::Ordering::Relaxed;
    let (mut engine, request) = repair_retry_fixture();
    engine.persist().unwrap();
    let sessions = serde_json::to_value(engine.session_manager.snapshot()).unwrap();
    let ciphertexts = engine.pending_group_sender_key_messages.clone();
    let storage = repair_retry_count_saves(&mut engine);
    storage.fail_next.store(true, Relaxed);
    let error = engine
        .retry_eligible_group_sender_key_messages()
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert!(engine.pending_group_sender_key_repairs.is_empty());
    assert!(engine.pending_group_fanouts.is_empty());
    assert!(engine.pending_group_sender_key_messages == ciphertexts);
    assert_eq!(
        serde_json::to_value(engine.session_manager.snapshot()).unwrap(),
        sessions
    );
    let result = engine.retry_eligible_group_sender_key_messages().unwrap();
    assert!(result.effects.is_empty());
    assert_eq!(engine.pending_group_sender_key_repairs.len(), 1);
    assert_eq!(engine.pending_group_fanouts.len(), 1);
    let mut restarted = repair_retry_reload(&engine);
    assert_eq!(
        restarted.pending_group_sender_key_repairs,
        engine.pending_group_sender_key_repairs
    );
    assert_eq!(
        restarted.pending_group_fanouts,
        engine.pending_group_fanouts
    );

    let mut groups = restarted.group_manager.snapshot();
    groups.groups[0]
        .members
        .retain(|owner| *owner != restarted.local_owner);
    restarted.group_manager = GroupEventManager::from_snapshot(groups).unwrap();
    let repairs = restarted.pending_group_sender_key_repairs.clone();
    storage.fail_next.store(true, Relaxed);
    let error = restarted
        .with_state_checkpoint(|engine| {
            engine.sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(1_000))
        })
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert_eq!(restarted.pending_group_sender_key_repairs, repairs);
    assert!(restarted
        .sender_key_repair_request_effects(request, NdrUnixSeconds(1_000))
        .unwrap()
        .is_empty());
    assert!(restarted.pending_group_sender_key_repairs.is_empty());
    assert!(repair_retry_reload(&restarted)
        .pending_group_sender_key_repairs
        .is_empty());
}

#[test]
fn repair_retry_new_recipient_fanout_is_durable_without_effects() {
    let (mut engine, request) = repair_retry_fixture();
    engine
        .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(100))
        .unwrap();
    let original = engine.pending_group_fanouts[0].clone();
    let additional = ndr_owner(Keys::generate().public_key());
    let mut groups = engine.group_manager.snapshot();
    groups.groups[0].members.push(additional);
    engine.group_manager = GroupEventManager::from_snapshot(groups).unwrap();
    engine.persist().unwrap();
    let sessions = serde_json::to_value(engine.session_manager.snapshot()).unwrap();
    let storage = repair_retry_count_saves(&mut engine);
    let effects = engine
        .sender_key_repair_request_effects(request, NdrUnixSeconds(1_000))
        .unwrap();
    assert!(effects.is_empty());
    assert_eq!(engine.pending_group_fanouts.len(), 2);
    assert_eq!(engine.pending_group_fanouts[0], original);
    assert!(matches!(engine.pending_group_fanouts[1].fanout.as_ref(),
        GroupPendingFanout::Remote { recipient_owner, .. } if *recipient_owner == additional));
    assert_eq!(
        serde_json::to_value(engine.session_manager.snapshot()).unwrap(),
        sessions
    );
    assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 1);
    let restarted = repair_retry_reload(&engine);
    assert_eq!(
        restarted.pending_group_fanouts,
        engine.pending_group_fanouts
    );
    assert_eq!(
        restarted.pending_group_sender_key_repairs,
        engine.pending_group_sender_key_repairs
    );
}

#[test]
fn repair_retry_successful_wire_and_throttle_survive_restart() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut f = remote_send_fixture();
    let (queued, _) = repair_retry_fixture();
    let mut groups = queued.group_manager.snapshot();
    groups.local_owner_pubkey = f.sender.local_owner;
    groups.groups = vec![group_snapshot_for_test(
        "queued-group",
        "Repair",
        1,
        &f.peer_owner,
        &[f.owner.public_key(), f.peer_owner.public_key()],
    )];
    groups.sender_keys[0].sender_owner = ndr_owner(f.peer_owner.public_key());
    groups.sender_keys[0].sender_device = ndr_device(f.ready_device.public_key());
    f.sender.group_manager = GroupEventManager::from_snapshot(groups).unwrap();
    f.sender.pending_group_sender_key_messages = queued.pending_group_sender_key_messages;
    f.sender.persist().unwrap();
    let storage = repair_retry_count_saves(&mut f.sender);
    let result = f.sender.retry_eligible_group_sender_key_messages().unwrap();
    assert!(!result.effects.is_empty());
    let mut messages = 0;
    for ProtocolEffect::Publish(publish) in &result.effects {
        publish.event.verify().unwrap();
        let wire = serde_json::to_string(&publish.event).unwrap();
        for identity in [&f.owner, &f.device, &f.peer_owner] {
            assert!(
                !wire.contains(&identity.public_key().to_hex()),
                "repair retry exposed an identity"
            );
        }
        messages += usize::from(publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND);
    }
    assert_eq!(messages, 1);
    let delivered = decrypt_own_sync_effects(&mut f.ready, result.effects);
    assert_eq!(delivered.len(), 1);
    assert!(
        matches!(JsonGroupPayloadCodecV1.decode_pairwise_command(delivered[0].content.as_bytes()).unwrap(),
        Some(GroupPairwiseCommand::SenderKeyRepairRequest { request }) if request.key_id == Some(999))
    );
    let repairs = f.sender.pending_group_sender_key_repairs.clone();
    let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let mut restarted = repair_retry_reload(&f.sender);
    assert_eq!(restarted.pending_group_sender_key_repairs, repairs);
    assert_eq!(
        serde_json::to_value(restarted.session_manager.snapshot()).unwrap(),
        sessions
    );
    let puts = storage.puts.load(Relaxed);
    assert!(restarted
        .retry_pending_group_sender_key_repairs(NdrUnixSeconds(repairs[0].next_retry_at_secs - 1))
        .unwrap()
        .is_empty());
    assert_eq!(storage.puts.load(Relaxed), puts);
    assert_eq!(restarted.pending_group_sender_key_repairs, repairs);
}

#[test]
fn repair_retry_gap_only_sibling_session_progress_is_durable() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut f = established_readiness_fixture();
    let sibling = Keys::generate();
    let sibling_device = ndr_device(sibling.public_key());
    f.sender
        .ingest_app_keys_event(&signed_app_keys(
            &f.owner,
            &[f.device.public_key(), sibling.public_key()],
            2,
        ))
        .unwrap();
    let (queued, request) = repair_retry_fixture();
    let mut groups = queued.group_manager.snapshot();
    for owner in &mut groups.groups[0].members {
        if *owner == groups.local_owner_pubkey {
            *owner = f.sender.local_owner;
        }
    }
    groups.local_owner_pubkey = f.sender.local_owner;
    f.sender.group_manager = GroupEventManager::from_snapshot(groups).unwrap();
    assert!(f
        .sender
        .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(100))
        .unwrap()
        .is_empty());
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
        .find(|device| device.device_pubkey == sibling_device)
        .unwrap();
    assert!(record.authorized && !record.is_stale && record.public_invite.is_none());
    record.active_session = None;
    record.inactive_sessions = vec![receive_only.clone()];
    record.last_activity = Some(NdrUnixSeconds(100));
    f.sender.session_manager =
        SessionManager::from_snapshot(sessions, f.device.secret_key().to_secret_bytes()).unwrap();
    f.sender.persist().unwrap();
    let before = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let repairs = f.sender.pending_group_sender_key_repairs.clone();
    let fanouts = f.sender.pending_group_fanouts.clone();
    let storage = repair_retry_count_saves(&mut f.sender);
    storage.fail_next.store(true, Relaxed);
    let error = f
        .sender
        .with_state_checkpoint(|engine| {
            engine.sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(1_000))
        })
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert_eq!(
        serde_json::to_value(f.sender.session_manager.snapshot()).unwrap(),
        before
    );
    assert_eq!(f.sender.pending_group_sender_key_repairs, repairs);
    assert_eq!(f.sender.pending_group_fanouts, fanouts);
    assert!(f
        .sender
        .sender_key_repair_request_effects(request, NdrUnixSeconds(1_000))
        .unwrap()
        .is_empty());
    assert_eq!(storage.puts.load(Relaxed), 2);
    let after = f.sender.session_manager.snapshot();
    let record = after
        .users
        .iter()
        .find(|user| user.owner_pubkey == f.sender.local_owner)
        .unwrap()
        .devices
        .iter()
        .find(|device| device.device_pubkey == sibling_device)
        .unwrap();
    assert_eq!(record.active_session.as_ref(), Some(&receive_only));
    assert!(record.inactive_sessions.is_empty());
    assert_eq!(record.last_activity, Some(NdrUnixSeconds(1_000)));
    assert_eq!(
        f.sender.pending_group_sender_key_repairs[0].request_count,
        repairs[0].request_count + 1
    );
    assert_eq!(
        f.sender.pending_group_sender_key_repairs[0].last_requested_at_secs,
        1_000
    );
    let restarted = repair_retry_reload(&f.sender);
    assert_eq!(
        serde_json::to_value(restarted.session_manager.snapshot()).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(
        restarted.pending_group_sender_key_repairs,
        f.sender.pending_group_sender_key_repairs
    );
    assert_eq!(
        restarted.pending_group_fanouts,
        f.sender.pending_group_fanouts
    );
}
