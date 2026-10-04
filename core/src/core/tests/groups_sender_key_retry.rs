#[test]
fn appcore_sender_key_repair_request_survives_restart_and_throttles() {
    let bob_storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut devices = (0..3)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    devices[1].engine = test_protocol_engine_with_storage(
        &devices[1].owner,
        &devices[1].device,
        bob_storage.clone(),
    );
    observe_sender_key_matrix_protocol_state(&mut devices);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner = devices[bob].owner.clone();
    let bob_device = devices[bob].device.clone();
    let bob_owner_pubkey = bob_owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key repair restart".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(304),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let removed = devices[alice]
        .engine
        .remove_group_member(&group_id, carol_owner_pubkey)
        .expect("remove carol and rotate sender key");
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &removed.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"repair after restart".to_vec(),
            Some("sender-key-repair-restart-inner".to_string()),
        )
        .expect("send with rotated sender key");
    let outer =
        sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
            .into_iter()
            .next()
            .expect("sender-key outer event")
            .clone();
    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing rotated key");
    let pending = finish_group_search_for_test(&mut devices[bob].engine, pending);
    assert!(!pending.effects.is_empty());
    let before_restart = devices[bob].engine.debug_snapshot();
    assert_eq!(before_restart.pending_group_sender_key_repair_count, 1);
    assert!(before_restart.pending_group_sender_key_repair_last_requested_at_secs > 0);

    devices[bob].engine =
        test_protocol_engine_with_storage(&bob_owner, &bob_device, bob_storage.clone());
    let after_restart = devices[bob].engine.debug_snapshot();
    assert_eq!(after_restart.pending_group_sender_key_repair_count, 1);
    assert_eq!(
        after_restart.pending_group_sender_key_repair_last_requested_at_secs,
        before_restart.pending_group_sender_key_repair_last_requested_at_secs
    );

    let early = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_restart
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(1),
        ))
        .expect("early retry");
    assert!(
        early.group_result.effects.is_empty(),
        "repair request should be throttled before retry delay"
    );

    let late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_restart
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(11),
        ))
        .expect("late retry");
    assert!(
        !late.group_result.effects.is_empty(),
        "repair request should be re-emitted after retry delay"
    );

    let after_late_retry = devices[bob].engine.debug_snapshot();
    let second_early = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(11),
        ))
        .expect("second early retry");
    assert!(
        second_early.group_result.effects.is_empty(),
        "repair request should back off after the second request"
    );

    let second_late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(31),
        ))
        .expect("second late retry");
    assert!(
        !second_late.group_result.effects.is_empty(),
        "repair request should re-emit after the backoff delay"
    );

    let after_second_late_retry = devices[bob].engine.debug_snapshot();
    let third_late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_second_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(61),
        ))
        .expect("third late retry");
    assert!(
        !third_late.group_result.effects.is_empty(),
        "repair retry backoff should stay capped at one minute"
    );

    let after_third_late_retry = devices[bob].engine.debug_snapshot();
    let fourth_late = devices[bob]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            after_third_late_retry
                .pending_group_sender_key_repair_last_requested_at_secs
                .saturating_add(61),
        ))
        .expect("fourth late retry");
    assert!(
        !fourth_late.group_result.effects.is_empty(),
        "later repair retries should also stay capped at one minute"
    );
}

#[test]
fn appcore_sender_key_missing_metadata_revision_repairs_and_applies_pending_outer() {
    let mut devices = sender_key_matrix_devices(3);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let bob_owner = devices[bob].owner.public_key();
    let carol_owner = devices[carol].owner.public_key();
    let alice_owner = devices[alice].owner.public_key();
    let alice_device = devices[alice].device.public_key();

    let created = devices[alice]
        .engine
        .create_group(
            "sender-key metadata repair".to_string(),
            vec![bob_owner, carol_owner],
            UnixSeconds(320),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &created.effects);

    let removed = devices[alice]
        .engine
        .remove_group_member(&group_id, carol_owner)
        .expect("remove carol and rotate sender key");
    deliver_protocol_effects_to_engine(&mut devices[carol].engine, &removed.effects);

    let distribution = latest_sender_key_distribution_for_test(
        &devices[alice].engine,
        &group_id,
        NdrUnixSeconds(321),
    );
    let codec = nostr_double_ratchet::JsonGroupPayloadCodecV1;
    let distribution_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device),
            created_at: NdrUnixSeconds(321),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { distribution },
    )
    .expect("sender-key distribution payload");
    devices[bob]
        .engine
        .process_group_pairwise_payload(&distribution_payload, alice_owner, Some(alice_device))
        .expect("process rotated distribution without metadata");

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"after missed metadata".to_vec(),
            Some("sender-key-metadata-repair-inner".to_string()),
        )
        .expect("send after metadata gap");
    let outer =
        sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
            .into_iter()
            .next()
            .expect("sender-key outer event")
            .clone();

    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing metadata revision");
    let pending = finish_group_search_for_test(&mut devices[bob].engine, pending);
    assert!(pending.pending);
    assert!(
        !pending.effects.is_empty(),
        "missing metadata revision should request repair"
    );

    let (_alice_events, metadata_response_effects) =
        deliver_protocol_effects_with_ready_work_once(&mut devices[alice].engine, &pending.effects);
    assert!(
        !metadata_response_effects.is_empty(),
        "sender should answer revision repair with metadata"
    );
    let (bob_events, _followup) = deliver_protocol_effects_with_ready_work_once(
        &mut devices[bob].engine,
        &metadata_response_effects,
    );
    assert!(
        group_events_contain_body(
            &bob_events,
            &group_id,
            alice_owner,
            alice_device,
            b"after missed metadata"
        ),
        "pending sender-key outer should apply after metadata repair"
    );
}

#[test]
fn appcore_sender_key_distribution_before_metadata_wakes_and_applies_pending_outer() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = test_protocol_engine(&alice_owner, &alice_device);
    let mut bob = test_protocol_engine(&bob_owner, &bob_device);
    observe_current_device_appkeys_for_test(&mut alice, &alice_owner, &alice_device);
    observe_current_device_appkeys_for_test(&mut bob, &bob_owner, &bob_device);
    observe_peer_device_invite_for_test(&mut alice, &bob_owner, &bob_device, 400);
    observe_peer_device_invite_for_test(&mut bob, &alice_owner, &alice_device, 400);

    let created = alice
        .create_group(
            "sender-key out of order".to_string(),
            vec![bob_owner.public_key()],
            UnixSeconds(401),
        )
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group");
    let group_id = group.group_id.clone();
    let distribution =
        latest_sender_key_distribution_for_test(&alice, &group_id, NdrUnixSeconds(401));

    let sent = alice
        .send_group_payload(
            &group_id,
            b"distribution before metadata".to_vec(),
            Some("sender-key-out-of-order-inner".to_string()),
        )
        .expect("send sender-key group payload");
    let outer = sender_key_outer_events_for_engine(&alice, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();
    let pending_outer = bob
        .process_group_outer_event(&outer)
        .expect("process outer before metadata");
    assert!(pending_outer.consumed);
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        1
    );

    let codec = nostr_double_ratchet::JsonGroupPayloadCodecV1;
    let distribution_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(402),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { distribution },
    )
    .expect("sender-key distribution payload");
    let queued_distribution = bob
        .process_group_pairwise_payload(
            &distribution_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("queue distribution before metadata");
    assert!(queued_distribution.consumed);
    assert_eq!(bob.debug_snapshot().pending_group_pairwise_payload_count, 1);

    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(403),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot: group },
    )
    .expect("metadata payload");
    let metadata_result = bob
        .process_group_pairwise_payload(
            &metadata_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("process metadata after queued distribution");

    assert!(
        !bob.known_group_sender_event_pubkeys().is_empty(),
        "metadata must wake queued sender-key distribution immediately"
    );
    assert_eq!(bob.debug_snapshot().pending_group_pairwise_payload_count, 0);
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        0
    );
    assert!(
        group_events_contain_body(
            &metadata_result.events,
            &group_id,
            alice_owner.public_key(),
            alice_device.public_key(),
            b"distribution before metadata"
        ),
        "pending sender-key outer should apply once metadata wakes the queued distribution"
    );
}

fn add_legacy_sender_second_ack(storage: &dyn StorageAdapter, group_id: &str, outer: &Event) {
    let mut state: serde_json::Value = serde_json::from_str(
        &storage
            .get(TEST_PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    state["delivered_group_sender_key_acks"] = serde_json::json!([{
        "group_id": group_id,
        "sender_event_pubkey_hex": outer.pubkey.to_hex(),
        "created_at_secs": outer.created_at.as_secs(),
    }]);
    storage
        .put(TEST_PROTOCOL_ENGINE_STATE_KEY, state.to_string())
        .unwrap();
}

#[test]
fn delivered_group_outer_replay_is_quiet_after_restart_and_event_cache_eviction() {
    let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut devices = (0..2)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    observe_sender_key_matrix_protocol_state(&mut devices);
    let recipient = devices[1].owner.public_key();
    let created = devices[0]
        .engine
        .create_group(
            "Exact replay acknowledgement".into(),
            vec![recipient],
            UnixSeconds(700),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    let sent = devices[0]
        .engine
        .send_group_payload(
            &group_id,
            b"already delivered".to_vec(),
            Some("exact-replay-inner".into()),
        )
        .unwrap();
    let outer =
        sender_key_outer_events_for_engine(&devices[0].engine, &sent.effects, &sent.event_ids)
            .into_iter()
            .next()
            .unwrap()
            .clone();
    let received = devices[1].engine.process_group_outer_event(&outer).unwrap();
    assert_eq!(received.events.len(), 1);
    add_legacy_sender_second_ack(storage.as_ref(), &group_id, &outer);
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    // The protocol boundary must remain safe after the app's unrelated-event
    // LRU has evicted this outer event, including a restart with older acks.
    let mut core = logged_in_test_core(
        "group-outer-replay-lru",
        &devices[1].owner,
        &devices[1].device,
    );
    core.remember_event(outer.id.to_hex());
    for index in 0..=MAX_SEEN_EVENT_IDS {
        core.remember_event(format!("unrelated-{index}"));
    }
    assert!(!core.has_seen_event(&outer.id.to_hex()));
    for _ in 0..3 {
        let replay = devices[1].engine.process_group_outer_event(&outer).unwrap();
        assert!(replay.consumed && !replay.pending);
        assert!(replay.events.is_empty());
        assert!(
            replay.effects.is_empty(),
            "delivered replay must not publish a repair"
        );
        assert_eq!(
            devices[1]
                .engine
                .debug_snapshot()
                .pending_group_sender_key_repair_count,
            0
        );
    }

    // A pre-upgrade checkpoint cannot identify already-delivered ciphertext.
    // It must retain one repair and its cooldown, rather than creating a fresh
    // encrypted request on each replay or timer tick.
    let mut old_state: serde_json::Value = serde_json::from_str(
        &storage
            .get(TEST_PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    old_state
        .as_object_mut()
        .unwrap()
        .remove("processed_group_sender_key_messages");
    storage
        .put(TEST_PROTOCOL_ENGINE_STATE_KEY, old_state.to_string())
        .unwrap();
    add_legacy_sender_second_ack(storage.as_ref(), &group_id, &outer);
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    let mut first_old_replay = devices[1].engine.process_group_outer_event(&outer).unwrap();
    // Blind recovery yields within a search. Its one repair is produced only
    // after the unchanged full key window has been exhausted.
    for _ in 0..128 {
        if !devices[1].engine.has_ready_group_sender_key_retry_work() {
            break;
        }
        let retry = devices[1]
            .engine
            .retry_pending_protocol(NdrUnixSeconds(701))
            .unwrap();
        assert!(retry.group_result.events.is_empty());
        first_old_replay.effects.extend(retry.group_result.effects);
    }
    assert!(!devices[1].engine.has_ready_group_sender_key_retry_work());
    assert!(first_old_replay.pending && !first_old_replay.effects.is_empty());
    let pending = devices[1].engine.debug_snapshot();
    assert_eq!(pending.pending_group_sender_key_repair_count, 1);
    let checkpoint = storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap();
    for _ in 0..20 {
        let replay = devices[1].engine.process_group_outer_event(&outer).unwrap();
        assert!(replay.pending && replay.effects.is_empty());
        let retry = devices[1]
            .engine
            .retry_pending_protocol(NdrUnixSeconds(
                pending.pending_group_sender_key_repair_last_requested_at_secs + 1,
            ))
            .unwrap();
        assert!(retry.group_result.effects.is_empty());
    }
    assert_eq!(
        storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY).unwrap(),
        checkpoint,
        "duplicate admission and early retry must not churn the checkpoint or repair cooldown"
    );
}

#[test]
fn legacy_sender_second_ack_preserves_unknown_ciphertext_and_repair_backoff() {
    let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut devices = (0..3)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    observe_sender_key_matrix_protocol_state(&mut devices);
    let recipient = devices[1].owner.public_key();
    let removed_member = devices[2].owner.public_key();
    let created = devices[0]
        .engine
        .create_group(
            "Same second is not identity".into(),
            vec![recipient, removed_member],
            UnixSeconds(710),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    devices[0]
        .engine
        .remove_group_member(&group_id, removed_member)
        .unwrap();
    let sent = devices[0]
        .engine
        .send_group_payload(
            &group_id,
            b"a different message in the same second".to_vec(),
            Some("same-second-unseen".into()),
        )
        .unwrap();
    let outer =
        sender_key_outer_events_for_engine(&devices[0].engine, &sent.effects, &sent.event_ids)
            .into_iter()
            .next()
            .unwrap()
            .clone();
    add_legacy_sender_second_ack(storage.as_ref(), &group_id, &outer);
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    let first = devices[1].engine.process_group_outer_event(&outer).unwrap();
    let first = finish_group_search_for_test(&mut devices[1].engine, first);
    assert!(first.pending && !first.effects.is_empty());
    let pending = devices[1].engine.debug_snapshot();
    assert_eq!(
        pending.pending_group_sender_key_message_count, 1,
        "a time-only acknowledgement cannot discard an unknown same-second ciphertext"
    );
    assert_eq!(pending.pending_group_sender_key_repair_count, 1);
    for _ in 0..10 {
        let replay = devices[1].engine.process_group_outer_event(&outer).unwrap();
        assert!(replay.consumed && replay.pending);
        assert!(replay.events.is_empty() && replay.effects.is_empty());
    }
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage);
    let early = devices[1]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            pending.pending_group_sender_key_repair_last_requested_at_secs + 1,
        ))
        .unwrap();
    assert!(early.group_result.effects.is_empty());
    let due = devices[1]
        .engine
        .retry_pending_protocol(NdrUnixSeconds(
            pending.pending_group_sender_key_repair_last_requested_at_secs + 11,
        ))
        .unwrap();
    assert!(
        !due.group_result.effects.is_empty(),
        "missing-key repair must still resume"
    );
    let (_, response) =
        deliver_protocol_effects_with_ready_work_once(&mut devices[0].engine, &due.group_result.effects);
    let (received, _) = deliver_protocol_effects_with_ready_work_once(&mut devices[1].engine, &response);
    assert!(group_events_contain_body(
        &received,
        &group_id,
        devices[0].owner.public_key(),
        devices[0].device.public_key(),
        b"a different message in the same second"
    ));
    let replay = devices[1].engine.process_group_outer_event(&outer).unwrap();
    assert!(
        replay.consumed && !replay.pending && replay.events.is_empty() && replay.effects.is_empty()
    );
}
