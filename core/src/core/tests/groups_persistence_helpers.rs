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
        let invite = Invite::create_new(
            device.public_key(),
            Some(device.public_key().to_hex()),
            None,
        )
        .expect("local invite");
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
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
    let invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
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

#[test]
fn pinning_chat_moves_it_above_newer_unpinned_chats_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("pin-chat", &owner, &device);
    let older_chat_id = Keys::generate().public_key().to_hex();
    let newer_chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        older_chat_id.clone(),
        ThreadRecord {
            chat_id: older_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(
                &older_chat_id,
                "older",
                "older",
                10,
                false,
            )],

            draft: String::new(),
        },
    );
    core.threads.insert(
        newer_chat_id.clone(),
        ThreadRecord {
            chat_id: newer_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 20,
            messages: vec![test_chat_message(
                &newer_chat_id,
                "newer",
                "newer",
                20,
                false,
            )],

            draft: String::new(),
        },
    );
    core.rebuild_state();
    assert_eq!(core.state.chat_list[0].chat_id, newer_chat_id);

    core.handle_action(AppAction::SetChatPinned {
        chat_id: older_chat_id.clone(),
        pinned: true,
    });

    assert_eq!(core.state.chat_list[0].chat_id, older_chat_id);
    assert!(core.state.chat_list[0].is_pinned);
    assert_eq!(core.state.chat_list[1].chat_id, newer_chat_id);
    assert!(!core.state.chat_list[1].is_pinned);
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("state persisted");
    assert_eq!(loaded.preferences.pinned_chat_ids, vec![older_chat_id]);
}

#[test]
fn set_chat_unread_toggles_local_unread_count() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("set-chat-unread", &owner, &device);
    let chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(&chat_id, "m1", "hello", 10, false)],

            draft: String::new(),
        },
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: true,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 1);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        1
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: false,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 0);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        0
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
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
        body: "already read".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 100,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-old".to_string()),
    };
    let latest_message = ChatMessageSnapshot {
        id: "latest-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
        body: "latest preview".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 200,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-latest".to_string()),
    };
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 200,
            messages: vec![old_message, latest_message.clone()],

            draft: String::new(),
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
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 100,
                    expires_at_secs: Some(150),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 200,
                    expires_at_secs: Some(300),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
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

/// Regression for iOS RUNNINGBOARD 0xdead10cc crashes: a relay event
/// queued just before `PrepareForSuspend` (or one that races in from
/// the FFI channel during the suspend window) used to keep running
/// inside `handle_relay_event_with_channel` and write to SQLite,
/// which iOS' watchdog kills mid-`pwrite`. After this fix the gate
/// in `handle_internal` drops queued background work once
/// `PrepareForSuspend` has run, and clears on `AppForegrounded`.
#[test]
fn suspend_gate_drops_internal_events_until_foregrounded() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    // DebugLog is a simple internal-event signal: the handler appends
    // to `core.debug_log`, no other code path mutates it, and it's
    // not pruned by `rebuild_state` like typing indicators are.
    let send_debug_log = |core: &mut AppCore, detail: &str| {
        core.handle_message(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "test".to_string(),
            detail: detail.to_string(),
        })));
    };
    let log_count = |core: &AppCore| -> usize {
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "test")
            .count()
    };

    // Sanity: an internal event before suspend lands in debug_log.
    send_debug_log(&mut core, "before-suspend");
    assert_eq!(log_count(&core), 1, "DebugLog must land before suspend");

    // Engage the gate via the real CoreMsg path that iOS uses.
    let (reply_tx, _reply_rx) = flume::bounded(1);
    core.handle_message(CoreMsg::PrepareForSuspend(reply_tx));

    // While suspended, internal events must be dropped — the gate is
    // what keeps SQLite from being written while iOS is killing us.
    send_debug_log(&mut core, "during-suspend");
    assert_eq!(
        log_count(&core),
        1,
        "suspend gate must drop internal events"
    );

    // Foregrounding lifts the gate; the next internal event is processed.
    core.handle_message(CoreMsg::Action(AppAction::AppForegrounded));
    send_debug_log(&mut core, "after-foreground");
    assert_eq!(
        log_count(&core),
        2,
        "after AppForegrounded, internal events flow again"
    );
}

#[test]
fn internal_prune_expired_messages_event_ignores_stale_tokens_and_updates_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-internal-event", &owner, &device);
    let now = unix_now().get();
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: now,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now.saturating_sub(20),
                    expires_at_secs: Some(now.saturating_sub(1)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now,
                    expires_at_secs: Some(now.saturating_add(3600)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    core.handle_prune_expired_messages(core.message_expiry_token.wrapping_add(1));

    assert_eq!(stored_message_count(&core), 2, "stale token ignored");
    assert_eq!(
        core.threads
            .get(&chat_id)
            .expect("thread after stale token")
            .messages
            .len(),
        2
    );

    let valid_token = core.message_expiry_token;
    core.handle_prune_expired_messages(valid_token);

    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread after prune");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat after prune")
            .messages
            .len(),
        1
    );
}

fn logged_in_test_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    logged_in_test_core_at_data_dir(
        owner,
        device,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
    )
}

fn logged_in_test_core_with_storage(
    label: &str,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> AppCore {
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
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn logged_in_test_core_at_data_dir(owner: &Keys, device: &Keys, data_dir: String) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite = Invite::create_new(device.public_key(), Some(device_id.clone()), None)
        .expect("local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn test_chat_message(
    chat_id: &str,
    id: &str,
    body: &str,
    created_at_secs: u64,
    is_outgoing: bool,
) -> ChatMessageSnapshot {
    ChatMessageSnapshot {
        id: id.to_string(),
        chat_id: chat_id.to_string(),
        kind: ChatMessageKind::User,
        author: chat_id.to_string(),
        author_owner_pubkey_hex: Some(chat_id.to_string()),
        author_picture_url: None,
        body: body.to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing,
        created_at_secs,
        expires_at_secs: None,
        delivery: if is_outgoing {
            DeliveryState::Sent
        } else {
            DeliveryState::Received
        },
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: None,
    }
}

fn test_protocol_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    test_protocol_engine_with_storage(owner, device, storage)
}

fn test_protocol_engine_with_storage(
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> ProtocolEngine {
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    let session_manager =
        SessionManager::new(local_owner, device.secret_key().to_secret_bytes()).snapshot();
    let group_manager = NostrGroupManager::new(local_owner).snapshot();
    ProtocolEngine::load_or_seed(
        storage,
        owner.public_key(),
        device,
        local_invite,
        session_manager,
        group_manager,
    )
    .expect("protocol engine")
}

fn observe_current_device_appkeys_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
) {
    let created_at = unix_now().get();
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("local appkeys");
}

fn observe_peer_device_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    created_at: u64,
) {
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("peer appkeys");
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(created_at), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(device.public_key()),
        Some(ndr_owner_pubkey(owner.public_key())),
        None,
    )
    .expect("peer invite");
    let event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(device)
        .expect("signed invite");
    engine
        .observe_invite_event(&event)
        .expect("observe peer invite");
}

fn protocol_payload_events_for_result<'a>(
    effects: &'a [ProtocolEffect],
    event_ids: &[String],
) -> Vec<&'a Event> {
    let event_ids = event_ids.iter().cloned().collect::<HashSet<_>>();
    protocol_effect_events(effects)
        .into_iter()
        .filter(|event| event_ids.contains(&event.id.to_string()))
        .collect()
}

fn protocol_effect_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| &publish.event)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn protocol_targeted_payload_count(effects: &[ProtocolEffect], owner_pubkey_hex: &str) -> usize {
    effects
        .iter()
        .map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                target_owner_pubkey_hex,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex) => 1,
            ProtocolEffect::PublishStagedFirstContact { payload, .. } => payload
                .iter()
                .filter(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex)
                })
                .count(),
            _ => 0,
        })
        .sum()
}

fn latest_sender_key_distribution_for_test(
    engine: &ProtocolEngine,
    group_id: &str,
    created_at: NdrUnixSeconds,
) -> nostr_double_ratchet::SenderKeyDistribution {
    let sender_key = engine
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("sender-key record for group");
    let key_id = sender_key.latest_key_id.expect("latest sender key id");
    let state = sender_key
        .states
        .iter()
        .find(|state| state.key_id() == key_id)
        .expect("sender-key state");
    nostr_double_ratchet::SenderKeyDistribution {
        group_id: group_id.to_string(),
        key_id,
        sender_event_pubkey: sender_key.sender_event_pubkey,
        chain_key: state.chain_key(),
        iteration: state.iteration(),
        created_at,
    }
}

fn install_test_protocol_engine(
    core: &mut AppCore,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
    seed_session_manager: Option<SessionManagerSnapshot>,
    seed_group_manager: Option<GroupManagerSnapshot>,
) {
    let local_invite = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .clone();
    let seed_session_manager = seed_session_manager.unwrap_or_else(|| {
        SessionManager::new(
            NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
            device.secret_key().to_secret_bytes(),
        )
        .snapshot()
    });
    let seed_group_manager = seed_group_manager.unwrap_or_else(|| {
        NostrGroupManager::new(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())).snapshot()
    });
    core.protocol_engine = Some(
        ProtocolEngine::load_or_seed(
            storage,
            owner.public_key(),
            device,
            local_invite,
            seed_session_manager,
            seed_group_manager,
        )
        .expect("protocol engine"),
    );
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

fn stored_chat_ttl(core: &AppCore, chat_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let conn = conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT ttl_seconds FROM chat_message_ttls WHERE chat_id = ?1")
        .unwrap();
    let mut rows = stmt.query([chat_id]).unwrap();
    rows.next()
        .unwrap()
        .map(|row| row.get::<_, i64>(0).unwrap() as u64)
}

fn runtime_state_json(core: &AppCore, owner: &Keys, device: &Keys) -> serde_json::Value {
    let storage = crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    );
    let value = storage
        .get("appcore/protocol-engine-state-v1")
        .expect("read appcore protocol state")
        .expect("appcore protocol state exists");
    serde_json::from_str(&value).expect("runtime state json")
}

fn stored_pending_group_sender_key_message_count(
    core: &AppCore,
    owner: &Keys,
    device: &Keys,
) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_group_sender_key_messages")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn stored_pending_decrypted_delivery_count(core: &AppCore, owner: &Keys, device: &Keys) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_decrypted_deliveries")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn unknown_group_sender_key_outer_event(sender_event: &Keys) -> Event {
    use base64::Engine;

    let mut payload = Vec::new();
    payload.extend_from_slice(&7_u32.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&[42_u8; 32]);
    let content = base64::engine::general_purpose::STANDARD.encode(payload);
    EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), content)
        .sign_with_keys(sender_event)
        .expect("unknown group sender-key outer")
}

fn delivered_texts() -> &'static std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>> {
    static DELIVERED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>>,
    > = std::sync::OnceLock::new();
    DELIVERED.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn runtime_key(runtime: &NdrRuntime) -> usize {
    runtime as *const NdrRuntime as usize
}

fn deliver_published_events(from: &NdrRuntime, signer: &Keys, to: &NdrRuntime) {
    for event in drain_signed_events(from, signer) {
        deliver_event_to_runtime(to, event);
    }
}

fn deliver_runtime_effects(
    from: &NdrRuntime,
    signer: &Keys,
    effects: Vec<SessionManagerEvent>,
    to: &NdrRuntime,
) {
    apply_runtime_persist_effects(from, &effects);
    let events = signed_events_from_effects(effects, signer);
    for event in &events {
        deliver_event_to_runtime(to, event.clone());
    }
    for event in events {
        from.ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(from, &from.drain_events());
    }
}

fn accept_invite_and_deliver(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    invite: &Invite,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .accept_invite(invite, Some(inviter_pubkey))
        .expect("accept invite");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn deliver_event_to_runtime(to: &NdrRuntime, event: Event) {
    to.process_received_event(event);
    let effects = to.drain_events();
    apply_runtime_persist_effects(to, &effects);
    let mut messages = Vec::new();
    for effect in effects {
        if let SessionManagerEvent::DecryptedMessage { content, .. } = effect {
            messages.push(
                serde_json::from_str::<UnsignedEvent>(&content)
                    .ok()
                    .map(|event| event.content)
                    .unwrap_or(content),
            );
        }
    }
    if !messages.is_empty() {
        delivered_texts()
            .lock()
            .unwrap()
            .entry(runtime_key(to))
            .or_default()
            .extend(messages);
    }
}

fn apply_runtime_persist_effects(_runtime: &NdrRuntime, _effects: &[SessionManagerEvent]) {
    // Runtime persistence is internal. This helper keeps existing simulated
    // relay-delivery tests readable where they previously modeled app steps.
}

fn pending_events_with_kind(core: &AppCore, kind: u32) -> Vec<Event> {
    core.pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .filter(|event| event.kind.as_u16() as u32 == kind)
        .collect()
}

fn complete_first_contact(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .send_text(
            inviter_pubkey,
            "__ndr_first_contact_bootstrap__".to_string(),
            None,
        )
        .expect("first-contact bootstrap send");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn signed_events_from_effects(effects: Vec<SessionManagerEvent>, signer: &Keys) -> Vec<Event> {
    effects
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

fn drain_signed_events(runtime: &NdrRuntime, signer: &Keys) -> Vec<Event> {
    let mut effects = runtime.drain_events();
    if effects.is_empty() {
        runtime.reload_from_storage().expect("reload runtime");
        effects.extend(runtime.drain_events());
    }
    let mut seen = HashSet::new();
    let events = signed_events_from_effects(effects, signer)
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .collect::<Vec<_>>();
    for event in &events {
        runtime
            .ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(runtime, &runtime.drain_events());
    }
    events
}

fn serializable_key_pair_for_test(keys: &Keys) -> nostr_double_ratchet::SerializableKeyPair {
    nostr_double_ratchet::SerializableKeyPair {
        public_key: NdrDevicePubkey::from_bytes(keys.public_key().to_bytes()),
        private_key: keys.secret_key().to_secret_bytes(),
    }
}

fn compact_event_payload_for_apns_test(event: &Event) -> serde_json::Value {
    let mut value = serde_json::to_value(event).expect("event json");
    if let Some(object) = value.as_object_mut() {
        let header_tags = object
            .get("tags")
            .and_then(|tags| tags.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|tag| {
                tag.as_array()
                    .and_then(|items| items.first())
                    .and_then(|name| name.as_str())
                    == Some("header")
            })
            .collect();
        object.insert("tags".to_string(), serde_json::Value::Array(header_tags));
    }
    value
}

fn drain_text_messages(runtime: &NdrRuntime) -> Vec<String> {
    delivered_texts()
        .lock()
        .unwrap()
        .remove(&runtime_key(runtime))
        .unwrap_or_default()
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
