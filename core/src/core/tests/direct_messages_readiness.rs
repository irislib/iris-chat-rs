#[test]
fn opening_uncached_direct_chat_starts_targeted_profile_fetch() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("open-direct-profile-fetch", &owner, &device);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls =
        relay_urls_from_strings(&core.preferences.nostr_relay_urls);
    core.debug_log.clear();

    core.handle_action(AppAction::CreateChat {
        peer_input: peer.public_key().to_hex(),
    });

    let peer_hex = peer.public_key().to_hex();
    assert!(
        core.debug_log.iter().any(|entry| {
            entry.category == "profile.metadata.fetch"
                && entry.detail.contains("reason=open_chat")
                && entry.detail.contains(&peer_hex)
        }),
        "opening an uncached direct chat should start a narrow profile metadata fetch"
    );
    assert!(
        core.debug_log
            .iter()
            .all(|entry| !entry.category.starts_with("protocol.catch_up")),
        "opening a direct chat should not start protocol catch-up work"
    );
}

#[test]
fn direct_chat_peer_id_only_reports_missing_appkeys_and_blocks_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("readiness-peer-id-only", &owner, &device);
    let peer_hex = peer.public_key().to_hex();

    core.handle_action(AppAction::CreateChat {
        peer_input: peer_hex.clone(),
    });

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::PeerAppKeysMissing
    );
    assert!(!chat.protocol_readiness.can_send);
    assert!(core
        .compute_protocol_subscription_plan()
        .expect("subscription plan")
        .roster_authors
        .contains(&peer_hex));

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: peer_hex.clone(),
        text: "blocked until ready".to_string(),
    });

    assert_eq!(
        core.state.toast.as_deref(),
        Some("This chat is not ready yet. Waiting for the recipient's app keys.")
    );
    assert!(
        core.threads
            .get(&peer_hex)
            .is_some_and(|thread| thread.messages.is_empty()),
        "blocked readiness send must not create a local message row"
    );
    assert!(
        core.pending_relay_publishes.is_empty(),
        "blocked readiness send must not produce signed relay publishes"
    );
}

#[test]
fn direct_chat_readiness_converges_from_appkeys_and_invite_events() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-direct-converges", &owner, &device);
    let peer_hex = peer_owner.public_key().to_hex();
    let peer_device_hex = peer_device.public_key().to_hex();
    let local_device_hex = device.public_key().to_hex();

    core.handle_action(AppAction::CreateChat {
        peer_input: peer_hex.clone(),
    });

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::PeerAppKeysMissing
    );
    assert!(!chat.protocol_readiness.can_send);
    let missing_appkeys_plan = core
        .compute_protocol_subscription_plan()
        .expect("missing-appkeys subscription plan");
    assert!(
        missing_appkeys_plan.roster_authors.contains(&peer_hex),
        "unknown direct chat must subscribe to the peer owner's AppKeys"
    );

    let app_keys_event = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 10)])
        .get_event(peer_owner.public_key())
        .sign_with_keys(&peer_owner)
        .expect("signed peer AppKeys");
    core.handle_relay_event(app_keys_event);

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::PeerSessionMissing
    );
    assert!(!chat.protocol_readiness.can_send);
    let missing_session_plan = core
        .compute_protocol_subscription_plan()
        .expect("missing-session subscription plan");
    assert!(
        missing_session_plan.invite_authors.contains(&peer_device_hex),
        "known peer devices must be tracked for public invite events"
    );
    assert!(
        missing_session_plan
            .message_recipients
            .contains(&local_device_hex),
        "local device recipient bootstrap must stay subscribed until the peer session exists"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(11), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(peer_device.public_key()),
        Some(ndr_owner_pubkey(peer_owner.public_key())),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed peer invite");
    core.handle_relay_event(invite_event);

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::Ready
    );
    assert!(chat.protocol_readiness.can_send);

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: peer_hex.clone(),
        text: "ready after subscription state".to_string(),
    });

    let thread = core.threads.get(&peer_hex).expect("peer thread");
    assert!(
        thread
            .messages
            .iter()
            .any(|message| message.is_outgoing
                && message.body == "ready after subscription state"),
        "ready send must create a local outgoing row"
    );
    assert!(
        !core.pending_relay_publishes.is_empty(),
        "ready send must preserve signed relay publish retry"
    );
    assert!(
        core.pending_relay_publishes
            .values()
            .all(|pending| serde_json::from_str::<Event>(&pending.event_json).is_ok()),
        "pending relay publishes must contain already-signed Nostr events"
    );
}

#[test]
fn direct_chat_roster_without_session_reports_peer_session_missing() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-roster-no-session", &owner, &device);

    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");

    core.handle_action(AppAction::CreateChat {
        peer_input: peer_owner.public_key().to_hex(),
    });

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::PeerSessionMissing
    );
    assert!(!chat.protocol_readiness.can_send);
}

#[test]
fn direct_chat_with_peer_invite_is_ready_and_publish_retry_still_tracks_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("readiness-peer-invite", &owner, &device);
    let peer_hex = peer_owner.public_key().to_hex();

    observe_peer_device_invite_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &peer_owner,
        &peer_device,
        10,
    );

    core.handle_action(AppAction::CreateChat {
        peer_input: peer_hex.clone(),
    });

    let chat = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        chat.protocol_readiness.reason,
        ProtocolReadinessReason::Ready
    );
    assert!(chat.protocol_readiness.can_send);

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: peer_hex.clone(),
        text: "ready send".to_string(),
    });

    assert_eq!(core.state.toast, None);
    assert!(
        core.threads
            .get(&peer_hex)
            .expect("peer thread")
            .messages
            .iter()
            .any(|message| message.is_outgoing && message.body == "ready send"),
        "ready send must create a local outgoing row"
    );
    assert!(
        !core.pending_relay_publishes.is_empty(),
        "ready send must still register signed relay publish retry"
    );
}

#[test]
fn group_readiness_reports_missing_metadata_not_joined_and_ready() {
    use nostr_double_ratchet::{group::GroupSnapshot, GroupProtocol, OwnerPubkey, UnixSeconds};

    let owner = Keys::generate();
    let device = Keys::generate();
    let other = Keys::generate();
    let mut core = logged_in_test_core("readiness-group", &owner, &device);

    let missing_group_id = "missing_group".to_string();
    let missing_chat_id = group_chat_id(&missing_group_id);
    core.ensure_thread_record(&missing_chat_id, 1);
    core.active_chat_id = Some(missing_chat_id.clone());
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("missing group chat")
            .protocol_readiness
            .reason,
        ProtocolReadinessReason::GroupMetadataMissing
    );

    core.pending_relay_publishes.clear();
    core.handle_action(AppAction::SendMessage {
        chat_id: missing_chat_id.clone(),
        text: "blocked group".to_string(),
    });
    assert_eq!(
        core.state.toast.as_deref(),
        Some("This group is not ready yet. Waiting for group metadata.")
    );
    assert!(
        core.threads
            .get(&missing_chat_id)
            .is_some_and(|thread| thread.messages.is_empty()),
        "missing group metadata must block sends without local rows"
    );
    assert!(core.pending_relay_publishes.is_empty());

    core.state.toast = None;
    let not_joined_group_id = "not_joined_group".to_string();
    let not_joined_chat_id = group_chat_id(&not_joined_group_id);
    core.groups.insert(
        not_joined_group_id.clone(),
        GroupSnapshot {
            group_id: not_joined_group_id.clone(),
            protocol: GroupProtocol::sender_key_v1(),
            name: "Not Joined".to_string(),
            picture: None,
            about: None,
            created_by: OwnerPubkey::from_bytes(other.public_key().to_bytes()),
            members: vec![OwnerPubkey::from_bytes(other.public_key().to_bytes())],
            admins: vec![OwnerPubkey::from_bytes(other.public_key().to_bytes())],
            revision: 1,
            created_at: UnixSeconds(1),
            updated_at: UnixSeconds(1),
        },
    );
    core.ensure_thread_record(&not_joined_chat_id, 1);
    core.active_chat_id = Some(not_joined_chat_id);
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("not joined group chat")
            .protocol_readiness
            .reason,
        ProtocolReadinessReason::GroupNotJoined
    );

    core.handle_action(AppAction::CreateGroup {
        name: "Ready Group".to_string(),
        member_inputs: Vec::new(),
    });
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("ready group chat")
            .protocol_readiness
            .reason,
        ProtocolReadinessReason::Ready
    );
}
