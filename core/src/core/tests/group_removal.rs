fn removed_group_fixture() -> (AppCore, Keys, Keys, GroupSnapshot, GroupSnapshot) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let admin = Keys::generate().public_key();
    let mut core = logged_in_test_core("removed-group", &owner, &device);
    let joined = test_group_snapshot(
        "removed-group",
        "Friends",
        admin,
        vec![admin, owner.public_key()],
        vec![admin],
        1,
    );
    let mut removed = joined.clone();
    removed
        .members
        .retain(|member| *member != ndr_owner_pubkey(owner.public_key()));
    removed.revision = 2;
    removed.updated_at = NdrUnixSeconds(2);
    core.protocol_engine
        .as_mut()
        .unwrap()
        .install_device_sync_group(joined.clone())
        .unwrap();
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(joined.clone()));
    (core, owner, device, joined, removed)
}

#[test]
fn group_removal_retains_history_notifies_once_and_blocks_sends_before_clearing_draft() {
    let (mut core, _owner, _device, joined, removed) = removed_group_fixture();
    let chat_id = group_chat_id(&joined.group_id);
    core.push_outgoing_message_with_id(
        "before-removal".into(),
        &chat_id,
        "Keep this history".into(),
        1,
        None,
        DeliveryState::Seen,
    );
    core.active_chat_id = Some(chat_id.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];
    core.threads.get_mut(&chat_id).unwrap().draft = "Unsent draft".into();
    core.protocol_engine
        .as_mut()
        .unwrap()
        .install_device_sync_group(removed.clone())
        .unwrap();
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(removed.clone()));
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(removed));
    core.rebuild_persist_and_emit_state();
    let chat = core.state.current_chat.as_ref().unwrap();
    assert!(chat.messages.iter().any(|m| m.body == "Keep this history"));
    assert_eq!(
        chat.messages
            .iter()
            .filter(|m| m.body == "You were removed from the group")
            .count(),
        1
    );
    assert!(!chat.participants.iter().any(|p| p.is_local_owner));
    core.send_message(&chat_id, "Do not send", None);
    assert_eq!(core.threads[&chat_id].draft, "Unsent draft");
    assert!(!core.threads[&chat_id]
        .messages
        .iter()
        .any(|m| m.body == "Do not send"));
    assert_eq!(
        core.state.toast.as_deref(),
        Some("You’re no longer in this group.")
    );
    core.send_attachment(
        &chat_id,
        "/missing/group-removal-fixture",
        "file.txt",
        "Blocked upload",
    );
    assert_eq!(
        core.state.toast.as_deref(),
        Some("You’re no longer in this group.")
    );
    assert!(!core.state.busy.uploading_attachment);
    core.handle_attachment_upload_finished(
        chat_id.clone(),
        Ok("Upload finished after removal".into()),
    );
    assert!(!core.threads[&chat_id]
        .messages
        .iter()
        .any(|m| m.body == "Upload finished after removal"));
    core.toggle_reaction(&chat_id, "before-removal", "👍");
    assert!(core.threads[&chat_id]
        .messages
        .iter()
        .find(|m| m.id == "before-removal")
        .unwrap()
        .reactions
        .is_empty());
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(joined.clone()));
    core.rebuild_state();
    assert!(!core
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .participants
        .iter()
        .any(|p| p.is_local_owner));
    let mut stale_restore = joined;
    stale_restore.revision = 2;
    stale_restore.updated_at = NdrUnixSeconds(3);
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(stale_restore.clone()));
    assert!(
        core.is_removed_from_group(&chat_id),
        "a later timestamp without a newer membership revision cannot restore access"
    );
    stale_restore.revision = 3;
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(stale_restore));
    core.rebuild_state();
    assert!(
        core.state
            .current_chat
            .as_ref()
            .unwrap()
            .participants
            .iter()
            .any(|p| p.is_local_owner),
        "a newer roster can explicitly add the user back"
    );
}

#[test]
fn group_removal_discards_durable_message_outbox_but_keeps_membership_controls() {
    let (mut core, owner, device, joined, removed) = removed_group_fixture();
    let chat_id = group_chat_id(&joined.group_id);
    for (content, message_id) in [("message", Some("message-id".into())), ("membership", None)] {
        let event = EventBuilder::new(Kind::Custom(1060), content)
            .sign_with_keys(&device)
            .unwrap();
        assert!(core.publish_protocol_event(ProtocolPublish {
            event,
            chat_id: chat_id.clone(),
            inner_event_id: message_id
        }));
    }
    assert_eq!(core.pending_relay_publishes.len(), 2);
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(removed));
    assert_eq!(core.pending_relay_publishes.len(), 1);
    let saved = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert!(
        saved[0].inner_event_id.is_none(),
        "removal notices must still reach linked devices"
    );
    let event = EventBuilder::new(Kind::Custom(1060), "late prepared message")
        .sign_with_keys(&device)
        .unwrap();
    assert!(!core.publish_protocol_event(ProtocolPublish {
        event,
        chat_id,
        inner_event_id: Some("late-message".into())
    }));
    assert_eq!(core.pending_relay_publishes.len(), 1);
}

#[test]
fn group_removal_from_authenticated_sibling_retains_history_and_rejects_stale_restore() {
    let (mut core, owner, device, joined, removed) = removed_group_fixture();
    let sibling = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex.clone(),
            created_at_secs: 1,
            devices: [&device, &sibling]
                .into_iter()
                .map(|key| KnownAppKeyDevice {
                    identity_pubkey_hex: key.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                })
                .collect(),
        },
    );
    let chat_id = group_chat_id(&joined.group_id);
    core.push_outgoing_message_with_id(
        "history".into(),
        &chat_id,
        "Still here".into(),
        1,
        None,
        DeliveryState::Seen,
    );
    let packet = |group: &GroupSnapshot| {
        serde_json::to_vec(&serde_json::json!({
        "type": "snapshot", "v": 1, "rosterAt": 1, "chats": [], "messages": [],
        "groups": [{ "id": group.group_id, "name": group.name,
            "createdBy": group.created_by.to_string(),
            "members": group.members.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "admins": group.admins.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "revision": group.revision, "createdAt": group.created_at.get(), "updatedAt": group.updated_at.get()
        }],
    })).unwrap()
    };
    core.handle_device_sync_packet(
        &Keys::generate().public_key().to_hex(),
        7369,
        &packet(&removed),
    );
    assert!(
        !core.is_removed_from_group(&chat_id),
        "unregistered devices cannot change membership"
    );
    core.handle_device_sync_packet(&sibling_hex, 7369, &packet(&removed));
    assert!(core.is_removed_from_group(&chat_id));
    assert!(core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.body == "Still here"));
    assert_eq!(
        core.threads[&chat_id]
            .messages
            .iter()
            .filter(|message| message.body == "You were removed from the group")
            .count(),
        1
    );
    core.handle_device_sync_packet(&sibling_hex, 7369, &packet(&joined));
    assert!(
        core.is_removed_from_group(&chat_id),
        "older linked devices cannot restore removed membership"
    );
    core.send_message(&chat_id, "blocked sibling send", None);
    assert_eq!(
        core.state.toast.as_deref(),
        Some("You’re no longer in this group.")
    );
}
