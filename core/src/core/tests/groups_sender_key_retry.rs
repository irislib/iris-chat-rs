#[test]
fn appcore_sender_key_repair_request_survives_restart_and_throttles() {
    let bob_storage =
        Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
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
    let outer = sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();
    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing rotated key");
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

    let distribution =
        latest_sender_key_distribution_for_test(&devices[alice].engine, &group_id, NdrUnixSeconds(321));
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
    let outer = sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
        .into_iter()
        .next()
        .expect("sender-key outer event")
        .clone();

    let pending = devices[bob]
        .engine
        .process_group_outer_event(&outer)
        .expect("process outer missing metadata revision");
    assert!(pending.pending);
    assert!(
        !pending.effects.is_empty(),
        "missing metadata revision should request repair"
    );

    let (_alice_events, metadata_response_effects) =
        deliver_protocol_effects_to_engine_once(&mut devices[alice].engine, &pending.effects);
    assert!(
        !metadata_response_effects.is_empty(),
        "sender should answer revision repair with metadata"
    );
    let (bob_events, _followup) = deliver_protocol_effects_to_engine_once(
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
    assert_eq!(bob.debug_snapshot().pending_group_sender_key_message_count, 1);

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
    assert_eq!(bob.debug_snapshot().pending_group_sender_key_message_count, 0);
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
