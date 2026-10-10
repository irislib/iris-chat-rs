fn restart_block_history_core(owner: &Keys, device: &Keys, data_dir: String) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_session(
        owner.public_key(),
        Some(owner.clone()),
        device.clone(),
        true,
        true,
    )
    .unwrap();
    core
}

#[test]
fn private_block_keeps_existing_direct_history_including_same_second_and_late_sync() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let chat = Keys::generate().public_key().to_hex();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("block-keeps-history", &owner, &device);
    core.app_store.bind_account(owner.public_key()).unwrap();
    for (id, at) in [
        ("before", 50),
        ("same-second", 100),
        ("already-received", 150),
    ] {
        core.push_incoming_message_from(
            &chat,
            Some(id.into()),
            "preserveneedle".into(),
            at,
            None,
            None,
            Some(chat.clone()),
            None,
        );
    }
    core.active_chat_id = Some(chat.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat.clone(),
    }];
    core.threads.get_mut(&chat).unwrap().draft = "unsent draft".into();
    core.preferences.pinned_chat_ids.push(chat.clone());
    core.preferences.muted_chat_ids.push(chat.clone());
    core.rebuild_persist_and_emit_state();
    // Legacy events carry deletedAt=100. Receiving one must no longer turn
    // blocking into deletion. Previously received history stays readable even
    // if a linked device learns the block later or timestamps share one second.
    for (revision, at, blocked) in [(1, 100, true), (2, 200, false)] {
        assert!(core.apply_private_block_event(signed_block_transition(
            &owner, &device, &chat, revision, at, 100, blocked
        )));
        core.rebuild_persist_and_emit_state();
        assert!(!core.chat_deletions.contains_key(&chat));
        assert_eq!(core.state.current_chat.as_ref().unwrap().messages.len(), 3);
        assert!(core.state.chat_list.iter().any(|item| item.chat_id == chat));
        assert_eq!(
            core.state.current_chat.as_ref().unwrap().draft,
            "unsent draft"
        );
        assert!(core.preferences.pinned_chat_ids.contains(&chat));
        assert!(core.preferences.muted_chat_ids.contains(&chat));
        for scope in [None, Some(chat.as_str())] {
            assert_eq!(
                core.app_store
                    .search_messages_fts("preserveneedle", scope, 10)
                    .unwrap()
                    .len(),
                3
            );
        }
        let mut state = core.state.clone();
        state.current_chat = None;
        let db = core.app_store.shared();
        let recent =
            crate::core::chat_snapshot_from_state_and_db(&state, Some(&db), &chat, 10).unwrap();
        assert_eq!(recent.messages.len(), 3);
        let older = crate::core::chat_snapshot_before_from_state_and_db(
            &state,
            Some(&db),
            &chat,
            "already-received",
            10,
        )
        .unwrap();
        assert_eq!(
            older
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["before", "same-second"]
        );
        let around = crate::core::chat_snapshot_around_message_from_state_and_db(
            &state,
            Some(&db),
            &chat,
            "same-second",
            1,
            1,
        )
        .unwrap();
        assert_eq!(around.messages.len(), 3);
    }
    core.persist_best_effort_inner();
    drop(core);
    let mut restarted =
        restart_block_history_core(&owner, &device, dir.path().to_string_lossy().into_owned());
    restarted.restore_device_sync_record_projection();
    restarted.rebuild_state();
    assert_eq!(restarted.threads[&chat].messages.len(), 3);
    assert!(!restarted.chat_deletions.contains_key(&chat));
}

#[test]
fn private_block_sync_keeps_earlier_history_and_explicit_delete_stays_separate() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    let (mut left, _, _left_dir) = logged_in_test_core_with_updates("block-keep-left", &owner, &a);
    let (mut right, _, right_dir) =
        logged_in_test_core_with_updates("block-keep-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    configure_test_device_sync_profile(&mut right, &owner, &b, &a, None);
    let base = unix_now().get() - 1000;
    for (id, at) in [("old", base + 50), ("unseen-blocked", base + 150)] {
        left.apply_runtime_text_message(
            peer.public_key(),
            None,
            id.into(),
            at,
            None,
            Some(id.into()),
            None,
        );
    }
    let blocked = signed_block_transition(&owner, &a, &chat, 1, base + 100, base + 100, true);
    assert!(left.apply_private_block_event(blocked.clone()));
    assert!(right.apply_private_block_event(blocked));
    deliver_sync_snapshot(&left, &mut right, &a);
    assert_eq!(
        left.threads[&chat].messages.len(),
        2,
        "already received history is retained"
    );
    assert_eq!(right.threads[&chat].messages.len(), 1);
    assert_eq!(
        right.threads[&chat].messages[0].id, "old",
        "blocking must allow pre-block history sync"
    );
    assert!(right.chat_deletions.is_empty());
    let stale = left.build_device_sync_packets_for_test(100, true);
    left.delete_chat(&chat);
    deliver_sync_snapshot(&left, &mut right, &a);
    assert!(right.chat_activity_is_deleted(&chat, base + 50));
    assert!(!right.threads.contains_key(&chat));
    right.set_user_blocked(&chat, false);
    for packet in &stale {
        right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, packet);
    }
    assert!(!right.threads.contains_key(&chat));
    right.persist_best_effort_inner();
    drop(right);
    let mut restarted =
        restart_block_history_core(&owner, &b, right_dir.path().to_string_lossy().into_owned());
    restarted.restore_device_sync_record_projection();
    assert!(!restarted.threads.contains_key(&chat));
    assert!(restarted.chat_activity_is_deleted(&chat, base + 50));
}
