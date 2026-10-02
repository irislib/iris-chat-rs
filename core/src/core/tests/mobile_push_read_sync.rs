#[test]
fn mobile_push_marks_only_own_session_and_group_authors_as_background() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("push-own-background", &owner, &device);
    install_local_sibling_session_for_test(&mut core, &owner, &device, &sibling);
    let _event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(), &peer, "hello", 200,
    );
    core.create_group("Own group", &[]);
    let engine = core.protocol_engine.as_ref().unwrap();
    let own = engine.message_author_pubkeys_for_owner(owner.public_key());
    let peer_authors = engine.message_author_pubkeys_for_owner(peer.public_key());
    let group_authors = engine.group_sender_event_pubkeys_for_owner(owner.public_key());
    assert!(!own.is_empty() && !peer_authors.is_empty() && !group_authors.is_empty());
    let push = core.build_mobile_push_sync_snapshot();
    for author in own.into_iter().chain(group_authors) {
        assert!(push.message_author_pubkeys.contains(&author.to_hex()));
        assert!(push.background_message_author_pubkeys.contains(&author.to_hex()));
    }
    for author in peer_authors {
        assert!(push.message_author_pubkeys.contains(&author.to_hex()));
        assert!(!push.background_message_author_pubkeys.contains(&author.to_hex()));
    }
    core.preferences.accept_unknown_direct_messages = false;
    let push = core.build_mobile_push_sync_snapshot();
    assert!(!push.background_message_author_pubkeys.is_empty());
    let own_direct = core.protocol_engine.as_ref().unwrap().message_author_pubkeys_for_owner(owner.public_key());
    let relay_authors = core.subscribable_message_author_hexes();
    assert!(own_direct.iter().all(|key| relay_authors.contains(&key.to_hex())));
    assert!(push.background_message_author_pubkeys.iter().all(|key| push.message_author_pubkeys.contains(key)));
}

#[test]
fn mobile_push_read_sync_dismisses_only_authenticated_read_messages() {
    let mut pair = chat_read_receipt_pair("push-read-dismiss");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut payloads = Vec::new();
    let mut events = Vec::new();
    let mut ids = Vec::new();
    for (text, timestamp) in [("read", 200), ("same second unread", 200), ("newer unread", 201)] {
        let event = appcore_direct_message_event_for_test(
            pair.b.protocol_engine.as_mut().unwrap(), &peer, text, timestamp,
        );
        let (_, id) = runtime_rumor_json(peer.public_key(), CHAT_MESSAGE_KIND, text, timestamp, Vec::new());
        ids.push(id);
        payloads.push(serde_json::json!({"event": event, "iris_dismiss": true}).to_string());
        events.push(event);
    }
    let data_dir = pair._b_dir.path().to_string_lossy().to_string();
    let owner = pair.owner.public_key().to_hex();
    let device = pair.b_device.secret_key().to_secret_hex();
    let resolve = |payloads: Vec<String>| read_mobile_push_notification_indexes(
        data_dir.clone(), owner.clone(), device.clone(), payloads,
    );
    assert!(resolve(payloads.clone()).is_empty());
    // The read update can precede foreground message ingestion. Deliver the
    // actual encrypted sibling receipt through the background push entry point.
    chat_read_sync_incoming(&mut pair.a, &peer, &ids[0], 200);
    pair.a.mark_messages_seen(&chat_id, &[ids[0].clone()]);
    let receipts = pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND);
    assert!(!receipts.is_empty());
    let background = pair.b.build_mobile_push_sync_snapshot().background_message_author_pubkeys;
    assert!(receipts.iter().any(|event| background.contains(&event.pubkey.to_hex())));
    for receipt in receipts {
        pair.b.ingest_mobile_push_payload(&serde_json::json!({"event": receipt}).to_string());
    }
    assert_eq!(resolve(payloads.clone()), vec![0]);
    let mut forged = events[0].clone();
    forged.content.push_str("tampered");
    assert!(resolve(vec![serde_json::json!({"event": forged, "iris_dismiss": true}).to_string()]).is_empty());
    assert!(read_mobile_push_notification_indexes(
        data_dir.clone(), Keys::generate().public_key().to_hex(), device.clone(), payloads.clone(),
    ).is_empty());
    for event in events { pair.b.handle_relay_event(event); }
    pair.b.persist_best_effort();
    assert_eq!(resolve(payloads), vec![0]);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 2);
}

#[test]
fn mobile_push_read_sync_does_not_initialize_or_migrate_the_database() {
    let mut pair = chat_read_receipt_pair("push-read-no-migrations");
    let peer = Keys::generate();
    let event = appcore_direct_message_event_for_test(
        pair.b.protocol_engine.as_mut().unwrap(), &peer, "read", 200,
    );
    let (_, id) = runtime_rumor_json(peer.public_key(), CHAT_MESSAGE_KIND, "read", 200, Vec::new());
    chat_read_sync_incoming(&mut pair.b, &peer, &id, 200);
    pair.b.mark_messages_seen(&peer.public_key().to_hex(), &[id]);
    pair.b.persist_best_effort();
    let dir = pair._b_dir.path();
    let conn = rusqlite::Connection::open(dir.join("core.sqlite3")).unwrap();
    // An older schema marker makes a writable preview connection run migrations.
    // Cleanup must only read the existing data, including before message ingestion.
    conn.pragma_update(None, "user_version", 0).unwrap();
    assert_eq!(read_mobile_push_notification_indexes(
        dir.to_string_lossy().into_owned(), pair.owner.public_key().to_hex(),
        pair.b_device.secret_key().to_secret_hex(),
        vec![serde_json::json!({"event": event}).to_string()],
    ), vec![0]);
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, 0, "notification cleanup must never migrate the live database");
}
