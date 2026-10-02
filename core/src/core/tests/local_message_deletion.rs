#[test]
fn local_message_deletion_survives_restart_and_protocol_and_sibling_replay() {
    for grouped in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, directory) =
            logged_in_test_core_with_updates("local-delete-replay", &owner, &device);
        configure_test_device_sync_profile(&mut core, &owner, &device, &sibling, None);
        let chat_id = if grouped {
            let group = test_group_snapshot(
                "local-delete-group",
                "Delete group",
                owner.public_key(),
                vec![owner.public_key(), peer.public_key()],
                vec![owner.public_key()],
                1,
            );
            let id = group_chat_id(&group.group_id);
            core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group));
            id
        } else {
            peer.public_key().to_hex()
        };
        let now = unix_now().get();
        for (author, id) in [
            (owner.public_key(), "deleted-mine"),
            (peer.public_key(), "deleted-theirs"),
        ] {
            core.apply_runtime_text_message(
                author,
                Some(chat_id.clone()),
                "locally-deleted history".into(),
                now,
                None,
                Some(id.into()),
                None,
            );
        }
        core.persist_best_effort();
        let stale = core.build_device_sync_packets_for_test(100, true);
        core.active_chat_id = Some(chat_id.clone());
        for id in ["deleted-mine", "deleted-theirs"] {
            core.handle_action(AppAction::DeleteLocalMessage {
                chat_id: chat_id.clone(),
                message_id: id.into(),
            });
            assert!(!core.threads[&chat_id].messages.iter().any(|m| m.id == id));
            assert!(!core
                .app_store
                .message_exists(&chat_id, Some(id), None)
                .unwrap());
        }
        assert!(core
            .app_store
            .search_messages_fts("locally-deleted", Some(&chat_id), 10)
            .unwrap()
            .is_empty());
        drop(core);
        let mut core = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            directory.path().to_string_lossy().into_owned(),
        );
        core.load_persisted().unwrap();
        configure_test_device_sync_profile(&mut core, &owner, &device, &sibling, None);
        for (author, id) in [
            (owner.public_key(), "deleted-mine"),
            (peer.public_key(), "deleted-theirs"),
        ] {
            core.apply_runtime_text_message(
                author,
                Some(chat_id.clone()),
                "locally-deleted history".into(),
                now,
                None,
                Some(id.into()),
                None,
            );
        }
        for packet in stale {
            core.handle_device_sync_packet(&sibling.public_key().to_hex(), 7369, &packet);
        }
        assert!(
            core.threads.get(&chat_id).is_none_or(|thread| {
                thread
                    .messages
                    .iter()
                    .all(|m| !m.id.starts_with("deleted-"))
            }),
            "local deletion was undone by old protocol/sibling history"
        );
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat_id.clone()),
            "A new message still arrives".into(),
            now,
            None,
            Some("different-id".into()),
            None,
        );
        assert!(core.threads[&chat_id]
            .messages
            .iter()
            .any(|m| m.id == "different-id"));
        let unrelated_peer = Keys::generate();
        let unrelated_chat = unrelated_peer.public_key().to_hex();
        core.apply_runtime_text_message(
            unrelated_peer.public_key(),
            None,
            "Same ID in a different chat".into(),
            now,
            None,
            Some("deleted-mine".into()),
            None,
        );
        assert!(core.threads[&unrelated_chat]
            .messages
            .iter()
            .any(|m| m.id == "deleted-mine"));
    }
}

#[test]
fn local_message_deletion_removes_a_row_outside_the_loaded_page() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("local-delete-unloaded", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    for id in ["older", "latest"] {
        core.apply_runtime_text_message(
            peer.public_key(),
            None,
            id.into(),
            unix_now().get(),
            None,
            Some(id.into()),
            None,
        );
    }
    core.persist_best_effort();
    core.threads
        .get_mut(&chat_id)
        .unwrap()
        .messages
        .retain(|m| m.id == "latest");
    core.handle_action(AppAction::DeleteLocalMessage {
        chat_id: chat_id.clone(),
        message_id: "older".into(),
    });
    assert!(!core
        .app_store
        .message_exists(&chat_id, Some("older"), None)
        .unwrap());
    assert!(core
        .app_store
        .message_exists(&chat_id, Some("latest"), None)
        .unwrap());
}

#[test]
fn local_message_deletion_keeps_visible_message_when_sqlite_delete_fails() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("local-delete-rollback", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        "Keep on failure".into(),
        unix_now().get(),
        None,
        Some("kept".into()),
        None,
    );
    core.persist_best_effort();
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_user_delete BEFORE DELETE ON messages
         BEGIN SELECT RAISE(ABORT, 'injected delete failure'); END;",
        )
        .unwrap();
    core.handle_action(AppAction::DeleteLocalMessage {
        chat_id: chat_id.clone(),
        message_id: "kept".into(),
    });
    assert!(core.threads[&chat_id]
        .messages
        .iter()
        .any(|m| m.id == "kept"));
    assert!(core
        .app_store
        .message_exists(&chat_id, Some("kept"), None)
        .unwrap());
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_user_delete")
        .unwrap();
    // Internal physical removal (also used by queued-ID replacement) must not
    // create a deletion marker, and the failed user transaction left none.
    core.app_store.delete_message(&chat_id, "kept").unwrap();
    core.threads.get_mut(&chat_id).unwrap().messages.clear();
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        "Keep on failure".into(),
        unix_now().get(),
        None,
        Some("kept".into()),
        None,
    );
    assert!(core.threads[&chat_id]
        .messages
        .iter()
        .any(|m| m.id == "kept"));
    core.handle_action(AppAction::DeleteLocalMessage {
        chat_id: chat_id.clone(),
        message_id: "kept".into(),
    });
    assert!(!core
        .app_store
        .message_exists(&chat_id, Some("kept"), None)
        .unwrap());
}

#[test]
fn local_message_deletion_rejects_late_notification_preview_and_source_alias() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("local-delete-preview", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let outer_id = "ab".repeat(32);
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        "Deleted notification".into(),
        now,
        None,
        Some("canonical-id".into()),
        Some(outer_id.clone()),
    );
    core.persist_best_effort();
    let preview = core.threads[&chat_id]
        .messages
        .iter()
        .find(|m| m.id == "canonical-id")
        .unwrap()
        .clone();
    let stale_thread = core.threads[&chat_id].clone();
    core.handle_action(AppAction::DeleteLocalMessage {
        chat_id: chat_id.clone(),
        message_id: "canonical-id".into(),
    });
    core.app_store
        .save_message_receipts(
            &chat_id,
            std::slice::from_ref(&preview),
            Some(&stale_thread),
        )
        .unwrap();
    assert!(
        !core
            .app_store
            .message_exists(&chat_id, Some("canonical-id"), None)
            .unwrap(),
        "a delayed receipt save must not restore the deleted row"
    );
    core.app_store
        .upsert_notification_preview_message(&chat_id, 0, now, &preview)
        .unwrap();
    assert!(
        !core
            .app_store
            .message_exists(&chat_id, Some("canonical-id"), None)
            .unwrap(),
        "a stale notification preview must not restore the deleted row"
    );
    core.push_incoming_message_from(
        &chat_id,
        None,
        "Deleted notification".into(),
        now,
        None,
        None,
        Some(peer.public_key().to_hex()),
        Some(outer_id),
    );
    assert!(
        core.threads[&chat_id].messages.is_empty(),
        "outer-event replay must stay deleted even without the canonical inner ID"
    );
}
