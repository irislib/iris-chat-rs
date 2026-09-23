#[test]
fn remote_signer_client_and_bunker_login_preserve_roster_and_restart_without_signer() {
    for (client_initiated, fault) in [
        (true, RemoteSignerFault::None),
        (false, RemoteSignerFault::UnsupportedSwitch),
        (false, RemoteSignerFault::IgnoreSwitch),
    ] {
        let relay = crate::local_relay::TestRelay::start();
        let switched = crate::local_relay::TestRelay::start();
        let owner = Keys::generate();
        let transport = Keys::generate();
        let old_device = Keys::generate();
        let temp = tempfile::TempDir::new().unwrap();
        let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
        let mut old_keys = AppKeys::new(vec![DeviceEntry::new(
            old_device.public_key(),
            unix_now().get() - 10,
        )]);
        old_keys.set_device_labels(
            old_device.public_key(),
            Some("Original phone".into()),
            None,
            None,
        );
        let old_event = old_keys
            .get_encrypted_event_at(&owner, unix_now().get() - 2)
            .unwrap()
            .sign_with_keys(&owner)
            .unwrap();
        publish_signer_test_event(&core, &relay, &old_event);
        let fixture = if client_initiated {
            core.handle_action(AppAction::StartRemoteSignerLogin);
            pump_signer_core_until(&mut core, &messages, |core| {
                core.state
                    .remote_signer_login
                    .as_ref()
                    .is_some_and(|snapshot| {
                        snapshot.phase == crate::RemoteSignerPhase::WaitingForSigner
                    })
            });
            let uri = core
                .state
                .remote_signer_login
                .as_ref()
                .unwrap()
                .connection_uri
                .clone();
            launch_remote_signer_fixture(
                &core,
                &relay,
                &transport,
                &owner,
                uri,
                RemoteSignerFault::None,
                Some(switched.url().into()),
            )
        } else {
            let fixture =
                launch_remote_signer_fixture(&core, &relay, &transport, &owner, None, fault, None);
            core.handle_action(AppAction::ConnectRemoteSigner {
                connection_uri: format!(
                    "bunker://{}?relay={}&secret=fixture-secret",
                    transport.public_key().to_hex(),
                    urlencoding::encode(relay.url())
                ),
            });
            fixture
        };
        if client_initiated {
            finish_remote_login_through_suspend(&mut core, &messages);
        } else {
            pump_signer_core_until(&mut core, &messages, |core| {
                !core.state.busy.restoring_session
            });
        }
        core.runtime.block_on(fixture).unwrap();
        assert!(core.state.toast.is_none(), "{:?}", core.state.toast);
        assert!(core.state.remote_signer_login.is_none());
        let logged_in = core.logged_in.as_ref().unwrap();
        assert_eq!(logged_in.owner_pubkey, owner.public_key());
        assert!(logged_in.owner_keys.is_none());
        assert_eq!(
            logged_in.authorization_state,
            LocalAuthorizationState::Authorized
        );
        let device_key = logged_in.device_keys.public_key();
        let published = relay
            .events()
            .iter()
            .filter_map(|value| serde_json::from_value::<Event>(value.clone()).ok())
            .filter(|event| event.kind.as_u16() == APP_KEYS_EVENT_KIND as u16)
            .max_by_key(|event| event.created_at)
            .unwrap();
        let roster = AppKeys::from_event_with_labels(&published, &owner).unwrap();
        assert!(roster.get_device(&old_device.public_key()).is_some());
        assert!(roster.get_device(&device_key).is_some());
        assert_eq!(
            roster
                .get_device_labels(&old_device.public_key())
                .unwrap()
                .device_label
                .as_deref(),
            Some("Original phone")
        );
        let mut saved = None;
        for update in updates.try_iter() {
            match update {
                AppUpdate::PersistAccountBundle {
                    owner_nsec,
                    device_nsec,
                    ..
                } => {
                    assert!(owner_nsec.is_none());
                    saved = Some(device_nsec);
                }
                AppUpdate::SignerLoginSignEvent { .. } => {
                    panic!("remote signing escaped to the local adapter")
                }
                _ => {}
            }
        }
        drop(core);
        let (mut restored, _, _) = signer_test_core(temp.path(), Vec::new());
        restored.handle_action(AppAction::RestoreAccountBundle {
            owner_nsec: None,
            owner_pubkey_hex: owner.public_key().to_hex(),
            device_nsec: saved.unwrap(),
        });
        assert_eq!(
            restored.logged_in.as_ref().unwrap().authorization_state,
            LocalAuthorizationState::Authorized
        );
        restored.logged_in.as_mut().unwrap().relay_urls.clear();
        restored.preferences.nostr_relay_urls.clear();
        assert_signer_device_can_message(&mut restored, owner.public_key());
    }
}

#[test]
fn remote_signer_rejects_changed_authorization_rejection_and_invalid_switch() {
    for fault in [
        RemoteSignerFault::ChangedEvent,
        RemoteSignerFault::Reject,
        RemoteSignerFault::InvalidSwitch,
    ] {
        let relay = crate::local_relay::TestRelay::start();
        let owner = Keys::generate();
        let transport = Keys::generate();
        let temp = tempfile::TempDir::new().unwrap();
        let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
        let fixture =
            launch_remote_signer_fixture(&core, &relay, &transport, &owner, None, fault, None);
        core.handle_action(AppAction::ConnectRemoteSigner {
            connection_uri: format!(
                "bunker://{}?relay={}&secret=fixture-secret",
                transport.public_key().to_hex(),
                urlencoding::encode(relay.url())
            ),
        });
        pump_signer_core_until(&mut core, &messages, |core| {
            !core.state.busy.restoring_session
        });
        core.runtime.block_on(fixture).unwrap();
        assert!(core.logged_in.is_none());
        assert!(core.state.remote_signer_login.is_none());
        assert!(core.state.toast.is_some());
        assert!(!updates
            .try_iter()
            .any(|update| matches!(update, AppUpdate::PersistAccountBundle { .. })));
        assert!(!relay
            .events()
            .iter()
            .any(|event| event["kind"] == APP_KEYS_EVENT_KIND));
    }
}

#[test]
fn remote_signer_cancellation_ignores_old_callbacks_and_local_login_supersedes_it() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let temp = tempfile::TempDir::new().unwrap();
    let (mut core, messages, updates) = signer_test_core(temp.path(), vec![relay.url().into()]);
    core.handle_action(AppAction::StartRemoteSignerLogin);
    let token = core.pending_remote_signer.as_ref().unwrap().token.clone();
    core.handle_action(AppAction::CancelRemoteSignerLogin);
    core.handle_internal(InternalEvent::RemoteSignerConnected {
        token,
        owner_pubkey_hex: owner.public_key().to_hex(),
    });
    assert!(core.pending_signer_login.is_none());
    assert!(!core.state.busy.restoring_session);
    core.handle_action(AppAction::StartRemoteSignerLogin);
    core.handle_action(AppAction::BeginSignerLogin {
        owner_pubkey_hex: owner.public_key().to_hex(),
    });
    let _ = next_signer_request(&mut core, &messages, &updates);
    assert!(core.pending_remote_signer.is_none());
    assert!(core.pending_signer_login.is_some());
    core.cancel_signer_login("");
    assert!(!core.state.busy.restoring_session);
}

#[test]
fn remote_signer_links_and_auth_urls_are_validated() {
    use super::remote_signer_uri::{parse_signer_connection, safe_auth_url};
    let signer = Keys::generate().public_key().to_hex();
    for input in [
        "https://example.com".to_string(),
        format!("bunker://{signer}?relay=https://example.com"),
        format!("bunker://{signer}?relay=wss://example.com&secret=a&secret=b"),
        format!("bunker://{signer}?relay=wss://user:secret@example.com"),
    ] {
        assert!(parse_signer_connection(&input).is_err());
    }
    for input in [
        "javascript:alert(1)",
        "file:///tmp/secret",
        "https://user:secret@example.com",
    ] {
        assert!(safe_auth_url(input).is_none());
    }
    assert_eq!(
        safe_auth_url("https://example.com/approve").as_deref(),
        Some("https://example.com/approve")
    );
}

fn finish_remote_login_through_suspend(core: &mut AppCore, messages: &flume::Receiver<CoreMsg>) {
    let mut signed_while_suspended = false;
    let mut published_while_suspended = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while core.state.busy.restoring_session {
        if let Ok(message) = messages.recv_timeout(Duration::from_millis(20)) {
            match message {
                CoreMsg::Internal(event)
                    if matches!(
                        event.as_ref(),
                        InternalEvent::RemoteSignerProgress {
                            auth_url: Some(_),
                            ..
                        }
                    ) =>
                {
                    core.handle_internal(*event);
                    assert_eq!(
                        core.state
                            .remote_signer_login
                            .as_ref()
                            .unwrap()
                            .auth_url
                            .as_deref(),
                        Some("https://signer.example/approve")
                    );
                    core.prepare_for_suspend();
                }
                CoreMsg::Internal(event)
                    if matches!(event.as_ref(), InternalEvent::RemoteSignerSigned { .. }) =>
                {
                    assert!(core.suspended);
                    core.handle_internal(*event);
                    assert!(!core.pending_signer_login.as_ref().unwrap().publishing);
                    core.handle_action(AppAction::AppForegrounded);
                    assert!(core.pending_signer_login.as_ref().unwrap().publishing);
                    signed_while_suspended = true;
                    core.prepare_for_suspend();
                }
                CoreMsg::Internal(event)
                    if matches!(event.as_ref(), InternalEvent::SignerLoginPublished { .. }) =>
                {
                    assert!(core.suspended);
                    core.handle_internal(*event);
                    assert!(core.logged_in.is_none());
                    core.handle_action(AppAction::AppForegrounded);
                    published_while_suspended = true;
                }
                message => {
                    core.handle_message(message);
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "signer suspend test timed out"
        );
    }
    assert!(signed_while_suspended && published_while_suspended);
}
