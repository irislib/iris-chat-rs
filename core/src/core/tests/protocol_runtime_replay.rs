#[test]
fn direct_message_storage_failure_can_be_received_again() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "direct-message-storage-retry",
        &owner,
        &device,
        storage.clone() as Arc<dyn StorageAdapter>,
    );
    let event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(),
        &sender,
        "message survives retry",
        200,
    );
    let event_id = event.id.to_hex();
    storage.set_fail_puts(true);
    core.handle_relay_event(event.clone());
    assert!(
        !core.has_seen_event(&event_id),
        "a failed receive is not a delivered message"
    );
    storage.set_fail_puts(false);
    core.handle_relay_event(event);
    let thread = core
        .threads
        .get(&sender.public_key().to_hex())
        .expect("received chat");
    assert_eq!(
        thread
            .messages
            .iter()
            .filter(|message| message.body == "message survives retry")
            .count(),
        1
    );
    assert_eq!(thread.messages[0].created_at_secs, 200);
    assert!(core.has_seen_event(&event_id));
}

#[test]
fn unrelated_save_preserves_unapplied_decrypted_message_across_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "direct-message-unapplied-restart",
        &owner,
        &device,
        storage.clone(),
    );
    let event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(),
        &sender,
        "not delivered yet",
        200,
    );
    core.protocol_engine
        .as_mut()
        .unwrap()
        .process_direct_message_event(&event)
        .expect("durably decrypt")
        .expect("decrypted delivery");
    core.persist_best_effort();
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        1
    );
    install_test_protocol_engine(&mut core, &owner, &device, storage, None, None);
    assert!(core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_pending_retry_work());
    core.retry_protocol_engine_pending_work("restart");
    assert_eq!(core.threads[&sender.public_key().to_hex()].messages[0].created_at_secs, 200);
    assert_eq!(
        core.threads[&sender.public_key().to_hex()]
            .messages
            .iter()
            .filter(|message| message.body == "not delivered yet")
            .count(),
        1
    );
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        0
    );
}

#[test]
fn failed_delivery_ack_preserves_journal_until_successful_save() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "direct-message-ack-failure",
        &owner,
        &device,
        storage.clone(),
    );
    let event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(),
        &sender,
        "ack retry",
        200,
    );
    core.protocol_engine
        .as_mut()
        .unwrap()
        .process_direct_message_event(&event)
        .expect("durably decrypt")
        .expect("decrypted delivery");
    storage.set_fail_puts(true);
    core.retry_protocol_engine_pending_work("failed_ack");
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        1
    );
    assert!(core
        .pending_decrypted_delivery_acks
        .contains(&event.id.to_hex()));
    storage.set_fail_puts(false);
    core.retry_protocol_engine_pending_work("retry_ack");
    assert_eq!(
        core.threads[&sender.public_key().to_hex()]
            .messages
            .iter()
            .filter(|message| message.body == "ack retry")
            .count(),
        1
    );
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        0
    );
    assert!(core.pending_decrypted_delivery_acks.is_empty());
}

#[test]
fn invite_response_observation_installs_session_author_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_peer_appkeys_for_test(
        &mut engine,
        &peer_owner,
        &[peer_device.public_key()],
        1,
    );

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

    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)])
        .get_event_at(peer_owner.public_key(), 12)
        .sign_with_keys(&peer_owner)
        .expect("signed peer appkeys");
    let batch = engine
        .ingest_app_keys_event(&peer_app_keys)
        .expect("peer appkeys event");
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
    assert!(outcome.effects.is_empty());
    assert_eq!(
        engine.debug_snapshot().pending_group_pairwise_payload_count,
        1
    );

    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)])
        .get_event_at(peer_owner.public_key(), 12)
        .sign_with_keys(&peer_owner)
        .expect("signed peer appkeys");
    let batch = engine
        .ingest_app_keys_event(&peer_app_keys)
        .expect("peer appkeys event");
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

#[test]
fn repeated_pending_message_does_not_rebuild_or_emit_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("pending-replay-idle", &owner, &device);
    let engine = core.protocol_engine.as_mut().unwrap();
    let invite = engine.local_invite().unwrap();
    let (mut session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            None,
            Some(peer_owner.public_key()),
        )
        .unwrap();
    engine
        .observe_invite_response_event(
            &nostr_double_ratchet::invite_response_event(&response).unwrap(),
        )
        .unwrap();
    let plan = session
        .plan_send(b"waiting for authorization", NdrUnixSeconds(200))
        .unwrap();
    let sent = session.apply_send(plan);
    let event = nostr_double_ratchet::message_event(&sent.envelope).unwrap();
    core.handle_relay_event(event.clone());
    assert!(core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_pending_inbound_direct_event_id(&event.id.to_hex()));
    drain_app_updates(&updates);
    let builds = core.debug_snapshot_build_count();
    for _ in 0..100 {
        core.handle_relay_event(event.clone());
    }
    assert!(
        updates.try_recv().is_err(),
        "unchanged pending replays must not emit app state"
    );
    assert_eq!(core.debug_snapshot_build_count(), builds);
    // A duplicate is still pending, not permanently discarded. New proof must deliver it.
    core.handle_relay_event(signed_app_keys_authorization_event(
        &peer_owner,
        peer_device.public_key(),
        201,
    ));
    let messages = &core.threads[&peer_owner.public_key().to_hex()].messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "waiting for authorization");
    assert_eq!(messages[0].created_at_secs, 200);
}

#[test]
fn legacy_message_journal_replay_keeps_original_time() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "legacy-timestamp-restart",
        &owner,
        &device,
        storage.clone(),
    );
    let engine = core.protocol_engine.as_mut().unwrap();
    let invite = engine.local_invite().unwrap();
    let (mut session, response) = invite
        .accept_with_owner(
            sender.public_key(),
            sender.secret_key().to_secret_bytes(),
            None,
            Some(sender.public_key()),
        )
        .unwrap();
    engine
        .observe_invite_response_event(
            &nostr_double_ratchet::invite_response_event(&response).unwrap(),
        )
        .unwrap();
    let plan = session
        .plan_send(b"old legacy plaintext", NdrUnixSeconds(200))
        .unwrap();
    let sent = session.apply_send(plan);
    let event = nostr_double_ratchet::message_event(&sent.envelope).unwrap();
    let decrypted = engine
        .process_direct_message_event(&event)
        .unwrap()
        .unwrap();
    assert_eq!(decrypted.content, "old legacy plaintext");
    assert_eq!(decrypted.created_at_secs, 200);
    core.persist_best_effort();
    install_test_protocol_engine(&mut core, &owner, &device, storage, None, None);
    core.retry_protocol_engine_pending_work("legacy_restart");
    let messages = &core.threads[&sender.public_key().to_hex()].messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].created_at_secs, 200);
}
