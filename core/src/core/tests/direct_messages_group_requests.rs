#[test]
fn group_created_by_unknown_creator_surfaces_as_request_and_clears_on_accept() {
    use nostr_double_ratchet::{group::GroupSnapshot, GroupProtocol, OwnerPubkey, UnixSeconds};

    let owner = Keys::generate();
    let device = Keys::generate();
    let creator = Keys::generate();
    let mut core = logged_in_test_core("group-stranger-request", &owner, &device);

    let group_id = "groupchat_stranger".to_string();
    let chat_id = format!("group:{group_id}");
    let group = GroupSnapshot {
        group_id: group_id.clone(),
        protocol: GroupProtocol::sender_key_v1(),
        name: "Stranger's group".to_string(),
        picture: None,
        about: None,
        created_by: OwnerPubkey::from_bytes(creator.public_key().to_bytes()),
        members: vec![
            OwnerPubkey::from_bytes(creator.public_key().to_bytes()),
            OwnerPubkey::from_bytes(owner.public_key().to_bytes()),
        ],
        admins: vec![OwnerPubkey::from_bytes(creator.public_key().to_bytes())],
        revision: 1,
        created_at: UnixSeconds(1_777_159_000),
        updated_at: UnixSeconds(1_777_159_000),
    };
    core.apply_group_decrypted_event(
        nostr_double_ratchet::group::GroupIncomingEvent::MetadataUpdated(group),
    );
    core.rebuild_state();

    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("group thread visible after metadata arrives");
    assert!(
        snapshot.is_request,
        "group add from an unknown creator must surface as a request"
    );

    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: chat_id.clone(),
    });
    let after = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("group still visible after accept");
    assert!(
        !after.is_request,
        "accepting a group request flips it out of request state"
    );
    assert!(
        core.state
            .preferences
            .accepted_owner_pubkeys
            .contains(&creator.public_key().to_hex()),
        "group accept whitelists the creator (Signal pattern) so future adds by them auto-accept"
    );
}

#[test]
fn group_created_by_accepted_peer_is_not_a_request() {
    use nostr_double_ratchet::{group::GroupSnapshot, GroupProtocol, OwnerPubkey, UnixSeconds};

    let owner = Keys::generate();
    let device = Keys::generate();
    let creator = Keys::generate();
    let mut core = logged_in_test_core("group-known-creator", &owner, &device);

    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: creator.public_key().to_hex(),
    });
    let chat_id = "group:trusted_group".to_string();
    let group = GroupSnapshot {
        group_id: "trusted_group".to_string(),
        protocol: GroupProtocol::sender_key_v1(),
        name: "Friend's group".to_string(),
        picture: None,
        about: None,
        created_by: OwnerPubkey::from_bytes(creator.public_key().to_bytes()),
        members: vec![
            OwnerPubkey::from_bytes(creator.public_key().to_bytes()),
            OwnerPubkey::from_bytes(owner.public_key().to_bytes()),
        ],
        admins: vec![OwnerPubkey::from_bytes(creator.public_key().to_bytes())],
        revision: 1,
        created_at: UnixSeconds(1_777_159_000),
        updated_at: UnixSeconds(1_777_159_000),
    };
    core.apply_group_decrypted_event(
        nostr_double_ratchet::group::GroupIncomingEvent::MetadataUpdated(group),
    );
    core.rebuild_state();

    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("group thread visible");
    assert!(
        !snapshot.is_request,
        "group added by an accepted peer must not gate behind a request"
    );
}

#[test]
fn group_created_on_linked_device_syncs_to_primary_as_self_chat() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let mut primary = logged_in_test_core("linked-group-primary", &owner, &primary_device);
    let mut linked = logged_in_test_core("linked-group-linked", &owner, &linked_device);
    linked.logged_in.as_mut().expect("linked logged in").owner_keys = None;

    install_two_way_local_sibling_state_for_test(
        &mut primary,
        &mut linked,
        &owner,
        &primary_device,
        &linked_device,
    );

    primary.pending_relay_publishes.clear();
    linked.pending_relay_publishes.clear();
    linked.handle_action(AppAction::CreateGroup {
        name: "Okkk".to_string(),
        member_inputs: Vec::new(),
    });

    let chat_id = linked
        .active_chat_id
        .clone()
        .expect("linked opens the created group");
    let group_id = parse_group_id_from_chat_id(&chat_id).expect("created group id");
    deliver_pending_relay_events_for_test(&linked, &mut primary);
    primary.rebuild_state();

    let created = primary
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .unwrap_or_else(|| panic!("primary did not learn linked-created group {chat_id}"));
    assert!(
        !created.is_request,
        "group created by another device on the same account must not ask for approval"
    );

    primary.pending_relay_publishes.clear();
    linked.pending_relay_publishes.clear();
    linked.handle_action(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "hello from linked".to_string(),
    });
    deliver_pending_relay_events_for_test(&linked, &mut primary);
    primary.rebuild_state();

    let thread = primary
        .threads
        .get(&chat_id)
        .unwrap_or_else(|| panic!("primary thread missing for {chat_id}"));
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "hello from linked")
        .unwrap_or_else(|| {
            panic!(
                "primary did not render linked-device group message; group_id={group_id} messages={:?} debug={:?}",
                thread.messages, primary.debug_log
            )
        });
    assert!(
        message.is_outgoing,
        "messages sent by another linked device on the same account must render as outgoing"
    );
}

#[test]
fn compact_linked_device_group_messages_sync_to_primary_without_request() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let mut primary = logged_in_test_core("compact-linked-group-primary", &owner, &primary_device);
    primary.pending_relay_publishes.clear();

    let linked_temp_dir = tempfile::TempDir::new().expect("linked temp dir");
    let mut linked = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        linked_temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    linked.preferences.nostr_relay_urls.clear();
    linked.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });
    let compact_code = linked
        .state
        .link_device
        .as_ref()
        .expect("compact link code")
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
        device_input: compact_code,
    });
    assert_eq!(primary.state.toast, None);

    for event in sorted_pending_events_for_test(&primary)
        .into_iter()
        .filter(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
    {
        linked.handle_relay_event(event);
    }
    for event in sorted_pending_events_for_test(&primary)
        .into_iter()
        .filter(|event| {
            event.kind.as_u16() as u32 == APP_KEYS_EVENT_KIND
                && event_has_tag_value(event, "device", &linked_device_hex)
        })
    {
        linked.handle_relay_event(event);
    }
    for event in sorted_pending_events_for_test(&primary)
        .into_iter()
        .filter(|event| event.kind.as_u16() as u32 == INVITE_EVENT_KIND)
    {
        linked.handle_relay_event(event);
    }
    linked.refresh_local_authorization_state();
    assert_eq!(
        linked
            .logged_in
            .as_ref()
            .expect("linked logged in")
            .authorization_state,
        LocalAuthorizationState::Authorized
    );

    primary.pending_relay_publishes.clear();
    linked.pending_relay_publishes.clear();
    linked.handle_action(AppAction::CreateGroup {
        name: "Okkk".to_string(),
        member_inputs: Vec::new(),
    });
    let chat_id = linked
        .active_chat_id
        .clone()
        .expect("linked opens compact-created group");
    deliver_pending_relay_events_for_test(&linked, &mut primary);
    primary.rebuild_state();
    let created = primary
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .unwrap_or_else(|| {
            let linked_pending_kinds = sorted_pending_events_for_test(&linked)
                .into_iter()
                .map(|event| event.kind.as_u16() as u32)
                .collect::<Vec<_>>();
            panic!(
                "primary did not learn compact-linked group {chat_id}; linked_pending_kinds={linked_pending_kinds:?} primary_app_keys={:?} linked_app_keys={:?} primary_debug={:?} linked_debug={:?}",
                primary.app_keys, linked.app_keys, primary.debug_log, linked.debug_log
            )
        });
    assert!(
        !created.is_request,
        "group created by a compact-linked same-account device must not ask for approval"
    );

    primary.pending_relay_publishes.clear();
    linked.pending_relay_publishes.clear();
    linked.handle_action(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "hello from compact linked".to_string(),
    });
    deliver_pending_relay_events_for_test(&linked, &mut primary);
    primary.rebuild_state();
    let thread = primary
        .threads
        .get(&chat_id)
        .unwrap_or_else(|| panic!("primary thread missing for {chat_id}"));
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "hello from compact linked")
        .unwrap_or_else(|| {
            panic!(
                "primary did not render compact-linked group message; messages={:?} debug={:?}",
                thread.messages, primary.debug_log
            )
        });
    assert!(
        message.is_outgoing,
        "compact-linked same-account group messages must render as outgoing"
    );
}

fn install_two_way_local_sibling_state_for_test(
    primary: &mut AppCore,
    linked: &mut AppCore,
    owner: &Keys,
    primary_device: &Keys,
    linked_device: &Keys,
) {
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    for core in [&mut *primary, &mut *linked] {
        core.apply_known_app_keys_snapshot(owner.public_key(), &local_app_keys, 1);
        core.protocol_engine
            .as_mut()
            .expect("protocol engine")
            .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
            .expect("local appkeys");
    }

    let linked_invite = linked
        .protocol_engine
        .as_ref()
        .expect("linked protocol engine")
        .local_invite()
        .expect("linked invite");
    let (primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .protocol_engine
        .as_mut()
        .expect("primary protocol engine")
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            primary_session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");

    let linked_response = nostr_double_ratchet::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet::invite_response_event(&response).expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .protocol_engine
        .as_mut()
        .expect("linked protocol engine")
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");

    let mut primary_invite = primary
        .protocol_engine
        .as_ref()
        .expect("primary protocol engine")
        .local_invite()
        .expect("primary invite");
    primary_invite.owner_public_key = Some(owner.public_key());
    primary_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let primary_invite_event = nostr_double_ratchet::invite_unsigned_event(&primary_invite)
        .expect("primary invite unsigned")
        .sign_with_keys(primary_device)
        .expect("primary invite event");
    linked
        .protocol_engine
        .as_mut()
        .expect("linked protocol engine")
        .observe_invite_event(&primary_invite_event)
        .expect("linked observes primary invite");
}

fn deliver_pending_relay_events_for_test(sender: &AppCore, recipient: &mut AppCore) {
    for event in sorted_pending_events_for_test(sender) {
        recipient.handle_relay_event(event);
    }
}

fn sorted_pending_events_for_test(core: &AppCore) -> Vec<Event> {
    let mut events = core
        .pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .collect::<Vec<_>>();
    events.sort_by_key(|event| {
        (
            pending_event_delivery_priority(event.kind.as_u16() as u32),
            event.created_at.as_secs(),
            event.id.to_string(),
        )
    });
    events
}

fn pending_event_delivery_priority(kind: u32) -> u8 {
    if kind == APP_KEYS_EVENT_KIND {
        0
    } else if kind == INVITE_EVENT_KIND || kind == INVITE_RESPONSE_KIND {
        1
    } else if kind == GROUP_ROSTER_FACT_KIND {
        2
    } else if kind == MESSAGE_EVENT_KIND {
        3
    } else {
        4
    }
}
