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
    set_shared_db(&app.shared_db, Some(database));
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

fn chat_page_database_fixture(
    count: usize,
) -> (
    Arc<FfiApp>,
    crate::core::SharedConnection,
    tempfile::TempDir,
    String,
) {
    let directory = tempfile::tempdir().unwrap();
    let app = ffi_app_failure("isolated chat-page fixture".into());
    let core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.path().to_string_lossy().to_string(),
        app.shared_state.clone(),
    );
    let database = core.shared_db();
    let mut state = build_large_test_app_state(1, 0, 0);
    let chat_id = nostr::Keys::generate().public_key().to_hex();
    state.chat_list[0].chat_id = chat_id.clone();
    state.current_chat = None;
    {
        let conn = database.lock().unwrap();
        conn.execute(
            "INSERT INTO threads(chat_id, unread_count, updated_at_secs, draft) VALUES (?1, 0, 100, '')",
            [&chat_id],
        ).unwrap();
        for index in 0..count {
            conn.execute(
                "INSERT INTO messages(chat_id, id, kind, author, body, is_outgoing, created_at_secs, delivery)
                 VALUES (?1, ?2, 'user', 'Author', 'Saved message', 0, ?3, 'received')",
                rusqlite::params![chat_id, format!("page-{index}"), index as i64 + 1],
            ).unwrap();
        }
    }
    *app.shared_state.write().unwrap() = state;
    set_shared_db(&app.shared_db, Some(database.clone()));
    (app, database, directory, chat_id)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn ffi_chat_page_reads_history_and_file_status_while_writer_is_busy() {
    use nostr::JsonUtil;
    let (app, database, _directory, chat_id) = chat_page_database_fixture(120);
    let account = app.shared_state.read().unwrap().account.clone().unwrap();
    let device = nostr::Keys::generate();
    let offer = serde_json::json!({
        "id": "ab".repeat(16), "token": "cd".repeat(32), "owner": chat_id,
        "recipient": account.public_key_hex, "device": device.public_key().to_hex(),
        "caption": "Saved file", "expires_at_secs": nostr::Timestamp::now().as_secs() + 3600,
        "files": [{"filename": "notes.txt", "size_bytes": 4, "sha256": "ef".repeat(32)}]
    });
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(21111), offer.to_string())
        .sign_with_keys(&device)
        .unwrap();
    let wire = format!("iris-direct-file-v1:{}", event.as_json());
    {
        let conn = database.lock().unwrap();
        conn.execute("UPDATE messages SET body=?1 WHERE id='page-119'", [&wire])
            .unwrap();
        let record = serde_json::json!({
            "chat_id": chat_id, "wire": wire, "offer": offer,
            "is_sender": false, "status": DirectFileTransferStatus::Completed,
            "paths": [], "peer": null, "transferred": 4, "error": null
        });
        conn.execute(
            "INSERT INTO direct_file_transfers(id, record_json) VALUES (?1, ?2)",
            rusqlite::params!["ab".repeat(16), record.to_string()],
        )
        .unwrap();
    }
    let writer = database.lock().unwrap();
    writer
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE messages SET body='Uncommitted edit' WHERE id='page-118';",
        )
        .unwrap();
    writer.execute(
        "INSERT INTO ndr_kv(owner_pubkey_hex, device_pubkey_hex, key, value) VALUES ('reader-test', 'reader-test', 'large-checkpoint', ?1)",
        ["x".repeat(69 * 1024 * 1024)],
    ).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader_app = app.clone();
    let reader_chat = chat_id.clone();
    let reader = std::thread::spawn(move || {
        let mut samples_ms = Vec::new();
        let mut page = None;
        for _ in 0..10 {
            let started = std::time::Instant::now();
            page = reader_app.chat_snapshot(reader_chat.clone(), 80);
            samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        tx.send((page, samples_ms)).unwrap();
    });
    let result = rx.recv_timeout(Duration::from_secs(2));
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    reader.join().unwrap();
    let (page, mut samples_ms) = result.expect("history must not wait for the writer mutex");
    let page = page.unwrap();
    samples_ms.sort_by(f64::total_cmp);
    eprintln!(
        "chat_page_reader: synthetic_checkpoint_bytes={} samples={} median_ms={:.3} max_ms={:.3}",
        69 * 1024 * 1024,
        samples_ms.len(),
        samples_ms[samples_ms.len() / 2],
        samples_ms.last().unwrap()
    );
    assert_eq!(
        page.messages.len(),
        80,
        "a busy writer must not look like exhausted history"
    );
    assert_eq!(page.messages[0].id, "page-40");
    assert_eq!(page.messages[78].body, "Saved message");
    let file = page.messages.last().unwrap();
    assert_eq!(file.body, "Saved file");
    assert_eq!(
        file.direct_transfer.as_ref().unwrap().status,
        DirectFileTransferStatus::Completed
    );
    let older = app
        .chat_snapshot_before(chat_id, page.messages[0].id.clone(), 80)
        .unwrap();
    assert_eq!(older.messages.len(), 40);
}

#[test]
fn chat_page_busy_shared_connection_is_unavailable_not_empty() {
    let (app, database, _directory, chat_id) = chat_page_database_fixture(1);
    let state = app.shared_state.read().unwrap().clone();
    let _writer = database.lock().unwrap();
    assert!(
        crate::core::chat_snapshot_from_state_and_db(&state, Some(&database), &chat_id, 80,)
            .is_none(),
        "temporary contention cannot prove that a chat has no history"
    );
}

#[test]
fn ffi_chat_page_distinguishes_empty_history_from_query_failure() {
    let (app, database, _directory, chat_id) = chat_page_database_fixture(0);
    let empty = app.chat_snapshot(chat_id.clone(), 80).unwrap();
    assert!(empty.messages.is_empty());
    database
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE messages")
        .unwrap();
    assert!(
        app.chat_snapshot(chat_id, 80).is_none(),
        "query failure cannot mark pagination exhausted"
    );
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn ffi_chat_page_reader_is_query_only_and_does_not_change_journaling() {
    let (app, database, _directory, _chat_id) = chat_page_database_fixture(0);
    let slot = app.shared_db_read();
    let reader = slot.as_ref().unwrap().chat_reader().unwrap();
    assert!(!Arc::ptr_eq(&reader, &database));
    let connection = reader.lock().unwrap();
    assert!(connection
        .pragma_query_value(None, "query_only", |row| row.get::<_, bool>(0))
        .unwrap());
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert!(connection.execute("DELETE FROM messages", []).is_err());
    assert!(connection
        .execute_batch("CREATE TABLE unexpected_write(id)")
        .is_err());
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
