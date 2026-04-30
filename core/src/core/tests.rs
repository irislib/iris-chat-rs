use super::*;

#[test]
fn restoring_invalid_secret_key_shows_normie_error() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.handle_action(AppAction::RestoreSession {
        owner_nsec: "not a secret key".to_string(),
    });

    assert_eq!(core.state.toast.as_deref(), Some("Invalid key."));
    assert!(!core.state.busy.restoring_session);
}

#[test]
fn removing_last_message_server_leaves_empty_list() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec![
        "wss://relay-one.example".to_string(),
        "wss://relay-two.example".to_string(),
    ];
    core.rebuild_state();

    core.handle_action(AppAction::RemoveNostrRelay {
        relay_url: "wss://relay-one.example".to_string(),
    });
    core.handle_action(AppAction::RemoveNostrRelay {
        relay_url: "wss://relay-two.example".to_string(),
    });

    assert!(core.preferences.nostr_relay_urls.is_empty());
    assert!(core.state.preferences.nostr_relay_urls.is_empty());
    assert_eq!(core.state.toast, None);
}

#[test]
fn direct_message_with_no_relays_is_queued_locally() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(owner.clone(), owner, false, false)
        .expect("primary session");

    core.handle_action(AppAction::CreateChat {
        peer_input: peer.public_key().to_hex(),
    });
    let chat_id = core
        .state
        .current_chat
        .as_ref()
        .expect("created chat")
        .chat_id
        .clone();

    core.handle_action(AppAction::SendMessage {
        chat_id,
        text: "queued offline".to_string(),
    });

    let current = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(core.state.toast, None);
    assert!(current.messages.iter().any(|message| {
        message.body == "queued offline"
            && message.is_outgoing
            && message.delivery == DeliveryState::Queued
    }));
}

#[test]
fn network_status_includes_configured_relay_connection_status() {
    let owner = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];

    core.start_primary_session(owner.clone(), owner, false, false)
        .expect("primary session");

    let status = core.state.network_status.as_ref().expect("network status");
    assert_eq!(status.relay_urls, vec!["wss://relay.invalid".to_string()]);
    assert_eq!(status.relay_connections.len(), 1);
    assert_eq!(status.relay_connections[0].url, "wss://relay.invalid");
    assert!(
        ["connecting", "offline"].contains(&status.relay_connections[0].status.as_str()),
        "unexpected relay status: {}",
        status.relay_connections[0].status
    );
    assert_eq!(status.connected_relay_count, 0);
    assert!(status.all_relays_offline_since_secs.is_some());
}

#[test]
fn relay_status_events_match_normalized_relay_urls() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("relay-status-normalized", &owner, &device);
    core.preferences.nostr_relay_urls = vec!["wss://relay.example".to_string()];

    core.handle_relay_status_changed("wss://relay.example/".to_string(), RelayStatus::Connected);

    assert!(core.debug_log.iter().any(|entry| {
        entry.category == "relay.status" && entry.detail.starts_with("url=wss://relay.example ")
    }));
}

#[test]
fn ndr_runtime_invite_session_round_trips_text() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        None,
        None,
    );
    bob.init().expect("bob init");
    bob.accept_invite(&invite, Some(alice_keys.public_key()))
        .expect("bob accepts alice invite");
    deliver_published_events(&bob, &bob_keys, &alice);

    alice
        .send_text(bob_keys.public_key(), "hello bob".to_string(), None)
        .expect("alice sends");
    deliver_published_events(&alice, &alice_keys, &bob);
    assert!(drain_text_messages(&bob)
        .iter()
        .any(|message| message == "hello bob"));

    bob.send_text(alice_keys.public_key(), "hello alice".to_string(), None)
        .expect("bob sends");
    deliver_published_events(&bob, &bob_keys, &alice);
    assert!(drain_text_messages(&alice)
        .iter()
        .any(|message| message == "hello alice"));
}

#[test]
fn local_identity_artifacts_offer_profile_metadata_to_nearby() {
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
    let device_id = device.public_key().to_hex();
    let invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None).expect("invite");
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device_id,
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: runtime,
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.owner_profiles.insert(
        owner.public_key().to_hex(),
        OwnerProfileRecord {
            name: None,
            display_name: None,
            picture: Some("htree://profile-picture".to_string()),
            updated_at_secs: 1,
        },
    );

    core.publish_local_identity_artifacts();

    let nearby_kinds = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent { kind, .. } => Some(kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        nearby_kinds.contains(&0),
        "profile metadata should be included in nearby inventory; got {nearby_kinds:?}"
    );
}

#[test]
fn create_account_without_name_still_offers_profile_metadata_to_nearby() {
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.create_account("");

    let account = core.state.account.as_ref().expect("account");
    let profile = core
        .owner_profiles
        .get(&account.public_key_hex)
        .expect("local profile record");
    assert_eq!(
        profile.display_name.as_deref(),
        Some(account.display_name.as_str())
    );

    let profile_event_json = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind, event_json, ..
            } if kind == 0 => Some(event_json),
            _ => None,
        })
        .last()
        .expect("profile event");
    let profile_event: Event = serde_json::from_str(&profile_event_json).expect("profile event");
    assert_eq!(profile_event.pubkey.to_hex(), account.public_key_hex);
}

#[test]
fn nearby_presence_event_binds_owner_to_ble_nonce_pair() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None).expect("invite");
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device_id,
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: runtime,
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });

    let event_json = core.build_nearby_presence_event_json(
        "peer-a",
        "nonce-a",
        "nonce-b",
        "f".repeat(64).as_str(),
    );
    let event: Event = serde_json::from_str(&event_json).expect("presence event");
    assert_eq!(event.kind.as_u16(), NEARBY_PRESENCE_KIND);
    event.verify().expect("valid signature");
    assert_eq!(event.pubkey, owner.public_key());

    let content: serde_json::Value = serde_json::from_str(&event.content).expect("content");
    assert_eq!(content["protocol"], "iris-nearby-v1");
    assert_eq!(content["peer_id"], "peer-a");
    assert_eq!(content["my_nonce"], "nonce-a");
    assert_eq!(content["their_nonce"], "nonce-b");
    assert_eq!(content["profile_event_id"], "f".repeat(64));

    let verified =
        crate::verify_nearby_presence_event_json(&event_json, "peer-a", "nonce-b", "nonce-a");
    let verified: serde_json::Value = serde_json::from_str(&verified).expect("verified presence");
    assert_eq!(verified["owner_pubkey_hex"], owner.public_key().to_hex());
    assert_eq!(verified["profile_event_id"], "f".repeat(64));

    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "peer-a",
        "wrong-receiver-nonce",
        "nonce-a",
    )
    .is_empty());
    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "wrong-peer",
        "nonce-b",
        "nonce-a",
    )
    .is_empty());
}

#[test]
fn mobile_push_decrypt_preview_does_not_mutate_persisted_ratchet_state() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_shared_conn = crate::core::storage::open_database(&data_dir).expect("bob db");
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        bob_shared_conn.clone(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_storage.clone()),
        None,
    );
    bob.init().expect("bob init");
    bob.accept_invite(&invite, Some(alice_keys.public_key()))
        .expect("bob accepts alice invite");
    deliver_published_events(&bob, &bob_keys, &alice);

    let user_record_key = format!("user/{}", alice_keys.public_key().to_hex());
    let before = bob_storage
        .get(&user_record_key)
        .expect("read stored ratchet before notification")
        .expect("stored ratchet before notification");

    let message = "closed-app preview stays read-only";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let published_events = drain_signed_events(&alice, &alice_keys);
    let message_event = published_events
        .iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "message event for Bob; published={}",
                serde_json::to_string(&published_events).unwrap_or_default()
            )
        });
    let payload = serde_json::json!({
        "event": serde_json::to_string(&message_event).expect("outer event json"),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();
    let apns_payload = serde_json::json!({
        "event": message_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, message);
    let apns_resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        apns_payload,
    );
    assert!(apns_resolution.should_show);
    assert_eq!(apns_resolution.body, message);

    let after = bob_storage
        .get(&user_record_key)
        .expect("read stored ratchet after notification")
        .expect("stored ratchet after notification");
    assert_eq!(
        before, after,
        "notification preview must not advance persisted ratchet state"
    );

    let bob_restarted_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("restarted db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let bob_restarted = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_restarted_storage),
        None,
    );
    bob_restarted.init().expect("bob restarted init");
    bob_restarted.process_received_event(message_event);
    assert!(
        drain_text_messages(&bob_restarted)
            .iter()
            .any(|body| body == message),
        "foreground runtime must still decrypt the relay event after notification preview"
    );
}

#[test]
fn mobile_push_payload_ingest_feeds_full_event_into_runtime() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
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
    bob_runtime
        .accept_invite(&invite, Some(alice_keys.public_key()))
        .expect("bob accepts alice invite");
    deliver_published_events(&bob_runtime, &bob_keys, &alice);

    let message = "push-only event";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let bob_message_authors = bob_runtime.get_all_message_push_author_pubkeys();
    let message_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Bob");
    let payload = serde_json::json!({
        "event": message_event.clone(),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let local_invite = Invite::create_new(
        bob_keys.public_key(),
        Some(bob_keys.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload.clone(),
    });
    let chat_id = alice_keys.public_key().to_hex();
    assert!(
        !core.threads.contains_key(&chat_id),
        "push event waits for session restore"
    );

    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: bob_runtime,
        local_invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });

    core.drain_pending_mobile_push_events();
    let thread = core.threads.get(&chat_id).expect("sender thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, message);
    let event_id = message_event.id.to_string();
    assert_eq!(
        thread.messages[0].source_event_id.as_deref(),
        Some(event_id.as_str())
    );

    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload,
    });
    let thread = core.threads.get(&chat_id).expect("sender thread after dup");
    assert_eq!(thread.messages.len(), 1, "duplicate push event is ignored");
}

#[test]
fn mobile_push_decrypt_suppresses_typing_rumors() {
    // Even with valid keys and a working ratchet, the notification
    // extension should suppress non-chat-message rumors. Typing/seen/
    // reaction wrappers are noise as standalone notifications — the
    // chat list updates when the user opens the app.
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_storage),
        None,
    );
    bob.init().expect("bob init");
    bob.accept_invite(&invite, Some(alice_keys.public_key()))
        .expect("bob accepts alice invite");
    deliver_published_events(&bob, &bob_keys, &alice);

    alice
        .send_typing(bob_keys.public_key(), None)
        .expect("alice sends typing");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let typing_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("typing event for Bob");
    let payload = serde_json::json!({
        "event": typing_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );

    assert!(
        !resolution.should_show,
        "typing rumors must not surface as standalone notifications"
    );
}

#[test]
fn mobile_push_fallback_suppresses_opaque_encrypted_events() {
    let encrypted_outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&Keys::generate())
        .expect("outer event");
    let payload = serde_json::json!({
        "event": encrypted_outer_event,
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert!(resolution.title.is_empty());
    assert!(resolution.body.is_empty());
}

#[test]
fn mobile_push_preview_resolves_from_sqlite_when_decrypt_fails() {
    // Simulates the "main app already consumed this event, ratchet has
    // moved on, push extension can't decrypt anymore" race: the
    // foreground app stored a message keyed by the outer event id; the
    // extension can't decrypt the same event, so it falls back to the
    // body/sender already on disk.
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("invite");
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device.public_key().to_hex(),
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: runtime,
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    // Pretend Alice's profile is already known.
    let alice = Keys::generate();
    core.owner_profiles.insert(
        alice.public_key().to_hex(),
        OwnerProfileRecord {
            name: Some("alice".to_string()),
            display_name: Some("Alice from work".to_string()),
            picture: None,
            updated_at_secs: 1,
        },
    );
    let outer_event_id = "a".repeat(64);
    let chat_id = alice.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: 200,
            messages: vec![ChatMessageSnapshot {
                id: "rumor-1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: alice.public_key().to_hex(),
                body: "lunch?".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 199,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                source_event_id: Some(outer_event_id.clone()),
            }],
        },
    );
    core.persist_best_effort_inner();

    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&alice)
        .expect("outer event");
    let payload = serde_json::json!({
        "event": serde_json::json!({
            "id": outer_event_id,
            "kind": MESSAGE_EVENT_KIND,
            "content": "",
            "tags": [],
            "pubkey": alice.public_key().to_hex(),
            "created_at": 200,
            "sig": outer_event.sig.to_string(),
        }),
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    // Use bogus device keys so NDR decrypt fails immediately and we
    // exercise the SQLite-backed preview path.
    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        owner.public_key().to_hex(),
        "nsec1invalid".to_string(),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, "lunch?");
    assert_eq!(resolution.title, "Alice from work");
}

#[test]
fn mobile_push_preview_survives_foreground_batch_ratchet_race() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_shared_conn = crate::core::storage::open_database(&data_dir).expect("bob db");
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        bob_shared_conn.clone(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut alice_invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("alice invite");
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
        Some(bob_storage.clone()),
        None,
    );
    bob_runtime.init().expect("bob init");
    bob_runtime
        .accept_invite(&alice_invite, Some(alice_keys.public_key()))
        .expect("bob accepts alice invite");
    deliver_published_events(&bob_runtime, &bob_keys, &alice);
    let bob_message_authors = bob_runtime.get_all_message_push_author_pubkeys();
    let user_record_key = format!("user/{}", alice_keys.public_key().to_hex());
    let ratchet_before = bob_storage
        .get(&user_record_key)
        .expect("read bob ratchet before message")
        .expect("bob ratchet before message");

    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let bob_local_invite = Invite::create_new(
        bob_keys.public_key(),
        Some(bob_keys.public_key().to_hex()),
        None,
    )
    .expect("bob local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: bob_runtime,
        local_invite: bob_local_invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.owner_profiles.insert(
        alice_keys.public_key().to_hex(),
        OwnerProfileRecord {
            name: Some("alice".to_string()),
            display_name: Some("Alice".to_string()),
            picture: None,
            updated_at_secs: 1,
        },
    );

    let message = "foreground batch preview";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let published_events = drain_signed_events(&alice, &alice_keys);
    let message_event = published_events
        .iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "message event for Bob; published={}",
                serde_json::to_string(&published_events).unwrap_or_default()
            )
        });
    let payload = serde_json::json!({
        "event": message_event,
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    core.enter_batch();
    core.handle_relay_event(message_event);
    assert!(
        core.batch_dirty_persist,
        "full state save should still be deferred inside the core batch"
    );
    let ratchet_after = bob_storage
        .get(&user_record_key)
        .expect("read bob ratchet after message")
        .expect("bob ratchet after message");
    assert_ne!(
        ratchet_before, ratchet_after,
        "foreground runtime should have consumed the relay event before the push handler runs"
    );

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );
    core.exit_batch();

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Alice");
    assert_eq!(resolution.body, message);
}

#[test]
fn mobile_push_fallback_suppresses_decrypted_non_message_kinds() {
    for kind in [
        0_u64,
        TYPING_KIND as u64,
        RECEIPT_KIND as u64,
        GROUP_METADATA_KIND as u64,
        CHAT_SETTINGS_KIND as u64,
        12345,
    ] {
        let inner_event = serde_json::json!({
            "kind": kind,
            "content": "not a chat message",
            "created_at": 1_777_159_483u64,
            "tags": [],
            "pubkey": "0".repeat(64),
            "id": format!("{kind:064x}"),
        });
        let payload = serde_json::json!({
            "inner_event_json": inner_event.to_string(),
            "title": "Iris Chat",
            "body": "New message",
        })
        .to_string();

        let resolution = resolve_mobile_push_notification(payload);

        assert!(
            !resolution.should_show,
            "inner kind {kind} should not produce a visible mobile push"
        );
    }
}

#[test]
fn mobile_push_fallback_allows_chat_message_kind() {
    let payload = serde_json::json!({
        "inner_kind": CHAT_MESSAGE_KIND.to_string(),
        "title": "Alice",
        "body": "hello",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Alice");
    assert_eq!(resolution.body, "hello");
}

#[test]
fn mobile_push_fallback_renders_invite_acceptance() {
    let keys = Keys::generate();
    let p_tag_pubkey = "a".repeat(64);
    let event = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "ciphertext")
        .tag(nostr::Tag::parse(["p", p_tag_pubkey.as_str()]).expect("p tag"))
        .sign_with_keys(&keys)
        .expect("invite response event");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&event).expect("event json"),
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Invite accepted");
    assert_eq!(resolution.body, "Someone joined your chat");
}

#[test]
fn mobile_push_subscription_body_includes_invite_response_filter() {
    let owner = Keys::generate();
    let author = Keys::generate().public_key().to_hex();
    let invite_response_pubkey = Keys::generate().public_key().to_hex();
    let request = build_mobile_push_create_subscription_request(
        owner
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| owner.secret_key().to_secret_hex()),
        "ios".to_string(),
        "apns-token".to_string(),
        Some("to.iris.chat".to_string()),
        vec![author.clone()],
        vec![invite_response_pubkey.clone()],
        true,
        None,
    )
    .expect("subscription request");
    let body: serde_json::Value =
        serde_json::from_str(request.body_json.as_deref().expect("body json")).expect("json");

    assert_eq!(body["filter"]["authors"][0].as_str(), Some(author.as_str()));
    assert_eq!(
        body["filters"][1]["kinds"][0],
        serde_json::json!(INVITE_RESPONSE_KIND)
    );
    assert_eq!(
        body["filters"][1]["#p"][0].as_str(),
        Some(invite_response_pubkey.as_str())
    );
}

#[test]
fn mobile_push_snapshot_tracks_local_invite_when_enabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("mobile-push-invite-response", &owner, &device);

    let snapshot = core.build_mobile_push_sync_snapshot();

    let invite_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_hex();
    assert_eq!(snapshot.invite_response_pubkeys, vec![invite_pubkey]);
}

#[test]
fn mobile_push_snapshot_omits_local_invite_when_disabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-invite-response-disabled", &owner, &device);
    core.set_invite_acceptance_notifications_enabled(false);

    let snapshot = core.build_mobile_push_sync_snapshot();

    assert!(snapshot.invite_response_pubkeys.is_empty());
}

#[test]
fn typing_indicators_default_to_opt_in() {
    assert!(!PersistedPreferences::default().send_typing_indicators);
    assert!(!AppState::empty().preferences.send_typing_indicators);
}

#[test]
fn startup_at_login_defaults_to_enabled() {
    assert!(PersistedPreferences::default().startup_at_login_enabled);
    assert!(AppState::empty().preferences.startup_at_login_enabled);
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
    assert_eq!(
        known_app_keys_to_ndr(&known)
            .expect("convert back")
            .get_all_devices()
            .len(),
        2
    );
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
        "older app-key events must not replace the cached roster"
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
        LocalAuthorizationState::Authorized
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
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.start_session(owner.public_key(), Some(owner), device.clone(), true, true)
        .expect("restored account bundle session");

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
    core.apply_app_keys_event(&remote_event);

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

    let remote_app_keys = AppKeys::new(vec![
        DeviceEntry::new(device.public_key(), 10),
        DeviceEntry::new(other_device.public_key(), 10),
    ]);
    let remote_event = remote_app_keys
        .get_event(owner.public_key())
        .sign_with_keys(&owner)
        .expect("app keys event");
    core.apply_app_keys_event(&remote_event);

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
    let invite = Invite::from_url(&snapshot.url).expect("parse link invite");
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
    let invite_url = invite.get_url(CHAT_INVITE_ROOT_URL).expect("invite url");

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
    let (_owner_session, response_event) = pending
        .invite
        .accept_with_owner(
            owner.public_key(),
            owner.secret_key().to_secret_bytes(),
            Some(owner.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("owner accepts");

    core.handle_relay_event(response_event);

    let logged_in = core.logged_in.as_ref().expect("linked session");
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::AwaitingApproval
    );
    assert!(core.pending_linked_device.is_none());
    assert!(logged_in
        .ndr_runtime
        .export_active_sessions()
        .iter()
        .any(|(peer, _, _)| *peer == owner.public_key()));
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
                source_event_id: None,
            }],
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
    let inner_id = "a".repeat(64);
    let first_outer_id = "b".repeat(64);
    let second_outer_id = "c".repeat(64);
    let content = serde_json::json!({
        "content": "ok",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_493u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": inner_id,
    })
    .to_string();

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
fn self_synced_direct_message_is_rendered_as_outgoing_on_linked_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-missing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let inner_id = "d".repeat(64);
    let content = serde_json::json!({
        "content": "sent from sibling",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_500u64,
        "tags": [["p", chat_id], ["ms", "1777159500123"]],
        "pubkey": owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(sibling_device.public_key()),
        content,
        Some("e".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.id, inner_id);
    assert_eq!(message.body, "sent from sibling");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn self_synced_direct_message_updates_existing_local_outgoing_without_duplicate() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-existing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let inner_id = "f".repeat(64);
    core.push_outgoing_message_with_id(
        inner_id.clone(),
        &chat_id,
        "local optimistic".to_string(),
        1_777_159_499,
        None,
        DeliveryState::Pending,
    );
    let content = serde_json::json!({
        "content": "sent from this device",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_500u64,
        "tags": [["p", chat_id], ["ms", "1777159500123"]],
        "pubkey": owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(device.public_key()),
        content,
        Some("a".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.body, "local optimistic");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn web_runtime_typing_rumors_do_not_become_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing", &owner, &device);
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&sender)
        .expect("outer event");
    let outer_event_id = outer_event.id.to_string();
    let content = serde_json::json!({
        "content": "typing",
        "kind": TYPING_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [["ms", "1777159483368"], ["expiration", "1777159543"]],
        "pubkey": "0".repeat(64),
        "id": "d".repeat(64),
    })
    .to_string();

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        content,
        Some(outer_event_id.clone()),
    );

    let chat_id = sender.public_key().to_hex();
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
    assert!(core.typing_indicators.values().any(|record| {
        record.chat_id == chat_id && record.author_owner_hex == sender.public_key().to_hex()
    }));

    // Typing rumors aren't durable, so the SQLite-backed
    // notification preview path doesn't find them and falls through
    // to the generic resolver — which suppresses non-message kinds.
    // Only durable chat messages get a real preview after the
    // foreground app has consumed the rumor.
    let payload = serde_json::json!({
        "event": outer_event,
        "title": "Iris Chat",
        "body": "New message",
    })
    .to_string();
    let resolution = decrypt_mobile_push_notification(
        core.data_dir.to_string_lossy().to_string(),
        "invalid-owner".to_string(),
        "invalid-device".to_string(),
        payload,
    );
    assert!(!resolution.should_show);
}

#[test]
fn web_runtime_typing_stop_clears_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing-stop", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 1);
    let content = serde_json::json!({
        "content": "typing",
        "kind": TYPING_KIND,
        "created_at": 1_777_159_484u64,
        "tags": [["expiration", "1"]],
        "pubkey": "0".repeat(64),
        "id": "a".repeat(64),
    })
    .to_string();

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("b".repeat(64)));

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
}

#[test]
fn newer_chat_message_clears_stale_typing_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-newer-message", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 10);
    core.push_outgoing_message_with_id(
        "newer-local-message".to_string(),
        &chat_id,
        "ok".to_string(),
        11,
        None,
        DeliveryState::Sent,
    );
    core.rebuild_state();

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
}

#[test]
fn first_received_message_clears_typing_indicator_in_chat_list() {
    // Repro for the "chat row stuck on Typing after the very first
    // peer message" complaint: the chat starts with no messages,
    // peer types, peer sends, the chat list row must drop the
    // typing badge as soon as the message lands. Goes through the
    // production path (`apply_runtime_text_message`), which is what
    // the relay event handler calls.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let row_before = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id);
    if let Some(row) = row_before {
        assert!(
            row.is_typing,
            "precondition: typing badge visible before the first message"
        );
    }

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(
        !row.is_typing,
        "chat list must drop the typing badge once the peer's first message lands"
    );
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn first_received_message_clears_typing_indicator_in_open_chat() {
    // Same case as the chat-list version, but with the chat actively
    // open. The in-chat typing badge reads from
    // `current_chat.typing_indicators`, which goes through the same
    // projection filter — must drop the indicator after the message.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg-open", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    // Open the chat: set active + create the (empty) thread record so
    // `current_chat` actually projects.
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 0,
            messages: Vec::new(),
        },
    );
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        !current.typing_indicators.is_empty(),
        "precondition: indicator visible in open chat"
    );

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        current.typing_indicators.is_empty(),
        "open chat must drop the typing badge after the first message"
    );
}

#[test]
fn first_received_message_at_same_second_clears_typing_indicator() {
    // Same bug, edge case: typing event and the chat message share
    // a one-second wire-clock tick. Production path again.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-same-second", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(!row.is_typing);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn typing_floor_blocks_late_typing_after_message() {
    // Bug shape from peer apps that don't send a stop-typing event:
    // a typing rumor with `created_at_secs` strictly greater than the
    // latest message slips through the projection's `>` filter and
    // re-arms the indicator after we've already shown the message.
    //
    // The floor is bumped when the message lands and gates any
    // subsequent typing event with `event_secs <= floor`. Even though
    // the typing rumor here has T=200 > the message's T=100, the
    // floor is also at 100 (it's the same chat) — the typing must
    // strictly exceed the floor *and* the floor itself was set by
    // the message we already saw, so a new typing event has to be
    // genuinely after that message's wire-clock second to surface.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-late", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );

    // Typing rumor races in after the message with the *same* wire
    // second — the floor at 100 keeps it suppressed.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 100, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));

    // A genuinely newer typing event (peer is typing again) does
    // arm the indicator.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 101, None);
    assert!(core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn typing_floor_persists_across_message_deletion() {
    // iris-chat (JS) keeps `lastMessageAt` monotonic so that deleting
    // a message doesn't let a stale typing rumor slip through. Same
    // contract here: once the floor reaches a given second, deleting
    // the message that put it there leaves the floor in place.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-delete", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.delete_local_message(&chat_id, "msg-1");

    // The thread is now empty (latest_message_secs would be 0) but
    // the floor must stay at 100 from the message we already saw.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 100, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn web_runtime_control_rumors_do_not_create_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let controls = [
        (
            RECEIPT_KIND,
            "seen",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
        (
            REACTION_KIND,
            "+",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
    ];

    for (index, (kind, body, tags)) in controls.into_iter().enumerate() {
        let mut core =
            logged_in_test_core(&format!("web-runtime-control-{index}"), &owner, &device);
        let content = serde_json::json!({
            "content": body,
            "kind": kind,
            "created_at": 1_777_159_483u64 + index as u64,
            "tags": tags,
            "pubkey": "0".repeat(64),
            "id": format!("{:064x}", index + 10),
        })
        .to_string();

        core.apply_decrypted_runtime_message(
            sender.public_key(),
            None,
            content,
            Some(format!("{:064x}", index + 20)),
        );

        let chat_id = sender.public_key().to_hex();
        assert!(
            core.threads
                .get(&chat_id)
                .map(|thread| thread.messages.is_empty())
                .unwrap_or(true),
            "control kind {kind} created a chat message"
        );
    }
}

#[test]
fn web_runtime_chat_settings_create_system_notice() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-chat-settings", &owner, &device);
    let content = serde_json::json!({
        "content": "60",
        "kind": CHAT_SETTINGS_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": "f".repeat(64),
    })
    .to_string();

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("1".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert!(thread.messages[0]
        .body
        .contains("set disappearing messages timer"));
}

#[test]
fn web_runtime_chat_message_expiration_tag_is_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-expiring-message", &owner, &device);
    let inner_id = "e".repeat(64);
    let content = serde_json::json!({
        "content": "secret",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [["expiration", "1777159543"]],
        "pubkey": "0".repeat(64),
        "id": inner_id,
    })
    .to_string();

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("f".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "secret");
    assert_eq!(thread.messages[0].expires_at_secs, Some(1_777_159_543));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &inner_id),
        Some(1_777_159_543)
    );
}

#[test]
fn chat_ttl_applies_to_outgoing_message_expiration() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("outgoing-message-ttl", &owner, &device);
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 60);

    let before = unix_now().get();
    core.send_message(&chat_id, "secret", None);
    let after = unix_now().get();

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("sent message");
    let expires_at = message.expires_at_secs.expect("message expiration");
    assert!(expires_at >= before.saturating_add(60));
    assert!(expires_at <= after.saturating_add(60));
}

#[test]
fn create_group_allows_self_only_group() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("self-only-group", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Notes".to_string(),
        member_inputs: Vec::new(),
    });

    let current = core.state.current_chat.as_ref().expect("opened group chat");
    let group_id = current.group_id.as_ref().expect("group id").clone();
    let group = core.groups.get(&group_id).expect("stored group");
    assert_eq!(group.name, "Notes");
    assert_eq!(group.members, vec![owner.public_key().to_hex()]);
    assert_eq!(group.admins, vec![owner.public_key().to_hex()]);
}

#[test]
fn group_metadata_changes_create_system_notices() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-metadata-notices", &owner, &device);
    let group_id = "group-notice-test".to_string();
    let chat_id = group_chat_id(&group_id);
    let initial = GroupData {
        id: group_id.clone(),
        name: "Original".to_string(),
        description: None,
        picture: None,
        members: vec![owner.public_key().to_hex()],
        admins: vec![owner.public_key().to_hex()],
        created_at: 1,
        secret: None,
        accepted: Some(true),
    };
    let renamed = GroupData {
        name: "Renamed".to_string(),
        ..initial.clone()
    };
    let member = Keys::generate().public_key().to_hex();
    let with_member = GroupData {
        members: vec![owner.public_key().to_hex(), member.clone()],
        ..renamed.clone()
    };
    let member_removed = GroupData {
        members: vec![owner.public_key().to_hex()],
        ..with_member.clone()
    };

    core.apply_group_metadata_notice(None, &initial);
    core.apply_group_metadata_notice(Some(&initial), &renamed);
    core.apply_group_metadata_notice(Some(&renamed), &with_member);
    core.apply_group_metadata_notice(Some(&with_member), &member_removed);
    let with_admin = GroupData {
        admins: with_member.members.clone(),
        ..with_member.clone()
    };
    core.apply_group_metadata_notice(Some(&with_member), &with_admin);

    let messages = &core.threads.get(&chat_id).expect("group thread").messages;
    assert!(messages
        .iter()
        .any(|message| message.body == "Group created: Original"));
    assert!(messages
        .iter()
        .any(|message| message.body == "Group renamed to Renamed"));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("joined the group")));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("left the group")));
    assert!(messages
        .iter()
        .any(|message| message.kind == ChatMessageKind::System));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("became an admin")));
}

#[test]
fn appcore_restart_restores_threads_groups_and_seen_events() {
    // End-to-end check that the persistence/load round trip survives a
    // full AppCore drop+recreate against the same `data_dir`.
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();

    let chat_id = "deadbeef".repeat(8);
    let group_id = "group-restart".to_string();
    let group_chat = group_chat_id(&group_id);

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            data_dir_str.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        let invite = Invite::create_new(
            device.public_key(),
            Some(device.public_key().to_hex()),
            None,
        )
        .expect("local invite");
        let runtime = NdrRuntime::new(
            device.public_key(),
            device.secret_key().to_secret_bytes(),
            device.public_key().to_hex(),
            owner.public_key(),
            None,
            Some(invite.clone()),
        );
        runtime.init().expect("runtime init");
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
            ndr_runtime: runtime,
            local_invite: invite,
            authorization_state: LocalAuthorizationState::Authorized,
        });

        core.next_message_id = 17;
        core.active_chat_id = Some(chat_id.clone());
        core.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 5)]),
                10,
            ),
        );
        core.groups.insert(
            group_id.clone(),
            GroupData {
                id: group_id.clone(),
                name: "Brunch".to_string(),
                description: None,
                picture: None,
                members: vec![owner.public_key().to_hex()],
                admins: vec![owner.public_key().to_hex()],
                created_at: 1_000,
                secret: None,
                accepted: Some(true),
            },
        );
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 3,
                updated_at_secs: 200,
                messages: vec![
                    ChatMessageSnapshot {
                        id: "m1".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: owner.public_key().to_hex(),
                        body: "hello world".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: true,
                        created_at_secs: 100,
                        expires_at_secs: None,
                        delivery: DeliveryState::Sent,
                        source_event_id: None,
                    },
                    ChatMessageSnapshot {
                        id: "m2".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: "peer".to_string(),
                        body: "right back atcha".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: false,
                        created_at_secs: 110,
                        expires_at_secs: None,
                        delivery: DeliveryState::Received,
                        source_event_id: None,
                    },
                ],
            },
        );
        core.threads.insert(
            group_chat.clone(),
            ThreadRecord {
                chat_id: group_chat.clone(),
                unread_count: 0,
                updated_at_secs: 50,
                messages: vec![ChatMessageSnapshot {
                    id: "g-system".to_string(),
                    chat_id: group_chat.clone(),
                    kind: ChatMessageKind::System,
                    author: owner.public_key().to_hex(),
                    body: "Group created: Brunch".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 50,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                    source_event_id: None,
                }],
            },
        );
        core.seen_event_order.push_back("evt-1".to_string());
        core.seen_event_order.push_back("evt-2".to_string());
        core.seen_event_ids = core.seen_event_order.iter().cloned().collect();
        core.preferences.send_typing_indicators = true;

        core.persist_best_effort_inner();
    }

    // New AppCore over the same directory: load_persisted should return
    // exactly what we wrote.
    let mut restarted = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str,
        Arc::new(RwLock::new(AppState::empty())),
    );
    let loaded = restarted
        .load_persisted()
        .expect("load_persisted")
        .expect("state persisted");
    assert_eq!(loaded.next_message_id, 17);
    assert_eq!(loaded.active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(loaded.preferences.send_typing_indicators);
    assert_eq!(loaded.threads.len(), 2);
    let dm_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .expect("dm thread present");
    assert_eq!(dm_thread.messages.len(), 2);
    assert_eq!(dm_thread.unread_count, 3);
    assert_eq!(dm_thread.messages[0].body, "hello world");
    assert_eq!(dm_thread.messages[1].body, "right back atcha");
    let group_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == group_chat)
        .expect("group thread present");
    assert!(matches!(
        group_thread.messages[0].kind,
        ChatMessageKind::System
    ));
    assert_eq!(loaded.groups.len(), 1);
    assert_eq!(loaded.groups[0].name, "Brunch");
    assert_eq!(loaded.app_keys.len(), 1);
    assert_eq!(loaded.seen_event_ids, vec!["evt-1", "evt-2"]);
    assert!(matches!(
        loaded.authorization_state,
        Some(PersistedAuthorizationState::Authorized)
    ));

    // Assert nothing was persisted in the legacy JSON layout.
    let legacy_meta = std::path::Path::new(&restarted.data_dir)
        .join("core")
        .join("meta.json");
    assert!(
        !legacy_meta.exists(),
        "legacy core/meta.json must not be created"
    );
}

#[test]
fn appcore_clear_persistence_drops_sqlite_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str.clone(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("invite");
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device.public_key().to_hex(),
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: runtime,
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.next_message_id = 5;
    core.persist_best_effort_inner();
    assert!(core.load_persisted().unwrap().is_some());

    core.clear_persistence_best_effort();
    assert!(core.load_persisted().unwrap().is_none());
}

#[test]
fn profile_picture_upload_propagates_to_account_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("profile-picture-upload", &owner, &device);
    core.rebuild_state();
    assert!(core.state.account.is_some(), "account snapshot exists");
    assert!(
        core.state.account.as_ref().unwrap().picture_url.is_none(),
        "no picture before upload"
    );

    let picture_url = "https://cdn.iris.to/abc123".to_string();
    core.handle_profile_picture_upload_finished(Ok(picture_url.clone()));

    let account = core.state.account.as_ref().expect("account after upload");
    assert_eq!(
        account.picture_url.as_deref(),
        Some(picture_url.as_str()),
        "picture url propagated to account snapshot"
    );
}

#[test]
fn delete_chat_removes_thread_and_navigates_back() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("delete-chat", &owner, &device);
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 100,
            messages: vec![ChatMessageSnapshot {
                id: "m1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: chat_id.clone(),
                body: "hi".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 100,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                source_event_id: None,
            }],
        },
    );
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 3600);
    core.active_chat_id = Some(chat_id.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];

    core.handle_action(AppAction::DeleteChat {
        chat_id: chat_id.clone(),
    });

    assert!(!core.threads.contains_key(&chat_id), "thread removed");
    assert!(
        !core.chat_message_ttl_seconds.contains_key(&chat_id),
        "ttl cleared"
    );
    assert!(core.active_chat_id.is_none(), "active chat cleared");
    assert!(
        !core
            .screen_stack
            .iter()
            .any(|s| matches!(s, Screen::Chat { chat_id: cid } if cid == &chat_id)),
        "chat screen popped"
    );
    assert!(
        !core
            .state
            .chat_list
            .iter()
            .any(|chat| chat.chat_id == chat_id),
        "chat_list snapshot reflects removal"
    );
}

#[test]
fn redelivered_persisted_message_after_restart_does_not_increment_unread() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("redelivered-persisted-message", &owner, &device);
    let old_message = ChatMessageSnapshot {
        id: "old-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        body: "already read".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 100,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        source_event_id: Some("outer-old".to_string()),
    };
    let latest_message = ChatMessageSnapshot {
        id: "latest-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        body: "latest preview".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 200,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        source_event_id: Some("outer-latest".to_string()),
    };
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 200,
            messages: vec![old_message, latest_message.clone()],
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    // Restart restores only a preview for inactive chats. Catch-up can then
    // redeliver older stored events that are not in memory anymore.
    let thread = core.threads.get_mut(&chat_id).expect("thread");
    thread.messages = vec![latest_message];
    thread.unread_count = 0;
    core.active_chat_id = None;
    core.screen_stack.clear();

    core.push_incoming_message_from(
        &chat_id,
        Some("old-message".to_string()),
        "already read".to_string(),
        100,
        None,
        Some(chat_id.clone()),
        Some("outer-old".to_string()),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(stored_message_count(&core), 2);
}

#[test]
fn prune_expired_messages_removes_loaded_messages_and_sqlite_rows() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-prune", &owner, &device);
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 200,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 100,
                    expires_at_secs: Some(150),
                    delivery: DeliveryState::Received,
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 200,
                    expires_at_secs: Some(300),
                    delivery: DeliveryState::Received,
                    source_event_id: None,
                },
            ],
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    let removed = core.prune_expired_messages(200);

    assert_eq!(removed, 1);
    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 1);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .messages
            .len(),
        1
    );
}

fn logged_in_test_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite = Invite::create_new(device.public_key(), Some(device_id.clone()), None)
        .expect("local invite");
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device_id,
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        ndr_runtime: runtime,
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core
}

fn stored_message_count(core: &AppCore) -> i64 {
    let conn = core.app_store.shared();
    let count = conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    count
}

fn stored_message_expiration(core: &AppCore, chat_id: &str, message_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let expires_at: Option<i64> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT expires_at_secs FROM messages WHERE chat_id = ?1 AND id = ?2",
            rusqlite::params![chat_id, message_id],
            |row| row.get(0),
        )
        .unwrap();
    expires_at.map(|secs| secs as u64)
}

fn deliver_published_events(from: &NdrRuntime, signer: &Keys, to: &NdrRuntime) {
    for event in drain_signed_events(from, signer) {
        to.process_received_event(event);
    }
}

fn drain_signed_events(runtime: &NdrRuntime, signer: &Keys) -> Vec<Event> {
    runtime
        .drain_events()
        .into_iter()
        .filter_map(|event| match event {
            SessionManagerEvent::Publish(unsigned) if unsigned.pubkey == signer.public_key() => {
                unsigned.sign_with_keys(signer).ok()
            }
            SessionManagerEvent::PublishSigned(event) => Some(event),
            SessionManagerEvent::PublishSignedForInnerEvent { event, .. } => Some(event),
            _ => None,
        })
        .collect()
}

fn drain_text_messages(runtime: &NdrRuntime) -> Vec<String> {
    runtime
        .drain_events()
        .into_iter()
        .filter_map(|event| match event {
            SessionManagerEvent::DecryptedMessage { content, .. } => {
                serde_json::from_str::<UnsignedEvent>(&content)
                    .ok()
                    .map(|event| event.content)
                    .or(Some(content))
            }
            _ => None,
        })
        .collect()
}

/// End-to-end round-trip: upload a real image to the hashtree network and
/// verify the same bytes can be read back via the same path the iOS shell
/// uses. Marked `ignore` because it depends on external network reachability.
/// Run manually with: cargo test profile_picture_hashtree_roundtrip --ignored -- --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn profile_picture_hashtree_roundtrip() {
    let owner = Keys::generate();
    let secret_hex = owner.secret_key().to_secret_hex();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ios/UITests/Fixtures/cat.jpg");
    let url = super::attachment_upload::upload_profile_picture_to_hashtree(&secret_hex, &path)
        .await
        .expect("upload");
    let nhash = url.strip_prefix("htree://").expect("htree:// prefix");
    let b64 = super::attachment_upload::download_hashtree_attachment_base64(nhash)
        .await
        .expect("download bytes");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("b64 decode");
    let original = std::fs::read(&path).expect("read original");
    assert_eq!(bytes, original, "downloaded bytes match original");
}
