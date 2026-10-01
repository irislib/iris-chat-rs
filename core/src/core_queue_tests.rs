use super::*;

fn background_msg(index: usize) -> CoreMsg {
    CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
        category: "test.background".to_string(),
        detail: index.to_string(),
    }))
}

#[test]
fn foreground_queue_preempts_background_backlog() {
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }
    foreground_tx
        .send(CoreMsg::Action(AppAction::NavigateBack))
        .unwrap();

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert!(matches!(
        batch.first(),
        Some(CoreMsg::Action(AppAction::NavigateBack))
    ));
    assert!(
        batch.iter().all(is_foreground_core_msg),
        "foreground work should not be bundled behind background backlog"
    );
}

#[test]
fn foreground_internal_preempts_background_backlog() {
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }
    foreground_tx
        .send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "test.priority".to_string(),
            detail: "priority".to_string(),
        })))
        .unwrap();

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert!(matches!(
        batch.first(),
        Some(CoreMsg::Internal(event))
            if matches!(
                event.as_ref(),
                InternalEvent::DebugLog { detail, .. } if detail == "priority"
            )
    ));
}

#[test]
fn background_queue_drains_in_bounded_chunks() {
    let (_foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert_eq!(batch.len(), CORE_BACKGROUND_BATCH_LIMIT);
    assert!(batch.iter().all(|msg| !is_foreground_core_msg(msg)));
}

#[test]
fn route_chat_snapshot_uses_chat_list_without_core_queue() {
    let state = build_large_test_app_state(80, 20, 1_200);
    let chat_id = state.chat_list[10].chat_id.clone();

    let snapshot =
        crate::core::chat_snapshot_from_state_and_db(&state, None, &chat_id, 80).unwrap();

    assert_eq!(snapshot.chat_id, chat_id);
    assert_eq!(snapshot.display_name, state.chat_list[10].display_name);
    assert!(snapshot.messages.is_empty());
}

#[test]
fn route_chat_snapshot_requires_account() {
    let mut state = build_large_test_app_state(80, 20, 1_200);
    state.account = None;
    let chat_id = state.chat_list[10].chat_id.clone();

    assert!(crate::core::chat_snapshot_from_state_and_db(&state, None, &chat_id, 80).is_none());
}

#[test]
fn ffi_chat_snapshot_bounds_active_history_without_losing_metadata() {
    let app = ffi_app_failure(String::new());
    let mut state = build_large_test_app_state(80, 20, 1_200);
    let current = state.current_chat.as_mut().unwrap();
    current.draft = "Keep this draft".to_string();
    current.message_ttl_seconds = Some(86_400);
    let chat_id = current.chat_id.clone();
    let mut expected = current.clone();
    expected.messages = expected.messages.split_off(expected.messages.len() - 3);
    *app.shared_state.write().unwrap() = state;

    let snapshot = app.chat_snapshot(chat_id.clone(), 3).unwrap();

    assert_eq!(
        snapshot.messages.len(),
        3,
        "first paint must respect its requested page size"
    );
    assert_eq!(snapshot, expected);
    let single = app.chat_snapshot(chat_id, 0).unwrap();
    assert_eq!(single.messages, expected.messages[2..]);
    assert_eq!(
        app.shared_state
            .read()
            .unwrap()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len(),
        1_200,
        "the bounded FFI read must leave the core's loaded history intact",
    );
}

#[test]
fn ffi_chat_pages_keep_participants_and_load_requested_database_range() {
    let app = ffi_app_failure(String::new());
    let directory = tempfile::tempdir().unwrap();
    let core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.path().to_string_lossy().to_string(),
        app.shared_state.clone(),
    );
    let mut state = build_large_test_app_state(80, 20, 1_200);
    let current = state.current_chat.as_mut().unwrap();
    let chat_id = current.chat_id.clone();
    let participants = vec![ChatParticipantSnapshot {
        social_connection: None,
        owner_pubkey_hex: chat_id.clone(),
        display_name: "Page author".to_string(),
        picture_url: None,
        is_local_owner: false,
    }];
    current.participants = participants.clone();
    let database = core.shared_db();
    {
        let connection = database.lock().unwrap();
        connection.execute(
            "INSERT INTO threads(chat_id, unread_count, updated_at_secs, draft) VALUES (?1, 0, 10, '')",
            [&chat_id],
        ).unwrap();
        for index in 0..6 {
            connection.execute(
                "INSERT INTO messages(chat_id, id, kind, author, body, is_outgoing, created_at_secs, delivery)
                 VALUES (?1, ?2, 'user', 'Original author', 'Stored message', 0, ?3, 'received')",
                rusqlite::params![chat_id, format!("stored-{index}"), index + 1],
            ).unwrap();
        }
    }
    *app.shared_db.write().unwrap() = Some(database);
    *app.shared_state.write().unwrap() = state;

    let before = app
        .chat_snapshot_before(chat_id.clone(), "stored-4".to_string(), 2)
        .unwrap();
    assert_eq!(
        before
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["stored-2", "stored-3"]
    );
    let around = app
        .chat_snapshot_around_message(chat_id, "stored-3".to_string(), 1, 1)
        .unwrap();
    assert_eq!(
        around
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["stored-2", "stored-3", "stored-4"]
    );
    for page in [before, around] {
        assert_eq!(page.participants, participants);
        assert!(page
            .messages
            .iter()
            .all(|message| message.author == "Page author"));
    }
}

#[test]
fn search_preserves_thread_results_with_large_active_history() {
    let app = ffi_app_failure(String::new());
    let mut state = build_large_test_app_state(80, 20, 1_200);
    state.chat_list[10].nickname = Some("Needle contact".to_string());
    state.chat_list[85].about = Some("Needle group".to_string());
    let expected_contact = state.chat_list[10].clone();
    let expected_group = state.chat_list[85].clone();
    let chat_id = expected_contact.chat_id.clone();
    *app.shared_state.write().unwrap() = state;

    let results = app.search("needle".to_string(), None, 80);

    assert_eq!(results.contacts, vec![expected_contact]);
    assert_eq!(results.groups, vec![expected_group]);
    assert!(results.people.is_empty());
    assert!(results.messages.is_empty());

    let scoped = app.search("needle".to_string(), Some(chat_id.clone()), 80);
    assert_eq!(scoped.scope_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(scoped.contacts.is_empty());
    assert!(scoped.groups.is_empty());
    assert_eq!(
        app.shared_state
            .read()
            .unwrap()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len(),
        1_200,
    );
}

#[test]
fn search_ranks_names_and_nicknames_before_other_fields_with_stable_ties() {
    let app = ffi_app_failure(String::new());
    let mut state = build_large_test_app_state(80, 20, 0);
    let template = state.chat_list[0].clone();
    state.chat_list.clear();
    for kind in [ChatKind::Direct, ChatKind::Group] {
        for (index, (name, nickname, profile_name, about)) in [
            ("Joanna", None, None, None),
            ("Second", Some("Family Anna"), None, None),
            ("Third", Some("Anna Smith"), None, None),
            ("Fourth", Some("Ann"), None, None),
            ("Fifth", None, Some("Rihanna"), None),
            ("Sixth", None, Some("O'Ann-Marie"), None),
            ("Seventh", None, None, Some("Ann")),
            ("Noélodie", None, None, None),
            ("Ninth", Some("Home/Élodie"), None, None),
            ("Tenth", Some("ÉLODIE"), None, None),
        ]
        .into_iter()
        .enumerate()
        {
            let mut chat = template.clone();
            chat.kind = kind.clone();
            chat.chat_id = format!("{kind:?}-{index}");
            chat.display_name = name.to_string();
            chat.nickname = nickname.map(str::to_string);
            chat.profile_name = profile_name.map(str::to_string);
            chat.about = about.map(str::to_string);
            chat.subtitle = None;
            chat.draft.clear();
            state.chat_list.push(chat);
        }
    }
    *app.shared_state.write().unwrap() = state;

    let results = app.search("  ANN  ".to_string(), None, 80);
    for (kind, chats) in [
        (ChatKind::Direct, results.contacts),
        (ChatKind::Group, results.groups),
    ] {
        assert_eq!(
            chats
                .iter()
                .map(|chat| chat.chat_id.clone())
                .collect::<Vec<_>>(),
            [3, 1, 2, 5, 0, 4, 6].map(|index| format!("{kind:?}-{index}")),
        );
    }

    let words = app.search("smi ann".to_string(), None, 80);
    assert_eq!(words.contacts.len(), 1);
    assert_eq!(words.contacts[0].nickname.as_deref(), Some("Anna Smith"));
    assert_eq!(words.groups.len(), 1);

    let unicode = app.search("élodie".to_string(), None, 80);
    assert_eq!(
        unicode
            .contacts
            .iter()
            .map(|chat| chat.chat_id.as_str())
            .collect::<Vec<_>>(),
        ["Direct-9", "Direct-8", "Direct-7"],
    );
}
