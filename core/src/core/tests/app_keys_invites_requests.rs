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
            .expect("convert back")
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

    let roundtrip = known_app_keys_to_ndr(&known).expect("convert back");
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

    let ndr_app_keys = known_app_keys_to_ndr(app_keys).expect("NDR AppKeys");
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
fn peer_profile_debug_reports_known_user_context() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("peer-profile-debug", &owner, &device);
    let peer_hex = peer.public_key().to_hex();
    let peer_device_hex = peer_device.public_key().to_hex();
    let app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 10)]);

    core.apply_known_app_keys_snapshot(peer.public_key(), &app_keys, 10);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(peer.public_key(), app_keys, 10)
        .expect("ingest app keys");
    core.process_protocol_engine_retry_batch("test_app_keys", batch);
    core.remember_recent_handshake_peer(peer_hex.clone(), peer_device_hex, 123);
    core.handle_action(AppAction::CreateChat {
        peer_input: peer_hex.clone(),
    });

    let debug = core
        .build_peer_profile_debug_snapshot(&peer_hex)
        .expect("peer debug");
    assert_eq!(debug.owner_pubkey_hex, peer_hex);
    assert_eq!(debug.roster_device_count, 1);
    assert_eq!(debug.known_device_count, 1);
    assert_eq!(debug.session_count, 0);
    assert_eq!(debug.active_session_count, 0);
    assert_eq!(debug.recent_handshake_device_count, 1);
    assert_eq!(debug.last_handshake_at_secs, Some(123));
    assert!(debug.tracked_for_messages);
}

#[test]
fn linked_device_authorization_follows_app_keys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!("iris-chat-rs-test-{}", owner.public_key().to_hex()))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    assert_eq!(
        core.local_authorization_state(None, owner.public_key(), device.public_key(), None),
        LocalAuthorizationState::AwaitingApproval
    );

    let other_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 10)]),
        10,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), other_keys);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::AwaitingApproval
    );
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Revoked
    );

    let approved_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), approved_keys);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::AwaitingApproval
    );

    core.start_session(owner.public_key(), None, device.clone(), false, false)
        .expect("linked session");
    let approved_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), approved_keys);
    install_local_sibling_session_for_test(&mut core, &owner, &device, &other_device);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::Authorized
    );
}

#[test]
fn restored_authorized_linked_device_is_not_revoked_by_cached_roster() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-restored-auth-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 10)]),
            10,
        ),
    );

    assert_eq!(
        core.restored_local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Authorized
    );
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Revoked
    );
}

#[test]
fn restored_linked_device_uses_persisted_protocol_session_for_authorization() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            data_dir.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.preferences.nostr_relay_urls.clear();
        core.start_session(
            owner.public_key(),
            None,
            linked_device.clone(),
            false,
            false,
        )
        .expect("linked session");
        core.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::new(vec![
                    DeviceEntry::new(primary_device.public_key(), 10),
                    DeviceEntry::new(linked_device.public_key(), 11),
                ]),
                11,
            ),
        );
        install_local_sibling_session_for_test(&mut core, &owner, &linked_device, &primary_device);
        core.refresh_local_authorization_state();
        core.persist_best_effort();

        assert_eq!(
            core.logged_in
                .as_ref()
                .expect("logged in")
                .authorization_state,
            LocalAuthorizationState::Authorized
        );
    }

    let mut restored = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    restored
        .start_session(owner.public_key(), None, linked_device, true, false)
        .expect("restored linked session");

    assert_eq!(
        restored
            .logged_in
            .as_ref()
            .expect("logged in")
            .authorization_state,
        LocalAuthorizationState::Authorized
    );
    assert_eq!(
        restored
            .state
            .account
            .as_ref()
            .expect("account")
            .authorization_state,
        DeviceAuthorizationState::Authorized
    );
}

#[test]
fn linked_device_missing_local_session_exposes_link_code() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_session(owner.public_key(), None, device.clone(), false, false)
        .expect("linked session");
    let app_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys.insert(owner.public_key().to_hex(), app_keys);
    core.refresh_local_authorization_state();
    core.rebuild_state();

    let account = core.state.account.as_ref().expect("account");
    assert_eq!(
        account.authorization_state,
        DeviceAuthorizationState::AwaitingApproval
    );
    let snapshot = core
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot");
    let invite =
        super::invites::parse_public_invite_input(&snapshot.url).expect("parse link invite");
    assert_eq!(invite.purpose.as_deref(), Some("link"));
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_eq!(
        invite.inviter.to_bech32().ok().as_deref(),
        Some(snapshot.device_input.as_str())
    );
}

#[test]
fn app_keys_cache_merges_older_roster_events_additively() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let old_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-roster-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
            20,
        ),
    );

    let stale = AppKeys::new(vec![DeviceEntry::new(old_device.public_key(), 10)]);
    assert!(
        core.apply_known_app_keys_snapshot(owner.public_key(), &stale, 10)
            .is_some(),
        "older owner-signed app-key events should add missing devices without replacing the cached timestamp"
    );

    let cached = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("cached roster");
    assert_eq!(cached.created_at_secs, 20);
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device.public_key().to_hex()));
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == old_device.public_key().to_hex()));
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::AwaitingApproval,
        "a cached roster without an active local protocol session must not mark the linked device approved"
    );
}

#[test]
fn app_keys_cache_merges_same_timestamp_roster_events() {
    let owner = Keys::generate();
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-roster-merge-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device_a.public_key(), 20)]),
            20,
        ),
    );

    let concurrent = AppKeys::new(vec![DeviceEntry::new(device_b.public_key(), 20)]);
    let applied = core
        .apply_known_app_keys_snapshot(owner.public_key(), &concurrent, 20)
        .expect("same-timestamp roster should merge");
    assert_eq!(applied.1, 20);

    let cached = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("cached roster");
    assert_eq!(cached.created_at_secs, 20);
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_a.public_key().to_hex()));
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_b.public_key().to_hex()));
}

#[test]
fn app_keys_runtime_storage_failure_does_not_mark_seen_or_mutate_app_cache() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let remote_owner = Keys::generate();
    let remote_device = Keys::generate();
    let runtime_storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "appkeys-runtime-storage-failure",
        &owner,
        &device,
        runtime_storage.clone() as Arc<dyn StorageAdapter>,
    );
    let remote_event = AppKeys::new(vec![DeviceEntry::new(remote_device.public_key(), 1)])
        .get_event(remote_owner.public_key())
        .sign_with_keys(&remote_owner)
        .expect("remote app keys event");
    let event_id = remote_event.id.to_string();

    runtime_storage.set_fail_puts(true);
    core.handle_relay_event(remote_event.clone());

    assert!(
        !core.seen_event_ids.contains(&event_id),
        "transient runtime persistence failure must not dedupe the protocol event"
    );
    assert!(
        !core
            .app_keys
            .contains_key(&remote_owner.public_key().to_hex()),
        "app projection must not commit AppKeys that runtime failed to persist"
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .known_device_identity_pubkeys_for_owner(remote_owner.public_key())
            .is_empty(),
        "runtime roster must remain unchanged after failed persistence"
    );

    runtime_storage.set_fail_puts(false);
    core.handle_relay_event(remote_event);

    assert!(core.seen_event_ids.contains(&event_id));
    assert!(core
        .app_keys
        .get(&remote_owner.public_key().to_hex())
        .is_some_and(|known| known
            .devices
            .iter()
            .any(|entry| { entry.identity_pubkey_hex == remote_device.public_key().to_hex() })));
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .known_device_identity_pubkeys_for_owner(remote_owner.public_key()),
        vec![remote_device.public_key()]
    );
}

#[test]
fn invite_runtime_storage_failure_does_not_mark_seen() {
    use nostr_double_ratchet_nostr::InviteNostrExt;

    let owner = Keys::generate();
    let device = Keys::generate();
    let remote_owner = Keys::generate();
    let remote_device = Keys::generate();
    let runtime_storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "invite-runtime-storage-failure",
        &owner,
        &device,
        runtime_storage.clone() as Arc<dyn StorageAdapter>,
    );
    let mut invite = Invite::create_new(
        remote_device.public_key(),
        Some(remote_device.public_key().to_hex()),
        Some(1),
    )
    .expect("remote invite");
    invite.owner_public_key = Some(remote_owner.public_key());
    let invite_event = invite
        .get_event()
        .expect("invite unsigned event")
        .sign_with_keys(&remote_device)
        .expect("invite event");
    let event_id = invite_event.id.to_string();

    runtime_storage.set_fail_puts(true);
    core.handle_relay_event(invite_event.clone());

    assert!(
        !core.seen_event_ids.contains(&event_id),
        "transient runtime persistence failure must not dedupe invite events"
    );

    runtime_storage.set_fail_puts(false);
    core.handle_relay_event(invite_event);

    assert!(core.seen_event_ids.contains(&event_id));
}

#[test]
fn restored_owner_session_does_not_publish_single_device_app_keys_before_backfill() {
    let owner = Keys::generate();
    let device = Keys::generate();
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

    let app_keys_events = update_rx
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
        app_keys_events, 0,
        "restored nsec login must not overwrite relay AppKeys before fetching them"
    );
}

#[test]
fn restored_account_bundle_publishes_existing_device_app_keys_on_startup() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let owner_nsec = owner
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| owner.secret_key().to_secret_hex());
    let device_nsec = device
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| device.secret_key().to_secret_hex());
    let temp_dir = tempfile::TempDir::new().expect("temp dir");

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            temp_dir.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.start_session(
            owner.public_key(),
            Some(owner.clone()),
            device.clone(),
            false,
            true,
        )
        .expect("created account bundle session");
        core.shutdown();
    }

    let (update_tx, update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.restore_account_bundle(Some(owner_nsec), &owner.public_key().to_hex(), &device_nsec);
    assert_eq!(core.state.toast, None);

    let app_keys_events = update_rx
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
    assert!(
        app_keys_events > 0,
        "same-device startup must continue publishing the current AppKeys roster"
    );
}

#[test]
fn restored_account_bundle_defers_app_keys_when_roster_was_not_backfilled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let owner_nsec = owner
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| owner.secret_key().to_secret_hex());
    let device_nsec = device
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| device.secret_key().to_secret_hex());
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.restore_account_bundle(Some(owner_nsec), &owner.public_key().to_hex(), &device_nsec);
    assert_eq!(core.state.toast, None);

    let app_keys_events = update_rx
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
        app_keys_events, 0,
        "restored account bundle must not publish a one-device AppKeys roster before backfill"
    );
}

#[test]
fn restored_owner_app_keys_backfill_merges_current_device_and_republishes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let web_device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.start_primary_session(owner.clone(), device.clone(), true, false)
        .expect("restored session");
    while update_rx.try_recv().is_ok() {}

    let remote_app_keys = AppKeys::new(vec![DeviceEntry::new(web_device.public_key(), 10)]);
    let remote_event = remote_app_keys
        .get_event(owner.public_key())
        .sign_with_keys(&owner)
        .expect("remote app keys event");
    core.apply_app_keys_event(&remote_event)
        .expect("apply remote app keys");

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("known app keys");
    assert!(known
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == web_device.public_key().to_hex()));
    assert!(known
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device.public_key().to_hex()));

    let published_app_keys = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent { event_json, .. } => {
                serde_json::from_str::<Event>(&event_json)
                    .ok()
                    .filter(is_app_keys_event)
                    .and_then(|event| AppKeys::from_event(&event).ok())
            }
            _ => None,
        })
        .last()
        .expect("republished merged app keys");
    assert!(published_app_keys
        .get_device(&web_device.public_key())
        .is_some());
    assert!(published_app_keys
        .get_device(&device.public_key())
        .is_some());
}

#[test]
fn app_keys_event_rerenders_device_roster_even_when_authorization_is_unchanged() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .expect("primary session");
    assert_eq!(
        core.state
            .device_roster
            .as_ref()
            .expect("device roster")
            .devices
            .len(),
        1
    );

    let local_created_at = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("local app keys")
        .created_at_secs;
    let remote_created_at = local_created_at + 1;
    let remote_app_keys = AppKeys::new(vec![
        DeviceEntry::new(device.public_key(), local_created_at),
        DeviceEntry::new(other_device.public_key(), remote_created_at),
    ]);
    let remote_event = remote_app_keys
        .get_event_at(owner.public_key(), remote_created_at)
        .sign_with_keys(&owner)
        .expect("app keys event");
    core.apply_app_keys_event(&remote_event)
        .expect("apply remote app keys");

    let roster = core.state.device_roster.as_ref().expect("device roster");
    assert_eq!(roster.devices.len(), 2);
    assert!(roster
        .devices
        .iter()
        .any(|entry| entry.device_pubkey_hex == other_device.public_key().to_hex()));
}

#[test]
fn start_linked_device_creates_ownerless_link_invite() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();

    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let snapshot = core
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot");
    let invite =
        super::invites::parse_public_invite_input(&snapshot.url).expect("parse link invite");
    assert_eq!(invite.purpose.as_deref(), Some("link"));
    assert!(invite.owner_public_key.is_none());
    assert_eq!(
        invite.inviter.to_bech32().ok().as_deref(),
        Some(snapshot.device_input.as_str())
    );
    assert!(matches!(core.screen_stack.as_slice(), [Screen::AddDevice]));
}

#[test]
fn owner_device_accepts_link_invite_and_registers_new_device() {
    let owner = Keys::generate();
    let new_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(owner.clone(), owner.clone(), false, false)
        .expect("primary session");

    let mut invite = Invite::create_new(
        new_device.public_key(),
        Some(new_device.public_key().to_hex()),
        Some(1),
    )
    .expect("link invite");
    invite.purpose = Some("link".to_string());
    let invite_url = super::invites::chat_invite_url(&invite).expect("invite url");

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: invite_url,
    });

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("owner app keys");
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == new_device.public_key().to_hex()));
    assert_eq!(core.state.toast, None);
}

#[test]
fn pending_linked_device_finishes_when_owner_accepts_invite() {
    let owner = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let pending = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite");
    let (_owner_session, response_envelope) = pending
        .invite
        .accept_with_owner(
            owner.public_key(),
            owner.secret_key().to_secret_bytes(),
            Some(owner.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("owner accepts");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response_envelope)
        .expect("invite response event");

    core.handle_relay_event(response_event);

    let logged_in = core.logged_in.as_ref().expect("linked session");
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::AwaitingApproval
    );
    assert!(core.pending_linked_device.is_none());
    assert!(core
        .protocol_engine
        .as_ref()
        .is_some_and(|engine| engine.active_session_count_for_owner(owner.public_key()) > 0));
}

#[test]
fn recent_protocol_filters_include_runtime_invite_response_backfill() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("protocol-backfill-invite-response", &owner, &device);
    let invite_response_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_hex();

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let response_filter = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .find(|filter| {
            let has_response_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(INVITE_RESPONSE_KIND as u64))
                });
            let has_invite_pubkey = filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|pubkey| pubkey.as_str() == Some(invite_response_pubkey.as_str()))
                });
            has_response_kind && has_invite_pubkey
        })
        .expect("invite response backfill filter");

    assert_eq!(
        response_filter
            .get("since")
            .and_then(|since| since.as_u64()),
        Some(1_777_159_500 - DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS)
    );
}

#[test]
fn protocol_filters_track_invite_responses_by_known_device_authors() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("protocol-invite-response-author", &owner, &device);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    core.app_keys.insert(
        peer_owner.public_key().to_hex(),
        known_app_keys_from_ndr(peer_owner.public_key(), &peer_app_keys, 1),
    );
    core.active_chat_id = Some(peer_owner.public_key().to_hex());

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        has_filter_with_kind_author(&filters, INVITE_RESPONSE_KIND, peer_device.public_key()),
        "invite response backfill should not depend only on #p indexing"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        has_filter_with_kind_author(&active_filters, INVITE_EVENT_KIND, peer_device.public_key()),
        "live invite subscription should track known device authors, not owner pubkeys"
    );
    assert!(
        has_filter_with_kind_author(
            &active_filters,
            INVITE_RESPONSE_KIND,
            peer_device.public_key()
        ),
        "live invite response subscription should also track known peer device authors"
    );
}

#[test]
fn single_protocol_plan_builds_filters_for_all_protocol_inputs() {
    let owner = Keys::generate();
    let invite_author = Keys::generate();
    let message_author = Keys::generate();
    let group_author = Keys::generate();
    let invite_response_recipient = Keys::generate();
    let plan = ProtocolSubscriptionPlan {
        runtime_subscriptions: vec!["ndr-protocol".to_string()],
        roster_authors: vec![owner.public_key().to_hex()],
        invite_authors: vec![invite_author.public_key().to_hex()],
        message_authors: vec![message_author.public_key().to_hex()],
        group_sender_key_authors: vec![group_author.public_key().to_hex()],
        invite_response_recipient: Some(invite_response_recipient.public_key().to_hex()),
    };

    let filters = build_protocol_subscription_filters(&plan);

    assert!(
        has_filter_with_kind_author(&filters, APP_KEYS_EVENT_KIND, owner.public_key()),
        "app-key filters must be derived from roster authors"
    );
    assert!(
        has_filter_with_kind_author_tag(
            &filters,
            APP_KEYS_EVENT_KIND,
            owner.public_key(),
            "#d",
            NDR_APP_KEYS_D_TAG
        ),
        "app-key filters must not fetch unrelated parameterized app data"
    );
    assert!(
        has_filter_with_kind_author(&filters, INVITE_EVENT_KIND, invite_author.public_key()),
        "invite filters must be derived from known device authors"
    );
    assert!(
        has_filter_with_kind_author_tag(
            &filters,
            INVITE_EVENT_KIND,
            invite_author.public_key(),
            "#l",
            NDR_INVITES_L_TAG
        ),
        "invite filters must not fetch unrelated parameterized app data"
    );
    assert!(
        has_filter_with_kind_author(&filters, INVITE_RESPONSE_KIND, invite_author.public_key()),
        "invite-response author filters must be derived from known device authors"
    );
    assert!(
        has_filter_with_kind_author(&filters, MESSAGE_EVENT_KIND, message_author.public_key()),
        "message filters must be derived from message authors"
    );
    assert!(
        has_filter_with_kind_author(
            &filters,
            GROUP_SENDER_KEY_MESSAGE_KIND,
            group_author.public_key()
        ),
        "group sender-key filters must be derived from group authors"
    );
    assert!(
        has_filter_with_kind_pubkey(
            &filters,
            INVITE_RESPONSE_KIND,
            invite_response_recipient.public_key()
        ),
        "private invite-response filters must be derived from recipient #p values"
    );
}

#[test]
fn recent_protocol_filters_backfill_messages_without_time_or_count_bounds() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let message_author = Keys::generate();
    let group_author = Keys::generate();
    let mut core = logged_in_test_core("protocol-backfill-unbounded", &owner, &device);
    core.protocol_subscription_runtime.desired_plan = Some(protocol_plan_for_test(
        vec![message_author.public_key()],
        vec![group_author.public_key()],
    ));

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let find_filter = |kind: u32, author: PublicKey| {
        let author_hex = author.to_hex();
        filters
            .iter()
            .map(|filter| serde_json::to_value(filter).expect("filter json"))
            .find(|filter| {
                let has_kind = filter
                    .get("kinds")
                    .and_then(|kinds| kinds.as_array())
                    .is_some_and(|kinds| {
                        kinds
                            .iter()
                            .any(|value| value.as_u64() == Some(kind as u64))
                    });
                let has_author = filter
                    .get("authors")
                    .and_then(|authors| authors.as_array())
                    .is_some_and(|authors| {
                        authors
                            .iter()
                            .any(|value| value.as_str() == Some(author_hex.as_str()))
                    });
                has_kind && has_author
            })
            .expect("history backfill filter")
    };
    let message_filter = find_filter(MESSAGE_EVENT_KIND, message_author.public_key());
    let group_filter = find_filter(GROUP_SENDER_KEY_MESSAGE_KIND, group_author.public_key());

    for filter in [message_filter, group_filter] {
        assert!(
            filter.get("since").is_none(),
            "message history catch-up must not have a time bound"
        );
        assert!(
            filter.get("limit").is_none(),
            "message history catch-up must not have a count bound"
        );
    }
}

#[test]
fn recent_protocol_filters_scope_message_backfill_to_known_authors() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let message_author = Keys::generate();
    let mut core = logged_in_test_core("protocol-backfill-scoped", &owner, &device);
    core.protocol_subscription_runtime.desired_plan = Some(protocol_plan_for_test(
        vec![message_author.public_key()],
        Vec::new(),
    ));

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let message_author_hex = message_author.public_key().to_hex();
    let message_filter = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .find(|filter| {
            let has_message_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(MESSAGE_EVENT_KIND as u64))
                });
            let has_author = filter
                .get("authors")
                .and_then(|authors| authors.as_array())
                .is_some_and(|authors| {
                    authors
                        .iter()
                        .any(|author| author.as_str() == Some(message_author_hex.as_str()))
                });
            has_message_kind && has_author
        })
        .expect("message backfill filter");

    assert!(
        message_filter.get("authors").is_some(),
        "message history catch-up must remain scoped to known authors"
    );
}

#[test]
fn recent_protocol_filters_do_not_include_unscoped_message_backfill_for_cold_tracked_peer() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("protocol-backfill-cold-peer", &owner, &device);
    core.active_chat_id = Some(peer.public_key().to_hex());

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        !has_bootstrap_message_filter(&filters),
        "cold peer discovery must fetch protocol state, not unscoped public message events"
    );
}

#[test]
fn unknown_direct_message_author_is_ignored_instead_of_bootstrapping_public_backfill() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let mallory_keys = Keys::generate();
    let carol_keys = Keys::generate();
    let mut alice_invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    alice_invite.owner_public_key = Some(alice_keys.public_key());
    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(alice_invite.clone()),
    );
    alice.init().expect("alice init");
    let bob_runtime = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        None,
        None,
    );
    bob_runtime.init().expect("bob init");
    accept_invite_and_deliver(
        &bob_runtime,
        &bob_keys,
        &alice_invite,
        alice_keys.public_key(),
        &alice,
    );
    complete_first_contact(&bob_runtime, &bob_keys, alice_keys.public_key(), &alice);
    let alice_session_state = bob_runtime
        .get_message_push_session_states(alice_keys.public_key())
        .into_iter()
        .next()
        .expect("Bob has Alice session")
        .state;

    let mut mallory_invite = Invite::create_new(
        mallory_keys.public_key(),
        Some(mallory_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("mallory invite");
    mallory_invite.owner_public_key = Some(mallory_keys.public_key());
    let mallory = NdrRuntime::new(
        mallory_keys.public_key(),
        mallory_keys.secret_key().to_secret_bytes(),
        mallory_keys.public_key().to_hex(),
        mallory_keys.public_key(),
        None,
        Some(mallory_invite.clone()),
    );
    mallory.init().expect("mallory init");
    let carol_runtime = NdrRuntime::new(
        carol_keys.public_key(),
        carol_keys.secret_key().to_secret_bytes(),
        carol_keys.public_key().to_hex(),
        carol_keys.public_key(),
        None,
        None,
    );
    carol_runtime.init().expect("carol init");
    accept_invite_and_deliver(
        &carol_runtime,
        &carol_keys,
        &mallory_invite,
        mallory_keys.public_key(),
        &mallory,
    );
    complete_first_contact(
        &carol_runtime,
        &carol_keys,
        mallory_keys.public_key(),
        &mallory,
    );
    mallory
        .send_text(
            carol_keys.public_key(),
            "queued until unrelated protocol state arrives".to_string(),
            None,
        )
        .expect("mallory sends");
    let carol_message_authors = carol_runtime.get_all_message_push_author_pubkeys();
    let message_event = drain_signed_events(&mallory, &mallory_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && carol_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Carol");
    let message_event_id = message_event.id.to_string();

    let mut core = logged_in_test_core("pending-inbound-keeps-bootstrap", &bob_keys, &bob_keys);
    core.active_chat_id = Some(alice_keys.public_key().to_hex());
    let alice_app_keys = AppKeys::new(vec![DeviceEntry::new(alice_keys.public_key(), 1)]);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(alice_keys.public_key(), alice_app_keys.clone(), 1)
        .expect("alice appkeys");
    core.process_protocol_engine_retry_batch("test_alice_appkeys", batch);
    core.app_keys.insert(
        alice_keys.public_key().to_hex(),
        known_app_keys_from_ndr(alice_keys.public_key(), &alice_app_keys, 1),
    );
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .import_session_state(
            alice_keys.public_key(),
            Some(alice_keys.public_key().to_hex()),
            alice_session_state,
            UnixSeconds(2),
        )
        .expect("alice session import");
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(alice_keys.public_key())
            .is_empty()
            == false,
        "tracked peer starts with app keys and known message authors"
    );
    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "without pending inbound work the known peer no longer needs broad bootstrap"
    );
    core.handle_relay_event(message_event);
    assert!(
        !core
            .protocol_engine
            .as_ref()
            .expect("protocol engine")
            .has_pending_inbound_direct_events(),
        "unknown public message authors must not become durable pending inbound work"
    );
    assert!(
        !core.has_seen_event(&message_event_id),
        "ignored encrypted message events must stay retryable because later bootstrap state can make the sender decryptable"
    );
    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "ignored unknown messages must not enable unscoped public backfill"
    );
}

#[test]
fn direct_message_discovery_backfill_stays_scoped_for_partial_tracked_peer_state() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core(
        "partial-tracked-peer-message-bootstrap",
        &owner,
        &linked_device,
    );

    install_local_sibling_session_for_test(&mut core, &owner, &linked_device, &primary_device);
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(owner.public_key())
            .len()
            > 0,
        "linked device should already know a primary-device message author"
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(peer.public_key())
            .is_empty(),
        "fresh peer should still need message-author discovery"
    );
    core.active_chat_id = Some(peer.public_key().to_hex());

    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "partial peer state must not trigger unscoped public message backfill"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        !has_bootstrap_message_filter(&active_filters),
        "unscoped live message subscriptions flood public relays; bootstrap discovery must stay bounded to backfill"
    );
}

#[test]
fn direct_message_discovery_does_not_install_cold_peer_live_bootstrap_subscription() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("cold-peer-no-live-bootstrap", &owner, &device);
    core.active_chat_id = Some(peer.public_key().to_hex());

    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "cold tracked peer discovery should stay on protocol-state filters"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        !has_bootstrap_message_filter(&active_filters),
        "cold direct chats should not create an unscoped live message subscription"
    );
}

fn has_bootstrap_message_filter(filters: &[Filter]) -> bool {
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_message_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(MESSAGE_EVENT_KIND as u64))
                });
            has_message_kind && filter.get("authors").is_none()
        })
}

fn desired_protocol_filters(core: &AppCore) -> Vec<Filter> {
    build_protocol_subscription_filters(
        core.protocol_subscription_runtime
            .desired_plan
            .as_ref()
            .expect("desired protocol plan"),
    )
}

fn has_filter_with_kind_author(filters: &[Filter], kind: u32, author: PublicKey) -> bool {
    let author_hex = author.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_author = filter
                .get("authors")
                .and_then(|authors| authors.as_array())
                .is_some_and(|authors| {
                    authors
                        .iter()
                        .any(|value| value.as_str() == Some(author_hex.as_str()))
                });
            has_kind && has_author
        })
}

fn has_filter_with_kind_author_tag(
    filters: &[Filter],
    kind: u32,
    author: PublicKey,
    tag_name: &str,
    tag_value: &str,
) -> bool {
    let author_hex = author.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_author = filter
                .get("authors")
                .and_then(|authors| authors.as_array())
                .is_some_and(|authors| {
                    authors
                        .iter()
                        .any(|value| value.as_str() == Some(author_hex.as_str()))
                });
            let has_tag = filter
                .get(tag_name)
                .and_then(|values| values.as_array())
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(tag_value)));
            has_kind && has_author && has_tag
        })
}

fn has_filter_with_kind_pubkey(filters: &[Filter], kind: u32, pubkey: PublicKey) -> bool {
    let pubkey_hex = pubkey.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_pubkey = filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|value| value.as_str() == Some(pubkey_hex.as_str()))
                });
            has_kind && has_pubkey
        })
}

fn install_local_sibling_session_for_test(
    core: &mut AppCore,
    owner: &Keys,
    linked_device: &Keys,
    primary_device: &Keys,
) {
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("local appkeys");

    let linked_invite = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .local_invite_for_test()
        .expect("linked invite");
    let (_primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet_nostr::invite_response_event(&response)
            .expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");
}

#[test]
fn create_invite_generates_private_link_without_public_republish() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("private-invite-create", &owner, &device);
    core.pending_relay_publishes.clear();

    let local_invite_response_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_string();

    core.handle_action(AppAction::CreatePublicInvite);

    assert_eq!(core.state.toast, None);
    let snapshot = core
        .state
        .public_invite
        .as_ref()
        .expect("private invite snapshot");
    let invite =
        super::invites::parse_public_invite_input(&snapshot.url).expect("parse private invite");
    assert_eq!(invite.purpose.as_deref(), Some("private"));
    assert_eq!(invite.max_uses, Some(1));
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_ne!(
        invite.inviter_ephemeral_public_key.to_string(),
        local_invite_response_pubkey,
        "private invite links must not reuse the relay-published local invite secret"
    );
    assert_eq!(
        core.private_chat_invites
            .values()
            .next()
            .map(|invite| invite.inviter_ephemeral_public_key),
        Some(invite.inviter_ephemeral_public_key)
    );
    assert!(
        pending_events_with_kind(&core, INVITE_EVENT_KIND).is_empty(),
        "creating a private invite link must not publish a relay-discoverable invite event"
    );

    let invite_pubkey_hex = invite.inviter_ephemeral_public_key.to_string();
    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let subscribed_for_response = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|pubkey| pubkey.as_str() == Some(invite_pubkey_hex.as_str()))
                })
        });
    assert!(subscribed_for_response);
}

#[test]
fn private_invite_first_message_installs_creator_session() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "private-invite-roundtrip-alice",
        &alice_owner,
        &alice_device,
    );
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("private-invite-roundtrip-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    assert_eq!(bob.state.toast, None);
    assert_eq!(bob.active_chat_id, Some(alice_owner.public_key().to_hex()));

    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "hello from private invite".to_string(),
    });
    assert!(
        bob.protocol_engine
            .as_ref()
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty()),
        "sending through a private invite must install scoped message authors"
    );
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");
    alice.handle_relay_event(response);

    assert!(
        alice.protocol_engine.as_ref().is_some_and(|engine| {
            engine.active_session_count_for_owner(bob_owner.public_key()) > 0
        }),
        "Alice should install Bob's session from the private invite response"
    );
    assert!(
        alice.private_chat_invites.is_empty(),
        "one-use private invite should be removed after a matching response"
    );
    assert!(
        alice
            .protocol_engine
            .as_ref()
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty()),
        "private invite response import must immediately enable scoped peer message discovery"
    );
}

// Regression: when iOS scans a chat-invite QR from chat.iris.to and accepts it,
// the iris-chat TypeScript runtime publishes the invite-response + a typing
// bootstrap immediately. Without that bootstrap, the inviter never learns the
// invitee's session pubkey, the invitee never installs a session for the
// inviter's ephemeral key, and the live relay REQ excludes that key — so
// the inviter's replies never reach the device. The user hit this end-to-end:
// chat.iris.to saw iOS's messages, but iOS never saw chat.iris.to's replies.
#[test]
fn accepting_invite_alone_installs_session_and_publishes_response() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice =
        logged_in_test_core("accept-invite-bootstrap-alice", &alice_owner, &alice_device);
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("accept-invite-bootstrap-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    assert_eq!(bob.state.toast, None);

    // After accept alone (no SendMessage yet), Bob must already have a session
    // installed for Alice — otherwise his subscription plan will exclude her
    // ephemeral key and her first reply will be invisible.
    assert!(
        bob.protocol_engine
            .as_ref()
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty()),
        "accepting an invite must install a session so the inviter's replies are subscribed to"
    );

    // And Bob must have published the invite-response so Alice can install
    // a session and start sending.
    assert!(
        !pending_events_with_kind(&bob, INVITE_RESPONSE_KIND).is_empty(),
        "accepting an invite must publish an invite-response so the inviter can establish the session"
    );

    // Round-trip: Alice processes the response and now has Bob in her message
    // authors, so Alice can send back.
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");
    alice.handle_relay_event(response);
    assert!(
        alice.protocol_engine.as_ref().is_some_and(|engine| engine
            .active_session_count_for_owner(bob_owner.public_key())
            > 0),
        "Alice must install Bob's session from the invite-response"
    );
}

#[test]
fn queued_runtime_publish_completion_uses_inner_message_id() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let inner_message_id = "inner-rumor-id".to_string();
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&owner)
        .expect("outer event");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-completion-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 1,
            messages: vec![ChatMessageSnapshot {
                id: inner_message_id.clone(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: owner.public_key().to_hex(),
                body: "queued".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: true,
                created_at_secs: 1,
                expires_at_secs: None,
                delivery: DeliveryState::Queued,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
            }],

            draft: String::new(),
        },
    );

    assert_eq!(
        core.runtime_publish_completion(
            &outer_event.id.to_string(),
            Some(&inner_message_id),
            &BTreeMap::new(),
        ),
        Some((inner_message_id, chat_id))
    );
}

#[test]
fn web_runtime_message_duplicates_dedupe_by_inner_rumor_id() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-dedupe", &owner, &device);
    let first_outer_id = "b".repeat(64);
    let second_outer_id = "c".repeat(64);
    let (content, inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "ok",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        content.clone(),
        Some(first_outer_id),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some(second_outer_id));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    let matching = thread
        .messages
        .iter()
        .filter(|message| message.body == "ok")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, inner_id);
}

#[test]
fn runtime_message_from_unknown_sender_can_be_blocked() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("runtime-block-unknown-direct", &owner, &device);
    core.preferences.accept_unknown_direct_messages = false;
    let (content, _inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "unknown direct",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("d".repeat(64)));

    assert!(!core.threads.contains_key(&sender.public_key().to_hex()));
}

#[test]
fn runtime_message_from_known_sender_is_kept_when_unknowns_are_blocked() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("runtime-block-unknown-known-direct", &owner, &device);
    core.preferences.accept_unknown_direct_messages = false;
    core.handle_action(AppAction::CreateChat {
        peer_input: sender.public_key().to_hex(),
    });
    let (content, inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "known direct",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("e".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("known thread");
    assert!(thread
        .messages
        .iter()
        .any(|message| message.id == inner_id && message.body == "known direct"));
}

#[test]
fn private_invite_response_from_unknown_sender_can_be_blocked() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "private-invite-block-unknown-alice",
        &alice_owner,
        &alice_device,
    );
    alice.pending_relay_publishes.clear();
    alice.preferences.accept_unknown_direct_messages = false;
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("private-invite-block-unknown-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "hello from stranger".to_string(),
    });
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");

    alice.handle_relay_event(response);

    assert!(
        !alice.threads.contains_key(&bob_owner.public_key().to_hex()),
        "blocked invite response must not create a thread for the stranger"
    );
    assert!(
        !alice.private_chat_invites.is_empty(),
        "blocked invite response must not consume the invite"
    );
}

#[test]
fn stranger_message_creates_is_request_thread_with_default_settings() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("stranger-request-default", &owner, &device);
    let (content, _inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hello from a stranger",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("a".repeat(64)));
    core.rebuild_state();

    let chat_id = sender.public_key().to_hex();
    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("stranger thread must surface in the chat list");
    assert!(
        snapshot.is_request,
        "stranger thread without accept is a request"
    );
}

#[test]
fn explicit_accept_clears_is_request_without_outgoing_message() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("stranger-explicit-accept", &owner, &device);
    let (content, _inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hi",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("b".repeat(64)));
    let chat_id = sender.public_key().to_hex();

    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: chat_id.clone(),
    });

    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("thread visible after accept");
    assert!(
        !snapshot.is_request,
        "explicit accept must clear the request gate even without a reply"
    );
    assert!(
        core.state
            .preferences
            .accepted_owner_pubkeys
            .contains(&chat_id),
        "accept persists the peer in the whitelist"
    );
}

