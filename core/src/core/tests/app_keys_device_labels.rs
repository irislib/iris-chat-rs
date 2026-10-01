fn legacy_labeled_roster_for_test(roster: &AppKeys, owner: &Keys, at: u64) -> Event {
    let mut event = roster.get_event_at(owner.public_key(), at);
    event.tags.push(
        nostr::Tag::parse([
            nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT,
            "retired-static-ciphertext-fixture",
        ])
        .unwrap(),
    );
    // Adding a legacy tag changes the event ID cached by the roster builder.
    event.id = None;
    event.sign_with_keys(owner).unwrap()
}

#[test]
fn app_keys_device_projection_is_deterministic() {
    let owner = Keys::generate().public_key();
    let device_a = Keys::generate().public_key();
    let device_b = Keys::generate().public_key();
    let app_keys = AppKeys::new(vec![
        DeviceEntry::new(device_b, 20),
        DeviceEntry::new(device_a, 10),
    ]);

    let known = known_app_keys_from_ndr(owner, &app_keys, 30);

    assert_eq!(known.owner_pubkey_hex, owner.to_hex());
    assert_eq!(known.created_at_secs, 30);
    let mut expected_devices = vec![device_a.to_hex(), device_b.to_hex()];
    expected_devices.sort();
    assert_eq!(
        known
            .devices
            .iter()
            .map(|device| device.identity_pubkey_hex.clone())
            .collect::<Vec<_>>(),
        expected_devices
    );
    assert_eq!(known_app_keys_to_ndr(&known).get_all_devices().len(), 2);
}

#[test]
fn app_keys_device_labels_roundtrip_through_known_snapshot() {
    let owner = Keys::generate().public_key();
    let device = Keys::generate().public_key();
    let mut app_keys = AppKeys::new(vec![DeviceEntry::new(device, 10)]);
    app_keys.set_device_labels(
        device,
        Some("virus.exe - iPhone 16 Pro - iOS 18.5".to_string()),
        Some("Iris Chat iOS".to_string()),
        Some(20),
    );

    let known = known_app_keys_from_ndr(owner, &app_keys, 30);
    let known_device = known.devices.first().expect("known device");
    assert_eq!(
        known_device.device_label.as_deref(),
        Some("virus.exe - iPhone 16 Pro - iOS 18.5")
    );
    assert_eq!(known_device.client_label.as_deref(), Some("Iris Chat iOS"));
    assert_eq!(known_device.label_updated_at_secs, 20);

    let roundtrip = known_app_keys_to_ndr(&known);
    let labels = roundtrip.get_device_labels(&device).expect("device labels");
    assert_eq!(
        labels.device_label.as_deref(),
        Some("virus.exe - iPhone 16 Pro - iOS 18.5")
    );
    assert_eq!(labels.client_label.as_deref(), Some("Iris Chat iOS"));
    assert_eq!(labels.updated_at, 20);
}

#[test]
fn published_rosters_contain_authorization_but_no_private_device_labels() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("private-label-roster", &owner, &device);
    core.set_current_device_labels("Private kitchen tablet", "Private client label");
    let (background, durable) = core.build_local_identity_artifacts();
    let roster = background
        .into_iter()
        .chain(durable)
        .map(|(_, event)| event)
        .find(is_app_keys_event)
        .expect("signed authorization roster");
    assert!(roster.content.is_empty());
    assert!(!roster
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().is_some_and(
            |name| name == nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT
        )));
    assert!(AppKeys::from_event(&roster)
        .unwrap()
        .get_device(&device.public_key())
        .is_some());
    let json = serde_json::to_string(&roster).unwrap();
    assert!(!json.contains("Private kitchen tablet"));
    assert!(!json.contains("Private client label"));
}

#[test]
fn signer_authorization_does_not_republish_old_private_label_ciphertext() {
    let owner = Keys::generate();
    let old_device = Keys::generate().public_key();
    let mut keys = AppKeys::new(vec![DeviceEntry::new(old_device, 1)]);
    keys.set_device_labels(old_device, Some("Old private device".into()), None, Some(1));
    let old = legacy_labeled_roster_for_test(&keys, &owner, 1);
    let new_device = Keys::generate().public_key();
    let unsigned =
        account_signer::prepare_signer_authorization(owner.public_key(), new_device, Some(&old), 2)
            .unwrap();
    assert!(unsigned.content.is_empty());
    assert!(!unsigned
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().is_some_and(
            |name| name == nostr_double_ratchet::APP_KEYS_ENCRYPTED_DEVICE_LABELS_FACT
        )));
    let event = unsigned.sign_with_keys(&owner).unwrap();
    let parsed = AppKeys::from_event(&event).unwrap();
    assert!(parsed.get_device(&old_device).is_some());
    assert!(parsed.get_device(&new_device).is_some());
}

#[test]
fn current_device_labels_update_app_keys_and_roster_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("device-labels", &owner, &device);

    core.handle_action(AppAction::SetCurrentDeviceLabels {
        device_label: "virus.exe - iPhone 16 Pro - iOS 18.5".to_string(),
        client_label: "Iris Chat iOS".to_string(),
    });

    let owner_hex = owner.public_key().to_hex();
    let device_hex = device.public_key().to_hex();
    let app_keys = core.app_keys.get(&owner_hex).expect("local AppKeys");
    let known_device = app_keys
        .devices
        .iter()
        .find(|candidate| candidate.identity_pubkey_hex == device_hex)
        .expect("current device");
    assert_eq!(
        known_device.device_label.as_deref(),
        Some("virus.exe - iPhone 16 Pro - iOS 18.5")
    );
    assert_eq!(known_device.client_label.as_deref(), Some("Iris Chat iOS"));

    let ndr_app_keys = known_app_keys_to_ndr(app_keys);
    let labels = ndr_app_keys
        .get_device_labels(&device.public_key())
        .expect("NDR labels");
    assert_eq!(
        labels.device_label.as_deref(),
        Some("virus.exe - iPhone 16 Pro - iOS 18.5")
    );
    assert_eq!(labels.client_label.as_deref(), Some("Iris Chat iOS"));

    let roster_device = core
        .state
        .device_roster
        .as_ref()
        .expect("device roster")
        .devices
        .iter()
        .find(|candidate| candidate.device_pubkey_hex == device_hex)
        .expect("roster device");
    assert_eq!(
        roster_device.device_label.as_deref(),
        Some("virus.exe - iPhone 16 Pro - iOS 18.5")
    );
    assert_eq!(roster_device.client_label.as_deref(), Some("Iris Chat iOS"));
}

#[test]
fn restored_owner_session_publishes_app_keys_snapshot_when_creating_public_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let device_pubkey = device.public_key();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.start_primary_session(owner, device, true, false)
        .expect("restored session");

    let app_keys_events_before_invite = update_rx
        .try_iter()
        .filter(|update| {
            if let AppUpdate::NearbyPublishedEvent { event_json, .. } = update {
                return serde_json::from_str::<Event>(event_json)
                    .map(|event| is_app_keys_event(&event))
                    .unwrap_or(false);
            }
            false
        })
        .count();
    assert_eq!(
        app_keys_events_before_invite, 0,
        "restored nsec login must not overwrite relay AppKeys before an explicit invite bootstrap"
    );

    core.handle_action(AppAction::CreatePublicInvite);

    let app_keys_events_after_invite = update_rx
        .try_iter()
        .filter_map(|update| {
            if let AppUpdate::NearbyPublishedEvent { event_json, .. } = update {
                return serde_json::from_str::<Event>(&event_json)
                    .ok()
                    .filter(is_app_keys_event);
            }
            None
        })
        .collect::<Vec<_>>();
    assert_eq!(
        app_keys_events_after_invite.len(),
        1,
        "creating a public invite publishes a current-device AppKeys snapshot"
    );
    let app_keys = AppKeys::from_event(&app_keys_events_after_invite[0]).expect("app keys event");
    assert!(app_keys.get_device(&device_pubkey).is_some());
}

#[test]
fn linked_device_labels_sync_without_owner_key_or_changing_authorization() {
    let mut pair = chat_read_sync_pair("linked-device-labels-sync");
    pair.a.logged_in.as_mut().unwrap().owner_keys = None;
    let owner = pair.owner.public_key().to_hex();
    let device = pair.a_device.public_key().to_hex();
    let before = pair.a.app_keys[&owner].created_at_secs;
    pair.a
        .set_current_device_labels("Study laptop", "Iris Chat macOS");
    assert_eq!(pair.a.app_keys[&owner].created_at_secs, before);
    assert_eq!(pair.a.app_keys[&owner].devices.len(), 2);
    // Receiver has a newer membership revision; the name still catches up.
    pair.b.app_keys.get_mut(&owner).unwrap().created_at_secs = before + 100;
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    let known = &pair.b.app_keys[&owner];
    assert_eq!(known.created_at_secs, before + 100);
    let sibling = known
        .devices
        .iter()
        .find(|entry| entry.identity_pubkey_hex == device)
        .unwrap();
    assert_eq!(sibling.device_label.as_deref(), Some("Study laptop"));
    assert_eq!(sibling.client_label.as_deref(), Some("Iris Chat macOS"));
    let old = pair.a.build_device_sync_packets_for_test(100, false);
    pair.a
        .set_current_device_labels("Travel laptop", "Iris Chat macOS");
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &old);
    assert_eq!(
        pair.b.app_keys[&owner]
            .devices
            .iter()
            .find(|entry| entry.identity_pubkey_hex == device)
            .unwrap()
            .device_label
            .as_deref(),
        Some("Travel laptop")
    );
}

#[test]
fn unnamed_device_labels_are_stable_and_real_names_take_precedence() {
    assert_eq!(
        crate::device_names::unnamed_device_name(&"a".repeat(64)),
        "Cozy Tiger"
    );
    assert_eq!(
        crate::device_names::unnamed_device_name(&"B".repeat(64)),
        "Cozy Koala"
    );
    let pair = chat_read_sync_pair("unnamed-devices");
    let roster = pair.a.build_device_roster_snapshot().unwrap();
    assert!(roster
        .devices
        .iter()
        .all(|device| device.display_name.ends_with(" (unnamed device)")));
    assert_ne!(
        roster.devices[0].display_name,
        roster.devices[1].display_name
    );
    let mut core = pair.a;
    core.set_current_device_labels("Kitchen tablet", "Iris Chat iOS");
    assert_eq!(
        core.build_device_roster_snapshot()
            .unwrap()
            .devices
            .iter()
            .find(|device| device.is_current_device)
            .unwrap()
            .display_name,
        "Kitchen tablet"
    );
}

#[test]
fn legacy_private_roster_is_retired_only_after_exact_public_replacement_ack() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("legacy-label-queue", &owner, &device);
    let now = unix_now().get();
    let mut roster = AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)]);
    roster.set_device_labels(
        device.public_key(),
        Some("Private tablet".into()),
        None,
        Some(now),
    );
    let legacy = legacy_labeled_roster_for_test(&roster, &owner, now);
    let pending = PendingRelayPublish {
        owner_pubkey_hex: owner.public_key().to_hex(),
        event_id: legacy.id.to_hex(),
        label: "app-keys".into(),
        event_json: legacy.as_json(),
        inner_event_id: None,
        chat_id: None,
        created_at_secs: now,
        attempt_count: 0,
        last_error: None,
    };
    core.app_store
        .upsert_pending_relay_publish(&pending)
        .unwrap();
    assert!(!core.publish_runtime_event(legacy.clone(), "app-keys", None));
    let different = AppKeys::new(vec![DeviceEntry::new(Keys::generate().public_key(), now)])
        .get_event_at(owner.public_key(), now + 1)
        .sign_with_keys(&owner)
        .unwrap();
    core.retire_private_label_publications_after_ack(&different);
    assert!(core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap()
        .iter()
        .any(|event| event.event_id == legacy.id.to_hex()));
    let public = roster
        .get_event_at(owner.public_key(), now + 1)
        .sign_with_keys(&owner)
        .unwrap();
    core.retire_private_label_publications_after_ack(&public);
    assert!(!core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap()
        .iter()
        .any(|event| event.event_id == legacy.id.to_hex()));
}

#[test]
fn legacy_local_device_label_survives_projection_until_a_newer_clear() {
    let owner = Keys::generate().public_key();
    let device = Keys::generate().public_key();
    let roster = AppKeys::new(vec![DeviceEntry::new(device, 1)]);
    let mut known = known_app_keys_from_ndr(owner, &roster, 10);
    known.devices.first_mut().unwrap().device_label = Some("Kept locally".into());
    let projected = known_app_keys_to_ndr(&known);
    let roundtrip = known_app_keys_from_ndr(owner, &projected, 10);
    let label = roundtrip.devices.first().unwrap();
    assert_eq!(label.device_label.as_deref(), Some("Kept locally"));
    assert_eq!(
        label.label_updated_at_secs, 0,
        "migration must not invent a newer edit"
    );

    let mut cleared = roster;
    cleared.set_device_labels(device, None, None, Some(11));
    preserve_known_app_key_labels(Some(&known), &mut cleared);
    let roundtrip = known_app_keys_from_ndr(owner, &cleared, 12);
    let label = roundtrip.devices.first().unwrap();
    assert!(label.device_label.is_none());
    assert_eq!(
        label.label_updated_at_secs, 11,
        "newer deletion must survive legacy preservation"
    );
    let projected = known_app_keys_to_ndr(&roundtrip);
    let tombstone = projected.get_device_labels(&device).unwrap();
    assert!(tombstone.device_label.is_none());
    assert_eq!(tombstone.updated_at, 11);
}
