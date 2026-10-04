fn receive_test_mutation(
    core: &mut AppCore,
    author: &Keys,
    chat: &str,
    target: &str,
    kind: u32,
    body: &str,
    millis: u64,
) -> String {
    let mut tags = vec![
        nostr::Tag::parse(["e", target]).unwrap(),
        nostr::Tag::parse(["k", "14"]).unwrap(),
        nostr::Tag::parse(["ms", &millis.to_string()]).unwrap(),
    ];
    if let Some(group) = parse_group_id_from_chat_id(chat) {
        tags.push(nostr::Tag::parse(["l", &group]).unwrap());
    } else {
        tags.push(nostr::Tag::parse(["p", chat]).unwrap());
    }
    let mut event = UnsignedEvent::new(
        author.public_key(),
        Timestamp::from_secs(millis / 1000),
        Kind::Custom(kind as u16),
        tags,
        body,
    );
    event.ensure_id();
    assert!(core.apply_decrypted_runtime_message_with_metadata(
        author.public_key(),
        None,
        None,
        serde_json::to_string(&event).unwrap(),
        None,
        millis / 1000
    ));
    event.id.unwrap().to_hex()
}

#[test]
fn message_mutations_preserve_versions_authority_order_and_delete_durably() {
    for grouped in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let attacker = Keys::generate();
        let (mut core, _, directory) =
            logged_in_test_core_with_updates("mutation-history", &owner, &device);
        let chat = if grouped {
            let group = test_group_snapshot(
                "mutation-group",
                "History",
                owner.public_key(),
                vec![owner.public_key(), peer.public_key(), attacker.public_key()],
                vec![owner.public_key()],
                1,
            );
            core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(group));
            group_chat_id("mutation-group")
        } else {
            peer.public_key().to_hex()
        };
        let now = unix_now().get();
        let target = "immutable-original";
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "original secret".into(),
            now,
            None,
            Some(target.into()),
            None,
        );
        core.persist_best_effort();
        let unread = core.threads[&chat].unread_count;
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "second version",
            now * 1000 + 20,
        );
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "first version",
            now * 1000 + 10,
        );
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "second version",
            now * 1000 + 20,
        );
        let message = core.message_for_mutation(&chat, target).unwrap();
        assert_eq!(message.body, "second version");
        assert_eq!(message.created_at_secs, now);
        assert_eq!(
            message
                .edit_history
                .iter()
                .map(|v| v.body.as_str())
                .collect::<Vec<_>>(),
            vec!["original secret", "first version", "second version"]
        );
        assert_eq!(core.threads[&chat].unread_count, unread);
        receive_test_mutation(
            &mut core,
            &attacker,
            &chat,
            target,
            MESSAGE_DELETE_KIND,
            "",
            now * 1000 + 30,
        );
        assert!(
            !core
                .message_for_mutation(&chat, target)
                .unwrap()
                .deleted_for_everyone
        );
        assert!(core.preferences.allow_message_deletion_by_others);
        core.handle_action(AppAction::SetAllowMessageDeletionByOthers { enabled: false });
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_DELETE_KIND,
            "",
            now * 1000 + 40,
        );
        assert_eq!(
            core.message_for_mutation(&chat, target).unwrap().body,
            "second version"
        );
        core.handle_action(AppAction::SetAllowMessageDeletionByOthers { enabled: true });
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_DELETE_KIND,
            "",
            now * 1000 + 50,
        );
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "must not return",
            now * 1000 + 60,
        );
        let message = core.message_for_mutation(&chat, target).unwrap();
        assert!(
            message.deleted_for_everyone
                && message.body.is_empty()
                && message.edit_history.is_empty()
        );
        assert!(core
            .message_mutation_records(&chat, target)
            .iter()
            .all(|m| m.content.is_empty()));
        assert!(core
            .app_store
            .search_messages_fts("secret", Some(&chat), 10)
            .unwrap()
            .is_empty());
        assert!(core
            .app_store
            .search_messages_fts("version", Some(&chat), 10)
            .unwrap()
            .is_empty());
        core.persist_best_effort();
        drop(core);
        let mut core = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            directory.path().to_string_lossy().into_owned(),
        );
        core.load_persisted().unwrap();
        assert!(
            core.message_for_mutation(&chat, target)
                .unwrap()
                .deleted_for_everyone
        );
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "original secret".into(),
            now,
            None,
            Some(target.into()),
            None,
        );
        assert!(core
            .message_for_mutation(&chat, target)
            .unwrap()
            .body
            .is_empty());
    }
}

#[test]
fn message_mutations_arriving_before_original_survive_restart() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("mutation-before-original", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "early-edit",
        MESSAGE_EDIT_KIND,
        "corrected",
        now * 1000 + 10,
    );
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "early-delete",
        MESSAGE_DELETE_KIND,
        "",
        now * 1000 + 10,
    );
    core.persist_best_effort();
    drop(core);
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        directory.path().to_string_lossy().into_owned(),
    );
    core.load_persisted().unwrap();
    for target in ["early-edit", "early-delete"] {
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "original".into(),
            now,
            None,
            Some(target.into()),
            None,
        );
    }
    let edited = core.message_for_mutation(&chat, "early-edit").unwrap();
    assert_eq!(edited.body, "corrected");
    assert_eq!(edited.edit_history.len(), 2);
    let deleted = core.message_for_mutation(&chat, "early-delete").unwrap();
    assert!(deleted.deleted_for_everyone);
    assert!(deleted.body.is_empty());
}

#[test]
fn message_mutations_update_unloaded_history_and_persist_opt_out() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("mutation-unloaded", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "old message".into(),
        now,
        None,
        Some("history".into()),
        None,
    );
    core.persist_best_effort();
    core.threads.get_mut(&chat).unwrap().messages.clear();
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "history",
        MESSAGE_EDIT_KIND,
        "changed history",
        now * 1000 + 10,
    );
    assert!(core.threads[&chat].messages.is_empty());
    assert_eq!(
        core.message_for_mutation(&chat, "history").unwrap().body,
        "changed history"
    );
    core.handle_action(AppAction::SetAllowMessageDeletionByOthers { enabled: false });
    drop(core);
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        directory.path().to_string_lossy().into_owned(),
    );
    core.load_persisted().unwrap();
    assert!(!core.preferences.allow_message_deletion_by_others);
    assert_eq!(
        core.message_for_mutation(&chat, "history")
            .unwrap()
            .edit_history
            .len(),
        2
    );
}

#[test]
fn message_mutations_linked_device_history_converges_without_changing_originals() {
    for capable in [true, false] {
        let owner = Keys::generate();
        let a = Keys::generate();
        let b = Keys::generate();
        let peer = Keys::generate();
        let chat = peer.public_key().to_hex();
        let (mut left, _, _left_dir) =
            logged_in_test_core_with_updates("mutation-sync-left", &owner, &a);
        let (mut right, _, _right_dir) =
            logged_in_test_core_with_updates("mutation-sync-right", &owner, &b);
        configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
        for known in left.app_keys.values_mut() {
            known
                .devices
                .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
        }
        right.app_keys = left.app_keys.clone();
        left.create_device_history_transfer(b.public_key(), true, "ab".repeat(32))
            .unwrap();
        right
            .record_device_history_approver(&a.public_key().to_hex(), 100, "ab".repeat(32))
            .unwrap();
        for id in ["sync-edit", "sync-delete"] {
            left.apply_runtime_text_message(
                peer.public_key(),
                Some(chat.clone()),
                "Original".into(),
                20,
                None,
                Some(id.into()),
                None,
            );
        }
        receive_test_mutation(
            &mut left,
            &peer,
            &chat,
            "sync-edit",
            MESSAGE_EDIT_KIND,
            "Edited",
            21_001,
        );
        receive_test_mutation(
            &mut left,
            &peer,
            &chat,
            "sync-delete",
            MESSAGE_DELETE_KIND,
            "",
            21_002,
        );
        left.persist_best_effort();
        let endpoint = Arc::new(
            left.runtime
                .block_on(
                    fips_core::FipsEndpoint::builder()
                        .without_system_tun()
                        .bind(),
                )
                .unwrap(),
        );
        let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        left.install_device_sync_sender_for_test(
            endpoint.clone(),
            left_tx,
            vec![test_fips_peer(&b)],
        );
        right.install_device_sync_sender_for_test(
            endpoint.clone(),
            right_tx,
            vec![test_fips_peer(&a)],
        );
        let request = serde_json::to_vec(
            &serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1}),
        )
        .unwrap();
        left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
        let mut trace = Vec::new();
        for _ in 0..1024 {
            let x = drain_mutation_history_wire(
                &mut left, &a, &mut right, &b, &left_rx, &mut trace, capable,
            );
            let y = drain_mutation_history_wire(
                &mut right, &b, &mut left, &a, &right_rx, &mut trace, capable,
            );
            if !x && !y {
                break;
            }
        }
        let edited = right.message_for_mutation(&chat, "sync-edit").unwrap();
        if capable {
            assert_eq!(edited.body, "Edited");
            assert_eq!(edited.edit_history[0].body, "Original");
            assert_eq!(edited.edit_history.len(), 2);
            assert!(
                right
                    .message_for_mutation(&chat, "sync-delete")
                    .unwrap()
                    .deleted_for_everyone
            );
            assert!(trace
                .iter()
                .any(|p| p["type"] == "historyOpen" && p["messageMutations"] == 1));
            assert!(trace
                .iter()
                .filter_map(|p| p["records"].as_array())
                .flatten()
                .any(|r| r["type"] == "messageMutation"));
        } else {
            assert_eq!(edited.body, "Original");
            assert!(right.message_for_mutation(&chat, "sync-delete")
                .is_none_or(|message| message.body.is_empty()),
                "Older readers must never receive retracted plaintext");
            assert!(
                !trace
                    .iter()
                    .filter_map(|p| p["records"].as_array())
                    .flatten()
                    .any(|r| r["type"] == "messageMutation"),
                "old initiators must never receive unknown typed records"
            );
        }
        if !capable {
            if let Some(path) = std::env::var_os("IRIS_MESSAGE_MUTATION_LEGACY_TRACE") {
                std::fs::write(path, serde_json::to_vec(&serde_json::json!({
                    "owner": owner.public_key().to_hex(), "packets": trace,
                    "messageId": "sync-edit", "body": edited.body,
                })).unwrap()).unwrap();
            }
        }
        assert_eq!(left.device_history_session_count_for_test(), 0);
        assert_eq!(right.device_history_session_count_for_test(), 0);
        left.device_sync.take();
        right.device_sync.take();
        left.runtime.block_on(endpoint.shutdown()).unwrap();
    }
}

#[test]
fn message_mutations_reject_ambiguous_forged_and_attachment_edits() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let attacker = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("mutation-validation", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "original".into(),
        now,
        None,
        Some("target".into()),
        None,
    );
    let mut forged = UnsignedEvent::new(
        peer.public_key(),
        Timestamp::from_secs(now),
        Kind::Custom(MESSAGE_EDIT_KIND as u16),
        vec![nostr::Tag::parse(["e", "target"]).unwrap()],
        "forged",
    );
    forged.ensure_id();
    core.apply_decrypted_runtime_message_with_metadata(
        attacker.public_key(),
        None,
        None,
        serde_json::to_string(&forged).unwrap(),
        None,
        now,
    );
    assert_eq!(
        core.message_for_mutation(&chat, "target").unwrap().body,
        "original"
    );
    for tags in [
        vec![],
        vec![
            nostr::Tag::parse(["e", "target"]).unwrap(),
            nostr::Tag::parse(["e", "other"]).unwrap(),
        ],
    ] {
        let mut invalid = UnsignedEvent::new(
            peer.public_key(),
            Timestamp::from_secs(now),
            Kind::Custom(MESSAGE_DELETE_KIND as u16),
            tags,
            "",
        );
        invalid.ensure_id();
        core.apply_decrypted_runtime_message_with_metadata(
            peer.public_key(),
            None,
            None,
            serde_json::to_string(&invalid).unwrap(),
            None,
            now,
        );
    }
    assert!(
        !core
            .message_for_mutation(&chat, "target")
            .unwrap()
            .deleted_for_everyone
    );
    let first = receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "target",
        MESSAGE_EDIT_KIND,
        "equal clock A",
        now * 1000 + 1,
    );
    let second = receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "target",
        MESSAGE_EDIT_KIND,
        "equal clock B",
        now * 1000 + 1,
    );
    assert_eq!(
        core.message_for_mutation(&chat, "target").unwrap().body,
        if first > second {
            "equal clock A"
        } else {
            "equal clock B"
        }
    );
    core.threads.get_mut(&chat).unwrap().messages[0]
        .attachments
        .push(MessageAttachmentSnapshot {
            filename: "photo.jpg".into(),
            filename_encoded: "photo.jpg".into(),
            is_image: true,
            is_video: false,
            is_audio: false,
            htree_url: "https://example.com/photo.jpg".into(),
            nhash: String::new(),
        });
    let before = core.message_for_mutation(&chat, "target").unwrap().body;
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "target",
        MESSAGE_EDIT_KIND,
        "not allowed",
        now * 1000 + 2,
    );
    assert_eq!(
        core.message_for_mutation(&chat, "target").unwrap().body,
        before
    );
}

fn drain_mutation_history_wire(
    source: &mut AppCore,
    source_key: &Keys,
    target: &mut AppCore,
    target_key: &Keys,
    records: &flume::Receiver<super::device_sync_tcp::SendBatch>,
    trace: &mut Vec<serde_json::Value>,
    capable: bool,
) -> bool {
    let mut pending = Vec::new();
    while let Ok(batch) = records.try_recv() {
        assert_eq!(batch.peer, test_fips_peer(target_key));
        pending.extend(batch.records);
    }
    if let Some(record) = source.take_device_sync_control_for_test(test_fips_peer(target_key)) {
        pending.push(record);
    }
    let progress = !pending.is_empty();
    for record in pending {
        let mut value: serde_json::Value = serde_json::from_slice(&record).unwrap();
        if !capable && value["type"] == "historyOpen" {
            value.as_object_mut().unwrap().remove("messageMutations");
        }
        target.handle_device_sync_packet(
            &source_key.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(&value).unwrap(),
        );
        trace.push(value);
    }
    progress
}

#[test]
fn message_mutations_real_session_restore_preserves_original_and_tombstone() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("mutation-session-restore", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "Original café 🌱".into(),
        now,
        None,
        Some("restart-original".into()),
        None,
    );
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "restart-original",
        MESSAGE_EDIT_KIND,
        "Corrected café 🌱",
        now * 1000 + 10,
    );
    core.persist_best_effort();
    drop(core);
    let (sender, _pending) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        sender,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_session_inner(
        owner.public_key(),
        Some(owner.clone()),
        device.clone(),
        true,
        true,
        false,
    )
    .unwrap();
    let message = core
        .message_for_mutation(&chat, "restart-original")
        .unwrap();
    assert_eq!(message.body, "Corrected café 🌱");
    assert_eq!(message.edit_history[0].body, "Original café 🌱");
    assert_eq!(message.edit_history.len(), 2);
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "restart-original",
        MESSAGE_DELETE_KIND,
        "",
        now * 1000 + 20,
    );
    core.persist_best_effort();
    core.stop_device_sync_now();
    drop(core);
    let (sender, _pending) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        sender,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_session_inner(owner.public_key(), Some(owner), device, true, true, false)
        .unwrap();
    let message = core
        .message_for_mutation(&chat, "restart-original")
        .unwrap();
    assert!(
        message.deleted_for_everyone && message.body.is_empty() && message.edit_history.is_empty()
    );
    core.stop_device_sync_now();
}

#[test]
fn message_mutations_storage_failures_retry_before_acknowledging_delivery() {
    for projection_failure in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, _directory) =
            logged_in_test_core_with_updates("mutation-storage-retry", &owner, &device);
        let chat = peer.public_key().to_hex();
        let now = unix_now().get();
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "original".into(),
            now,
            None,
            Some("retry-original".into()),
            None,
        );
        core.persist_best_effort();
        let mut event = UnsignedEvent::new(
            peer.public_key(),
            Timestamp::from_secs(now),
            Kind::Custom(MESSAGE_EDIT_KIND as u16),
            vec![nostr::Tag::parse(["e", "retry-original"]).unwrap()],
            "retry edit",
        );
        event.ensure_id();
        let json = serde_json::to_string(&event).unwrap();
        let shared = core.app_store.shared();
        let trigger = if projection_failure {
            "CREATE TEMP TRIGGER reject_mutation BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT, 'injected projection failure'); END"
        } else {
            "CREATE TEMP TRIGGER reject_mutation BEFORE INSERT ON app_meta WHEN NEW.key LIKE 'iris-chat-sync-record-v1:%' BEGIN SELECT RAISE(ABORT, 'injected record failure'); END"
        };
        shared.lock().unwrap().execute_batch(trigger).unwrap();
        assert!(
            !core.apply_decrypted_runtime_message_with_metadata(
                peer.public_key(),
                None,
                None,
                json.clone(),
                None,
                now
            ),
            "storage failure must leave the protocol delivery unacknowledged"
        );
        assert_eq!(
            core.message_for_mutation(&chat, "retry-original")
                .unwrap()
                .body,
            "original"
        );
        shared
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_mutation")
            .unwrap();
        assert!(core.apply_decrypted_runtime_message_with_metadata(
            peer.public_key(),
            None,
            None,
            json.clone(),
            None,
            now
        ));
        assert!(core.apply_decrypted_runtime_message_with_metadata(
            peer.public_key(),
            None,
            None,
            json,
            None,
            now
        ));
        let message = core.message_for_mutation(&chat, "retry-original").unwrap();
        assert_eq!(message.body, "retry edit");
        assert_eq!(message.edit_history.len(), 2);
    }
}

#[test]
fn message_mutations_expired_deferred_controls_are_not_stored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("mutation-expiry", &owner, &device);
    let now = unix_now().get();
    let mut event = UnsignedEvent::new(
        peer.public_key(),
        Timestamp::from_secs(now - 5),
        Kind::Custom(MESSAGE_EDIT_KIND as u16),
        vec![
            nostr::Tag::parse(["e", "missing-original"]).unwrap(),
            nostr::Tag::parse(["expiration", &now.to_string()]).unwrap(),
        ],
        "expired private edit",
    );
    event.ensure_id();
    assert!(core.apply_decrypted_runtime_message_with_metadata(
        peer.public_key(),
        None,
        None,
        serde_json::to_string(&event).unwrap(),
        None,
        now
    ));
    assert!(core
        .message_mutation_records(&peer.public_key().to_hex(), "missing-original")
        .is_empty());
}

#[test]
fn message_mutations_deferred_delete_hides_original_when_projection_write_fails() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("mutation-deferred-write-failure", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "late-original",
        MESSAGE_DELETE_KIND,
        "",
        now * 1000,
    );
    let shared = core.app_store.shared();
    shared.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_projection BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'injected message storage failure'); END").unwrap();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "must stay private".into(),
        now,
        None,
        Some("late-original".into()),
        Some("late-envelope".into()),
    );
    let message = core.message_for_mutation(&chat, "late-original").unwrap();
    assert!(
        message.deleted_for_everyone && message.body.is_empty() && message.edit_history.is_empty()
    );
    core.rebuild_state();
    assert!(!format!("{:?}", core.state).contains("must stay private"));
    shared
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_projection")
        .unwrap();
    assert!(core.project_message_mutations(&chat, "late-original"));
    let persisted = core
        .app_store
        .load_messages_around(&chat, "late-original", 0, 0)
        .unwrap();
    assert!(persisted[0].deleted_for_everyone && persisted[0].body.is_empty());
}

#[test]
fn message_mutations_sync_exports_original_versions_but_never_retracted_unloaded_text() {
    fn exported_body(core: &AppCore) -> String {
        use base64::Engine;
        for packet in core.build_device_sync_packets_for_test(0, true) {
            let packet: serde_json::Value = serde_json::from_slice(&packet).unwrap();
            if let Some(message) = packet["messages"].as_array().and_then(|messages| {
                messages
                    .iter()
                    .find(|message| message["id"] == "export-original")
            }) {
                return String::from_utf8(
                    base64::engine::general_purpose::STANDARD
                        .decode(message["body"].as_str().unwrap())
                        .unwrap(),
                )
                .unwrap();
            }
        }
        panic!("original message missing from sync export");
    }
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("mutation-export-retry", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "original for edit history".into(),
        now,
        None,
        Some("export-original".into()),
        None,
    );
    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "export-original",
        MESSAGE_EDIT_KIND,
        "replacement text",
        now * 1000 + 10,
    );
    assert_eq!(
        exported_body(&core),
        "original for edit history",
        "message records must retain the original body"
    );
    core.persist_best_effort();
    core.threads.get_mut(&chat).unwrap().messages.clear();
    let shared = core.app_store.shared();
    shared.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_projection BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT, 'injected projection failure'); END").unwrap();
    let mut event = UnsignedEvent::new(
        peer.public_key(),
        Timestamp::from_secs(now),
        Kind::Custom(MESSAGE_DELETE_KIND as u16),
        vec![
            nostr::Tag::parse(["e", "export-original"]).unwrap(),
            nostr::Tag::parse(["ms", &(now * 1000 + 20).to_string()]).unwrap(),
        ],
        "",
    );
    event.ensure_id();
    assert!(!core.apply_decrypted_runtime_message_with_metadata(
        peer.public_key(),
        None,
        None,
        serde_json::to_string(&event).unwrap(),
        None,
        now
    ));
    assert!(exported_body(&core).is_empty());
    shared
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_projection")
        .unwrap();
}
