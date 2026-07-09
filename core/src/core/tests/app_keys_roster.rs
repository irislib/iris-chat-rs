#[test]
fn owner_device_publishes_app_keys_snapshot_for_manual_device_npub() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let new_device = Keys::generate();
    let mut core = logged_in_test_core("manual-device-npub-appkeys", &owner, &device);
    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    core.sync_local_app_keys_to_protocol_engine("test_seed_appkeys");
    core.pending_relay_publishes.clear();

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: new_device
            .public_key()
            .to_bech32()
            .expect("device npub"),
    });

    assert_eq!(core.state.toast.as_deref(), Some("Device added"));
    let app_keys_events = pending_events_with_kind(&core, APP_KEYS_EVENT_KIND);
    assert_eq!(
        app_keys_events.len(),
        1,
        "manual device approval publishes the current owner AppKeys snapshot"
    );
    let app_keys_event = &app_keys_events[0];
    assert!(is_app_keys_event(app_keys_event));
    assert_eq!(app_keys_event.pubkey, owner.public_key());
    assert!(event_has_tag_value(
        app_keys_event,
        "owner_pubkey",
        &owner.public_key().to_hex()
    ));
    assert!(event_has_tag_value(
        app_keys_event,
        "device",
        &new_device.public_key().to_hex()
    ));

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("AppKeys projection");
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == new_device.public_key().to_hex()));
}

#[test]
fn owner_device_accepts_full_link_request_and_publishes_app_keys_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let linked_device = Keys::generate();
    let request_keys = Keys::generate();
    let linked_device_hex = linked_device.public_key().to_hex();
    let request = create_nostr_identity_device_approval_request(
        &linked_device,
        CreateNostrIdentityDeviceApprovalRequestOptions {
            request_keys: Some(request_keys),
            request_secret: Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
            ),
            requested_at: 41,
            request_type: Some("device_link".to_string()),
            resources: Vec::new(),
            expires_at: None,
            profile_id: None,
            admin_app_key_pubkey: None,
            label: Some("Safari on macOS".to_string()),
        },
    )
    .expect("approval request");
    let request_url = encode_nostr_identity_device_approval_request(&request.request, None)
        .expect("encode approval request");

    let mut core = logged_in_test_core("owner-full-appkeys-approval", &owner, &device);
    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    core.sync_local_app_keys_to_protocol_engine("test_seed_appkeys");
    core.pending_relay_publishes.clear();

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: request_url,
    });

    assert_eq!(core.state.toast.as_deref(), Some("Device added"));
    let app_keys_events = pending_events_with_kind(&core, APP_KEYS_EVENT_KIND);
    assert_eq!(
        app_keys_events.len(),
        1,
        "approval publishes an owner-signed AppKeys snapshot"
    );
    let app_keys_event = &app_keys_events[0];
    assert!(is_app_keys_event(app_keys_event));
    assert_eq!(app_keys_event.pubkey, owner.public_key());
    assert!(event_has_tag_value(app_keys_event, "device", &linked_device_hex));
    assert_eq!(
        pending_events_with_kind(&core, INVITE_RESPONSE_KIND).len(),
        1,
        "approval still responds to the deterministic NDR link invite"
    );
    assert_eq!(
        pending_events_with_kind(&core, u32::from(FACT_OP_KIND))
            .into_iter()
            .filter(|event| event_has_tag_value(
                event,
                "type",
                "nostr_identity_device_approval_receipt"
            ))
            .count(),
        1,
        "approval publishes an encrypted receipt for the request"
    );

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("AppKeys projection");
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == linked_device_hex));
    let linked = known
        .devices
        .iter()
        .find(|device| device.identity_pubkey_hex == linked_device_hex)
        .expect("linked device");
    assert_eq!(linked.device_label.as_deref(), Some("Safari on macOS"));
    assert_eq!(linked.client_label.as_deref(), None);
}

#[test]
fn create_account_publishes_app_keys_snapshot() {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join("iris-chat-rs-test-create-account-appkeys")
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.handle_action(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });

    assert_eq!(core.state.toast, None);
    let app_keys_events = pending_events_with_kind(&core, APP_KEYS_EVENT_KIND);
    assert_eq!(
        app_keys_events.len(),
        1,
        "account bootstrap publishes the current owner AppKeys snapshot"
    );
    let owner_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .owner_pubkey;
    assert!(app_keys_events.iter().all(is_app_keys_event));
    assert!(app_keys_events
        .iter()
        .all(|event| event.pubkey == owner_pubkey));
}

#[test]
fn account_bootstrap_app_keys_snapshot_installs_peer_device_roster() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let mut alice = logged_in_test_core(
        "account-bootstrap-peer-appkeys",
        &alice_owner,
        &alice_device,
    );
    let mut bob = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join("iris-chat-rs-test-bob-bootstrap-appkeys")
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    bob.handle_action(AppAction::CreateAccount {
        name: "Bob".to_string(),
    });
    let bob_login = bob.logged_in.as_ref().expect("bob logged in");
    let bob_owner = bob_login.owner_pubkey;
    let bob_device = bob_login.device_keys.public_key();
    let app_keys_events = pending_events_with_kind(&bob, APP_KEYS_EVENT_KIND);

    for event in app_keys_events {
        alice.handle_relay_event(event);
    }

    let known = alice
        .app_keys
        .get(&bob_owner.to_hex())
        .expect("bob roster projected");
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == bob_device.to_hex()));
}

fn event_has_tag_value(event: &Event, tag_name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some(tag_name)
            && values.get(1).map(|candidate| candidate.as_str()) == Some(value)
    })
}
