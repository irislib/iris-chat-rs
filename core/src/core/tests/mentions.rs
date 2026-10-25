#[test]
fn mentions_group_push_and_foreground_use_identity_and_respect_mute_choice() {
    let mut farm = GroupScaleFarm::new(&[1, 1]);
    farm.pump();
    farm.devices[0].core.create_group("Mention test", &[]);
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    farm.add_members(&group_id, 1..2);
    farm.retry();
    let chat = group_chat_id(&group_id);
    let owner = farm.owners[1].public_key().to_hex();
    let mention = crate::format_message_mention(owner.clone(), "Alex".into());
    let other_mention =
        crate::format_message_mention(farm.owners[0].public_key().to_hex(), "Alex".into());
    farm.devices[1].core.set_chat_muted(&chat, true);
    farm.devices[1].core.active_chat_id = None;
    farm.devices[1].core.rebuild_persist_and_emit_state();
    for (body, enabled, expected) in [
        ("ordinary message".into(), true, false),
        ("hello @Alex".into(), true, false),
        (other_mention, true, false),
        (format!("Hello {mention}"), true, true),
        (format!("Quiet {mention}"), false, false),
        (format!("Again {mention}"), true, true),
        (format!("↩ Someone: {mention}\n\nQuoted"), true, false),
    ] {
        farm.devices[1]
            .core
            .set_chat_mention_notifications(&chat, enabled);
        let before = farm.devices[1].core.state.chat_list.clone();
        farm.devices[0].core.send_message(&chat, &body, None);
        let authors = farm.devices[0]
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .known_group_sender_event_pubkeys();
        let events = sorted_pending_events_for_test(&farm.devices[0].core);
        let receiver = &farm.devices[1];
        let event = events
            .iter()
            .find(|event| {
                event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                    && event.tags.public_keys().copied().collect::<Vec<_>>()
                        == vec![receiver.keys.public_key()]
                    && farm.devices[0]
                        .core
                        .pending_relay_publishes
                        .get(&event.id.to_hex())
                        .is_some_and(|pending| pending.chat_id.as_deref() == Some(chat.as_str()))
            })
            .expect("recipient-specific private group wake")
            .clone();
        let push = receiver.core.build_mobile_push_sync_snapshot();
        assert!(
            !authors
                .iter()
                .any(|author| push.message_author_pubkeys.contains(&author.to_hex())),
            "shared group authors must never enter notification subscriptions"
        );
        assert!(
            push.message_author_pubkeys.contains(&event.pubkey.to_hex()),
            "private transport stays subscribed; mention policy is applied after decryption"
        );
        let payload =
            serde_json::json!({"event":event,"title":"Iris Chat","body":"New message"}).to_string();
        let resolve = |device: &GroupScaleDevice| {
            decrypt_mobile_push_notification(
                device._dir.path().to_string_lossy().into_owned(),
                owner.clone(),
                device.keys.secret_key().to_secret_hex(),
                payload.clone(),
            )
        };
        let decrypted = resolve(receiver);
        assert_eq!(decrypted.should_show, expected, "decrypted push for {body}");
        if expected {
            assert!(decrypted.body.contains("@Alex"));
            assert!(!decrypted.body.contains("nostr:"));
        } else {
            assert!(
                decrypted.title.is_empty() && decrypted.body.is_empty(),
                "no generic placeholder for suppressed groups"
            );
        }
        farm.pump();
        farm.devices[1].core.persist_best_effort_inner();
        let cached = resolve(&farm.devices[1]);
        assert_eq!(
            cached.should_show, expected,
            "SQLite push fallback for {body}"
        );
        let receiver = &farm.devices[1].core;
        let candidates = notifications::decide_notifications(
            &before,
            &receiver.state.chat_list,
            &receiver.preferences,
            false,
            None,
        );
        assert_eq!(
            candidates.iter().any(|item| item.chat_id == chat),
            expected,
            "shell notification for {body}"
        );
        if expected {
            let id = farm.devices[1].core.threads[&chat]
                .messages
                .last()
                .unwrap()
                .id
                .clone();
            farm.devices[1].core.mark_messages_seen(&chat, &[id]);
            farm.devices[1].core.persist_best_effort_inner();
            let seen = resolve(&farm.devices[1]);
            assert!(!seen.should_show, "a seen mention never re-notifies");
            assert!(seen.title.is_empty() && seen.body.is_empty());
        }
    }
    // A mention never overrides a block, even with its group exception enabled.
    farm.devices[1]
        .core
        .preferences
        .blocked_owner_pubkeys
        .push(farm.owners[0].public_key().to_hex());
    farm.devices[1].core.persist_best_effort_inner();
    farm.devices[0]
        .core
        .send_message(&chat, &format!("Blocked {mention}"), None);
    let authors = farm.devices[0]
        .core
        .protocol_engine
        .as_ref()
        .unwrap()
        .known_group_sender_event_pubkeys();
    let event = sorted_pending_events_for_test(&farm.devices[0].core)
        .into_iter()
        .find(|event| {
            event
                .tags
                .public_keys()
                .any(|key| *key == farm.devices[1].keys.public_key())
                && !authors.contains(&event.pubkey)
        })
        .unwrap();
    let receiver = &farm.devices[1];
    assert!(
        !decrypt_mobile_push_notification(
            receiver._dir.path().to_string_lossy().into_owned(),
            owner,
            receiver.keys.secret_key().to_secret_hex(),
            serde_json::json!({"event":event}).to_string()
        )
        .should_show
    );
}

#[test]
fn mentions_opt_out_is_private_durable_and_order_independent() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, dir) = logged_in_test_core_with_updates("mention-setting", &owner, &device);
    let group = "group:mentions";
    let event = |key: PublicKey, enabled, clock| {
        nostr_identity::build_private_setting(
            key,
            "mentionNotifications",
            group,
            &serde_json::json!({"chatId":group,"enabled":enabled,"updatedAtMs":clock}),
        )
        .unwrap()
    };
    let quiet = event(owner.public_key(), false, 1001);
    let stale = event(owner.public_key(), true, 1000);
    assert!(core.merge_private_setting(&quiet));
    assert!(core.merge_private_setting(&stale));
    assert!(!core.merge_private_setting(&event(Keys::generate().public_key(), true, 1002)));
    assert_eq!(core.preferences.silent_mention_chat_ids, vec![group]);
    let encoded = serde_json::to_value(&quiet).unwrap();
    assert!(
        encoded.get("sig").is_none(),
        "internal settings stay unsigned and encrypted to linked devices"
    );
    assert!(core
        .private_state_fixture_for_test()
        .iter()
        .any(|item| item["record"]["event"] == encoded));
    core.persist_best_effort_inner();
    drop(core);
    let mut restored =
        logged_in_test_core_at_data_dir(&owner, &device, dir.path().to_string_lossy().into());
    restored.load_persisted().unwrap();
    restored.restore_device_sync_record_projection().unwrap();
    assert_eq!(restored.preferences.silent_mention_chat_ids, vec![group]);
    restored.set_chat_mention_notifications(group, true);
    assert!(restored.preferences.silent_mention_chat_ids.is_empty());
    assert!(restored.merge_private_setting(&quiet));
    assert!(
        restored.preferences.silent_mention_chat_ids.is_empty(),
        "old opt-out cannot undo a newer choice"
    );
}

#[test]
fn mentions_coalesced_state_keeps_new_identity_without_renotifying_old_unread_mentions() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("mentions-coalesced", &owner, &device);
    let chat = "group:coalesced";
    core.ensure_thread_record(chat, 10);
    core.set_chat_muted(chat, true);
    core.active_chat_id = None;
    core.rebuild_persist_and_emit_state();
    let before = core.state.chat_list.clone();
    let mention = crate::format_message_mention(owner.public_key().to_hex(), "You".into());
    core.enter_batch();
    for (id, body, timestamp) in [
        ("mention-one", mention.clone(), 100),
        ("ordinary-one", "Later ordinary message".into(), 101),
    ] {
        core.push_incoming_message_from(
            chat,
            Some(id.into()),
            body,
            timestamp,
            None,
            None,
            Some(sender.public_key().to_hex()),
            None,
        );
        core.rebuild_persist_and_emit_state();
    }
    core.exit_batch();
    let after = core.state.chat_list.clone();
    assert_eq!(after[0].last_message_id.as_deref(), Some("ordinary-one"));
    let candidates =
        notifications::decide_notifications(&before, &after, &core.preferences, false, None);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].message_id.as_deref(), Some("mention-one"));
    assert_eq!(candidates[0].body, "@You");
    core.push_incoming_message_from(
        chat,
        Some("ordinary-two".into()),
        "Another ordinary message".into(),
        102,
        None,
        None,
        Some(sender.public_key().to_hex()),
        None,
    );
    core.rebuild_persist_and_emit_state();
    assert!(notifications::decide_notifications(
        &after,
        &core.state.chat_list,
        &core.preferences,
        false,
        None
    )
    .is_empty());
    let before_seen = core.state.chat_list.clone();
    core.mark_messages_seen(chat, &["mention-one".into()]);
    core.rebuild_persist_and_emit_state();
    assert!(core.state.chat_list[0].unread_mention_messages.is_empty());
    assert!(notifications::decide_notifications(
        &before_seen,
        &core.state.chat_list,
        &core.preferences,
        false,
        None
    )
    .is_empty());
}

#[test]
fn mentions_coalesced_synced_reads_cannot_hide_a_new_mention() {
    for read_count in [1, 2] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sender = Keys::generate();
        let mut core = logged_in_test_core("mentions-read-batch", &owner, &device);
        let chat = "group:read-batch";
        core.ensure_thread_record(chat, 10);
        core.set_chat_muted(chat, true);
        core.active_chat_id = None;
        let mention = crate::format_message_mention(owner.public_key().to_hex(), "You".into());
        for id in ["old-one", "old-two"] {
            core.push_incoming_message_from(
                chat,
                Some(id.into()),
                mention.clone(),
                100,
                None,
                None,
                Some(sender.public_key().to_hex()),
                None,
            );
        }
        core.rebuild_persist_and_emit_state();
        let before = core.state.chat_list.clone();
        core.enter_batch();
        assert!(core.apply_chat_read_state(
            chat,
            ChatReadState {
                updated_at_ms: unix_now().get() * 1000,
                device_id: Keys::generate().public_key().to_hex(),
                seen_messages: ["old-one", "old-two"]
                    .into_iter()
                    .take(read_count)
                    .map(|id| (
                        id.into(),
                        MessageReadState {
                            created_at_secs: 100,
                            expires_at_secs: None
                        }
                    ))
                    .collect(),
                ..Default::default()
            }
        ));
        core.rebuild_persist_and_emit_state();
        core.push_incoming_message_from(
            chat,
            Some("new-mention".into()),
            mention,
            101,
            None,
            None,
            Some(sender.public_key().to_hex()),
            None,
        );
        core.rebuild_persist_and_emit_state();
        core.exit_batch();
        let after = core.state.chat_list.clone();
        assert_eq!(
            after[0].unread_count,
            before[0].unread_count + 1 - read_count as u64
        );
        let candidates =
            notifications::decide_notifications(&before, &after, &core.preferences, false, None);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].message_id.as_deref(), Some("new-mention"));
        core.push_incoming_message_from(
            chat,
            Some("ordinary".into()),
            "Later ordinary".into(),
            102,
            None,
            None,
            Some(sender.public_key().to_hex()),
            None,
        );
        core.rebuild_persist_and_emit_state();
        assert!(notifications::decide_notifications(
            &after,
            &core.state.chat_list,
            &core.preferences,
            false,
            None
        )
        .is_empty());
    }
}

#[test]
fn mentions_private_setting_matches_shared_wire_fixture() {
    let event = nostr_identity::build_private_setting(
        PublicKey::from_hex(&"a".repeat(64)).unwrap(),
        "mentionNotifications",
        "group:friends",
        &serde_json::json!({"chatId":"group:friends","enabled":false,"updatedAtMs":100}),
    )
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mention-notifications.json"
    )))
    .unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), expected);
}
