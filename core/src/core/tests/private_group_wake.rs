fn private_group_wake_farm() -> (GroupScaleFarm, String, String) {
    let mut farm = GroupScaleFarm::new(&[1, 1, 1]);
    farm.pump();
    let members = farm.owners[1..]
        .iter()
        .map(|owner| owner.public_key().to_hex())
        .collect::<Vec<_>>();
    farm.devices[0].core.create_group("Private wakes", &members);
    farm.pump();
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    let chat_id = group_chat_id(&group_id);
    for device in &mut farm.devices {
        assert!(device.core.groups.contains_key(&group_id));
        device
            .core
            .handle_action(AppAction::SetMessageRequestAccepted {
                chat_id: chat_id.clone(),
            });
        device.core.persist_best_effort();
    }
    (farm, group_id, chat_id)
}

fn private_group_wake_events(farm: &GroupScaleFarm, group_id: &str) -> (Event, Vec<Event>) {
    let sender = &farm.devices[0].core;
    let authors = sender
        .protocol_engine
        .as_ref()
        .unwrap()
        .group_sender_event_pubkeys_for_group(group_id);
    let pending = sorted_pending_events_for_test(sender);
    let wakes = pending
        .into_iter()
        .filter(|event| {
            event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND && !authors.contains(&event.pubkey)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        wakes.len(),
        2,
        "each other account gets its own private wake"
    );
    let wake = wakes.first().expect("private group carrier");
    let receiver = farm
        .devices
        .iter()
        .find(|device| {
            wake.tags
                .public_keys()
                .any(|key| *key == device.keys.public_key())
        })
        .unwrap();
    let owner = farm.owners[receiver.owner].public_key();
    let message = mobile_push::preview_direct_messages(
        &receiver._dir.path().to_string_lossy(),
        owner,
        &receiver.keys,
        std::slice::from_ref(wake),
    )
    .expect("decrypt private group carrier without consuming the live ratchet");
    let rumor = parse_runtime_rumor(&message.content).unwrap();
    let broadcast = private_group_wake::decode(&rumor.content).expect("nested signed group event");
    (broadcast, wakes)
}

#[test]
fn mobile_push_rejects_raw_group_encodings_without_preview_or_group_work() {
    use base64::Engine;

    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    farm.send(0, &chat_id, "an unrelated recent cached group message");
    farm.pump();
    farm.devices[1].core.persist_best_effort();
    farm.send(
        0,
        &chat_id,
        "raw shared ciphertext must not produce a preview",
    );
    let (header_event, _) = private_group_wake_events(&farm, &group_id);
    assert!(parse_group_sender_key_message_event_unchecked(&header_event).is_ok());
    let legacy_author = Keys::generate();
    let mut legacy_bytes = 7_u32.to_be_bytes().to_vec();
    legacy_bytes.extend_from_slice(&0_u32.to_be_bytes());
    legacy_bytes.extend_from_slice(b"unmapped legacy group ciphertext");
    let legacy_event = EventBuilder::new(
        Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16),
        base64::engine::general_purpose::STANDARD.encode(legacy_bytes),
    )
    .tag(nostr::Tag::parse(["v", "2"]).unwrap())
    .sign_with_keys(&legacy_author)
    .unwrap();
    assert!(parse_group_sender_key_message_event(&legacy_event).is_ok());

    let owner = &farm.owners[1];
    let entry = &farm.devices[1];
    let saved = runtime_state_json(&entry.core, owner, &entry.keys);
    for (key, event) in [("event", header_event), ("outer_event_json", legacy_event)] {
        let wire = serde_json::to_string(&event).unwrap();
        let payload = serde_json::json!({(key): wire, "body":"New message"}).to_string();
        let preview = decrypt_mobile_push_notification(
            entry._dir.path().to_string_lossy().into_owned(),
            owner.public_key().to_hex(),
            entry.keys.secret_key().to_secret_hex(),
            payload,
        );
        assert!(
            !preview.should_show,
            "raw {key} group delivery must be suppressed"
        );
        assert!(
            preview.body.is_empty(),
            "no unrelated recent-group fallback"
        );
        let mut engine = mobile_push::direct_message_preview_engine(
            &entry._dir.path().to_string_lossy(),
            owner.public_key(),
            &entry.keys,
        )
        .unwrap();
        let before = engine.debug_snapshot();
        assert!(mobile_push::preview_direct_messages_in_engine(
            &mut engine,
            std::slice::from_ref(&event),
        )
        .is_none());
        let after = engine.debug_snapshot();
        assert_eq!(
            after.pending_group_sender_key_message_count,
            before.pending_group_sender_key_message_count
        );
        assert_eq!(
            after.pending_group_sender_key_repair_count,
            before.pending_group_sender_key_repair_count
        );
        assert_eq!(
            after.pending_group_sender_key_unmapped_count,
            before.pending_group_sender_key_unmapped_count
        );
    }
    assert_eq!(runtime_state_json(&entry.core, owner, &entry.keys), saved);
}

#[test]
fn mobile_push_cached_group_previews_require_authenticated_carrier_ids() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let body = "cached private group notification";
    farm.send(0, &chat_id, body);
    let (nested, wakes) = private_group_wake_events(&farm, &group_id);
    let owner = farm.owners[1].public_key();
    let receiver = &mut farm.devices[1];
    let wake = wakes
        .iter()
        .find(|event| {
            event
                .tags
                .public_keys()
                .any(|key| *key == receiver.keys.public_key())
        })
        .unwrap();
    receiver.core.handle_relay_event(wake.clone());
    receiver.core.persist_best_effort();
    let data_dir = receiver._dir.path().to_string_lossy().into_owned();
    let conn = mobile_push::open_lookup_connection(&data_dir).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT body FROM messages WHERE source_event_id = ?1",
            [nested.id.to_hex()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored, body,
        "the exact raw ciphertext ID now has a cached row"
    );
    assert_eq!(
        private_group_wake::preview_source_id(&conn, &nested.id.to_hex()),
        None
    );
    assert_eq!(
        private_group_wake::preview_source_id(&conn, &wake.id.to_hex()),
        Some(nested.id.to_hex())
    );
    let resolve = |payload: serde_json::Value| {
        decrypt_mobile_push_notification(
            data_dir.clone(),
            owner.to_hex(),
            // A consumed ratchet must use persisted carrier evidence, without
            // requiring fresh decryption to rediscover that evidence.
            "unavailable preview key".into(),
            payload.to_string(),
        )
    };
    let cached = resolve(serde_json::json!({"event":wake}));
    assert!(cached.should_show);
    assert!(
        cached.body.ends_with(body),
        "foreground consumption must retain the wake preview: {}",
        cached.body
    );
    for encoded in [
        serde_json::json!(nested),
        serde_json::json!(serde_json::to_string(&nested).unwrap()),
    ] {
        let rejected = resolve(serde_json::json!({
            "event":encoded, "source_event_id":wake.id.to_hex(),
            "outer_event":wake, "chat_id":chat_id,
            "title":"spoofed cached wrapper", "body":body
        }));
        assert!(!rejected.should_show);
        assert!(rejected.body.is_empty());
    }
    let mut forged = serde_json::to_value(&nested).unwrap();
    forged["id"] = serde_json::json!(wake.id.to_hex());
    let rejected = resolve(serde_json::json!({"event":forged}));
    assert!(!rejected.should_show);
    assert!(
        rejected.body.is_empty(),
        "the complete carrier signature must verify"
    );
}

#[test]
fn mobile_push_group_wake_binds_outer_rumor_and_nested_sender() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let body = "authenticated nested group preview";
    farm.send(0, &chat_id, body);
    let (broadcast, _) = private_group_wake_events(&farm, &group_id);
    let receiver_owner = farm.owners[1].public_key();
    let receiver_device = farm.devices[1].keys.public_key();
    // The first case has a valid carrier and nested sender, but a forged rumor
    // author. The second authenticates another member carrying this sender's
    // ciphertext. Neither may render a group notification.
    for (sender_index, rumor_author, should_show) in [
        (0, farm.owners[2].public_key(), false),
        (2, farm.owners[2].public_key(), false),
        (0, farm.owners[0].public_key(), true),
        (0, farm.devices[0].keys.public_key(), true),
    ] {
        let rumor = UnsignedEvent::new(
            rumor_author,
            Timestamp::now(),
            Kind::Custom(private_group_wake::GROUP_WAKE_KIND as u16),
            [],
            serde_json::json!({"v":1,"type":"group-wake","event":broadcast}).to_string(),
        );
        let result = farm.devices[sender_index]
            .core
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_direct_unsigned_event_to_peer_only(receiver_owner, &chat_id, rumor, unix_now())
            .unwrap();
        let wake = protocol_publish_events(&result.effects)
            .into_iter()
            .find(|event| {
                event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                    && event.tags.public_keys().any(|key| *key == receiver_device)
            })
            .expect("recipient-specific ratchet envelope");
        let preview = decrypt_mobile_push_notification(
            farm.devices[1]._dir.path().to_string_lossy().into_owned(),
            receiver_owner.to_hex(),
            farm.devices[1].keys.secret_key().to_secret_hex(),
            serde_json::json!({"event":serde_json::to_string(wake).unwrap()}).to_string(),
        );
        assert_eq!(preview.should_show, should_show);
        if should_show {
            assert_eq!(preview.body, body);
            let decoded: serde_json::Value = serde_json::from_str(&preview.payload_json).unwrap();
            assert_eq!(decoded["chat_id"], chat_id);
            assert_eq!(decoded["event"]["id"], wake.id.to_hex());
        } else {
            assert!(preview.body.is_empty());
        }
    }
}

#[test]
fn mobile_push_group_wake_rejects_nested_rumor_author_mismatch() {
    for device_authored in [false, true] {
        let (mut farm, group_id, chat_id) = private_group_wake_farm();
        let author = if device_authored {
            farm.devices[0].keys.public_key()
        } else {
            farm.owners[2].public_key()
        };
        let body = "nested rumor author must be authenticated";
        let (rumor, id) =
            runtime_rumor_json(author, CHAT_MESSAGE_KIND, body, unix_now().get(), vec![]);
        let result = farm.devices[0]
            .core
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_group_payload(&group_id, rumor.into_bytes(), Some(id))
            .unwrap();
        let owner = farm.owners[1].public_key();
        let receiver = &mut farm.devices[1];
        let wake = protocol_publish_events(&result.effects)
            .into_iter()
            .find(|event| {
                event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                    && event
                        .tags
                        .public_keys()
                        .any(|key| *key == receiver.keys.public_key())
            })
            .expect("recipient-specific ratchet envelope");
        let preview = decrypt_mobile_push_notification(
            receiver._dir.path().to_string_lossy().into_owned(),
            owner.to_hex(),
            receiver.keys.secret_key().to_secret_hex(),
            serde_json::json!({"event":wake}).to_string(),
        );
        assert_eq!(preview.should_show, device_authored);
        assert_eq!(preview.body, if device_authored { body } else { "" });
        receiver.core.handle_relay_event(wake.clone());
        assert_eq!(
            receiver.core.threads[&chat_id]
                .messages
                .iter()
                .any(|message| message.body == body),
            device_authored
        );
    }
}

#[test]
fn mobile_push_group_wake_control_preview_survives_foreground_decryption() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let (rumor, id) = runtime_rumor_json(
        farm.owners[0].public_key(),
        RECEIPT_KIND,
        "seen",
        unix_now().get(),
        vec![],
    );
    let result = farm.devices[0]
        .core
        .protocol_engine
        .as_mut()
        .unwrap()
        .send_group_payload(&group_id, rumor.into_bytes(), Some(id))
        .unwrap();
    let owner = farm.owners[1].public_key();
    let receiver = &mut farm.devices[1];
    let wake = protocol_publish_events(&result.effects)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                && event
                    .tags
                    .public_keys()
                    .any(|key| *key == receiver.keys.public_key())
        })
        .expect("recipient-specific ratchet envelope")
        .clone();
    let direct = mobile_push::preview_direct_messages(
        &receiver._dir.path().to_string_lossy(),
        owner,
        &receiver.keys,
        std::slice::from_ref(&wake),
    )
    .unwrap();
    let nested =
        private_group_wake::decode(&parse_runtime_rumor(&direct.content).unwrap().content).unwrap();
    for consumed in [false, true] {
        if consumed {
            receiver.core.handle_relay_event(wake.clone());
            receiver.core.persist_best_effort();
        }
        let preview = decrypt_mobile_push_notification(
            receiver._dir.path().to_string_lossy().into_owned(),
            owner.to_hex(),
            receiver.keys.secret_key().to_secret_hex(),
            serde_json::json!({"event":wake}).to_string(),
        );
        assert!(!preview.should_show, "group controls stay suppressed");
        assert_eq!(preview.body, "Seen");
        let decoded: serde_json::Value = serde_json::from_str(&preview.payload_json).unwrap();
        assert_eq!(decoded["chat_id"], chat_id);
        assert_eq!(decoded["event"]["id"], wake.id.to_hex());
        if consumed {
            let raw = decrypt_mobile_push_notification(
                receiver._dir.path().to_string_lossy().into_owned(),
                owner.to_hex(),
                receiver.keys.secret_key().to_secret_hex(),
                serde_json::json!({"event":nested, "source_event_id":wake.id.to_hex()}).to_string(),
            );
            assert!(!raw.should_show);
            assert!(
                raw.body.is_empty(),
                "cached nested controls are not public carriers"
            );
        }
    }
}

#[test]
fn mobile_push_device_authored_direct_rumors_match_foreground_sender_validation() {
    let (mut farm, _, _) = private_group_wake_farm();
    let sender_owner = farm.owners[0].public_key();
    let sender_device = farm.devices[0].keys.public_key();
    let receiver_owner = farm.owners[1].public_key();
    let receiver_device = farm.devices[1].keys.public_key();
    let other_authorized_device = Keys::generate();
    let now = unix_now().get();
    let roster = AppKeys::new(vec![
        DeviceEntry::new(sender_device, now),
        DeviceEntry::new(other_authorized_device.public_key(), now),
    ])
    .get_event_at(sender_owner, now + 1)
    .sign_with_keys(&farm.owners[0])
    .unwrap();
    farm.devices[1].core.handle_relay_event(roster);
    farm.devices[1].core.persist_best_effort();
    assert!(farm.devices[1]
        .core
        .protocol_engine
        .as_ref()
        .unwrap()
        .owner_device_binding_is_verified(sender_owner, other_authorized_device.public_key()));
    for (author, body, accepted) in [
        (sender_device, "authenticated sending device", true),
        (
            other_authorized_device.public_key(),
            "proven sibling author",
            true,
        ),
        (
            farm.owners[2].public_key(),
            "forged unrelated author",
            false,
        ),
    ] {
        let rumor = UnsignedEvent::new(
            author,
            Timestamp::now(),
            Kind::Custom(CHAT_MESSAGE_KIND as u16),
            [],
            body,
        );
        let sent = farm.devices[0]
            .core
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_direct_unsigned_event_to_peer_only(
                receiver_owner,
                &receiver_owner.to_hex(),
                rumor,
                unix_now(),
            )
            .unwrap();
        let event = protocol_publish_events(&sent.effects)
            .into_iter()
            .find(|event| {
                event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                    && event.tags.public_keys().any(|key| *key == receiver_device)
            })
            .unwrap()
            .clone();
        let receiver = &mut farm.devices[1];
        let payload = serde_json::json!({"event":event}).to_string();
        let data_dir = receiver._dir.path().to_string_lossy().into_owned();
        let device_secret = receiver.keys.secret_key().to_secret_hex();
        let resolve = || {
            decrypt_mobile_push_notification(
                data_dir.clone(),
                receiver_owner.to_hex(),
                device_secret.clone(),
                payload.clone(),
            )
        };
        let cold = resolve();
        assert_eq!(cold.should_show, accepted);
        assert_eq!(cold.body, if accepted { body } else { "" });
        receiver.core.handle_relay_event(event);
        receiver.core.persist_best_effort();
        assert_eq!(
            receiver
                .core
                .threads
                .values()
                .any(|thread| thread.messages.iter().any(|message| message.body == body)),
            accepted
        );
        if accepted {
            let cached = resolve();
            assert!(cached.should_show);
            assert_eq!(cached.body, body);
        }
    }
}

#[test]
fn mobile_push_direct_group_copies_keep_authenticated_cached_previews() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let owner = farm.owners[1].public_key();
    let device = farm.devices[1].keys.public_key();
    for (kind, body, expected, visible) in [
        (
            CHAT_MESSAGE_KIND,
            "private direct group copy",
            "private direct group copy",
            true,
        ),
        (RECEIPT_KIND, "seen", "Seen", false),
    ] {
        let rumor = UnsignedEvent::new(
            farm.devices[0].keys.public_key(),
            Timestamp::now(),
            Kind::Custom(kind as u16),
            [nostr::Tag::parse(["l", group_id.as_str()]).unwrap()],
            body,
        );
        let sent = farm.devices[0]
            .core
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_direct_unsigned_event_to_peer_only(owner, &chat_id, rumor, unix_now())
            .unwrap();
        let event = protocol_publish_events(&sent.effects)
            .into_iter()
            .find(|event| {
                event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                    && event.tags.public_keys().any(|key| *key == device)
            })
            .unwrap()
            .clone();
        let receiver = &mut farm.devices[1];
        receiver.core.handle_relay_event(event.clone());
        receiver.core.persist_best_effort();
        let data_dir = receiver._dir.path().to_string_lossy().into_owned();
        let conn = mobile_push::open_lookup_connection(&data_dir).unwrap();
        assert_eq!(
            private_group_wake::preview_source_id(&conn, &event.id.to_hex()),
            Some(event.id.to_hex())
        );
        let cached = decrypt_mobile_push_notification(
            data_dir,
            owner.to_hex(),
            "unavailable preview key".into(),
            serde_json::json!({"event":event}).to_string(),
        );
        assert_eq!(cached.should_show, visible);
        assert!(
            cached.body.ends_with(expected),
            "foreground consumption must retain the direct group preview: {}",
            cached.body
        );
        let payload: serde_json::Value = serde_json::from_str(&cached.payload_json).unwrap();
        assert_eq!(payload["chat_id"], chat_id);
    }
}

#[test]
fn private_group_wake_cold_preview_and_foreground_orders_preserve_exactly_one_message() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let body = "private group notification";
    farm.send(0, &chat_id, body);
    let (broadcast, wakes) = private_group_wake_events(&farm, &group_id);
    let shared_author = broadcast.pubkey;
    for index in 1..3 {
        let recipient = farm.devices[index].keys.public_key();
        let wake = wakes
            .iter()
            .find(|event| event.tags.public_keys().any(|key| *key == recipient))
            .expect("recipient-routed wake")
            .clone();
        let wire = serde_json::to_string(&wake).unwrap();
        assert!(!wire.contains(&farm.owners[0].public_key().to_hex()));
        assert!(!wire.contains(&farm.devices[0].keys.public_key().to_hex()));
        assert!(!wire.contains(&shared_author.to_hex()));
        assert!(!wire.contains(&group_id));

        let owner = farm.owners[index].clone();
        let device = farm.devices[index].keys.clone();
        let entry = &mut farm.devices[index];
        let subscriptions = entry.core.build_mobile_push_sync_snapshot();
        assert!(
            subscriptions
                .message_author_pubkeys
                .contains(&wake.pubkey.to_hex()),
            "private wakes must match the installed notification subscription"
        );
        assert!(!subscriptions
            .message_author_pubkeys
            .contains(&shared_author.to_hex()));
        let saved = runtime_state_json(&entry.core, &owner, &device);
        let payload = serde_json::json!({"event": wake}).to_string();
        for _ in 0..2 {
            let preview = decrypt_mobile_push_notification(
                entry._dir.path().to_string_lossy().into_owned(),
                owner.public_key().to_hex(),
                device.secret_key().to_secret_hex(),
                payload.clone(),
            );
            assert!(
                preview.should_show,
                "a valid group wake must render a cold preview"
            );
            assert_eq!(preview.body, body);
            let decoded: serde_json::Value = serde_json::from_str(&preview.payload_json).unwrap();
            assert_eq!(decoded["chat_id"], chat_id);
            assert_eq!(decoded["event"]["id"], wake.id.to_hex());
        }
        assert_eq!(
            runtime_state_json(&entry.core, &owner, &device),
            saved,
            "cold preview must not advance the saved live ratchet"
        );

        let order = if index == 1 {
            [wake.clone(), broadcast.clone()]
        } else {
            [broadcast.clone(), wake.clone()]
        };
        for event in order.into_iter().chain([wake.clone(), broadcast.clone()]) {
            entry.core.handle_relay_event(event);
        }
        entry.core.persist_best_effort();
        assert_eq!(
            entry.core.threads[&chat_id]
                .messages
                .iter()
                .filter(|message| message.body == body)
                .count(),
            1,
            "broadcast plus private wake must create one logical message in either order"
        );
        let after = decrypt_mobile_push_notification(
            entry._dir.path().to_string_lossy().into_owned(),
            owner.public_key().to_hex(),
            device.secret_key().to_secret_hex(),
            payload,
        );
        assert!(
            after.body.ends_with(body),
            "foreground consumption must retain the wake preview: {}",
            after.body
        );
    }
}

#[test]
fn private_group_wake_rejects_another_members_signature_and_malformed_inner_events() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    let body = "must not be injected by another member";
    farm.send(0, &chat_id, body);
    let (broadcast, _) = private_group_wake_events(&farm, &group_id);
    // Only the third account's authenticated DR delivery reaches the recipient.
    // The legitimate sender's original and private copies remain offline.
    farm.devices[0].online = false;
    let wrong_sender = farm.owners[2].public_key();
    let recipient = farm.owners[1].public_key();
    let ordinary = EventBuilder::new(Kind::TextNote, "not a group envelope")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    for content in [
        serde_json::json!({"v":1,"type":"group-wake","event":broadcast}).to_string(),
        serde_json::json!({"v":1,"type":"group-wake","event":ordinary}).to_string(),
        "{\"v\":1,\"type\":\"group-wake\",\"event\":{}}".into(),
    ] {
        let mut rumor = UnsignedEvent::new(
            wrong_sender,
            Timestamp::now(),
            Kind::Custom(super::private_group_wake::GROUP_WAKE_KIND as u16),
            [],
            content,
        );
        rumor.ensure_id();
        assert!(farm.devices[2]
            .core
            .send_protocol_engine_unsigned_event_to_peer_only(
                recipient,
                &chat_id,
                rumor,
                "private-group-wake-test",
            ));
        farm.pump();
        assert!(farm.devices[1].core.threads[&chat_id]
            .messages
            .iter()
            .all(|message| message.body != body));
        assert!(
            farm.devices[1]
                .core
                .threads
                .values()
                .flat_map(|thread| &thread.messages)
                .all(|message| !message.body.contains("group-wake")),
            "private controls must never appear as ordinary chat text"
        );
    }
}

#[test]
fn private_group_wake_outbox_failure_and_crash_before_ack_recover_the_complete_batch() {
    for committed_before_crash in [false, true] {
        let (mut farm, group_id, chat_id) = private_group_wake_farm();
        let core = &mut farm.devices[0].core;
        let result = core
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_group_payload(
                &group_id,
                b"durable delivery".to_vec(),
                Some("durable-group-message".into()),
            )
            .unwrap();
        let ids = result
            .effects
            .iter()
            .map(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                publish.event.id.to_hex()
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            ids.len(),
            2,
            "only two recipient-specific private deliveries"
        );
        let owner = farm.owners[0].clone();
        let device = farm.devices[0].keys.clone();
        let core = &mut farm.devices[0].core;
        if committed_before_crash {
            // Simulate death after committing the application transaction but
            // before acknowledging the protocol checkpoint.
            let rows = result
                .effects
                .iter()
                .map(|effect| {
                    let ProtocolEffect::Publish(publish) = effect;
                    (
                        PendingRelayPublish {
                            owner_pubkey_hex: owner.public_key().to_hex(),
                            event_id: publish.event.id.to_hex(),
                            label: APPCORE_PROTOCOL_LABEL.into(),
                            event_json: serde_json::to_string(&publish.event).unwrap(),
                            inner_event_id: publish.inner_event_id.clone(),
                            chat_id: Some(chat_id.clone()),
                            created_at_secs: publish.event.created_at.as_secs(),
                            attempt_count: 7,
                            last_error: Some("preserved retry state".into()),
                        },
                        publish.authored_at_secs,
                    )
                })
                .collect::<Vec<_>>();
            core.app_store
                .insert_pending_relay_publish_batch(&rows)
                .unwrap();
        } else {
            let shared = core.app_store.shared();
            shared.lock().unwrap().execute_batch(
                "CREATE TEMP TRIGGER reject_private_wake BEFORE INSERT ON pending_relay_publishes
                 WHEN json_extract(NEW.event_json, '$.kind') = 1060
                  AND EXISTS (SELECT 1 FROM json_each(NEW.event_json, '$.tags')
                              WHERE json_extract(value, '$[0]') = 'p')
                 BEGIN SELECT RAISE(ABORT, 'injected private wake failure'); END"
            ).unwrap();
            core.process_protocol_engine_effects(result.effects.clone());
            let rows = core
                .app_store
                .load_pending_relay_publishes(&owner.public_key().to_hex())
                .unwrap();
            assert!(
                rows.iter().all(|row| !ids.contains(&row.event_id)),
                "a failed private wake insert must roll back the shared event too"
            );
            assert!(
                ids.iter()
                    .all(|id| !core.pending_relay_publishes.contains_key(id)),
                "no carrier may publish a partial batch"
            );
            shared
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER reject_private_wake")
                .unwrap();
        }
        assert_eq!(
            core.protocol_engine
                .as_ref()
                .unwrap()
                .pending_group_publish_effects()
                .len(),
            2
        );
        core.persist_best_effort();

        let GroupScaleDevice {
            core: old_core,
            _dir: directory,
            ..
        } = farm.devices.remove(0);
        drop(old_core);
        let mut restarted = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            directory.path().to_string_lossy().into_owned(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        restarted
            .start_session(owner.public_key(), Some(owner.clone()), device, true, true)
            .unwrap();
        restarted.retry_protocol_engine_pending_work("private-group-wake-restart");
        let rows = restarted
            .app_store
            .load_pending_relay_publishes(&owner.public_key().to_hex())
            .unwrap();
        assert!(
            ids.iter()
                .all(|id| rows.iter().any(|row| &row.event_id == id)),
            "restart recovers the same complete signed batch"
        );
        assert!(
            restarted
                .protocol_engine
                .as_ref()
                .unwrap()
                .pending_group_publish_effects()
                .is_empty(),
            "only the committed application outbox may acknowledge protocol publications"
        );
        if committed_before_crash {
            assert!(
                rows.iter()
                    .filter(|row| ids.contains(&row.event_id))
                    .all(|row| row.attempt_count >= 7),
                "replayed protocol offers must not reset existing outbox attempts"
            );
        }
    }
}

#[test]
fn private_groups_never_publish_or_subscribe_shared_channels_or_rosters() {
    let (mut farm, group_id, chat_id) = private_group_wake_farm();
    farm.send(0, &chat_id, "private transport only");
    for entry in &farm.devices {
        let authors = entry
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .group_sender_event_pubkeys_for_group(&group_id);
        let plan = entry.core.compute_protocol_subscription_plan().unwrap();
        let filters = protocol::build_protocol_subscription_filters(&plan);
        let history = entry.core.recent_protocol_filters(unix_now());
        for filter in filters.iter().chain(&history) {
            let wire = serde_json::to_string(filter).unwrap();
            assert!(
                !wire.contains(&group_id),
                "a group identifier escaped into a network filter"
            );
            for author in &authors {
                assert!(
                    !wire.contains(&author.to_hex()),
                    "a shared group author escaped into a network filter"
                );
            }
        }
        for pending in entry.core.pending_relay_publishes.values() {
            let event: Event = serde_json::from_str(&pending.event_json).unwrap();
            assert!(!nostr_double_ratchet::is_group_roster_fact_event(&event));
            assert!(!authors.contains(&event.pubkey));
            if event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND {
                assert_eq!(event.tags.public_keys().count(), 1);
            }
        }
    }
}
