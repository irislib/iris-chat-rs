fn deferred_registration_core(label: &str) -> (AppCore, Keys, Keys) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core(label, &owner, &device);
    core.app_keys.remove(&owner.public_key().to_hex());
    core.defer_owner_app_keys_publish = true;
    (core, owner, device)
}

#[test]
fn restored_account_registers_after_completed_empty_server_lookup() {
    let (mut core, owner, device) = deferred_registration_core("empty-registration-recovery");
    core.complete_owner_registration_lookup(
        core.relay_status_watch_generation,
        owner.public_key(),
        device.public_key(),
        1,
        1,
        vec![],
    );
    assert!(!core.defer_owner_app_keys_publish);
    let own = &core.app_keys[&owner.public_key().to_hex()];
    assert_eq!(own.devices.len(), 1);
    assert_eq!(
        own.devices[0].identity_pubkey_hex,
        device.public_key().to_hex()
    );
    let published = core
        .pending_relay_publishes
        .values()
        .find(|pending| pending.label == "app-keys")
        .expect("registration must be queued durably");
    let event: Event = serde_json::from_str(&published.event_json).unwrap();
    assert!(event.verify().is_ok());
    assert!(AppKeys::from_event(&event)
        .unwrap()
        .get_device(&device.public_key())
        .is_some());
}

#[test]
fn incomplete_registration_lookup_never_publishes_an_empty_replacement() {
    let (mut core, owner, device) = deferred_registration_core("incomplete-registration-recovery");
    for (completed, queried) in [(0, 0), (0, 1), (1, 2)] {
        core.complete_owner_registration_lookup(
            core.relay_status_watch_generation,
            owner.public_key(),
            device.public_key(),
            completed,
            queried,
            vec![],
        );
        assert!(core.defer_owner_app_keys_publish);
        assert!(!core.app_keys.contains_key(&owner.public_key().to_hex()));
    }
}

#[test]
fn registration_recovery_migrates_signed_legacy_devices_without_reviving_current_revocations() {
    let (mut core, owner, device) = deferred_registration_core("legacy-registration-recovery");
    let old_device = Keys::generate();
    let legacy = EventBuilder::new(Kind::Custom(30078), "")
        .tags([
            nostr::Tag::parse(["d", "double-ratchet/app-keys"]).unwrap(),
            nostr::Tag::parse(vec![
                "device".to_string(),
                old_device.public_key().to_hex(),
                "10".to_string(),
            ])
            .unwrap(),
        ])
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&owner)
        .unwrap();
    core.complete_owner_registration_lookup(
        core.relay_status_watch_generation,
        owner.public_key(),
        device.public_key(),
        1,
        1,
        vec![legacy.clone()],
    );
    assert_eq!(core.app_keys[&owner.public_key().to_hex()].devices.len(), 2);

    let current = AppKeys::new(vec![])
        .get_encrypted_event_at(&owner, 20)
        .unwrap()
        .sign_with_keys(&owner)
        .unwrap();
    core.app_keys.remove(&owner.public_key().to_hex());
    core.defer_owner_app_keys_publish = true;
    core.complete_owner_registration_lookup(
        core.relay_status_watch_generation,
        owner.public_key(),
        device.public_key(),
        1,
        1,
        vec![legacy, current],
    );
    let devices = &core.app_keys[&owner.public_key().to_hex()].devices;
    assert_eq!(
        devices.len(),
        1,
        "current revocation must win over a legacy device list"
    );
    assert_eq!(devices[0].identity_pubkey_hex, device.public_key().to_hex());
}

#[test]
fn stale_registration_lookup_cannot_register_a_different_session() {
    let (mut core, owner, device) = deferred_registration_core("stale-registration-recovery");
    core.complete_owner_registration_lookup(
        core.relay_status_watch_generation.wrapping_add(1),
        owner.public_key(),
        device.public_key(),
        1,
        1,
        vec![],
    );
    assert!(core.defer_owner_app_keys_publish);
    core.complete_owner_registration_lookup(
        core.relay_status_watch_generation,
        owner.public_key(),
        Keys::generate().public_key(),
        1,
        1,
        vec![],
    );
    assert!(core.defer_owner_app_keys_publish);
}

#[test]
fn restored_account_invite_waits_for_local_approval_then_resumes_automatically() {
    for same_key_peer in [false, true] {
        let (mut sender, owner, device) = deferred_registration_core("restore-invite-approval");
        let peer_owner = Keys::generate();
        let peer_device = if same_key_peer { peer_owner.clone() } else { Keys::generate() };
        let mut peer = logged_in_test_core("restore-invite-peer", &peer_owner, &peer_device);
        let invite = create_private_invite_for_test(&mut peer);
        prove_invite_owner(&mut sender, &peer_owner, &peer_device, 10);
        sender.pending_relay_publishes.clear();
        sender.handle_action(AppAction::AcceptInvite { invite_input: invite });
        assert!(pending_events_with_kind(&sender, INVITE_RESPONSE_KIND).is_empty(),
            "an imported account must not send an unprovable device claim");
        assert!(sender.pending_outgoing_invite_acceptance.is_some());
        sender.complete_owner_registration_lookup(
            sender.relay_status_watch_generation, owner.public_key(), device.public_key(), 1, 1, vec![],
        );
        assert!(sender.pending_outgoing_invite_acceptance.is_none());
        let response = pending_events_with_kind(&sender, INVITE_RESPONSE_KIND).into_iter().next().unwrap();
        assert!(response.tags.iter().any(|tag| tag.as_slice()[0] == "owner-proof"));
        peer.handle_relay_event(response);
        assert_eq!(active_session_device_pubkeys(&peer, owner.public_key()), vec![device.public_key()]);
    }
}

#[test]
fn restored_account_publishes_verifiable_registration_to_empty_local_relay() {
    let relay = crate::local_relay::TestRelay::start();
    let temp = tempfile::TempDir::new().unwrap();
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, rx) = profile_restart_core(temp.path(), &relay);
    core.start_primary_session(owner.clone(), device.clone(), true, false)
        .unwrap();
    assert!(core.defer_owner_app_keys_publish);
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let mut receiver = logged_in_test_core("registration-recovery-recipient", &receiver_owner, &receiver_device);
    let invite = receiver.protocol_engine.as_ref().unwrap().local_invite().unwrap();
    let (mut sending_session, response) = invite.accept_with_owner(
        device.public_key(), device.secret_key().to_secret_bytes(),
        Some(device.public_key().to_hex()), Some(owner.public_key()),
    ).unwrap();
    receiver.handle_relay_event(invite_response_event(&response).unwrap());
    let plan = sending_session.plan_send(b"previously missing message", NdrUnixSeconds(unix_now().get())).unwrap();
    let message = message_event(&sending_session.apply_send(plan).envelope).unwrap();
    receiver.handle_relay_event(message);
    assert!(!receiver.threads.get(&owner.public_key().to_hex()).is_some_and(|thread| thread.messages.iter().any(|message| message.body == "previously missing message")));
    wait_for_profile_restart(&mut core, &rx, |core| {
        !core.defer_owner_app_keys_publish
            && relay_events(&relay).iter().any(|event| {
                event.pubkey == owner.public_key() && AppKeys::from_event(event).is_ok()
            })
    });
    let events = relay_events(&relay);
    let registration = events
        .iter()
        .find(|event| event.pubkey == owner.public_key() && AppKeys::from_event(event).is_ok())
        .unwrap();
    assert!(registration.verify().is_ok());
    assert!(AppKeys::from_event(registration)
        .unwrap()
        .get_device(&device.public_key())
        .is_some());
    receiver.handle_relay_event(registration.clone());
    assert!(receiver.threads[&owner.public_key().to_hex()].messages.iter().any(|message| message.body == "previously missing message"),
        "verified registration must release the previously blocked chat message without resending it");
}
