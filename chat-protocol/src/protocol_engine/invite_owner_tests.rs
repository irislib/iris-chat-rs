#[test]
fn verified_invite_acceptance_preserves_full_signed_roster() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let sibling_device = Keys::generate();
    let invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let mut engine = test_engine(&local_owner, &local_device);
    let roster = signed_app_keys(
        &claimed_owner,
        &[inviter_device.public_key(), sibling_device.public_key()],
        10,
    );
    engine
        .ingest_app_keys_event(&roster)
        .expect("verified roster");

    let accepted = engine
        .accept_invite(&invite, Some(claimed_owner.public_key()))
        .expect("accept outcome");
    assert!(matches!(accepted, ProtocolAcceptInviteOutcome::Accepted(_)));

    let roster = engine
        .session_manager_snapshot_for_test()
        .users
        .into_iter()
        .find(|user| user.owner_pubkey == ndr_owner(claimed_owner.public_key()))
        .and_then(|user| user.roster)
        .expect("preserved roster");
    assert!(roster
        .get_device(&ndr_device(inviter_device.public_key()))
        .is_some());
    assert!(roster
        .get_device(&ndr_device(sibling_device.public_key()))
        .is_some());

    let second_invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let second = engine
        .accept_invite(&second_invite, Some(claimed_owner.public_key()))
        .expect("second invite outcome");
    assert!(matches!(second, ProtocolAcceptInviteOutcome::Accepted(_)));
}

#[test]
fn signed_invite_owner_evidence_survives_restart() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let claimed_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let invite = claimed_owner_invite(&claimed_owner, &inviter_device);
    let storage = Arc::new(InMemoryStorage::new());
    let adapter: Arc<dyn StorageAdapter> = storage.clone();
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        adapter,
        local_owner.public_key(),
        &local_device,
    )
    .expect("protocol engine");
    let roster = signed_app_keys(&claimed_owner, &[inviter_device.public_key()], 10);
    engine
        .ingest_app_keys_event(&roster)
        .expect("signed owner roster");
    drop(engine);

    let adapter: Arc<dyn StorageAdapter> = storage;
    assert!(ProtocolEngine::persisted_invite_owner_device_is_authorized(
        adapter.clone(),
        local_owner.public_key(),
        &local_device,
        claimed_owner.public_key(),
        inviter_device.public_key(),
    )
    .expect("persisted exact evidence"));
    let mut restarted = ProtocolEngine::load_or_create_for_local_device(
        adapter,
        local_owner.public_key(),
        &local_device,
    )
    .expect("restarted engine");
    let outcome = restarted
        .accept_invite(&invite, Some(claimed_owner.public_key()))
        .expect("restart invite outcome");
    assert!(matches!(outcome, ProtocolAcceptInviteOutcome::Accepted(_)));
}

#[test]
fn signed_app_keys_event_promotes_parked_invite_response_session() {
    let local_owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_engine(&local_owner, &local_device);

    let local_invite = engine.local_invite().expect("local invite");
    let (_peer_session, response) = local_invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = invite_response_event(&response).expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    assert_eq!(engine.active_session_count_for_owner(peer_owner.public_key()), 0);

    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 10)]),
            10,
        )
        .expect("projected peer roster");
    assert_eq!(
        engine.active_session_count_for_owner(peer_owner.public_key()),
        0,
        "a roster projection must not promote a claimed-owner session"
    );

    let signed = signed_app_keys(&peer_owner, &[peer_device.public_key()], 10);
    engine
        .ingest_app_keys_event(&signed)
        .expect("signed peer AppKeys");

    assert_eq!(engine.active_session_count_for_owner(peer_owner.public_key()), 1);
    assert!(engine
        .session_manager_snapshot_for_test()
        .verified_peer_app_keys_events
        .iter()
        .any(|event| event.id == signed.id));
}

#[test]
fn upgrade_restores_receive_authorization_from_existing_signed_evidence() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    let roster = signed_app_keys(&peer_owner, &[peer_device.public_key()], 10);
    engine.ingest_app_keys_event(&roster).unwrap();
    let (mut peer_session, response) = engine
        .local_invite()
        .unwrap()
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .unwrap();
    engine
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    let plan = peer_session
        .plan_send(b"message after upgrade", NdrUnixSeconds(20))
        .unwrap();
    let message =
        nostr_double_ratchet::message_event(&peer_session.apply_send(plan).envelope).unwrap();
    let mut old: serde_json::Value =
        serde_json::from_str(&storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap()).unwrap();
    // Earlier versions kept the signed evidence in ProtocolEngine but the
    // ratchet library did not yet have its own signed-evidence index.
    old["session_manager"]
        .as_object_mut()
        .unwrap()
        .remove("verified_peer_app_keys_events");
    storage
        .put(PROTOCOL_ENGINE_STATE_KEY, old.to_string())
        .unwrap();
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    let received = restored
        .process_direct_message_event(&message)
        .unwrap()
        .expect("existing signed owner proof authorizes reception after upgrade");
    assert_eq!(received.sender, peer_owner.public_key());
    assert_eq!(received.content, "message after upgrade");

    // An older outer proof cannot resurrect a device revoked by a newer
    // signed head already present in the ratchet's index.
    let revoked = signed_app_keys(&peer_owner, &[], 30);
    old["session_manager"]["verified_peer_app_keys_events"] = serde_json::json!([revoked]);
    storage
        .put(PROTOCOL_ENGINE_STATE_KEY, old.to_string())
        .unwrap();
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
            .unwrap();
    assert!(restored
        .process_direct_message_event(&message)
        .unwrap()
        .is_none());
}
