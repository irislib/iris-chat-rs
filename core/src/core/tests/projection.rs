use super::*;
use crate::core::tests::logged_in_test_core_with_updates;

#[test]
fn background_diagnostics_do_not_republish_history_and_foreground_refreshes_them() {
    let (update_tx, update_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let data_dir = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        data_dir.path().to_string_lossy().into_owned(),
        shared_state,
    );
    core.state = crate::build_large_test_app_state(16, 8, 200);
    core.state.network_status = Some(core.build_network_status_snapshot());
    core.emit_state();
    update_rx.try_recv().unwrap();
    core.handle_action(AppAction::AppBackgrounded);
    for index in 1..=20 {
        let network = core.state.network_status.as_mut().unwrap();
        network.recent_event_count = index;
        network.recent_log_count = index;
        network.last_debug_category = Some("network".into());
        network.last_debug_detail = Some(format!("status {index}"));
        core.emit_state();
    }
    assert!(
        update_rx.try_recv().is_err(),
        "debug-only changes must not serialize chat history while hidden"
    );
    core.handle_action(AppAction::AppForegrounded);
    let AppUpdate::FullState(latest) = update_rx.try_recv().unwrap() else {
        panic!("foreground snapshot");
    };
    assert_eq!(latest.network_status.unwrap().recent_event_count, 20);
    assert!(update_rx.try_recv().is_err());

    core.handle_action(AppAction::AppBackgrounded);
    core.state
        .network_status
        .as_mut()
        .unwrap()
        .connected_relay_count += 1;
    core.emit_state();
    assert!(
        matches!(update_rx.try_recv(), Ok(AppUpdate::FullState(_))),
        "connection changes remain immediate"
    );
    core.state.current_chat.as_mut().unwrap().messages[0].body = "Changed while hidden".into();
    core.emit_state();
    assert!(
        matches!(update_rx.try_recv(), Ok(AppUpdate::FullState(_))),
        "messages remain immediate"
    );
}

#[test]
fn state_publication_preserves_large_history_and_suppresses_duplicates() {
    let (update_tx, update_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let data_dir = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        data_dir.path().to_string_lossy().into_owned(),
        shared_state.clone(),
    );
    core.state = crate::build_large_test_app_state(80, 20, 1_200);

    core.emit_state();
    let AppUpdate::FullState(first) = update_rx.try_recv().unwrap() else {
        panic!("expected full state");
    };
    assert_eq!(first, core.state);
    assert_eq!(*shared_state.read().unwrap(), first);

    core.emit_state();
    assert!(update_rx.try_recv().is_err());
    assert_eq!(core.state.rev, first.rev);

    core.state.current_chat.as_mut().unwrap().messages[0].body = "Edited".to_string();
    core.emit_state();
    let AppUpdate::FullState(edited) = update_rx.try_recv().unwrap() else {
        panic!("expected changed full state");
    };
    assert_eq!(edited.rev, first.rev + 1);
    assert_eq!(edited, core.state);
    assert_eq!(*shared_state.read().unwrap(), edited);
    assert_ne!(edited.current_chat, first.current_chat);
}

#[test]
fn blocked_group_history_hiding_is_optional_reversible_and_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let blocked = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("blocked-group-history", &owner, &device);
    let chat = group_chat_id("history-visibility");
    let mut fixture = crate::build_large_test_app_state(1, 1, 1)
        .current_chat
        .unwrap()
        .messages
        .remove(0);
    fixture.chat_id = chat.clone();
    fixture.is_outgoing = false;
    fixture.kind = ChatMessageKind::User;
    fixture.author_owner_pubkey_hex = Some(other);
    fixture.id = "visible-message".into();
    fixture.body = "searchneedle visible history".into();
    fixture.created_at_secs = 10;
    let mut hidden = fixture.clone();
    hidden.id = "blocked-message".into();
    hidden.body = "searchneedle blocked history".into();
    hidden.author_owner_pubkey_hex = Some(blocked.clone());
    hidden.created_at_secs = 20;
    let mut notice = hidden.clone();
    notice.id = "membership-notice".into();
    notice.kind = ChatMessageKind::System;
    notice.body = "Membership changed".into();
    notice.created_at_secs = 5;
    core.preferences.blocked_owner_pubkeys.push(blocked);
    core.active_chat_id = Some(chat.clone());
    core.threads.insert(
        chat.clone(),
        ThreadRecord {
            chat_id: chat.clone(),
            unread_count: 0,
            updated_at_secs: 20,
            messages: vec![notice, fixture, hidden],
            draft: String::new(),
        },
    );
    core.rebuild_persist_and_emit_state();
    assert!(!core.preferences.hide_blocked_group_messages);
    assert_eq!(core.state.current_chat.as_ref().unwrap().messages.len(), 3);
    assert_eq!(
        core.state.chat_list[0].last_message_preview.as_deref(),
        Some("searchneedle blocked history")
    );

    core.handle_action(AppAction::SetHideBlockedGroupMessages { enabled: true });
    let visible = &core.state.current_chat.as_ref().unwrap().messages;
    assert_eq!(
        visible
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["membership-notice", "visible-message"]
    );
    assert_eq!(
        core.state.chat_list[0].last_message_preview.as_deref(),
        Some("searchneedle visible history")
    );
    for scope in [None, Some(chat.as_str())] {
        let hits = core
            .app_store
            .search_messages_fts("searchneedle", scope, 1)
            .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "hidden results must not consume the search limit"
        );
        assert_eq!(hits[0].message_id, "visible-message");
    }
    let mut page_state = core.state.clone();
    page_state.current_chat = None;
    let shared = core.app_store.shared();
    let pages = [
        crate::core::chat_snapshot_from_state_and_db(&page_state, Some(&shared), &chat, 1),
        crate::core::chat_snapshot_before_from_state_and_db(
            &page_state,
            Some(&shared),
            &chat,
            "blocked-message",
            1,
        ),
        crate::core::chat_snapshot_around_message_from_state_and_db(
            &page_state,
            Some(&shared),
            &chat,
            "blocked-message",
            1,
            1,
        ),
    ];
    for page in pages {
        let messages = page.expect("visible history page").messages;
        assert_eq!(
            messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["visible-message"],
            "hidden messages must not reappear in paginated or search-jump reads"
        );
    }
    assert_eq!(
        core.app_store
            .load_recent_messages(&chat, 10)
            .unwrap()
            .len(),
        3,
        "private sync must retain access to unfiltered saved history"
    );
    assert_eq!(
        core.threads[&chat].messages.len(),
        3,
        "hiding never deletes saved history"
    );
    let mut reopened = AppStore::new(open_database(directory.path()).unwrap());
    assert!(
        reopened
            .load_preferences_snapshot()
            .unwrap()
            .unwrap()
            .hide_blocked_group_messages
    );

    core.handle_action(AppAction::SetHideBlockedGroupMessages { enabled: false });
    assert_eq!(core.state.current_chat.as_ref().unwrap().messages.len(), 3);
    assert_eq!(
        core.app_store
            .search_messages_fts("searchneedle", Some(&chat), 1)
            .unwrap()[0]
            .message_id,
        "blocked-message"
    );
    core.handle_action(AppAction::SetHideBlockedGroupMessages { enabled: true });
    core.preferences.blocked_owner_pubkeys.clear();
    core.rebuild_state();
    assert_eq!(
        core.state.current_chat.as_ref().unwrap().messages.len(),
        3,
        "unblocking restores hidden group history"
    );
}

#[test]
fn blocked_people_remain_manageable_after_direct_chat_deletion() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("blocked-people-settings", &owner, &device);
    core.ensure_thread_record(&chat, 1);
    let event = EventBuilder::new(Kind::Metadata, r#"{"name":"Ada"}"#)
        .sign_with_keys(&peer)
        .unwrap();
    assert!(core.apply_profile_metadata_event(&event));
    core.set_user_blocked(&chat, true);
    core.delete_chat(&chat);
    core.rebuild_persist_and_emit_state();
    assert!(core
        .state
        .chat_list
        .iter()
        .all(|thread| thread.chat_id != chat));
    assert_eq!(core.state.blocked_people.len(), 1);
    let person = &core.state.blocked_people[0];
    assert_eq!(person.display_label, "Ada");
    assert_eq!(person.owner_pubkey_hex, chat);
    assert_eq!(person.user_id, peer.public_key().to_bech32().unwrap());
    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: chat.clone(),
        blocked: false,
    });
    assert!(core.state.blocked_people.is_empty());
    assert!(!core.preferences.blocked_owner_pubkeys.contains(&chat));
}

#[test]
fn late_blocked_intervals_invalidate_and_filter_all_visible_history_reads() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("late-block-visibility", &owner, &device);
    core.app_store.bind_account(owner.public_key()).unwrap();
    let chat = "group:late-block";
    for (id, time) in [("visible", 50), ("blocked", 150)] {
        core.push_incoming_message_from(
            chat,
            Some(id.into()),
            "intervalneedle".into(),
            time,
            None,
            None,
            Some(target.clone()),
            None,
        );
    }
    core.active_chat_id = Some(chat.into());
    core.rebuild_persist_and_emit_state();
    let revision = core.state.message_visibility_revision;
    assert_eq!(core.state.current_chat.as_ref().unwrap().messages.len(), 2);
    // Receiving an old completed interval changes visibility while this person
    // remains unblocked and the optional old-history setting remains disabled.
    let event = EventBuilder::new(
        Kind::Custom(super::super::block_sync::BLOCK_CONTROL_KIND as u16),
        serde_json::json!({"v":1,"owner":owner.public_key().to_hex(),"target":target,
            "blocked":false,"revision":2,"blockedSince":100,"deletedAt":100})
        .to_string(),
    )
    .tag(nostr::Tag::public_key(owner.public_key()))
    .tag(nostr::Tag::identifier(format!("iris:block:{target}")))
    .custom_created_at(Timestamp::from_secs(200))
    .allow_self_tagging()
    .sign_with_keys(&device)
    .unwrap();
    assert!(core.apply_private_block_event(event));
    core.rebuild_persist_and_emit_state();
    assert_ne!(core.state.message_visibility_revision, revision);
    assert!(!core.preferences.hide_blocked_group_messages);
    assert!(core.state.blocked_people.is_empty());
    assert_eq!(core.state.current_chat.as_ref().unwrap().messages.len(), 1);
    assert_eq!(
        core.state.current_chat.as_ref().unwrap().messages[0].id,
        "visible"
    );
    for scope in [None, Some(chat)] {
        let hits = core
            .app_store
            .search_messages_fts("intervalneedle", scope, 1)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "visible");
    }
    let mut state = core.state.clone();
    state.current_chat = None;
    let shared = core.app_store.shared();
    for page in [
        crate::core::chat_snapshot_from_state_and_db(&state, Some(&shared), chat, 1),
        crate::core::chat_snapshot_before_from_state_and_db(
            &state,
            Some(&shared),
            chat,
            "blocked",
            1,
        ),
        crate::core::chat_snapshot_around_message_from_state_and_db(
            &state,
            Some(&shared),
            chat,
            "blocked",
            1,
            1,
        ),
    ] {
        assert_eq!(
            page.unwrap()
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["visible"]
        );
    }
    assert_eq!(
        core.app_store.load_recent_messages(chat, 10).unwrap().len(),
        2
    );
    core.rebuild_state();
    assert_eq!(
        core.state.message_visibility_revision,
        state.message_visibility_revision
    );
}
