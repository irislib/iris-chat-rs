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
fn app_keys_cache_ignores_older_roster_events() {
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
            .is_none(),
        "older owner-signed app-key events must not resurrect removed devices"
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
    assert!(!cached
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
fn removing_authorized_device_advances_app_keys_timestamp() {
    let owner = Keys::generate();
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-roster-remove-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let previous_created_at = unix_now().get().saturating_add(60);
    let linked_device_created_at = previous_created_at + 1;
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![
                DeviceEntry::new(device_a.public_key(), previous_created_at),
                DeviceEntry::new(device_b.public_key(), linked_device_created_at),
            ]),
            previous_created_at,
        ),
    );

    core.remove_local_app_key_device(owner.public_key(), device_b.public_key());

    let cached = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("cached roster");
    assert_eq!(cached.created_at_secs, linked_device_created_at + 2);
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_a.public_key().to_hex()));
    assert!(!cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_b.public_key().to_hex()));

    let stale_linked_cache = AppKeys::new(vec![
        DeviceEntry::new(device_a.public_key(), previous_created_at),
        DeviceEntry::new(device_b.public_key(), linked_device_created_at),
    ]);
    let published_removal = known_app_keys_to_ndr(cached).expect("published removal");
    let applied = apply_app_keys_snapshot_with_required_device(
        Some(&stale_linked_cache),
        linked_device_created_at,
        &published_removal,
        cached.created_at_secs,
        None,
    );
    assert!(applied
        .app_keys
        .get_device(&device_b.public_key())
        .is_none());

    let resurrected = apply_app_keys_snapshot_with_required_device(
        Some(&published_removal),
        cached.created_at_secs,
        &stale_linked_cache,
        linked_device_created_at,
        None,
    );
    assert!(resurrected
        .app_keys
        .get_device(&device_b.public_key())
        .is_none());
}

#[test]
fn remove_authorized_device_action_updates_roster_immediately() {
    let owner = Keys::generate();
    let current_device = Keys::generate();
    let linked_device = Keys::generate();
    let mut core = logged_in_test_core(
        "device-roster-remove-immediate",
        &owner,
        &current_device,
    );

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: linked_device.public_key().to_hex(),
    });
    assert!(core
        .state
        .device_roster
        .as_ref()
        .expect("device roster after add")
        .devices
        .iter()
        .any(|entry| entry.device_pubkey_hex == linked_device.public_key().to_hex()));

    let rev_before_remove = core.state.rev;
    core.handle_action(AppAction::RemoveAuthorizedDevice {
        device_pubkey_hex: linked_device.public_key().to_hex(),
    });

    let roster = core
        .state
        .device_roster
        .as_ref()
        .expect("device roster after remove");
    assert!(
        !core.state.busy.updating_roster,
        "remove device must not wait for a roster publish drain"
    );
    assert!(
        core.state.rev > rev_before_remove,
        "remove device should emit an immediate state update"
    );
    assert!(
        roster
            .devices
            .iter()
            .all(|entry| entry.device_pubkey_hex != linked_device.public_key().to_hex()),
        "removed linked device should disappear from the visible roster"
    );
    assert!(core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("local app keys roster")
        .devices
        .iter()
        .all(|entry| entry.identity_pubkey_hex != linked_device.public_key().to_hex()));
}

#[test]
fn removing_authorized_device_beats_equal_timestamp_roster_merge() {
    assert_eq!(account_app_keys::next_removed_app_keys_created_at(100, 80, 90), 102);
    assert_eq!(account_app_keys::next_removed_app_keys_created_at(100, 110, 90), 112);
    assert_eq!(account_app_keys::next_removed_app_keys_created_at(100, 80, 120), 122);
    assert_eq!(account_app_keys::next_removed_app_keys_created_at(100, 100, 90), 102);
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
    use nostr_double_ratchet::InviteNostrExt;

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
fn device_roster_lists_current_device_then_newest_linked_devices() {
    let owner = Keys::generate();
    let current_device = Keys::generate();
    let older_device = Keys::generate();
    let newer_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_primary_session(owner.clone(), current_device.clone(), false, false)
        .expect("primary session");
    let remote_created_at = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("local app keys")
        .created_at_secs
        + 10;
    let remote_app_keys = AppKeys::new(vec![
        DeviceEntry::new(older_device.public_key(), remote_created_at + 1),
        DeviceEntry::new(newer_device.public_key(), remote_created_at + 3),
        DeviceEntry::new(current_device.public_key(), remote_created_at + 2),
    ]);
    let remote_event = remote_app_keys
        .get_event_at(owner.public_key(), remote_created_at)
        .sign_with_keys(&owner)
        .expect("app keys event");

    core.apply_app_keys_event(&remote_event)
        .expect("apply remote app keys");

    let ordered_devices = core
        .state
        .device_roster
        .as_ref()
        .expect("device roster")
        .devices
        .iter()
        .map(|entry| entry.device_pubkey_hex.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_devices,
        vec![
            current_device.public_key().to_hex(),
            newer_device.public_key().to_hex(),
            older_device.public_key().to_hex(),
        ]
    );
}

#[test]
fn start_linked_device_creates_full_device_approval_request() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();

    core.handle_action(AppAction::SetCurrentDeviceLabels {
        device_label: "Safari on macOS".to_string(),
        client_label: "Iris Chat Web".to_string(),
    });
    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let snapshot = core
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot");
    let request = parse_nostr_identity_device_approval_request(&snapshot.url, &[])
        .expect("parse approval request")
        .expect("device approval request");
    assert!(snapshot.url.starts_with("nostr-identity://device-approval/"));
    assert!(!request.device_app_key_proof.is_empty());
    assert_eq!(request.request_type.as_deref(), Some("device_link"));
    assert_eq!(
        request.device_app_key_pubkey,
        core.pending_linked_device
            .as_ref()
            .expect("pending link")
            .device_keys
            .public_key()
            .to_hex()
    );
    assert_eq!(
        request.request_pubkey,
        core.pending_linked_device
            .as_ref()
            .expect("pending link")
            .request_keys
            .public_key()
            .to_hex()
    );
    assert_eq!(request.request_secret.len(), 64);
    assert_eq!(request.label.as_deref(), Some("Safari on macOS"));
    assert_eq!(
        PublicKey::from_hex(&request.device_app_key_pubkey)
            .expect("request device pubkey")
            .to_bech32()
            .ok()
            .as_deref(),
        Some(snapshot.device_input.as_str())
    );
    assert!(matches!(core.screen_stack.as_slice(), [Screen::AddDevice]));
}

fn signed_app_keys_authorization_event(
    owner: &Keys,
    device_pubkey: PublicKey,
    created_at: u64,
) -> Event {
    AppKeys::new(vec![DeviceEntry::new(device_pubkey, created_at)])
        .get_event_at(owner.public_key(), created_at)
        .sign_with_keys(owner)
        .expect("signed app keys authorization")
}

fn signed_device_approval_receipt_event(
    owner_device: &Keys,
    request_keys: &Keys,
    linked_device_pubkey: PublicKey,
    owner_pubkey: PublicKey,
    created_at: u64,
) -> Event {
    build_nostr_identity_device_approval_receipt_event(
        owner_device,
        NostrIdentityDeviceApprovalReceipt {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
            profile_id: account::nostr_identity_profile_id_for_owner(owner_pubkey),
            request_pubkey: request_keys.public_key().to_hex(),
            device_app_key_pubkey: linked_device_pubkey.to_hex(),
            approved_by_pubkey: owner_device.public_key().to_hex(),
            approved_at: i64::try_from(created_at).expect("created_at fits i64"),
            request_secret: request_keys.secret_key().to_secret_hex(),
            subject_pubkey: Some(owner_pubkey.to_hex()),
            roster_op_id: None,
            signed_roster_event: None,
        },
    )
    .expect("signed device approval receipt")
}

#[test]
fn full_link_request_finishes_pairing_and_authorizes_linked_device() {
    let owner = Keys::generate();
    let temp_dir_primary = tempfile::TempDir::new().expect("primary temp dir");
    let temp_dir_linked = tempfile::TempDir::new().expect("linked temp dir");
    let mut primary = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir_primary.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    primary.preferences.nostr_relay_urls.clear();
    primary
        .start_primary_session(owner.clone(), owner.clone(), false, false)
        .expect("primary session");
    primary.pending_relay_publishes.clear();

    let mut linked = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir_linked.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    linked.preferences.nostr_relay_urls.clear();
    linked.handle_action(AppAction::SetCurrentDeviceLabels {
        device_label: "Safari on macOS".to_string(),
        client_label: "Iris Chat Web".to_string(),
    });
    linked.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });
    let approval_request = linked
        .state
        .link_device
        .as_ref()
        .expect("device approval request")
        .url
        .clone();
    let linked_device_hex = linked
        .pending_linked_device
        .as_ref()
        .expect("pending linked device")
        .device_keys
        .public_key()
        .to_hex();

    primary.handle_action(AppAction::AddAuthorizedDevice {
        device_input: approval_request,
    });

    assert_eq!(primary.state.toast.as_deref(), Some("Device added"));
    let response_event = pending_events_with_kind(&primary, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("approval publishes deterministic invite response");
    let app_keys_event = pending_events_with_kind(&primary, APP_KEYS_EVENT_KIND)
        .into_iter()
        .find(|event| event_has_tag_value(event, "device", &linked_device_hex))
        .expect("approval publishes AppKeys authorization");
    let receipt_event = pending_events_with_kind(&primary, u32::from(FACT_OP_KIND))
        .into_iter()
        .find(|event| event_has_tag_value(
            event,
            "type",
            "nostr_identity_device_approval_receipt"
        ))
        .expect("approval publishes encrypted receipt");

    linked.handle_relay_event(response_event);
    assert!(
        linked.pending_linked_device.is_some(),
        "invite response alone must wait for device approval"
    );

    linked.handle_relay_event(app_keys_event);
    assert!(
        linked.pending_linked_device.is_some(),
        "AppKeys without the encrypted receipt must not finish device linking"
    );

    linked.handle_relay_event(receipt_event);
    let linked_device = linked
        .logged_in
        .as_ref()
        .expect("linked session")
        .device_keys
        .public_key();

    linked.refresh_local_authorization_state();
    linked.rebuild_state();
    let logged_in = linked.logged_in.as_ref().expect("linked logged in");
    let active_session_count = linked
        .protocol_engine
        .as_ref()
        .map(|engine| engine.active_session_count_for_owner(owner.public_key()))
        .unwrap_or_default();
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::Authorized,
        "linked_device={} active_sessions={} app_keys={:?} debug={:?}",
        linked_device.to_hex(),
        active_session_count,
        linked.app_keys,
        linked.debug_log
    );
    assert!(linked.pending_linked_device.is_none());
    assert!(active_session_count > 0);
    let roster = linked
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("linked learned owner roster");
    assert!(roster
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == linked_device.to_hex()));
    let linked_roster_device = primary
        .app_keys
        .get(&owner.public_key().to_hex())
        .and_then(|roster| {
            roster
                .devices
                .iter()
                .find(|device| device.identity_pubkey_hex == linked_device.to_hex())
        })
        .expect("primary signed linked device labels");
    assert_eq!(
        linked_roster_device.device_label.as_deref(),
        Some("Safari on macOS")
    );
}

#[test]
fn owner_device_rejects_legacy_link_invite_for_device_approval() {
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

    let known = core.app_keys.get(&owner.public_key().to_hex());
    assert!(!known
        .into_iter()
        .flat_map(|known| known.devices.iter())
        .any(|device| device.identity_pubkey_hex == new_device.public_key().to_hex()));
    assert_eq!(core.state.toast.as_deref(), Some("Invalid device key."));
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

    let linked_device_pubkey = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite")
        .device_keys
        .public_key();
    let request_keys = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite")
        .request_keys
        .clone();
    let pending = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite");
    let (_owner_session, response_envelope) = pending
        .pairing_invite
        .accept_with_owner(
            owner.public_key(),
            owner.secret_key().to_secret_bytes(),
            Some(owner.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("owner accepts");
    let response_event = nostr_double_ratchet::invite_response_event(&response_envelope)
        .expect("invite response event");

    core.handle_relay_event(response_event);
    assert!(
        core.pending_linked_device.is_some(),
        "invite response waits for owner-signed AppKeys authorization and approval receipt"
    );
    assert!(core.logged_in.is_none());

    core.handle_relay_event(signed_app_keys_authorization_event(
        &owner,
        linked_device_pubkey,
        42,
    ));
    assert!(
        core.pending_linked_device.is_some(),
        "AppKeys authorization without the encrypted approval receipt must not finish"
    );

    core.handle_relay_event(signed_device_approval_receipt_event(
        &owner,
        &request_keys,
        linked_device_pubkey,
        owner.public_key(),
        42,
    ));

    let logged_in = core.logged_in.as_ref().expect("linked session");
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::Authorized
    );
    assert!(core.pending_linked_device.is_none());
    assert!(core
        .protocol_engine
        .as_ref()
        .is_some_and(|engine| engine.active_session_count_for_owner(owner.public_key()) > 0));
}

#[test]
fn completed_pairing_discards_pairing_invite_and_creates_stable_local_invite() {
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
    let pairing_invite = pending.pairing_invite.clone();
    let linked_device_pubkey = pending.device_keys.public_key();
    let request_keys = pending.request_keys.clone();
    assert_eq!(pairing_invite.purpose.as_deref(), Some("link"));
    assert!(pairing_invite.owner_public_key.is_none());

    let (_owner_session, response_envelope) = pairing_invite
        .accept_with_owner(
            owner.public_key(),
            owner.secret_key().to_secret_bytes(),
            Some(owner.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("owner accepts");
    let response_event = nostr_double_ratchet::invite_response_event(&response_envelope)
        .expect("invite response event");

    core.handle_relay_event(signed_app_keys_authorization_event(
        &owner,
        linked_device_pubkey,
        42,
    ));
    core.handle_relay_event(signed_device_approval_receipt_event(
        &owner,
        &request_keys,
        linked_device_pubkey,
        owner.public_key(),
        42,
    ));
    core.handle_relay_event(response_event);

    let stable_invite = core
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("stable local invite");
    assert!(core.pending_linked_device.is_none());
    assert_eq!(stable_invite.owner_public_key, Some(owner.public_key()));
    assert_ne!(
        stable_invite.inviter_ephemeral_public_key,
        pairing_invite.inviter_ephemeral_public_key
    );
    assert_ne!(stable_invite.purpose.as_deref(), Some("link"));
    assert_ne!(stable_invite.max_uses, Some(1));
}

#[test]
fn local_relay_pairing_e2e_uses_stable_protocol_invite_after_login() {
    let owner = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let relay_url = relay.url().to_string();
    let relay_urls = relay_urls_from_strings(std::slice::from_ref(&relay_url));
    let primary_temp_dir = tempfile::TempDir::new().expect("primary temp dir");
    let linked_temp_dir = tempfile::TempDir::new().expect("linked temp dir");
    let (primary_update_tx, _) = flume::unbounded();
    let (primary_core_tx, primary_core_rx) = flume::unbounded();
    let mut primary = AppCore::new(
        primary_update_tx,
        primary_core_tx,
        primary_temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    primary.preferences.nostr_relay_urls = vec![relay_url.clone()];
    primary
        .start_primary_session(owner.clone(), owner.clone(), false, false)
        .expect("primary session");
    {
        let logged_in = primary.logged_in.as_mut().expect("primary logged in");
        logged_in.relay_urls = relay_urls.clone();
        let client = logged_in.client.clone();
        let connected = primary.runtime.block_on(async {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let connected = client
                    .relays()
                    .await
                    .values()
                    .filter(|relay| relay.status() == RelayStatus::Connected)
                    .count();
                if connected > 0 || Instant::now() >= deadline {
                    break connected;
                }
                sleep(Duration::from_millis(50)).await;
            }
        });
        assert!(connected > 0, "test relay must be connected");
    }
    primary.refresh_relay_connection_status();

    let mut linked = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        linked_temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    linked.preferences.nostr_relay_urls = vec![relay_url];
    linked.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });
    let pairing_invite = linked
        .pending_linked_device
        .as_ref()
        .expect("pending linked device")
        .pairing_invite
        .clone();
    let linked_device_hex = linked
        .pending_linked_device
        .as_ref()
        .expect("pending linked device")
        .device_keys
        .public_key()
        .to_hex();
    let pairing_response_pubkey = pairing_invite
        .inviter_ephemeral_public_key
        .to_nostr()
        .expect("pairing response pubkey");
    let pairing_url = linked
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot")
        .url
        .clone();

    primary.handle_action(AppAction::AddAuthorizedDevice {
        device_input: pairing_url,
    });
    let app_keys_event = pending_events_with_kind(&primary, APP_KEYS_EVENT_KIND)
        .into_iter()
        .find(|event| event_has_tag_value(event, "device", &linked_device_hex))
        .expect("owner publishes AppKeys authorization for linked device");
    let receipt_event = pending_events_with_kind(&primary, u32::from(FACT_OP_KIND))
        .into_iter()
        .find(|event| event_has_tag_value(
            event,
            "type",
            "nostr_identity_device_approval_receipt"
        ))
        .expect("owner publishes encrypted device approval receipt");
    let response_event = wait_for_relay_event_with_kind(
        &mut primary,
        &primary_core_rx,
        &relay,
        INVITE_RESPONSE_KIND,
    );
    linked.handle_relay_event(app_keys_event);
    linked.handle_relay_event(receipt_event);
    linked.handle_relay_event(response_event);

    let stable_invite = linked
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("stable local invite");
    let stable_response_pubkey = stable_invite
        .inviter_ephemeral_public_key
        .to_nostr()
        .expect("stable response pubkey");
    assert!(linked.pending_linked_device.is_none());
    assert_eq!(stable_invite.owner_public_key, Some(owner.public_key()));
    assert_ne!(stable_response_pubkey, pairing_response_pubkey);
    assert_ne!(stable_invite.purpose.as_deref(), Some("link"));

    let public_invite = linked
        .build_public_invite_snapshot()
        .and_then(|snapshot| super::invites::parse_public_invite_input(&snapshot.url).ok())
        .expect("public invite snapshot");
    assert_eq!(
        public_invite.inviter_ephemeral_public_key,
        stable_invite.inviter_ephemeral_public_key
    );
    assert_ne!(
        public_invite.inviter_ephemeral_public_key,
        pairing_invite.inviter_ephemeral_public_key
    );

    let filters = linked.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        has_filter_with_kind_pubkey(&filters, INVITE_RESPONSE_KIND, stable_response_pubkey),
        "protocol filters must track the stable invite response key"
    );
    assert!(
        !has_filter_with_kind_pubkey(&filters, INVITE_RESPONSE_KIND, pairing_response_pubkey),
        "protocol filters must not keep tracking the temporary pairing invite"
    );

    let push = linked.build_mobile_push_sync_snapshot();
    assert!(push
        .invite_response_pubkeys
        .contains(&stable_response_pubkey.to_hex()));
    assert!(!push
        .invite_response_pubkeys
        .contains(&pairing_response_pubkey.to_hex()));
}

#[test]
fn recent_protocol_filters_include_runtime_invite_response_backfill() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("protocol-backfill-invite-response", &owner, &device);
    let invite_response_pubkey = core
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("local invite")
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
    let group_admin = Keys::generate();
    let group_author = Keys::generate();
    let invite_response_recipient = Keys::generate();
    let plan = ProtocolSubscriptionPlan {
        runtime_subscriptions: vec!["ndr-protocol".to_string()],
        roster_authors: vec![owner.public_key().to_hex()],
        invite_authors: vec![invite_author.public_key().to_hex()],
        message_authors: vec![message_author.public_key().to_hex()],
        message_recipients: Vec::new(),
        group_roster_group_ids: vec!["test-group".to_string()],
        group_roster_authors: vec![group_admin.public_key().to_hex()],
        group_sender_key_authors: vec![group_author.public_key().to_hex()],
        invite_response_recipient: Some(invite_response_recipient.public_key().to_hex()),
    };

    let filters = build_protocol_subscription_filters(&plan);

    assert!(
        has_filter_with_kind_author(&filters, APP_KEYS_EVENT_KIND, owner.public_key()),
        "device roster fact filters must be derived from roster authors"
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
            INVITE_LIST_LABEL
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
            GROUP_ROSTER_FACT_KIND,
            group_admin.public_key()
        ),
        "group roster fact filters must be derived from group admins"
    );
    assert!(
        has_filter_with_kind_author_tag(
            &filters,
            GROUP_ROSTER_FACT_KIND,
            group_admin.public_key(),
            "#d",
            "test-group"
        ),
        "group roster fact filters must be scoped to known group ids"
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
    assert!(
        !has_filter_with_kind(&filters, 0),
        "profile metadata should stay a targeted lookup, not a live subscription"
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
    let alice_session_state = established_peer_session_state_for_test(&alice_keys, &bob_keys);
    let message_event = unrelated_direct_message_event_for_test(&mallory_keys, &carol_keys);
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
        !core
            .protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(alice_keys.public_key())
            .is_empty(),
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
        !core
            .protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(owner.public_key())
            .is_empty(),
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
            has_message_kind && filter.get("authors").is_none() && filter.get("#p").is_none()
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

fn has_filter_with_kind(filters: &[Filter], kind: u32) -> bool {
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                })
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

fn wait_for_relay_event_with_kind(
    core: &mut AppCore,
    core_rx: &flume::Receiver<CoreMsg>,
    relay: &crate::local_relay::TestRelay,
    kind: u32,
) -> Event {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        while let Ok(msg) = core_rx.try_recv() {
            core.handle_message(msg);
        }
        core.refresh_relay_connection_status();
        core.retry_pending_relay_publishes("test_relay_event_wait");
        if let Some(event) = relay
            .events()
            .into_iter()
            .filter_map(|event| serde_json::from_value::<Event>(event).ok())
            .find(|event| event.kind.as_u16() as u32 == kind)
        {
            return event;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("relay event with kind {kind} was not published");
}

fn established_peer_session_state_for_test(
    peer_keys: &Keys,
    local_keys: &Keys,
) -> nostr_double_ratchet::SessionState {
    use nostr_double_ratchet::SessionNostrExt;

    let mut invite = Invite::create_new(
        peer_keys.public_key(),
        Some(peer_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(peer_keys.public_key());

    let (mut local_session, response) = invite
        .accept_with_owner(
            local_keys.public_key(),
            local_keys.secret_key().to_secret_bytes(),
            Some(local_keys.public_key().to_hex()),
            Some(local_keys.public_key()),
        )
        .expect("local accepts peer invite");
    let response_event = invite_response_event(&response).expect("invite response event");
    let mut peer_session = nostr_double_ratchet::process_invite_response_event(
        &invite,
        &response_event,
        peer_keys.secret_key().to_secret_bytes(),
    )
    .expect("peer processes response")
    .expect("response addressed to peer")
    .session;

    let local_bootstrap = local_session
        .send_event(
            nostr_double_ratchet::build_text_rumor(
                local_keys.public_key(),
                "bootstrap",
                vec![],
            )
            .expect("bootstrap rumor"),
        )
        .expect("local bootstrap event");
    peer_session
        .receive(&local_bootstrap)
        .expect("peer receives local bootstrap");

    let peer_reply = peer_session
        .send_event(
            nostr_double_ratchet::build_text_rumor(
                peer_keys.public_key(),
                "reply",
                vec![],
            )
            .expect("reply rumor"),
        )
        .expect("peer reply event");
    local_session
        .receive(&peer_reply)
        .expect("local receives peer reply");

    local_session.state
}

fn unrelated_direct_message_event_for_test(sender: &Keys, receiver: &Keys) -> Event {
    use nostr_double_ratchet::SessionNostrExt;

    let mut invite = Invite::create_new(
        sender.public_key(),
        Some(sender.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(sender.public_key());
    let (mut receiver_session, _response) = invite
        .accept_with_owner(
            receiver.public_key(),
            receiver.secret_key().to_secret_bytes(),
            Some(receiver.public_key().to_hex()),
            Some(receiver.public_key()),
        )
        .expect("receiver accepts unrelated invite");
    receiver_session
        .send_event(
            nostr_double_ratchet::build_text_rumor(
                receiver.public_key(),
                "queued until unrelated protocol state arrives",
                vec![],
            )
            .expect("unrelated rumor"),
        )
        .expect("unrelated message event")
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
        .local_invite()
        .expect("linked invite");
    let (_primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    let linked_response = nostr_double_ratchet::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet::invite_response_event(&response)
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
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("local invite")
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
fn queued_runtime_publish_registration_persists_inner_message_id() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let inner_message_id = "inner-rumor-id".to_string();
    let mut core = logged_in_test_core("publish-registration-inner-id", &owner, &device);
    core.push_outgoing_message_with_id(
        inner_message_id.clone(),
        &chat_id,
        "queued".to_string(),
        1,
        None,
        DeliveryState::Queued,
    );
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&owner)
        .expect("outer event");

    let event_id = outer_event.id.to_string();
    assert!(core.publish_protocol_event(ProtocolPublish {
        event: outer_event,
        chat_id: chat_id.clone(),
        inner_event_id: Some(inner_message_id.clone()),
    }));
    let pending = core
        .pending_relay_publishes
        .get(&event_id)
        .expect("pending publish");
    assert_eq!(pending.chat_id.as_deref(), Some(chat_id.as_str()));
    assert_eq!(
        pending.inner_event_id.as_deref(),
        Some(inner_message_id.as_str())
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
