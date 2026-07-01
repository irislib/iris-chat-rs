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
fn set_chat_message_ttl_action_sets_clears_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());

    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });

    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .message_ttl_seconds,
        Some(3600)
    );
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(loaded.chat_message_ttl_seconds.get(&chat_id), Some(&3600));

    let notice_count = core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.len())
        .unwrap_or_default();
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });
    assert_eq!(
        core.threads
            .get(&chat_id)
            .map(|thread| thread.messages.len())
            .unwrap_or_default(),
        notice_count,
        "reselecting the active timer must not publish another chat-settings notice"
    );

    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: None,
    });

    assert!(!core.chat_message_ttl_seconds.contains_key(&chat_id));
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let loaded = core
        .load_persisted()
        .expect("load persisted after clear")
        .expect("persisted state after clear");
    assert!(
        !loaded.chat_message_ttl_seconds.contains_key(&chat_id),
        "cleared ttl is not restored"
    );
}

#[test]
fn send_disappearing_message_action_uses_explicit_expiration_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("send-disappearing-message-action", &owner, &device);
    let expires_at = unix_now().get().saturating_add(600);

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "secret".to_string(),
        expires_at_secs: expires_at,
    });

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("disappearing message");
    assert_eq!(message.expires_at_secs, Some(expires_at));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &message.id),
        Some(expires_at)
    );
    assert!(
        core.message_expiry_token > 0,
        "expiring sends schedule message pruning"
    );
}

/// Repro for the macOS / Android bug: tapping a direct chat title pushes
/// the info screen, and back must return to the chat instead of the chat
/// list. Mirrors the group-details flow but for the direct-message case
/// after we converted both UIs from local overlays to the router push.
#[test]
fn back_from_direct_chat_info_returns_to_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("back-from-direct-info", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    core.handle_action(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }],
        "chat opened"
    );

    core.handle_action(AppAction::PushScreen {
        screen: Screen::DirectChatInfo {
            chat_id: chat_id.clone(),
        },
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![
            Screen::Chat {
                chat_id: chat_id.clone(),
            },
            Screen::DirectChatInfo {
                chat_id: chat_id.clone(),
            },
        ],
        "info pushed on top of the chat"
    );

    let mut next_stack = core.state.router.screen_stack.clone();
    next_stack.pop();
    core.handle_action(AppAction::UpdateScreenStack { stack: next_stack });

    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: chat_id.clone(),
        }],
        "back tap returns to the chat"
    );
    assert_eq!(
        core.active_chat_id.as_deref(),
        Some(chat_id.as_str()),
        "active chat is restored from the router"
    );
}

/// Repro for the Android bug: opening group details from a chat and then
/// pressing back must return to the chat — not jump to the chat list.
/// The Android UI sends `UpdateScreenStack(stack.dropLast())` for back
/// taps, so a `[Chat, GroupDetails]` → `[Chat]` round-trip through the
/// core has to keep `Chat` on the stack.
#[test]
fn back_from_group_details_returns_to_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("back-from-group-details", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Crew".to_string(),
        member_inputs: Vec::new(),
    });

    let group_id = core
        .state
        .current_chat
        .as_ref()
        .and_then(|chat| chat.group_id.clone())
        .expect("group id");
    let group_chat_id = format!("group:{group_id}");

    // After CreateGroup the chat is already active. Push group details
    // as the chat title tap would.
    core.handle_action(AppAction::PushScreen {
        screen: Screen::GroupDetails {
            group_id: group_id.clone(),
        },
    });
    assert_eq!(
        core.state.router.screen_stack,
        vec![
            Screen::Chat {
                chat_id: group_chat_id.clone(),
            },
            Screen::GroupDetails {
                group_id: group_id.clone(),
            },
        ],
        "details pushed on top of the chat"
    );

    // Mimic Android's back tap: drop the last screen and let the core
    // reconcile via UpdateScreenStack.
    let mut next_stack = core.state.router.screen_stack.clone();
    next_stack.pop();
    core.handle_action(AppAction::UpdateScreenStack { stack: next_stack });

    assert_eq!(
        core.state.router.screen_stack,
        vec![Screen::Chat {
            chat_id: group_chat_id.clone(),
        }],
        "back tap returns to the chat, not the chat list"
    );
    assert_eq!(
        core.active_chat_id.as_deref(),
        Some(group_chat_id.as_str()),
        "active chat is restored from the router"
    );
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .map(|chat| chat.chat_id.clone()),
        Some(group_chat_id),
        "projection re-emits current_chat on back"
    );
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
    let owner = ndr_owner_pubkey(owner.public_key());
    assert_eq!(group.name, "Notes");
    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(group.members, vec![owner]);
    assert_eq!(group.admins, vec![owner]);
    assert!(
        core.pending_relay_publishes.values().any(|pending| {
            serde_json::from_str::<Event>(&pending.event_json)
                .ok()
                .filter(nostr_double_ratchet::is_group_roster_fact_event)
                .and_then(|event| {
                    nostr_double_ratchet::parse_group_roster_fact_event(&event).ok()
                })
                .is_some_and(|fact| fact.group_id == group_id && fact.snapshot.name == "Notes")
        }),
        "creating a group should queue a signed group roster fact"
    );
}

#[test]
fn group_picture_projects_to_chat_list_current_chat_and_details() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-picture-projection", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Photo Group".to_string(),
        member_inputs: Vec::new(),
    });

    let current = core.state.current_chat.as_ref().expect("opened group chat");
    let group_id = current.group_id.as_ref().expect("group id").clone();
    let chat_id = group_chat_id(&group_id);
    let picture_url = "htree://nhash1group/photo.jpg".to_string();
    core.set_group_picture(&group_id, Some(picture_url.clone()));

    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str())
    );
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str())
    );

    core.screen_stack = vec![Screen::GroupDetails {
        group_id: group_id.clone(),
    }];
    core.rebuild_state();
    assert_eq!(
        core.state
            .group_details
            .as_ref()
            .and_then(|details| details.picture_url.as_deref()),
        Some(picture_url.as_str())
    );
}

/// Picture lives inside the protocol's `GroupSnapshot` now (ndr >=0.0.144),
/// so setting one and reloading must round-trip the field through the
/// engine's persisted group_json — not via the legacy `group_pictures` map.
#[test]
fn group_picture_persists_inside_protocol_group_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir);

    core.handle_action(AppAction::CreateGroup {
        name: "Persisted Photo Group".to_string(),
        member_inputs: Vec::new(),
    });

    let group_id = core
        .state
        .current_chat
        .as_ref()
        .and_then(|chat| chat.group_id.clone())
        .expect("group id");
    let picture_url = "htree://nhash1persisted/photo%201.jpg".to_string();
    core.set_group_picture(&group_id, Some(picture_url.clone()));

    let persisted = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(
        persisted
            .groups
            .iter()
            .find(|group| group.group_id == group_id)
            .and_then(|group| group.picture.as_deref()),
        Some(picture_url.as_str()),
        "picture lives on the persisted GroupSnapshot, not in a side table"
    );
}

/// Member changes from a peer admin arrive as a fresh `MetadataUpdated`
/// snapshot. With ndr >=0.0.144 the picture is part of that snapshot, so
/// preservation across membership updates is the peer admin's responsibility:
/// they must include the current picture in the snapshot they broadcast.
/// This test pins down the local apply behavior — what we render must
/// reflect whatever the latest snapshot says.
#[test]
fn group_picture_follows_metadata_snapshot_on_incoming_changes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-picture-membership", &owner, &device);
    let group_id = "group-picture-membership".to_string();
    let owner_pubkey = owner.public_key();
    let new_member = Keys::generate().public_key();
    let picture_url = "htree://nhash1retained/photo.jpg".to_string();

    // Seed: single-member group with a picture, as a peer admin would have
    // broadcast it (revision 1 carries the picture).
    let mut initial = test_group_snapshot(
        &group_id,
        "Photos",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        1,
    );
    initial.picture = Some(picture_url.clone());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(initial.clone()));

    // Peer admin adds a member: a well-behaved admin keeps the picture set
    // in the new revision's snapshot, so members on the other end keep
    // seeing it.
    let mut after_add = test_group_snapshot(
        &group_id,
        "Photos",
        owner_pubkey,
        vec![owner_pubkey, new_member],
        vec![owner_pubkey],
        2,
    );
    after_add.picture = Some(picture_url.clone());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(after_add));

    core.rebuild_state();
    let chat_id = group_chat_id(&group_id);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .and_then(|chat| chat.picture_url.as_deref()),
        Some(picture_url.as_str()),
        "picture set on the new revision must show up in chat list"
    );
}

#[test]
fn group_metadata_changes_create_system_notices() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-metadata-notices", &owner, &device);
    let group_id = "group-notice-test".to_string();
    let chat_id = group_chat_id(&group_id);
    let owner_pubkey = owner.public_key();
    let member = Keys::generate().public_key();
    let initial = test_group_snapshot(
        &group_id,
        "Original",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        1,
    );
    let renamed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        2,
    );
    let with_member = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey],
        3,
    );
    let member_removed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        4,
    );

    core.apply_group_metadata_notice(None, &initial);
    core.apply_group_metadata_notice(Some(&initial), &renamed);
    core.apply_group_metadata_notice(Some(&renamed), &with_member);
    core.apply_group_metadata_notice(Some(&with_member), &member_removed);
    let with_admin = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey, member],
        5,
    );
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
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
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
            test_group_snapshot(
                &group_id,
                "Brunch",
                owner.public_key(),
                vec![owner.public_key()],
                vec![owner.public_key()],
                1_000,
            ),
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
                        author_owner_pubkey_hex: Some(owner.public_key().to_hex()),
                        author_picture_url: None,
                        body: "hello world".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: true,
                        created_at_secs: 100,
                        expires_at_secs: None,
                        delivery: DeliveryState::Sent,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                    ChatMessageSnapshot {
                        id: "m2".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: "peer".to_string(),
                        author_owner_pubkey_hex: None,
                        author_picture_url: None,
                        body: "right back atcha".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: false,
                        created_at_secs: 110,
                        expires_at_secs: None,
                        delivery: DeliveryState::Received,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                ],

                draft: String::new(),
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
                    author_owner_pubkey_hex: Some(owner.public_key().to_hex()),
                    author_picture_url: None,
                    body: "Group created: Brunch".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 50,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                }],

                draft: String::new(),
            },
        );
        core.seen_event_order.push_back("evt-1".to_string());
        core.seen_event_order.push_back("evt-2".to_string());
        core.seen_event_ids = core.seen_event_order.iter().cloned().collect();
        core.preferences.send_typing_indicators = true;
        core.preferences.nearby_bluetooth_enabled = true;
        core.preferences.nearby_lan_enabled = true;

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
    assert!(loaded.preferences.nearby_bluetooth_enabled);
    assert!(loaded.preferences.nearby_lan_enabled);
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
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
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
                author_owner_pubkey_hex: Some(chat_id.clone()),
                author_picture_url: None,
                body: "hi".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 100,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
            }],

            draft: String::new(),
        },
    );
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 3600);
    core.preferences.pinned_chat_ids.push(chat_id.clone());
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
    assert!(
        !core
            .preferences
            .pinned_chat_ids
            .iter()
            .any(|pinned| pinned == &chat_id),
        "pinned state cleared"
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
