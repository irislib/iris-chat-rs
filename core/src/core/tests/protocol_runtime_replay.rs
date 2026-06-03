#[test]
fn seen_invite_event_replays_into_protocol_engine_for_queued_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("seen-invite-replay", &owner, &device);
    {
        let engine = core.protocol_engine.as_mut().expect("protocol engine");
        observe_current_device_appkeys_for_test(engine, &owner, &device);
        engine
            .ingest_app_keys_snapshot(
                peer_owner.public_key(),
                AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 4)]),
                4,
            )
            .expect("peer appkeys");
    }

    core.send_direct_message(
        &peer_owner.public_key().to_hex(),
        "queued until seen invite replays",
        UnixSeconds(5),
        None,
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .debug_snapshot()
            .pending_outbound_targets
            .contains(&peer_device.public_key().to_hex()),
        "send should wait for the peer device invite"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(6), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");

    core.remember_event(invite_event.id.to_string());
    core.handle_relay_event(invite_event);

    let debug = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert!(
        !debug
            .pending_outbound_targets
            .contains(&peer_device.public_key().to_hex()),
        "seen invite events must still rebuild protocol state and drain queued sends"
    );
}

#[test]
fn appcore_direct_send_keeps_local_sibling_probe_until_local_appkeys_and_invite_arrive() {
    let owner = Keys::generate();
    let fresh_device = Keys::generate();
    let old_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &fresh_device);

    let send = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "self sync should not be dropped",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", peer_owner.public_key().to_hex())),
        "remote owner discovery should remain queued"
    );
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "local sibling roster discovery must be queued until local AppKeys have been observed"
    );

    let local_app_keys_created_at = unix_now().get();
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(old_device.public_key(), 1),
        DeviceEntry::new(fresh_device.public_key(), local_app_keys_created_at),
    ]);
    let app_keys_batch = engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            local_app_keys,
            local_app_keys_created_at,
        )
        .expect("local appkeys");
    assert_eq!(app_keys_batch.direct_results.len(), 1);
    assert!(
        app_keys_batch.direct_results[0]
            .queued_targets
            .contains(&old_device.public_key().to_hex()),
        "local AppKeys should turn the local owner probe into the old device invite target"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(5), &mut rng);
    let old_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(old_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())),
        None,
    )
    .expect("old device invite");
    let old_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&old_invite)
        .expect("invite event")
        .sign_with_keys(&old_device)
        .expect("signed invite");

    let invite_batch = engine
        .observe_invite_event(&old_invite_event)
        .expect("observe old device invite");
    assert_eq!(invite_batch.direct_results.len(), 1);
    let retry = &invite_batch.direct_results[0];
    assert_eq!(retry.message_id, send.message_id);
    assert!(
        protocol_has_publish_target(
        &retry.effects,
            &owner.public_key().to_hex(),
            &old_device.public_key().to_hex(),
        ),
        "old local device should receive a sender-copy publish after its invite arrives"
    );
}

#[test]
fn self_direct_send_retries_to_restored_sibling_after_invite_arrives() {
    let owner = Keys::generate();
    let phone_device = Keys::generate();
    let desktop_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &phone_device);

    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![
                DeviceEntry::new(phone_device.public_key(), 1),
                DeviceEntry::new(desktop_device.public_key(), 1),
            ]),
            1,
        )
        .expect("local AppKeys");

    let send = engine
        .send_direct_text(
            owner.public_key(),
            &owner.public_key().to_hex(),
            "self message should reach the restored sibling",
            None,
            UnixSeconds(3),
        )
        .expect("self direct send");
    assert!(
        send.queued_targets
            .contains(&desktop_device.public_key().to_hex()),
        "sibling device should stay queued until its invite is observed"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(5), &mut rng);
    let desktop_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(desktop_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())),
        None,
    )
    .expect("desktop invite");
    let desktop_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&desktop_invite)
        .expect("invite event")
        .sign_with_keys(&desktop_device)
        .expect("signed invite");

    let invite_batch = engine
        .observe_invite_event(&desktop_invite_event)
        .expect("observe desktop invite");
    assert!(
        invite_batch.direct_results.iter().any(|result| {
            result.message_id == send.message_id
                && !result.queued_targets.contains(&desktop_device.public_key().to_hex())
                && protocol_has_publish_target(
        &result.effects,
                    &owner.public_key().to_hex(),
                    &desktop_device.public_key().to_hex(),
                )
        }),
        "observing the sibling invite should retry the queued self-send to that device"
    );
}

#[test]
fn appcore_local_appkeys_backfill_replaces_seeded_single_device_roster() {
    let owner = Keys::generate();
    let fresh_device = Keys::generate();
    let old_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &fresh_device);

    let send = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "older local appkeys should still discover old sibling",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "freshly seeded local device should start with owner-level sibling discovery"
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![
                DeviceEntry::new(old_device.public_key(), 1),
                DeviceEntry::new(fresh_device.public_key(), 1),
            ]),
            1,
        )
        .expect("stale local appkeys");
    assert_eq!(batch.direct_results.len(), 1);
    assert!(
        batch.direct_results[0]
            .queued_targets
            .contains(&old_device.public_key().to_hex()),
        "AppCore's merged local roster is authoritative even when its event timestamp predates the seeded local invite"
    );

    let snapshot = engine.debug_snapshot();
    let pending = snapshot
        .pending_outbound_details
        .iter()
        .find(|pending| pending.message_id == send.message_id)
        .expect("pending send detail");
    assert_eq!(
        pending.remaining_local_sibling_targets,
        vec![old_device.public_key().to_hex()]
    );
    assert!(
        !pending
            .queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "once the merged local roster is installed, retries should target the concrete old device"
    );
}

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
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
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
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");

    engine
        .observe_invite_response_event(&response_event)
        .expect("first invite response");
    let duplicate = engine
        .observe_invite_response_event(&response_event)
        .expect("duplicate invite response should be ignored");
    assert!(duplicate.direct_results.is_empty());
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
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    let plan = peer_session
        .plan_send(b"hello-before-appkeys", NdrUnixSeconds(11))
        .expect("peer plans message");
    let sent = peer_session.apply_send(plan);
    let message_event =
        nostr_double_ratchet_nostr::message_event(&sent.envelope).expect("message event");

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
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
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
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
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
fn queued_direct_send_schedules_fast_protocol_retry_tick() {
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
        "queued direct work should schedule a fast retry tick, not wait for the normal liveness interval"
    );
}
