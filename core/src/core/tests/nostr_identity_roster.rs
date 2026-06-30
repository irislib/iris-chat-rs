#[test]
fn nostr_identity_roster_facts_update_core_device_roster() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("nostr-identity-roster-facts", &owner, &device);

    let bootstrap = nostr_identity_roster_event_for_test(
        &peer_owner,
        vec![
            roster_tag_values(["op", "add_key"]),
            roster_tag_values(["key_pubkey", peer_owner.public_key().to_hex().as_str()]),
            roster_tag_values(["key_purpose", "app"]),
            roster_tag_values(["key_capability", "admin"]),
            roster_tag_values(["key_capability", "write"]),
            roster_tag_values(["key_added_at", "10"]),
        ],
        10,
    );
    let add_device = nostr_identity_roster_event_for_test(
        &peer_owner,
        vec![
            roster_tag_values(["op", "add_key"]),
            roster_tag_values(["key_pubkey", peer_device.public_key().to_hex().as_str()]),
            roster_tag_values(["key_purpose", "app"]),
            roster_tag_values(["key_capability", "write"]),
            roster_tag_values(["key_added_at", "11"]),
        ],
        11,
    );

    core.handle_relay_event(bootstrap);
    core.handle_relay_event(add_device);

    let cached = core
        .app_keys
        .get(&peer_owner.public_key().to_hex())
        .expect("projected peer roster");
    let device_hexes = cached
        .devices
        .iter()
        .map(|device| device.identity_pubkey_hex.clone())
        .collect::<Vec<_>>();
    let mut expected_device_hexes = vec![peer_device.public_key().to_hex()];
    expected_device_hexes.sort();
    assert_eq!(device_hexes, expected_device_hexes);

    let engine_devices = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .known_device_identity_pubkeys_for_owner(peer_owner.public_key());
    assert_eq!(engine_devices.len(), 1);
    assert!(engine_devices.contains(&peer_device.public_key()));
}

#[test]
fn owner_device_publishes_nostr_identity_roster_op_for_manual_device_npub() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let new_device = Keys::generate();
    let mut core = logged_in_test_core("manual-device-npub-roster-op", &owner, &device);
    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    core.sync_local_app_keys_to_protocol_engine("test_seed_appkeys");
    core.pending_relay_publishes.clear();

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: new_device
            .public_key()
            .to_bech32()
            .expect("device npub"),
    });

    assert_eq!(core.state.toast, None);
    assert!(
        pending_events_with_kind(&core, APP_KEYS_EVENT_KIND).is_empty(),
        "manual device approval must not publish legacy AppKeys snapshots"
    );
    let roster_ops = pending_events_with_kind(&core, NOSTR_IDENTITY_ROSTER_OP_KIND);
    assert_eq!(
        roster_ops.len(),
        3,
        "manual device approval bootstraps owner/current history, then adds the approved device"
    );
    assert!(roster_ops.iter().all(is_nostr_identity_roster_op_event));
    assert!(roster_ops.iter().all(|event| event.pubkey == owner.public_key()));
    let new_device_event = roster_ops
        .iter()
        .find(|event| event_has_tag_value(event, "key_pubkey", &new_device.public_key().to_hex()))
        .expect("manual device approval publishes an op for the new device");
    assert!(new_device_event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some("op")
            && values.get(1).map(|value| value.as_str()) == Some("add_key")
    }));
    assert!(new_device_event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some("key_purpose")
            && values.get(1).map(|value| value.as_str()) == Some("app")
    }));
    assert!(new_device_event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some("key_capability")
            && values.get(1).map(|value| value.as_str()) == Some("write")
    }));

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("roster-op projection");
    assert!(
        !known
            .devices
            .iter()
            .any(|device| device.identity_pubkey_hex == owner.public_key().to_hex()),
        "owner admin bootstrap must not be projected as a device"
    );
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == new_device.public_key().to_hex()));
}

#[test]
fn owner_device_accepts_ownerless_nostr_identity_approval_request_and_publishes_receipt() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let linked_device = Keys::generate();
    let request_keys = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let linked_device_hex = linked_device.public_key().to_hex();
    let request_url = format!(
        "nostr-identity://device-approval/{}.{}",
        linked_device_hex,
        request_keys.secret_key().to_secret_hex()
    );

    let mut core = logged_in_test_core("ownerless-nostr-identity-approval", &owner, &device);
    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    core.sync_local_app_keys_to_protocol_engine("test_seed_appkeys");
    core.pending_relay_publishes.clear();

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: request_url,
    });

    assert_eq!(core.state.toast, None);
    let pending_events = pending_events_with_kind(&core, NOSTR_IDENTITY_ROSTER_OP_KIND);
    let approval_event = pending_events
        .iter()
        .find(|event| {
            is_nostr_identity_roster_op_event(event)
                && event_has_tag_value(event, "key_pubkey", &linked_device_hex)
        })
        .expect("owner approval publishes a roster op for the linked device");
    let approval_op = nostr_identity::parse_nostr_identity_roster_op_event(approval_event)
        .expect("parse approval roster op");
    assert_eq!(approval_op.content.actor_pubkey, owner_hex);
    match &approval_op.content.op {
        nostr_identity::NostrIdentityRosterOp::AddFacet { facet } => {
            assert_eq!(facet.pubkey, linked_device_hex);
        }
        _ => panic!("approval roster op should add the linked device"),
    }

    let receipt_event = pending_events
        .iter()
        .find(|event| {
            event_has_tag_value(
                event,
                "type",
                nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE,
            )
        })
        .expect("owner approval publishes an encrypted receipt for the requester");
    assert_eq!(receipt_event.pubkey, owner.public_key());
    let receipt = nostr_identity::parse_nostr_identity_device_approval_receipt_event(
        receipt_event,
        &request_keys,
    )
    .expect("linked device can decrypt approval receipt");
    assert_eq!(receipt.request_pubkey, request_keys.public_key().to_hex());
    assert_eq!(receipt.device_app_key_pubkey, linked_device_hex);
    assert_eq!(receipt.subject_pubkey.as_deref(), Some(owner_hex.as_str()));
    let receipt_roster_op =
        nostr_identity::parse_nostr_identity_device_approval_receipt_roster_op(&receipt)
            .expect("receipt carries signed roster approval");
    assert_eq!(receipt_roster_op.op_id, approval_op.op_id);
}

#[test]
fn create_account_bootstraps_nostr_identity_roster_ops() {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join("iris-chat-rs-test-create-account-nostr-identity-roster")
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.handle_action(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });

    assert_eq!(core.state.toast, None);
    let roster_ops = pending_events_with_kind(&core, NOSTR_IDENTITY_ROSTER_OP_KIND);
    assert_eq!(
        roster_ops.len(),
        2,
        "account bootstrap publishes owner admin and current device roster facts"
    );
    assert!(roster_ops.iter().all(is_nostr_identity_roster_op_event));
    let owner_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .owner_pubkey;
    assert!(roster_ops.iter().all(|event| event.pubkey == owner_pubkey));
}

#[test]
fn account_bootstrap_roster_ops_install_peer_device_roster() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let mut alice = logged_in_test_core(
        "account-bootstrap-peer-nostr-identity-roster",
        &alice_owner,
        &alice_device,
    );
    let mut bob = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join("iris-chat-rs-test-bob-bootstrap-nostr-identity-roster")
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
    let roster_ops = pending_events_with_kind(&bob, NOSTR_IDENTITY_ROSTER_OP_KIND);

    for event in roster_ops {
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

fn nostr_identity_roster_event_for_test(
    signer: &Keys,
    facts: Vec<Vec<String>>,
    created_at: u64,
) -> Event {
    const PROFILE_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
    let created_at_string = created_at.to_string();
    let signer_hex = signer.public_key().to_hex();
    let nonce = format!("nonce-{created_at}");
    let mut tags = vec![
        nostr::Tag::parse(["i", PROFILE_ID, "subject"]).expect("profile tag"),
        nostr::Tag::parse(["type", "nostr_identity_roster_op"]).expect("type tag"),
        nostr::Tag::parse(["schema", "1"]).expect("schema tag"),
        nostr::Tag::parse(["actor_pubkey", signer_hex.as_str()]).expect("actor tag"),
        nostr::Tag::parse(["client_nonce", nonce.as_str()]).expect("nonce tag"),
        nostr::Tag::parse(["created_at", created_at_string.as_str()]).expect("created_at tag"),
    ];
    for fact in facts {
        let values = fact.iter().map(String::as_str).collect::<Vec<_>>();
        tags.push(nostr::Tag::parse(values).expect("fact tag"));
    }
    EventBuilder::new(Kind::from(NOSTR_IDENTITY_ROSTER_OP_KIND as u16), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(signer)
        .expect("signed roster event")
}

fn event_has_tag_value(event: &Event, tag_name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some(tag_name)
            && values.get(1).map(|candidate| candidate.as_str()) == Some(value)
    })
}

fn roster_tag_values<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(ToString::to_string).collect()
}
