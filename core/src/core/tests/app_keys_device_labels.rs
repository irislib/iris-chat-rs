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
    assert_eq!(
        known_app_keys_to_ndr(&known)
            .get_all_devices()
            .len(),
        2
    );
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
    pair.a.set_current_device_labels("Study laptop", "Iris Chat macOS");
    assert_eq!(pair.a.app_keys[&owner].created_at_secs, before);
    assert_eq!(pair.a.app_keys[&owner].devices.len(), 2);
    // Receiver has a newer membership revision; the name still catches up.
    pair.b.app_keys.get_mut(&owner).unwrap().created_at_secs = before + 100;
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    let known = &pair.b.app_keys[&owner];
    assert_eq!(known.created_at_secs, before + 100);
    let sibling = known.devices.iter().find(|entry| entry.identity_pubkey_hex == device).unwrap();
    assert_eq!(sibling.device_label.as_deref(), Some("Study laptop"));
    assert_eq!(sibling.client_label.as_deref(), Some("Iris Chat macOS"));
    let old = pair.a.build_device_sync_packets_for_test(100, false);
    pair.a.set_current_device_labels("Travel laptop", "Iris Chat macOS");
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &old);
    assert_eq!(pair.b.app_keys[&owner].devices.iter().find(|entry| entry.identity_pubkey_hex == device).unwrap().device_label.as_deref(), Some("Travel laptop"));
}

#[test]
fn unnamed_device_labels_are_stable_and_real_names_take_precedence() {
    assert_eq!(crate::device_names::unnamed_device_name(&"a".repeat(64)), "Cozy Tiger");
    assert_eq!(crate::device_names::unnamed_device_name(&"B".repeat(64)), "Cozy Koala");
    let pair = chat_read_sync_pair("unnamed-devices");
    let roster = pair.a.build_device_roster_snapshot().unwrap();
    assert!(roster.devices.iter().all(|device| device.display_name.ends_with(" (unnamed device)")));
    assert_ne!(roster.devices[0].display_name, roster.devices[1].display_name);
    let mut core = pair.a;
    core.set_current_device_labels("Kitchen tablet", "Iris Chat iOS");
    assert_eq!(core.build_device_roster_snapshot().unwrap().devices.iter().find(|device| device.is_current_device).unwrap().display_name, "Kitchen tablet");
}
