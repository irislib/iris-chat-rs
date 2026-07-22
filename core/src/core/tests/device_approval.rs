#[test]
fn start_linked_device_stores_bounded_bootstrap_without_publishing_request_event() {
    let approval_relay = crate::local_relay::TestRelay::start();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec!["wss://ordinary.example".to_string()];
    core.device_approval_relay_urls =
        relay_urls_from_strings(&[approval_relay.url().to_string()]);

    core.handle_action(AppAction::SetCurrentDeviceLabels {
        device_label: "abcdefghijklmnop more".to_string(),
        client_label: "Iris Chat Web".to_string(),
    });
    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let snapshot = core
        .state
        .link_device
        .as_ref()
        .unwrap_or_else(|| panic!("link-device snapshot; toast={:?}", core.state.toast))
        .clone();
    let bootstrap = parse_nostr_identity_device_approval_bootstrap(&snapshot.url, &[])
        .expect("parse bootstrap")
        .expect("device approval bootstrap");
    assert!(snapshot.url.starts_with("nostr-identity://device-approval/"));
    assert_eq!(
        serde_json::to_value(&bootstrap)
            .expect("bootstrap JSON")
            .as_object()
            .expect("bootstrap object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["deviceAppKeyNpub", "label", "requestNpub", "requestSecret"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    assert_eq!(bootstrap.label.as_deref(), Some("abcdefghijklmnop"));
    assert_eq!(bootstrap.label.as_ref().map(String::len), Some(16));
    assert!(
        relay_events(&approval_relay).is_empty(),
        "starting a link must not publish a request event"
    );
    PublicKey::parse(&bootstrap.request_npub).expect("request npub");
    assert_eq!(
        bootstrap.request_secret.len(),
        43,
        "nostr-identity encodes 32 request-secret bytes as unpadded base64url"
    );
    assert!(bootstrap
        .request_secret
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')));
    let pairing_client = core
        .pending_linked_device
        .as_ref()
        .expect("pending link")
        .pairing_client
        .clone();
    let pending_approval_relays = core.runtime.block_on(async {
        for _ in 0..50 {
            let relays = pairing_client
                .relays()
                .await
                .keys()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if !relays.is_empty() {
                return relays;
            }
            sleep(Duration::from_millis(10)).await;
        }
        Vec::new()
    });
    assert_eq!(pending_approval_relays, vec![approval_relay.url()]);
    assert!(!pending_approval_relays
        .iter()
        .any(|relay| relay.contains("ordinary.example")));
    assert_eq!(
        bootstrap.device_app_key_npub,
        snapshot.device_input
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

fn device_approval_bootstrap_for_test(
    device_keys: &Keys,
    request_keys: &Keys,
    request_secret: &str,
    label: Option<&str>,
) -> String {
    let local_request = create_nostr_identity_device_approval_request(
        device_keys,
        CreateNostrIdentityDeviceApprovalRequestOptions {
            request_keys: Some(request_keys.clone()),
            request_secret: Some(request_secret.to_string()),
            requested_at: 41,
            request_type: Some("device_link".to_string()),
            resources: Vec::new(),
            expires_at: None,
            profile_id: None,
            admin_app_key_pubkey: None,
            label: label.map(str::to_string),
        },
    )
    .expect("approval request");
    let bootstrap =
        nostr_identity_device_approval_bootstrap(&local_request.request).expect("bootstrap");
    encode_nostr_identity_device_approval_bootstrap(&bootstrap, None).expect("encode bootstrap")
}

fn dispatch_device_approval_for_test(
    core: &mut AppCore,
    relay_url: &str,
    bootstrap: String,
) {
    core.device_approval_relay_urls = relay_urls_from_strings(&[relay_url.to_string()]);
    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: bootstrap,
    });
}

fn relay_events(relay: &crate::local_relay::TestRelay) -> Vec<Event> {
    relay
        .events()
        .into_iter()
        .filter_map(|event| serde_json::from_value::<Event>(event).ok())
        .collect()
}

#[test]
fn bootstrap_only_link_finishes_pairing_and_authorizes_linked_device() {
    let owner = Keys::generate();
    let ordinary_relay = crate::local_relay::TestRelay::start();
    let approval_relay = crate::local_relay::TestRelay::start();
    let temp_dir_primary = tempfile::TempDir::new().expect("primary temp dir");
    let temp_dir_linked = tempfile::TempDir::new().expect("linked temp dir");
    let mut primary = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir_primary.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    primary.preferences.nostr_relay_urls = vec![ordinary_relay.url().to_string()];
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
    linked.device_approval_relay_urls =
        relay_urls_from_strings(&[approval_relay.url().to_string()]);
    linked.handle_action(AppAction::SetCurrentDeviceLabels {
        device_label: "Safari on macOS".to_string(),
        client_label: "Iris Chat Web".to_string(),
    });
    linked.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });
    let approval_bootstrap = linked
        .state
        .link_device
        .as_ref()
        .expect("approval bootstrap")
        .url
        .clone();
    let linked_device_hex = linked
        .pending_linked_device
        .as_ref()
        .unwrap_or_else(|| panic!("pending linked device; toast={:?}", linked.state.toast))
        .device_keys
        .public_key()
        .to_hex();

    dispatch_device_approval_for_test(&mut primary, approval_relay.url(), approval_bootstrap);

    assert_eq!(primary.state.toast.as_deref(), Some("Device added"));
    let approval_events = relay_events(&approval_relay);
    let response_event = approval_events
        .into_iter()
        .find(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
        .expect("approval publishes deterministic invite response");
    let approval_events = relay_events(&approval_relay);
    let app_keys_event = approval_events
        .into_iter()
        .find(|event| {
            event.kind.as_u16() as u32 == APP_KEYS_EVENT_KIND
                && event_has_tag_value(event, "device", &linked_device_hex)
        })
        .expect("approval publishes AppKeys authorization");
    let receipt_deadline = Instant::now() + Duration::from_secs(2);
    let receipt_event = loop {
        if let Some(event) = relay_events(&approval_relay).into_iter().find(|event| {
            event_has_tag_value(event, "type", "nostr_identity_device_approval_receipt")
        }) {
            break event;
        }
        assert!(
            Instant::now() < receipt_deadline,
            "approval should publish the optional encrypted receipt"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(relay_events(&approval_relay).iter().all(|event| {
        !event_has_tag_value(
            event,
            "type",
            "nostr_identity_device_approval_request",
        )
    }));
    let approval_event_ids = [
        response_event.id.to_string(),
        app_keys_event.id.to_string(),
        receipt_event.id.to_string(),
    ];
    let ordinary_event_ids = relay_events(&ordinary_relay)
        .into_iter()
        .map(|event| event.id.to_string())
        .collect::<HashSet<_>>();
    assert!(approval_event_ids
        .iter()
        .all(|event_id| !ordinary_event_ids.contains(event_id)));

    linked.handle_relay_event(response_event);
    assert!(
        linked.pending_linked_device.is_some(),
        "invite response alone must wait for device approval"
    );

    linked.handle_relay_event(app_keys_event);
    assert!(
        linked.pending_linked_device.is_none(),
        "the QR-bound handshake and exact AppKeys authorization complete linking"
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
