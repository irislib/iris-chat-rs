#[test]
fn device_sync_typed_records_repair_reactions_and_current_state_with_both_history_choices() {
    run_device_sync_typed_records(None);
}

#[test]
fn device_sync_typed_records_above_inventory_cap_complete_all_scopes() {
    run_device_sync_typed_records(Some(1));
}

fn run_device_sync_typed_records(record_limit: Option<usize>) {
    for include_history in [true, false] {
        let owner = Keys::generate();
        let a = Keys::generate();
        let b = Keys::generate();
        let contact = Keys::generate();
        let stranger = Keys::generate();
        let chat = contact.public_key().to_hex();
        let owner_hex = owner.public_key().to_hex();
        let (mut left, _, _left_dir) = logged_in_test_core_with_updates("typed-left", &owner, &a);
        let (mut right, _, right_dir) = logged_in_test_core_with_updates("typed-right", &owner, &b);
        configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
        for known in left.app_keys.values_mut() {
            known
                .devices
                .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
        }
        right.app_keys = left.app_keys.clone();
        left.create_device_history_transfer(b.public_key(), include_history, "ac".repeat(32))
            .unwrap();
        right
            .record_device_history_approver(&a.public_key().to_hex(), 100, "ac".repeat(32))
            .unwrap();
        for (period, created) in [("old", 50), ("new", 200)] {
            for (operation, kind, content) in [
                ("edit", MESSAGE_EDIT_KIND, "Updated"),
                ("delete", MESSAGE_DELETE_KIND, ""),
            ] {
                let target = format!("mutation-{period}-{operation}");
                left.push_incoming_message_from(
                    &chat, Some(target.clone()), "Original".into(), created,
                    None, None, Some(chat.clone()), None,
                );
                assert!(left.capture_device_sync_control(
                    &chat, &format!("control-{target}"), &chat, created + 1,
                    kind, content, &[nostr::Tag::parse(["e", target.as_str()]).unwrap()],
                ));
            }
        }
        for (id, time) in [("old", 50), ("new", 200)] {
            left.push_incoming_message_from(
                &chat,
                Some(id.into()),
                id.into(),
                time,
                None,
                None,
                Some(chat.clone()),
                None,
            );
        }
        if record_limit.is_some() {
            for n in 0..8 {
                left.push_incoming_message_from(
                    &chat,
                    Some(format!("same-second-{n}")),
                    "same time".into(),
                    50,
                    None,
                    None,
                    Some(chat.clone()),
                    None,
                );
            }
        }
        left.apply_incoming_reaction_to_chat(&chat, "old", &owner_hex, "❤");
        left.apply_incoming_reaction_to_chat(&chat, "new", &chat, "😀");
        assert!(left.capture_device_sync_control(
            &chat,
            "old-captured",
            &chat,
            51,
            REACTION_KIND,
            "😀",
            &[nostr::Tag::parse(["e", "old"]).unwrap()]
        ));
        assert!(!left.capture_device_sync_control(
            &chat,
            "invalid-ms",
            &chat,
            200,
            REACTION_KIND,
            "x",
            &[
                nostr::Tag::parse(["e", "new"]).unwrap(),
                nostr::Tag::parse(["ms", "199999"]).unwrap()
            ]
        ));
        for (id, time, target, emoji) in [
            ("add", 201, "new", "😀"),
            ("remove", 202, "new", ""),
            (
                "late",
                203,
                "later-target",
                r#"{"type":"reaction","messageId":"later-target","emoji":"❤"}"#,
            ),
        ] {
            assert!(left.capture_device_sync_control(
                &chat,
                id,
                &chat,
                time,
                REACTION_KIND,
                emoji,
                &[nostr::Tag::parse(["e", target]).unwrap()]
            ));
        }
        let metadata = |keys: &Keys, name: &str, time| {
            EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
                .custom_created_at(Timestamp::from_secs(time))
                .sign_with_keys(keys)
                .unwrap()
        };
        assert!(left.apply_profile_metadata_event(&metadata(&contact, "Contact offline", 60)));
        assert!(left.apply_profile_metadata_event(&metadata(&stranger, "Must stay private", 60)));
        let groups = serde_json::json!({"v":1,"type":"snapshot","rosterAt":100,"groups":[{
            "id":"typed-group","name":"Latest name","picture":"https://example.invalid/group.png",
            "createdBy":owner_hex,"members":[owner_hex,chat],"admins":[owner_hex],"revision":4,"createdAt":10,"updatedAt":80,
            "legacyMessageTtlSeconds":300
        }],"messages":[]});
        left.handle_device_sync_packet(
            &b.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(&groups).unwrap(),
        );
        assert!(left.groups.contains_key("typed-group"));
        let mut legacy_group = groups.clone();
        legacy_group["groups"][0]["id"] = "legacy-group".into();
        left.handle_device_sync_packet(
            &b.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(&legacy_group).unwrap(),
        );
        assert!(left.capture_device_sync_control(
            "group:typed-group",
            "timer-off",
            &owner_hex,
            90,
            CHAT_SETTINGS_KIND,
            r#"{"type":"chat-settings","v":1,"messageTtlSeconds":0}"#,
            &[]
        ));
        assert!(!left.capture_device_sync_control(
            "group:typed-group",
            "not-admin",
            &chat,
            91,
            CHAT_SETTINGS_KIND,
            r#"{"type":"chat-settings","v":1,"messageTtlSeconds":60}"#,
            &[]
        ));
        left.persist_best_effort_inner();
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
        if let Some(limit) = record_limit {
            left.set_device_history_record_limit_for_test(limit);
            right.set_device_history_record_limit_for_test(limit);
        }
        let request = serde_json::to_vec(
            &serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1}),
        )
        .unwrap();
        left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
        right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
        let mut trace = Vec::new();
        for _ in 0..2048 {
            let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
            let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
            if !x && !y {
                break;
            }
        }
        assert_eq!(
            right.owner_display_name(&chat).as_deref(),
            Some("Contact offline"),
            "wire: {}",
            serde_json::to_string(&trace).unwrap()
        );
        assert!(!right
            .owner_profiles
            .contains_key(&stranger.public_key().to_hex()));
        assert_eq!(right.groups["typed-group"].name, "Latest name");
        assert!(
            !right
                .chat_message_ttl_seconds
                .contains_key("group:typed-group"),
            "authenticated off wins legacy TTL"
        );
        assert_eq!(right.chat_message_ttl_seconds["group:legacy-group"], 300);
        legacy_group["groups"][0]["legacyMessageTtlSeconds"] = 600.into();
        right.handle_device_sync_packet(
            &a.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(&legacy_group).unwrap(),
        );
        assert_eq!(
            right.chat_message_ttl_seconds["group:legacy-group"], 300,
            "legacy baseline never overwrites existing choice"
        );
        assert_eq!(
            has_device_sync_message(&right, &chat, "old"),
            include_history
        );
        assert!(has_device_sync_message(&right, &chat, "new"));
        for period in ["old", "new"] {
            for operation in ["edit", "delete"] {
                let target = format!("mutation-{period}-{operation}");
                let expected = period == "new" || include_history;
                let message = right.message_for_mutation(&chat, &target);
                assert_eq!(message.is_some(), expected);
                assert_eq!(!right.message_mutation_records(&chat, &target).is_empty(), expected,
                    "Chats-only must exclude pre-link edit and deletion records too");
                if let Some(message) = message {
                    assert_eq!(message.deleted_for_everyone, operation == "delete");
                    assert_eq!(message.body, if operation == "edit" { "Updated" } else { "" });
                }
            }
        }
        assert_eq!(trace.iter().filter_map(|packet| packet["records"].as_array())
            .flatten().any(|record| record["type"] == "messageMutation"
                && record["mutation"]["messageId"].as_str()
                    .is_some_and(|id| id.starts_with("mutation-old-"))), include_history,
            "Pre-link mutations must never escape through state reconciliation");
        assert!(
            right.threads[&chat]
                .messages
                .iter()
                .find(|message| message.id == "new")
                .unwrap()
                .reactors
                .is_empty(),
            "removal survives repair"
        );
        if include_history {
            let old = right.threads[&chat]
                .messages
                .iter()
                .find(|message| message.id == "old")
                .unwrap();
            assert!(old
                .reactors
                .iter()
                .any(|r| r.author == owner_hex && r.emoji == "❤"));
            assert!(old
                .reactors
                .iter()
                .any(|r| r.author == chat && r.emoji == "😀"));
        }
        assert!(!has_device_sync_message(&right, &chat, "later-target"));
        right.push_incoming_message_from(
            &chat,
            Some("later-target".into()),
            "arrived later".into(),
            204,
            None,
            None,
            Some(chat.clone()),
            None,
        );
        assert_eq!(
            right.threads[&chat]
                .messages
                .iter()
                .find(|message| message.id == "later-target")
                .unwrap()
                .reactors[0]
                .emoji,
            "❤"
        );
        assert!(trace.iter().any(|packet| packet["type"] == "historyOpen"
            && packet["scope"] == "state"
            && packet["since"] == 0
            && packet["until"] == 0));
        assert!(trace
            .iter()
            .any(|packet| packet["type"] == "historyRecords"));
        // Simulate a projection save interrupted after the durable head commit.
        right.apply_incoming_reaction_to_chat(&chat, "new", &chat, "stale");
        right
            .chat_message_ttl_seconds
            .insert("group:typed-group".into(), 600);
        right.owner_profiles.remove(&chat);
        right.restore_device_sync_record_projection();
        assert!(right.threads[&chat]
            .messages
            .iter()
            .find(|m| m.id == "new")
            .unwrap()
            .reactors
            .is_empty());
        assert!(!right
            .chat_message_ttl_seconds
            .contains_key("group:typed-group"));
        assert_eq!(
            right.owner_display_name(&chat).as_deref(),
            Some("Contact offline")
        );
        if record_limit.is_some() {
            assert!(trace.iter().any(|packet| packet["type"] == "historyOpen"
                && packet["prefix"]
                    .as_str()
                    .is_some_and(|prefix| !prefix.is_empty())));
            assert!(trace
                .iter()
                .any(|packet| packet["type"] == "historyOverflow"));
            assert!(trace
                .iter()
                .filter(|packet| packet["type"] == "historyOpen"
                    && packet["scope"] == "history"
                    && packet["since"] == 0)
                .all(|packet| packet["until"] == 99 && packet["linkId"] == "ac".repeat(32)));
            for n in 0..8 {
                assert_eq!(
                    has_device_sync_message(&right, &chat, &format!("same-second-{n}")),
                    include_history
                );
            }
            assert!(
                right
                    .device_history_transfer(&a.public_key().to_hex())
                    .unwrap()
                    .complete
            );
            if include_history {
                assert_eq!(
                    right
                        .state
                        .device_history_sync
                        .as_ref()
                        .unwrap()
                        .imported_messages,
                    11 // Nine ordinary messages plus the two pre-link mutation targets.
                );
            }
        }
        right.persist_best_effort_inner();
        let keys = right.app_keys.clone();
        drop(right);
        let mut right = logged_in_test_core_at_data_dir(
            &owner,
            &b,
            right_dir.path().to_string_lossy().into_owned(),
        );
        right.app_keys = keys;
        right.owner_profiles = right.load_persisted().unwrap().unwrap().owner_profiles;
        right.ensure_thread_record(&chat, 204);
        assert!(
            !right.capture_device_sync_control(
                &chat,
                "add",
                &chat,
                201,
                REACTION_KIND,
                "😀",
                &[nostr::Tag::parse(["e", "new"]).unwrap()]
            ),
            "old reaction cannot defeat durable removal after restart"
        );
        assert_eq!(
            right.owner_display_name(&chat).as_deref(),
            Some("Contact offline")
        );
        left.runtime.block_on(endpoint.shutdown()).unwrap();
    }
}

#[test]
fn device_sync_profile_heads_reject_bad_signatures_and_preserve_latest_signed_event() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let contact = Keys::generate();
    let chat = contact.public_key().to_hex();
    let (mut core, _, dir) = logged_in_test_core_with_updates("typed-profile", &owner, &device);
    core.ensure_thread_record(&chat, 1);
    let event = |name: &str, time| {
        EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
            .custom_created_at(Timestamp::from_secs(time))
            .sign_with_keys(&contact)
            .unwrap()
    };
    let latest = event("Latest", 200);
    assert!(core.apply_profile_metadata_event(&latest));
    assert!(!core.apply_profile_metadata_event(&event("Old", 100)));
    let mut forged = latest.clone();
    forged.content = r#"{"name":"forged"}"#.into();
    assert!(!core.apply_profile_metadata_event(&forged));
    assert_eq!(core.owner_display_name(&chat).as_deref(), Some("Latest"));
    let mut ties = [event("A", 201), event("B", 201)];
    ties.sort_by_key(|event| event.id);
    assert!(core.apply_profile_metadata_event(&ties[1]));
    assert!(core.apply_profile_metadata_event(&ties[0]));
    assert!(!core.apply_profile_metadata_event(&ties[1]));

    core.owner_profiles.get_mut(&chat).unwrap().extra_metadata_json =
        r#"{"custom":{"keep":true}}"#.into();
    core.persist_best_effort();
    core.restore_device_sync_record_projection();
    assert_eq!(core.owner_profiles[&chat].extra_metadata_json,
        r#"{"custom":{"keep":true}}"#,
        "already projected signed heads must not overwrite persisted local fields");

    let mut interrupted = [event("Crash A", 202), event("Crash B", 202)];
    interrupted.sort_by_key(|event| event.id);
    assert!(core.apply_profile_metadata_event(&interrupted[1]));
    core.persist_best_effort();
    assert!(core.cache_device_sync_profile(&interrupted[0]));
    assert_eq!(core.owner_profiles[&chat].source_event_id.as_deref(),
        Some(interrupted[1].id.to_hex().as_str()));
    drop(core);
    let mut restored = logged_in_test_core_at_data_dir(&owner, &device,
        dir.path().to_string_lossy().into_owned());
    restored.owner_profiles = restored.load_persisted().unwrap().unwrap().owner_profiles;
    restored.ensure_thread_record(&chat, 1);
    restored.restore_device_sync_record_projection();
    assert_eq!(restored.owner_profiles[&chat].source_event_id.as_deref(),
        Some(interrupted[0].id.to_hex().as_str()),
        "same-time winning head must finish projection after interrupted save");
}

#[test]
fn device_sync_typed_completion_rejects_duplicate_ids_before_applying_records() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let contact = Keys::generate();
    let chat = contact.public_key().to_hex();
    let (mut left, _, _left_dir) =
        logged_in_test_core_with_updates("typed-invalid-left", &owner, &a);
    let (mut right, _, _right_dir) =
        logged_in_test_core_with_updates("typed-invalid-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    left.ensure_thread_record(&chat, 100);
    for keys in [&owner, &contact] {
        let event = EventBuilder::new(Kind::Metadata, r#"{"name":"Private profile"}"#)
            .custom_created_at(Timestamp::from_secs(50))
            .sign_with_keys(keys)
            .unwrap();
        assert!(left.apply_profile_metadata_event(&event));
    }
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
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    let request = serde_json::to_vec(
        &serde_json::json!({"v":1,"type":"request","rosterAt":100,"recordReconcile":1}),
    )
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    let mut tampered = false;
    for _ in 0..64 {
        while let Ok(batch) = left_rx.try_recv() {
            for bytes in batch.records {
                let mut packet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if !tampered
                    && packet["type"] == "historyRecords"
                    && packet["requested"]
                        .as_array()
                        .is_some_and(|ids| ids.len() == 2)
                {
                    packet["requested"][1] = packet["requested"][0].clone();
                    tampered = true;
                }
                right.handle_device_sync_packet(
                    &a.public_key().to_hex(),
                    DEVICE_SYNC_PORT,
                    &serde_json::to_vec(&packet).unwrap(),
                );
            }
        }
        drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if tampered {
            break;
        }
    }
    assert!(
        tampered,
        "fixture must reach a real two-record demand completion"
    );
    assert!(!right.owner_profiles.contains_key(&chat));
    assert!(!right
        .owner_profiles
        .contains_key(&owner.public_key().to_hex()));
    right.restore_device_sync_record_projection();
    assert!(
        !right.owner_profiles.contains_key(&chat),
        "no record may be persisted before validating completion"
    );
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    for _ in 0..64 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert_eq!(
        right.owner_display_name(&chat).as_deref(),
        Some("Private profile")
    );
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn device_sync_reactions_expire_with_target_and_keep_durable_suppression() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let contact = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("typed-expiry", &owner, &device);
    let now = unix_now().get();
    core.push_incoming_message_from(
        &contact,
        Some("expires".into()),
        "short lived".into(),
        now,
        Some(now + 5),
        None,
        Some(contact.clone()),
        None,
    );
    assert!(core.capture_device_sync_control(
        &contact,
        "reaction",
        &contact,
        now,
        REACTION_KIND,
        "x",
        &[nostr::Tag::parse(["e", "expires"]).unwrap()]
    ));
    core.threads
        .get_mut(&contact)
        .unwrap()
        .messages
        .iter_mut()
        .find(|m| m.id == "expires")
        .unwrap()
        .expires_at_secs = Some(now - 1);
    core.persist_best_effort_inner();
    assert!(!core.capture_device_sync_control(
        &contact,
        "before-prune",
        &contact,
        now,
        REACTION_KIND,
        "x",
        &[nostr::Tag::parse(["e", "expires"]).unwrap()]
    ));
    core.prune_expired_messages(now);
    assert!(core
        .app_store
        .message_was_locally_deleted(&contact, Some("expires"), None)
        .unwrap());
    assert!(!core.capture_device_sync_control(
        &contact,
        "after-prune",
        &contact,
        now,
        REACTION_KIND,
        "x",
        &[nostr::Tag::parse(["e", "expires"]).unwrap()]
    ));
    assert!(core.capture_device_sync_control(
        &contact,
        "not-yet-arrived",
        &contact,
        now,
        REACTION_KIND,
        "x",
        &[nostr::Tag::parse(["e", "future-target"]).unwrap()]
    ));
}

#[test]
fn device_sync_typed_need_withholds_replaced_reactions_and_profiles() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let contact = Keys::generate();
    let chat = contact.public_key().to_hex();
    let (mut left, _, _left_dir) =
        logged_in_test_core_with_updates("typed-current-left", &owner, &a);
    let (mut right, _, _right_dir) =
        logged_in_test_core_with_updates("typed-current-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    left.create_device_history_transfer(b.public_key(), true, "de".repeat(32))
        .unwrap();
    right
        .record_device_history_approver(&a.public_key().to_hex(), 100, "de".repeat(32))
        .unwrap();
    left.push_incoming_message_from(
        &chat,
        Some("old-target".into()),
        "old".into(),
        50,
        None,
        None,
        Some(chat.clone()),
        None,
    );
    assert!(left.capture_device_sync_control(
        &chat,
        "superseded-reaction",
        &chat,
        51,
        REACTION_KIND,
        "x",
        &[nostr::Tag::parse(["e", "old-target"]).unwrap()]
    ));
    let profile = |name: &str, time| {
        EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
            .custom_created_at(Timestamp::from_secs(time))
            .sign_with_keys(&contact)
            .unwrap()
    };
    let old_profile = profile("Old", 60);
    assert!(left.apply_profile_metadata_event(&old_profile));
    left.persist_best_effort_inner();
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
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    let request = serde_json::to_vec(
        &serde_json::json!({"v":1,"type":"request","rosterAt":100,"recordReconcile":1}),
    )
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
    // Both immutable inventories are captured; no demand has reached the source yet.
    assert!(left.capture_device_sync_control(
        &chat,
        "winning-remove",
        &chat,
        52,
        REACTION_KIND,
        "",
        &[nostr::Tag::parse(["e", "old-target"]).unwrap()]
    ));
    assert!(left.apply_profile_metadata_event(&profile("Current", 70)));
    for _ in 0..64 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    let records = trace
        .iter()
        .filter(|p| p["type"] == "historyRecords")
        .flat_map(|p| p["records"].as_array().unwrap())
        .collect::<Vec<_>>();
    assert!(records
        .iter()
        .all(|r| r["reaction"]["id"] != "superseded-reaction"
            && r["event"]["id"] != old_profile.id.to_hex()));
    let old_hash = <sha2::Sha256 as sha2::Digest>::digest(
        serde_json::json!(["reaction", chat, "superseded-reaction"])
            .to_string()
            .as_bytes(),
    )
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect::<String>();
    assert!(
        trace.iter().any(|p| p["type"] == "historyNeed"
            && p["ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &old_hash)),
        "stale head was actually demanded"
    );
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    for _ in 0..64 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert_eq!(right.owner_display_name(&chat).as_deref(), Some("Current"));
    assert!(
        right
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .complete
    );
    assert!(right.threads[&chat]
        .messages
        .iter()
        .find(|m| m.id == "old-target")
        .unwrap()
        .reactors
        .is_empty());
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}
