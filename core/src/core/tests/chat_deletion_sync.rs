fn deliver_sync_snapshot(sender: &AppCore, receiver: &mut AppCore, device: &Keys) {
    for packet in sender.build_device_sync_packets_for_test(100, true) {
        receiver.handle_device_sync_packet(&device.public_key().to_hex(), 7369, &packet);
    }
}

fn deletion_packet(chat_id: &str, deleted_at: u64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "type": "snapshot", "v": 1, "rosterAt": 100,
        "deletedChats": [{"id": chat_id, "deletedAt": deleted_at}]
    }))
    .unwrap()
}

#[test]
fn chat_deletion_sync_removes_sibling_history_and_survives_stale_snapshots() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut a, _, _a_dir) = logged_in_test_core_with_updates("delete-sync-a", &owner, &alice);
    let (mut b, _, _b_dir) = logged_in_test_core_with_updates("delete-sync-b", &owner, &bob);
    configure_test_device_sync_profile(&mut a, &owner, &alice, &bob, None);
    configure_test_device_sync_profile(&mut b, &owner, &bob, &alice, None);
    a.apply_runtime_text_message(
        peer.public_key(),
        None,
        "old history".into(),
        200,
        None,
        Some("old".into()),
        None,
    );
    deliver_sync_snapshot(&a, &mut b, &alice);
    assert!(b.threads.contains_key(&chat_id));
    let stale = b.build_device_sync_packets_for_test(100, true);
    b.active_chat_id = Some(chat_id.clone());
    b.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];

    a.handle_action(AppAction::DeleteChat {
        chat_id: chat_id.clone(),
    });
    deliver_sync_snapshot(&a, &mut b, &alice);

    assert!(
        !b.threads.contains_key(&chat_id),
        "deletion must reach the sibling"
    );
    assert!(b.active_chat_id.is_none());
    assert!(b.screen_stack.is_empty());
    assert!(!b
        .app_store
        .message_exists(&chat_id, Some("old"), None)
        .unwrap());
    for packet in stale {
        a.handle_device_sync_packet(&bob.public_key().to_hex(), 7369, &packet);
        b.handle_device_sync_packet(&alice.public_key().to_hex(), 7369, &packet);
    }
    assert!(
        !a.threads.contains_key(&chat_id),
        "stale sibling history cannot undo deletion"
    );
    assert!(!b.threads.contains_key(&chat_id));
}

#[test]
fn chat_deletion_sync_is_durable_and_offline_siblings_learn_it_after_restart() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut a, _, a_dir) = logged_in_test_core_with_updates("delete-restart-a", &owner, &alice);
    let (mut b, _, _b_dir) = logged_in_test_core_with_updates("delete-restart-b", &owner, &bob);
    configure_test_device_sync_profile(&mut a, &owner, &alice, &bob, None);
    configure_test_device_sync_profile(&mut b, &owner, &bob, &alice, None);
    a.apply_runtime_text_message(
        peer.public_key(),
        None,
        "old history".into(),
        200,
        None,
        Some("old".into()),
        None,
    );
    deliver_sync_snapshot(&a, &mut b, &alice);
    a.delete_chat(&chat_id);
    drop(a);
    let mut restarted = logged_in_test_core_at_data_dir(
        &owner,
        &alice,
        a_dir.path().to_string_lossy().into_owned(),
    );
    restarted.load_persisted().unwrap();
    configure_test_device_sync_profile(&mut restarted, &owner, &alice, &bob, None);

    deliver_sync_snapshot(&restarted, &mut b, &alice);
    assert!(
        !b.threads.contains_key(&chat_id),
        "offline sibling must learn persisted deletion"
    );
    deliver_sync_snapshot(&b, &mut restarted, &bob);
    assert!(!restarted.threads.contains_key(&chat_id));
}

#[test]
fn chat_deletion_sync_keeps_newer_messages_and_rejects_old_protocol_replays() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("delete-ordering", &owner, &alice);
    configure_test_device_sync_profile(&mut core, &owner, &alice, &bob, None);
    for (id, at) in [("old", 200), ("new", 400)] {
        core.apply_runtime_text_message(
            peer.public_key(),
            None,
            id.into(),
            at,
            None,
            Some(id.into()),
            None,
        );
    }
    core.persist_best_effort();
    let packet = deletion_packet(&chat_id, 300);
    core.handle_device_sync_packet(&bob.public_key().to_hex(), 7369, &packet);
    core.handle_device_sync_packet(&bob.public_key().to_hex(), 7369, &packet);
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        "delayed old".into(),
        250,
        None,
        Some("delayed".into()),
        None,
    );

    let ids = core.threads[&chat_id]
        .messages
        .iter()
        .map(|m| m.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["new"]);
    assert!(!core
        .app_store
        .message_exists(&chat_id, Some("old"), None)
        .unwrap());
    assert!(core
        .app_store
        .message_exists(&chat_id, Some("new"), None)
        .unwrap());
}

#[test]
fn chat_deletion_sync_rejects_unregistered_devices_and_future_deletions() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("delete-auth", &owner, &alice);
    configure_test_device_sync_profile(&mut core, &owner, &alice, &bob, None);
    core.ensure_thread_record(&chat_id, 200);
    core.handle_device_sync_packet(
        &peer.public_key().to_hex(),
        7369,
        &deletion_packet(&chat_id, 300),
    );
    core.handle_device_sync_packet(
        &bob.public_key().to_hex(),
        7369,
        &deletion_packet(&chat_id, u64::MAX),
    );
    assert!(core.threads.contains_key(&chat_id));
}

#[test]
fn chat_deletion_sync_allows_explicit_recreation_without_restoring_history() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut a, _, _a_dir) = logged_in_test_core_with_updates("delete-recreate-a", &owner, &alice);
    let (mut b, _, _b_dir) = logged_in_test_core_with_updates("delete-recreate-b", &owner, &bob);
    configure_test_device_sync_profile(&mut a, &owner, &alice, &bob, None);
    configure_test_device_sync_profile(&mut b, &owner, &bob, &alice, None);
    a.ensure_thread_record(&chat_id, 200);
    a.delete_chat(&chat_id);
    let deletion = a.build_device_sync_packets_for_test(100, false);

    a.create_chat(&chat_id);
    deliver_sync_snapshot(&a, &mut b, &alice);
    assert!(
        b.threads.contains_key(&chat_id),
        "explicitly recreated chat must sync"
    );
    for packet in deletion {
        b.handle_device_sync_packet(&alice.public_key().to_hex(), 7369, &packet);
    }
    assert!(
        b.threads.contains_key(&chat_id),
        "duplicate deletion must not undo recreation"
    );
    a.send_message(&chat_id, "new start", None);
    assert!(a.threads[&chat_id]
        .messages
        .iter()
        .all(|m| !a.chat_activity_is_deleted(&chat_id, m.created_at_secs)));
}

#[test]
fn chat_deletion_sync_does_not_restore_groups_from_stale_metadata() {
    let owner = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("delete-group", &owner, &alice);
    configure_test_device_sync_profile(&mut core, &owner, &alice, &bob, None);
    let packet = serde_json::to_vec(&serde_json::json!({
        "type": "snapshot", "v": 1, "rosterAt": 100,
        "groups": [{ "id": "friends", "name": "Friends", "createdBy": owner.public_key().to_hex(),
            "members": [owner.public_key().to_hex()], "admins": [owner.public_key().to_hex()],
            "revision": 1, "createdAt": 100, "updatedAt": 200 }]
    }))
    .unwrap();
    core.handle_device_sync_packet(&bob.public_key().to_hex(), 7369, &packet);
    let group = core.groups["friends"].clone();
    core.delete_chat("group:friends");
    core.handle_device_sync_packet(&bob.public_key().to_hex(), 7369, &packet);
    core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group));
    assert!(!core.threads.contains_key("group:friends"));
    assert!(!core.groups.contains_key("friends"));
    let restored = core.load_persisted().unwrap().unwrap();
    assert!(restored.groups.is_empty());
    assert!(restored.threads.is_empty());
}
