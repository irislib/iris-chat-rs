#[test]
fn pending_linked_device_finishes_when_owner_accepts_invite() {
    let owner = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let (update_tx, update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });
    let _ = update_rx.try_iter().collect::<Vec<_>>();

    let linked_device_pubkey = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite")
        .device_keys
        .public_key();
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
        "invite response waits for owner-signed AppKeys authorization"
    );
    assert!(core.logged_in.is_none());

    core.handle_relay_event(signed_app_keys_authorization_event(
        &owner,
        Keys::generate().public_key(),
        41,
    ));
    assert!(
        core.pending_linked_device.is_some(),
        "an owner-signed roster that does not contain this QR's device key must be ignored"
    );

    core.handle_relay_event(signed_app_keys_authorization_event(
        &owner,
        linked_device_pubkey,
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

    let completion_updates = update_rx.try_iter().collect::<Vec<_>>();
    assert!(completion_updates
        .iter()
        .any(|update| matches!(update, AppUpdate::PersistAccountBundle { .. })));
    assert!(completion_updates
        .iter()
        .all(|update| !matches!(update, AppUpdate::ClearPendingDeviceLink)));
    assert!(completion_updates.iter().all(|update| {
        !matches!(
            update,
            AppUpdate::FullState(state)
                if state.account.as_ref().is_some_and(|account| {
                    account.authorization_state == DeviceAuthorizationState::AwaitingApproval
                })
        )
    }), "link completion must never expose an intermediate awaiting-approval account");
    assert!(completion_updates.iter().any(|update| matches!(
        update,
        AppUpdate::FullState(state)
            if state.router.default_screen == Screen::ChatList
                && state.account.as_ref().is_some_and(|account| {
                    account.authorization_state == DeviceAuthorizationState::Authorized
                })
    )));
}

#[test]
fn legacy_incomplete_link_can_logout_and_create_fresh_account() {
    let old_owner = Keys::generate();
    let old_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let (update_tx, update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_session(
        old_owner.public_key(),
        None,
        old_device,
        false,
        false,
    )
    .expect("legacy incomplete linked session");
    assert_eq!(
        core.state
            .account
            .as_ref()
            .expect("legacy linked account")
            .authorization_state,
        DeviceAuthorizationState::AwaitingApproval
    );
    let _ = update_rx.try_iter().collect::<Vec<_>>();

    core.handle_action(AppAction::Logout);
    assert!(core.logged_in.is_none());
    assert!(core.pending_linked_device.is_none());
    assert!(core.state.account.is_none());
    assert_eq!(core.state.router.default_screen, Screen::Welcome);

    core.handle_action(AppAction::CreateAccount {
        name: "Fresh profile".to_string(),
    });
    let account = core.state.account.as_ref().expect("fresh account");
    assert_ne!(account.public_key_hex, old_owner.public_key().to_hex());
    assert_eq!(
        account.authorization_state,
        DeviceAuthorizationState::Authorized
    );
    assert_eq!(core.state.router.default_screen, Screen::ChatList);
    assert!(core.state.toast.is_none());
    assert!(update_rx.try_iter().any(|update| matches!(
        update,
        AppUpdate::PersistAccountBundle {
            owner_nsec: Some(_),
            ..
        }
    )));
}

#[test]
fn create_account_failure_reports_error_and_resets_busy_state() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.screen_stack = vec![Screen::CreateAccount];

    let database = core.app_store.shared();
    let _ = std::thread::spawn(move || {
        let _database_guard = database.lock().expect("database lock");
        panic!("poison database connection");
    })
    .join();

    core.handle_action(AppAction::CreateAccount {
        name: "Fresh profile".to_string(),
    });

    assert!(core.state.account.is_none());
    assert!(!core.state.busy.creating_account);
    assert!(core
        .state
        .toast
        .as_deref()
        .is_some_and(|message| !message.trim().is_empty()));
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
