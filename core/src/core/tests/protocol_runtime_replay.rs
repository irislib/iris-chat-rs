#[test]
fn invite_response_observation_installs_session_author_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");

    let invite = engine.local_invite().expect("local invite");
    let (_peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet::invite_response_event(&response)
        .expect("invite response event");

    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    assert!(
        !engine
            .message_author_pubkeys_for_owner(peer_owner.public_key())
            .is_empty(),
        "observing the invite response should install receiver state for the peer"
    );
}

#[test]
fn invite_response_replay_after_consumed_invite_is_idempotent() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");

    let invite = engine.local_invite().expect("local invite");
    let (_peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet::invite_response_event(&response)
        .expect("invite response event");

    engine
        .observe_invite_response_event(&response_event)
        .expect("first invite response");
    let duplicate = engine
        .observe_invite_response_event(&response_event)
        .expect("duplicate invite response should be ignored");
    assert!(duplicate.direct_messages.is_empty());
    assert!(duplicate.effects.is_empty());
    assert!(duplicate.group_result.events.is_empty());
    assert!(duplicate.group_result.effects.is_empty());
}

#[test]
fn appcore_direct_message_from_unverified_claimed_owner_retries_after_appkeys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);

    let invite = engine.local_invite().expect("local invite");
    let (mut peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet::invite_response_event(&response)
        .expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    let plan = peer_session
        .plan_send(b"hello-before-appkeys", NdrUnixSeconds(11))
        .expect("peer plans message");
    let sent = peer_session.apply_send(plan);
    let message_event =
        nostr_double_ratchet::message_event(&sent.envelope).expect("message event");

    let decrypted = engine
        .process_direct_message_event(&message_event)
        .expect("process direct message");
    assert!(
        decrypted.is_none(),
        "claimed-owner messages must wait until the owner claim is verified"
    );
    assert_eq!(engine.debug_snapshot().pending_inbound_count, 1);
    let sender_message_pubkey_hex = sent.envelope.sender.to_hex();
    let peer_owner_hex = peer_owner.public_key().to_hex();
    let pending_inbound = engine.pending_inbound_for_test();
    let pending = pending_inbound.first().expect("pending inbound");
    assert_eq!(pending.event_id, message_event.id.to_string());
    assert!(
        pending.has_envelope,
        "pending inbound must store the parsed envelope so retries do not verify the outer event again"
    );
    assert_eq!(
        pending.sender_message_pubkey_hex.as_deref(),
        Some(sender_message_pubkey_hex.as_str())
    );
    assert_eq!(
        pending.claimed_owner_pubkey_hex.as_deref(),
        Some(peer_owner_hex.as_str())
    );
    assert!(
        pending.metadata_verified,
        "queued pending inbound metadata should be produced by the already verified parse"
    );
    assert_eq!(
        engine.queued_owner_claim_targets(),
        vec![format!("owner:{}", peer_owner.public_key().to_hex())]
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)]),
            12,
        )
        .expect("peer appkeys");
    assert_eq!(batch.direct_messages.len(), 1);
    assert_eq!(batch.direct_messages[0].sender, peer_owner.public_key());
    assert_eq!(
        batch.direct_messages[0].sender_device,
        Some(peer_device.public_key())
    );
    assert_eq!(batch.direct_messages[0].content, "hello-before-appkeys");
    assert_eq!(engine.debug_snapshot().pending_inbound_count, 0);
}

#[test]
fn appcore_pending_group_payload_from_claimed_device_uses_owner_after_appkeys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);

    let invite = engine.local_invite().expect("local invite");
    let (_peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet::invite_response_event(&response)
        .expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    let group_id = "claimed-owner-group".to_string();
    let snapshot = test_group_snapshot(
        &group_id,
        "Claimed Owner Group",
        peer_owner.public_key(),
        vec![peer_owner.public_key(), owner.public_key()],
        vec![peer_owner.public_key()],
        1,
    );
    let codec = nostr_double_ratchet::JsonGroupPayloadCodecV1;
    let payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(peer_device.public_key()),
            created_at: NdrUnixSeconds(11),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("group metadata payload");

    let outcome = engine
        .process_group_pairwise_payload(
            &payload,
            peer_device.public_key(),
            Some(peer_device.public_key()),
        )
        .expect("process group payload");
    assert!(outcome.consumed);
    assert!(outcome.events.is_empty());
    assert_eq!(
        outcome.queued_targets,
        vec![format!("owner:{}", peer_owner.public_key().to_hex())]
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_pairwise_payload_count,
        1
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)]),
            12,
        )
        .expect("peer appkeys");
    let created = batch
        .group_result
        .events
        .iter()
        .find_map(|event| match event {
            GroupIncomingEvent::MetadataUpdated(snapshot) if snapshot.group_id == group_id => {
                Some(snapshot)
            }
            _ => None,
        })
        .expect("group metadata applied after owner claim verification");
    assert_eq!(
        created.created_by,
        ndr_owner_pubkey(peer_owner.public_key())
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_pairwise_payload_count,
        0
    );
}

#[test]
fn queued_direct_send_schedules_subscription_liveness_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-direct-fast-retry", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.send_direct_message(
        &peer.public_key().to_hex(),
        "queued until app keys arrive",
        UnixSeconds(1_777_000_000),
        None,
    );

    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("queued protocol work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "queued direct work should schedule a fast subscription liveness tick, not wait for the normal liveness interval"
    );
}
