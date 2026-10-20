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
