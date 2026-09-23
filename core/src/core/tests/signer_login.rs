use super::account_signer::{prepare_signer_authorization, validate_signer_authorization};

#[test]
fn signer_authorization_preserves_devices_and_private_labels() {
    let owner = Keys::generate();
    let old_device = Keys::generate();
    let new_device = Keys::generate();
    let now = unix_now().get();
    let mut roster = AppKeys::new(vec![DeviceEntry::new(old_device.public_key(), now - 10)]);
    roster.set_device_labels(
        old_device.public_key(),
        Some("Old phone".into()),
        None,
        Some(now - 10),
    );
    let old_event = roster
        .get_encrypted_event_at(&owner, now - 1)
        .unwrap()
        .sign_with_keys(&owner)
        .unwrap();
    let unsigned = prepare_signer_authorization(
        owner.public_key(),
        new_device.public_key(),
        Some(&old_event),
        now,
    )
    .unwrap();
    let signed = unsigned.clone().sign_with_keys(&owner).unwrap();
    validate_signer_authorization(&unsigned, &signed).unwrap();
    let restored = AppKeys::from_event_with_labels(&signed, &owner).unwrap();
    assert_eq!(restored.get_all_devices().len(), 2);
    assert_eq!(
        restored
            .get_device(&old_device.public_key())
            .unwrap()
            .created_at,
        now - 10
    );
    assert_eq!(
        restored
            .get_device(&new_device.public_key())
            .unwrap()
            .created_at,
        now
    );
    assert_eq!(
        restored
            .get_device_labels(&old_device.public_key())
            .unwrap()
            .device_label
            .as_deref(),
        Some("Old phone")
    );
}

#[test]
fn signer_authorization_rejects_changed_signed_fields() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let unsigned = prepare_signer_authorization(
        owner.public_key(),
        device.public_key(),
        None,
        unix_now().get(),
    )
    .unwrap();
    for field in ["content", "created_at", "kind", "tags", "pubkey"] {
        let mut changed = unsigned.clone();
        changed.id = None;
        match field {
            "content" => changed.content = "Unexpected change".into(),
            "created_at" => changed.created_at = Timestamp::from(changed.created_at.as_secs() + 1),
            "kind" => changed.kind = Kind::TextNote,
            "tags" => changed
                .tags
                .push(nostr::Tag::parse(["extra", "unexpected"]).unwrap()),
            "pubkey" => changed.pubkey = Keys::generate().public_key(),
            _ => unreachable!(),
        }
        let signed = changed.sign_with_keys(&owner).unwrap();
        assert!(
            validate_signer_authorization(&unsigned, &signed).is_err(),
            "must reject changed {field}"
        );
    }
}

fn signer_test_core(
    path: &std::path::Path,
    relays: Vec<String>,
) -> (
    AppCore,
    flume::Receiver<CoreMsg>,
    flume::Receiver<AppUpdate>,
) {
    let (update_tx, updates) = flume::unbounded();
    let (tx, messages) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        tx,
        path.to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = relays;
    core.preferences.nearby_enabled = false;
    (core, messages, updates)
}

fn pump_signer_core_until(
    core: &mut AppCore,
    messages: &flume::Receiver<CoreMsg>,
    condition: impl Fn(&AppCore) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !condition(core) {
        if let Ok(message) = messages.recv_timeout(Duration::from_millis(20)) {
            core.handle_message(message);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "signer state did not converge: toast={:?}",
            core.state.toast
        );
    }
}

fn next_signer_request(
    core: &mut AppCore,
    messages: &flume::Receiver<CoreMsg>,
    updates: &flume::Receiver<AppUpdate>,
) -> (String, UnsignedEvent) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        for update in updates.try_iter() {
            if let AppUpdate::SignerLoginSignEvent {
                request_id,
                unsigned_event_json,
                ..
            } = update
            {
                return (
                    request_id,
                    serde_json::from_str(&unsigned_event_json).unwrap(),
                );
            }
        }
        if let Ok(message) = messages.recv_timeout(Duration::from_millis(20)) {
            core.handle_message(message);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no signer request: toast={:?}",
            core.state.toast
        );
    }
}

fn publish_signer_test_event(core: &AppCore, relay: &crate::local_relay::TestRelay, event: &Event) {
    core.runtime.block_on(async {
        let client = Client::default();
        client.add_relay(relay.url()).await.unwrap();
        client.connect().await;
        client.send_event(event).await.unwrap();
        client.shutdown().await;
    });
}

#[test]
fn signer_login_authorizes_persists_restarts_and_sends_without_identity_secret() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let old_device = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    let old_event = app_keys_event(&owner, &[&old_device], unix_now().get() - 2);
    publish_signer_test_event(&core, &relay, &old_event);
    core.handle_action(AppAction::BeginSignerLogin {
        owner_pubkey_hex: owner.public_key().to_hex(),
    });
    assert!(core.state.busy.restoring_session);
    assert!(core.logged_in.is_none());
    let (request_id, unsigned) = next_signer_request(&mut core, &messages, &updates);
    let signed = unsigned.clone().sign_with_keys(&owner).unwrap();
    let mut invalid_signature = signed.clone();
    invalid_signature.sig = app_keys_event(&Keys::generate(), &[], unix_now().get()).sig;
    assert!(validate_signer_authorization(&unsigned, &invalid_signature).is_err());
    let roster = AppKeys::from_event(&signed).unwrap();
    assert_eq!(roster.get_all_devices().len(), 2);
    assert!(roster.get_device(&old_device.public_key()).is_some());
    core.handle_action(AppAction::CompleteSignerLogin {
        request_id,
        signed_event_json: serde_json::to_string(&signed).unwrap(),
    });
    // Once publication begins, cancellation and the signing timer must not
    // discard the device key while an authorization write is in flight.
    core.cancel_signer_login("");
    assert!(core.state.busy.restoring_session);
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.state.toast.is_none(), "{:?}", core.state.toast);
    let logged_in = core.logged_in.as_ref().unwrap();
    assert!(logged_in.owner_keys.is_none());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::Authorized
    );
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .active_session_count_for_owner(owner.public_key()),
        0
    );
    let device_nsec = updates
        .try_iter()
        .find_map(|update| match update {
            AppUpdate::PersistAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
                ..
            } => {
                assert!(owner_nsec.is_none());
                assert_eq!(owner_pubkey_hex, owner.public_key().to_hex());
                Some(device_nsec)
            }
            _ => None,
        })
        .expect("persist authorized account only");
    assert!(relay
        .events()
        .iter()
        .any(|event| event["id"] == signed.id.to_hex()));
    drop(core);

    let (mut restored, _, _) = signer_test_core(temp.path(), Vec::new());
    restored.handle_action(AppAction::RestoreAccountBundle {
        owner_nsec: None,
        owner_pubkey_hex: owner.public_key().to_hex(),
        device_nsec,
    });
    assert_eq!(
        restored.logged_in.as_ref().unwrap().authorization_state,
        LocalAuthorizationState::Authorized
    );
    assert!(restored
        .protocol_engine
        .as_ref()
        .unwrap()
        .signed_local_device_authorization()
        .unwrap());
    restored.logged_in.as_mut().unwrap().relay_urls.clear();
    restored.preferences.nostr_relay_urls.clear();

    // Exercise production invite acceptance, carried owner proof, encryption,
    // and receiver delivery with the restored device secret alone.
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let mut receiver =
        logged_in_test_core("signer-chat-receiver", &receiver_owner, &receiver_device);
    let invite = create_private_invite_for_test(&mut receiver);
    prove_invite_owner(
        &mut restored,
        &receiver_owner,
        &receiver_device,
        unix_now().get(),
    );
    restored.pending_relay_publishes.clear();
    restored.handle_action(AppAction::AcceptInvite {
        invite_input: invite,
    });
    restored.handle_action(AppAction::SendMessage {
        chat_id: receiver_owner.public_key().to_hex(),
        text: "Signed in with a signer".into(),
    });
    let response = pending_events_with_kind(&restored, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("device handshake");
    assert!(response
        .tags
        .iter()
        .any(|tag| tag.as_slice()[0] == "owner-proof"));
    for event in pending_events_with_kind(&restored, MESSAGE_EVENT_KIND) {
        receiver.handle_relay_event(event);
    }
    receiver.handle_relay_event(response);
    assert!(receiver
        .threads
        .get(&owner.public_key().to_hex())
        .is_some_and(|thread| thread
            .messages
            .iter()
            .any(|message| message.body == "Signed in with a signer")));

    let revocation = app_keys_event(&owner, &[&old_device], signed.created_at.as_secs() + 1);
    restored.handle_relay_event(revocation);
    assert_eq!(
        restored.logged_in.as_ref().unwrap().authorization_state,
        LocalAuthorizationState::Revoked
    );
    assert!(!restored
        .protocol_engine
        .as_ref()
        .unwrap()
        .signed_local_device_authorization()
        .unwrap());
    restored.handle_relay_event(signed.clone());
    assert_eq!(
        restored.logged_in.as_ref().unwrap().authorization_state,
        LocalAuthorizationState::Revoked,
        "stale signed proof cannot undo revocation"
    );
    let new_device = restored.logged_in.as_ref().unwrap().device_keys.clone();
    restored.handle_relay_event(app_keys_event(
        &owner,
        &[&old_device, &new_device],
        signed.created_at.as_secs() + 1,
    ));
    assert_eq!(
        restored.logged_in.as_ref().unwrap().authorization_state,
        LocalAuthorizationState::Revoked,
        "conflicting current proofs must fail closed"
    );
}

#[test]
fn signer_login_rejects_stale_callbacks_cancel_and_invalid_signature_then_retries() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    core.begin_signer_login(&owner.public_key().to_hex());
    let (old_request, unsigned) = next_signer_request(&mut core, &messages, &updates);
    core.cancel_signer_login(&old_request);
    assert!(!core.state.busy.restoring_session);
    let signed = unsigned.sign_with_keys(&owner).unwrap();
    core.complete_signer_login(&old_request, &serde_json::to_string(&signed).unwrap());
    assert!(core.logged_in.is_none());
    assert!(relay.events().is_empty());
    core.begin_signer_login(&owner.public_key().to_hex());
    let (request_id, _) = next_signer_request(&mut core, &messages, &updates);
    core.complete_signer_login(&old_request, &serde_json::to_string(&signed).unwrap());
    assert!(
        core.state.busy.restoring_session,
        "stale callback cannot terminate current sign-in"
    );
    core.complete_signer_login(&request_id, "{}");
    assert!(core.logged_in.is_none());
    assert!(!core.state.busy.restoring_session);
    assert!(core.state.toast.is_some());
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::PersistAccountBundle { .. })));
    core.begin_signer_login(&owner.public_key().to_hex());
    let (request_id, _) = next_signer_request(&mut core, &messages, &updates);
    core.handle_signer_login_timeout(&request_id);
    assert!(!core.state.busy.restoring_session);
    assert!(core.pending_signer_login.is_none());
}

#[test]
fn signer_login_rejects_concurrent_roster_change_without_publishing() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    core.begin_signer_login(&owner.public_key().to_hex());
    let (request_id, unsigned) = next_signer_request(&mut core, &messages, &updates);
    let changed = app_keys_event(&owner, &[&Keys::generate()], unix_now().get());
    publish_signer_test_event(&core, &relay, &changed);
    let signed = unsigned.sign_with_keys(&owner).unwrap();
    core.complete_signer_login(&request_id, &serde_json::to_string(&signed).unwrap());
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.logged_in.is_none());
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Your device list changed. Sign in again.")
    );
    assert!(!relay
        .events()
        .iter()
        .any(|event| event["id"] == signed.id.to_hex()));
}

#[test]
fn signer_login_rejects_conflicting_rosters_and_unreachable_servers() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    let now = unix_now().get();
    for device in [Keys::generate(), Keys::generate()] {
        publish_signer_test_event(&core, &relay, &app_keys_event(&owner, &[&device], now));
    }
    core.begin_signer_login(&owner.public_key().to_hex());
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.logged_in.is_none());
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Conflicting device lists. Try again later.")
    );
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::SignerLoginSignEvent { .. })));
    core.preferences
        .nostr_relay_urls
        .push("ws://127.0.0.1:1".into());
    core.begin_signer_login(&Keys::generate().public_key().to_hex());
    pump_signer_core_until(&mut core, &messages, |core| {
        !core.state.busy.restoring_session
    });
    assert!(core.logged_in.is_none());
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Could not check all message servers. Try again.")
    );
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::SignerLoginSignEvent { .. })));
}
