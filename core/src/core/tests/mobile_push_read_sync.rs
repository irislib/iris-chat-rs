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

#[test]
fn mobile_push_large_group_notification_cleanup_has_one_protocol_load_and_bounded_checkpoint_bytes()
{
    let mut pair = chat_read_receipt_pair("large-group-notification-work");
    let peers: Vec<_> = (0..100).map(|_| Keys::generate()).collect();
    pair.b.create_group(
        "Large group",
        &peers
            .iter()
            .map(|p| p.public_key().to_hex())
            .collect::<Vec<_>>(),
    );
    let mut payloads = Vec::new();
    let mut events = Vec::new();
    for (i, peer) in peers.iter().take(16).enumerate() {
        let body = format!("read notification {i}");
        let event = appcore_direct_message_event_for_test(
            pair.b.protocol_engine.as_mut().unwrap(),
            peer,
            &body,
            200,
        );
        let (_, id) =
            runtime_rumor_json(peer.public_key(), CHAT_MESSAGE_KIND, &body, 200, Vec::new());
        chat_read_sync_incoming(&mut pair.b, peer, &id, 200);
        pair.b
            .mark_messages_seen(&peer.public_key().to_hex(), &[id]);
        payloads.push(serde_json::json!({"event": event}).to_string());
        events.push(event);
    }
    pair.b.persist_best_effort();
    // Notification delivery can repeat or arrive out of order. The resolver must
    // preserve each index while sharing one read-only protocol preview.
    payloads.reverse();
    payloads.push(payloads[0].clone());
    let expected: Vec<u64> = (0..payloads.len() as u64).collect();
    let conn = rusqlite::Connection::open(pair._b_dir.path().join("core.sqlite3")).unwrap();
    let read_state = || {
        conn.query_row(
            "SELECT value FROM ndr_kv WHERE key = 'appcore/protocol-engine-state-v1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    };
    let before = read_state();
    super::mobile_push::PREVIEW_WORK.with(|work| work.set((0, 0)));
    let started = std::time::Instant::now();
    let indexes = read_mobile_push_notification_indexes(
        pair._b_dir.path().to_string_lossy().into_owned(),
        pair.owner.public_key().to_hex(),
        pair.b_device.secret_key().to_secret_hex(),
        payloads,
    );
    let (loads, checkpoint_bytes) = super::mobile_push::PREVIEW_WORK.with(|work| work.get());
    eprintln!("large-group notification cleanup: {:?}, loads={loads}, checkpoint_bytes={checkpoint_bytes}, state_bytes={}", started.elapsed(), before.len());
    assert_eq!(indexes, expected);
    assert_eq!(
        read_state(),
        before,
        "preview must leave the durable ratchet untouched"
    );
    assert_eq!(
        loads, 1,
        "a notification batch must not reload all groups and sessions for each alert"
    );
    assert!(
        checkpoint_bytes < before.len() * 2,
        "read-only preview must not serialize the entire protocol state for each alert"
    );
    // A real receive must persist its ratchet and delivery journal immediately,
    // but must not rewrite the unrelated multi-megabyte group/fanout history.
    let storage = Arc::new(super::storage::SqliteStorageAdapter::new(
        pair.b.app_store.shared(),
        pair.owner.public_key().to_hex(),
        pair.b_device.public_key().to_hex(),
    ));
    let mut engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        pair.owner.public_key(),
        &pair.b_device,
    )
    .unwrap();
    let shared = pair.b.app_store.shared();
    let sqlite_written_pages = |shared: &iris_chat_protocol::SharedConnection| {
        let conn = shared.lock().unwrap();
        let mut current = 0;
        let mut highwater = 0;
        // The locked connection owns the handle throughout this read-only
        // SQLite counter query; both output pointers refer to live integers.
        let status = unsafe {
            rusqlite::ffi::sqlite3_db_status(
                conn.handle(),
                rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,
                &mut current,
                &mut highwater,
                0,
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        current as u64
    };
    let page_size: u64 = shared
        .lock()
        .unwrap()
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .unwrap();
    let written_before = sqlite_written_pages(&shared);
    let received = engine
        .process_direct_message_event(&events[0])
        .unwrap()
        .expect("real foreground decryption");
    let written = (sqlite_written_pages(&shared) - written_before) * page_size;
    let after = read_state();
    let before_json: serde_json::Value = serde_json::from_str(&before).unwrap();
    let after_json: serde_json::Value = serde_json::from_str(&after).unwrap();
    let changed_fields: Vec<_> = before_json.as_object().unwrap().iter()
        .filter(|(key, value)| after_json.get(*key) != Some(*value))
        .map(|(key, _)| key.as_str()).collect();
    let changed_chunks = before.as_bytes().chunks(4096)
        .zip(after.as_bytes().chunks(4096)).enumerate()
        .filter(|(_, (a, b))| a != b).map(|(index, _)| index).collect::<Vec<_>>();
    eprintln!(
        "large-group foreground receive: sqlite_page_bytes_written={written}, state_bytes={}, after_bytes={}, changed_fields={changed_fields:?}, changed_chunk_count={}, first_changed_chunks={:?}",
        before.len(), after.len(), changed_chunks.len(), &changed_chunks[..changed_chunks.len().min(8)]
    );
    assert!(
        written < before.len() as u64 / 4,
        "one receive must not rewrite unrelated large-group state"
    );
    drop(engine);
    let reopened = Arc::new(std::sync::Mutex::new(
        rusqlite::Connection::open(pair._b_dir.path().join("core.sqlite3")).unwrap(),
    ));
    let storage = Arc::new(super::storage::SqliteStorageAdapter::new(
        reopened.clone(),
        pair.owner.public_key().to_hex(),
        pair.b_device.public_key().to_hex(),
    ));
    let mut restarted = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        pair.owner.public_key(),
        &pair.b_device,
    )
    .unwrap();
    let recovered = restarted
        .retry_pending_protocol(iris_chat_protocol::NdrUnixSeconds(unix_now().0))
        .unwrap();
    assert!(
        recovered
            .direct_messages
            .iter()
            .any(|message| message.event_id == received.event_id
                && message.content == received.content),
        "this unacknowledged decrypted delivery must survive a fresh database connection"
    );
    let ids = std::collections::HashSet::from([received.event_id.expect("durable event id")]);
    let written_before = sqlite_written_pages(&reopened);
    restarted.ack_decrypted_delivery_ids(&ids).unwrap();
    let written = (sqlite_written_pages(&reopened) - written_before) * page_size;
    eprintln!("large-group delivery acknowledgement: sqlite_page_bytes_written={written}");
    assert!(
        written < before.len() as u64 / 4,
        "delivery acknowledgement must not rewrite unrelated large-group state"
    );
    drop(restarted);
    let mut restarted = ProtocolEngine::load_or_create_for_local_device(
        storage,
        pair.owner.public_key(),
        &pair.b_device,
    )
    .unwrap();
    assert!(
        restarted
            .retry_pending_protocol(iris_chat_protocol::NdrUnixSeconds(unix_now().0))
            .unwrap()
            .direct_messages
            .iter()
            .all(|message| !message.event_id.as_ref().is_some_and(|id| ids.contains(id))),
        "acknowledged delivery must stay acknowledged after restart"
    );
}
