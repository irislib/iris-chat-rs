use crate::private_contact_sync_v2::*;

#[test]
fn private_contact_recovery_ignores_secondary_relay_reconnects() {
    let mut pair = chat_read_receipt_pair("private-recovery-secondary-relay");
    let core = &mut pair.a;
    core.preferences.nostr_relay_urls = vec![
        "wss://stable.example".into(),
        "wss://flapping.example".into(),
    ];
    core.relay_status_by_url
        .insert("wss://stable.example".into(), RelayStatus::Connected);
    core.refresh_relay_connection_status_from_cached_statuses();
    let before = pending_publish_changes(core);
    for _ in 0..12 {
        for status in [RelayStatus::Connected, RelayStatus::Disconnected] {
            core.enter_batch();
            core.handle_relay_status_changed("wss://flapping.example".into(), status);
            core.exit_batch();
        }
    }
    assert_eq!(core.relay_connected_count, 1);
    assert_eq!(
        pending_publish_changes(core),
        before,
        "a secondary relay reconnect must not mutate ratchets or write new recovery messages"
    );
    assert!(pending_events_with_kind(core, MESSAGE_EVENT_KIND).is_empty());
}

#[test]
fn private_contact_recovery_coalesces_rapid_requests_and_still_converges() {
    let mut pair = chat_read_receipt_pair("private-recovery-coalescing");
    let contact = Keys::generate().public_key().to_hex();
    pair.b.edit_private_contact_fields(
        &contact,
        private_contact_patch(serde_json::json!({"note":"recovered after outage"})),
    );
    pair.b.pending_relay_publishes.clear();
    pair.a.recover_private_contact_history();
    assert!(!pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND).is_empty());
    let before = pending_publish_changes(&pair.a);
    for _ in 0..12 {
        pair.a.recover_private_contact_history();
    }
    assert_eq!(
        pending_publish_changes(&pair.a),
        before,
        "rapid foreground/reconnect recovery must reuse the durable request"
    );
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    deliver_pending_relay_events_for_test(&pair.b, &mut pair.a);
    assert_eq!(
        pair.a.owner_profiles[&contact].contact_note.as_deref(),
        Some("recovered after outage")
    );
}

#[test]
fn private_contact_recovery_retries_failed_admission_and_later_outages() {
    let mut pair = chat_read_receipt_pair("private-recovery-retry");
    let core = &mut pair.a;
    let engine = core.protocol_engine.take();
    core.recover_private_contact_history();
    assert!(core.private_contacts.last_recovery_request_at.is_none());
    core.protocol_engine = engine;
    core.preferences.nostr_relay_urls = vec!["wss://relay.example".into()];
    core.handle_relay_status_changed("wss://relay.example".into(), RelayStatus::Connected);
    assert_eq!(
        protocol_send_log_count(core, "private_contacts.self_sync_v2"),
        1
    );
    for _ in 0..3 {
        core.handle_relay_status_changed("wss://relay.example".into(), RelayStatus::Disconnected);
        core.handle_relay_status_changed("wss://relay.example".into(), RelayStatus::Connected);
    }
    assert_eq!(
        protocol_send_log_count(core, "private_contacts.self_sync_v2"),
        1
    );

    core.private_contacts.last_recovery_request_at = Some(Instant::now() - Duration::from_secs(61));
    core.handle_relay_status_changed("wss://relay.example".into(), RelayStatus::Disconnected);
    core.handle_relay_status_changed("wss://relay.example".into(), RelayStatus::Connected);
    assert_eq!(
        protocol_send_log_count(core, "private_contacts.self_sync_v2"),
        2
    );

    core.private_contacts.reset();
    core.recover_private_contact_history();
    assert_eq!(
        protocol_send_log_count(core, "private_contacts.self_sync_v2"),
        3
    );
}

fn private_contact_patch(value: serde_json::Value) -> PrivateContactPatchV2 {
    serde_json::from_value(value).unwrap()
}

// Public bootstrap invites use the same kind as retired private app records.
// Allow only verified empty-content invites, never sealed contact data.
fn assert_only_public_invites_pending(core: &AppCore) {
    for event in pending_events_with_kind(core, 30078) {
        assert!(
            event.content.is_empty(),
            "private ciphertext must not be published as app data"
        );
        assert!(
            nostr_double_ratchet::parse_invite_event(&event).is_ok(),
            "only a valid public ratchet invite may use this kind"
        );
        assert!(
            !event
                .tags
                .iter()
                .any(|tag| { tag.as_slice() == ["t", "nostr-social-memory/v1"] }),
            "retired contact records must never enter the public outbox"
        );
    }
}

#[test]
fn private_contacts_ratcheted_outbox_restores_and_linked_device_converges() {
    let mut pair = chat_read_receipt_pair("private-v2-outbox");
    pair.b.logged_in.as_mut().unwrap().owner_keys = None;
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    save_contact_details(&mut pair.a, &peer, "Tea friend", "Met at the park");
    let events = pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND);
    assert!(!events.is_empty());
    for event in &events {
        assert!(!event.content.contains("Tea friend"));
        assert!(!event.content.contains(&id));
        assert!(!event.content.contains("private-contact"));
    }
    assert_only_public_invites_pending(&pair.a);
    assert!(
        pair.a
            .private_contacts
            .state
            .as_ref()
            .unwrap()
            .pending
            .is_empty(),
        "durable DR handoff acknowledged"
    );
    pair.a.private_contacts.state = None;
    let restored = pair.a.private_contact_state().unwrap();
    assert!(private_contact_values_v2(&restored, &id).favorite);
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    assert!(pair.b.owner_profiles[&id].contact_memory.favorite);
    assert!(pair.b.social_connection(&id).unwrap().is_favorite);
    assert_eq!(
        pair.b.owner_profiles[&id].nickname.as_deref(),
        Some("Tea friend")
    );
    assert_eq!(
        pair.b.owner_profiles[&id].contact_note.as_deref(),
        Some("Met at the park")
    );
    assert!(pair.b.threads.is_empty(), "private control creates no chat");
    let old = pair.b.private_contact_snapshot().remove(0);
    pair.b.edit_private_contact_fields(
        &id,
        private_contact_patch(serde_json::json!({"favorite":false,"note":null})),
    );
    deliver_pending_relay_events_for_test(&pair.b, &mut pair.a);
    pair.a.merge_private_contact_from_sibling(&old);
    assert!(!pair.a.owner_profiles[&id].contact_memory.favorite);
    assert!(!pair
        .a
        .social_connection(&id)
        .is_some_and(|connection| connection.is_favorite));
    assert!(pair.a.owner_profiles[&id].contact_note.is_none());
    assert_eq!(
        pair.a.owner_profiles[&id].nickname.as_deref(),
        Some("Tea friend")
    );
}

#[test]
fn private_contacts_snapshot_uses_v2_fields_and_rejects_legacy_and_foreign_controls() {
    let mut pair = chat_read_sync_pair("private-v2-snapshot");
    let id = Keys::generate().public_key().to_hex();
    pair.b.edit_private_contact_fields(
        &id,
        private_contact_patch(serde_json::json!({"favorite":true,"note":"private"})),
    );
    let snapshots = pair.b.build_device_sync_packets_for_test(100, false);
    for packet in &snapshots {
        let json: serde_json::Value = serde_json::from_slice(packet).unwrap();
        assert!(json.get("privateContacts").is_none());
        for chat in json["chats"].as_array().unwrap() {
            assert!(chat.get("contactDetails").is_none());
        }
    }
    deliver_chat_read_packets(&mut pair.a, &Keys::generate(), &snapshots);
    assert!(!pair.a.owner_profiles.contains_key(&id));
    deliver_chat_read_packets(&mut pair.a, &pair.b_device, &snapshots);
    assert!(pair.a.owner_profiles[&id].contact_memory.favorite);
    let document = pair.b.private_contact_snapshot().remove(0);
    let mut changed = document.clone();
    changed.fields.get_mut("note").unwrap().value = serde_json::json!("foreign edit");
    changed.fields.get_mut("note").unwrap().counter += 1;
    for content in [
        serde_json::json!({"type":"private-contact-sync","v":1,"document":changed}),
        serde_json::json!({"type":"private-contact-sync","v":2,"document":changed,"request":true}),
    ] {
        pair.a.receive_private_contact_control(
            pair.owner.public_key(),
            Some(pair.b_device.public_key()),
            &content.to_string(),
        );
    }
    let content =
        serde_json::to_string(&build_private_contact_control_v2(&changed).unwrap()).unwrap();
    pair.a.receive_private_contact_control(
        pair.owner.public_key(),
        Some(Keys::generate().public_key()),
        &content,
    );
    assert_eq!(
        pair.a.owner_profiles[&id].contact_note.as_deref(),
        Some("private")
    );
}

#[test]
fn private_contacts_migrate_saved_registers_but_never_read_or_publish_legacy_ciphertext() {
    use crate::private_contact_sync as old;
    let mut pair = chat_read_sync_pair("private-v2-migration");
    let owner = pair.owner.public_key().to_hex();
    let id = Keys::generate().public_key().to_hex();
    let legacy = old::create_private_contact_sync(&owner, &"a".repeat(32)).unwrap();
    let legacy = old::edit_private_contact(
        &legacy,
        &id,
        &private_contact_patch(serde_json::json!({"favorite":true,"note":"legacy saved"})),
        Some(&"b".repeat(32)),
    )
    .unwrap();
    let prepared =
        old::prepare_private_contact_event(&legacy, &id, &pair.owner, unix_now().get()).unwrap();
    let sealed = prepared.event.unwrap();
    pair.a.handle_relay_event(sealed.clone());
    assert!(!pair.a.owner_profiles.contains_key(&id));
    pair.a
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM app_meta WHERE key = 'private_contact_sync_v2'",
            [],
        )
        .unwrap();
    pair.a
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_meta(key,value) VALUES('private_contact_sync_v1',?1)",
            [serde_json::to_string(&prepared.state).unwrap()],
        )
        .unwrap();
    pair.a.private_contacts.state = None;
    let migrated = pair.a.private_contact_state().unwrap();
    assert_eq!(migrated.version, 2);
    assert_eq!(migrated.clock, legacy.clock);
    assert_eq!(
        migrated.contacts[&id]["note"].counter,
        legacy.contacts[&id]["note"].counter
    );
    assert_eq!(
        pair.a.owner_profiles[&id].contact_note.as_deref(),
        Some("legacy saved")
    );
    assert!(!pair
        .a
        .publish_runtime_event(sealed, "private-contact-sync", None));
    let saved = pair
        .a
        .app_store
        .load_private_contact_sync(&owner)
        .unwrap()
        .unwrap();
    assert!(!saved.contains("ciphertext"));
    assert!(!saved.contains("30078"));
}

#[test]
fn private_contact_facts_and_inner_controls_never_enter_public_outboxes() {
    let mut pair = chat_read_sync_pair("private-fact-publication-guard");
    let before = pair.a.pending_relay_publishes.len();
    for kind in [7368, 37368, 10454, 10455, 10456] {
        let event = EventBuilder::new(Kind::from(kind as u16), "private data")
            .tags([nostr::Tag::parse(["type", "private_contact"]).unwrap()])
            .sign_with_keys(&pair.owner)
            .unwrap();
        assert!(!pair.a.publish_runtime_event(event, "private-fact", None));
    }
    assert_eq!(pair.a.pending_relay_publishes.len(), before);
}

#[test]
fn private_contacts_failed_durable_write_does_not_mutate_visible_favorite() {
    let mut pair = chat_read_sync_pair("private-v2-failed-write");
    let id = Keys::generate().public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.private_contact_state().unwrap();
    pair.a.app_store.shared().lock().unwrap().execute_batch("CREATE TRIGGER reject_private_contact BEFORE UPDATE ON app_meta WHEN NEW.key = 'private_contact_sync_v3' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    assert!(!pair.a.owner_profiles[&id].contact_memory.favorite);
    assert!(pair
        .a
        .state
        .toast
        .as_deref()
        .unwrap()
        .contains("Could not save"));
}

#[test]
fn private_contacts_snapshot_request_replays_current_facts_and_notification_mutes() {
    let mut pair = chat_read_receipt_pair("private-v2-recovery");
    let id = Keys::generate().public_key().to_hex();
    pair.a.edit_private_contact_fields(
        &id,
        private_contact_patch(serde_json::json!({"favorite":true,"note":"recovered"})),
    );
    pair.a.set_synced_chat_mute(&id, Some(0));
    pair.a.pending_relay_publishes.clear();
    let request = serde_json::to_string(
        &build_private_contact_request_v2(&pair.owner.public_key().to_hex()).unwrap(),
    )
    .unwrap();
    pair.a.receive_private_contact_control(
        pair.owner.public_key(),
        Some(pair.b_device.public_key()),
        &request,
    );
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    assert!(pair.b.owner_profiles[&id].contact_memory.favorite);
    assert!(pair.b.is_chat_muted(&id));
    assert!(pair.b.threads.is_empty());
}

#[test]
fn device_labels_v2_authenticate_membership_preserve_tombstones_and_hide_legacy_snapshot_fields() {
    let mut pair = chat_read_receipt_pair("labels-v2");
    pair.a
        .set_current_device_labels("Kitchen tablet", "Iris Chat");
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    let owner = pair.owner.public_key().to_hex();
    let device = pair.a_device.public_key().to_hex();
    let label = |core: &AppCore| {
        core.app_keys[&owner]
            .devices
            .iter()
            .find(|entry| entry.identity_pubkey_hex == device)
            .unwrap()
            .device_label
            .clone()
    };
    assert_eq!(label(&pair.b).as_deref(), Some("Kitchen tablet"));
    let timestamp = unix_now().get() + 2;
    let clear = serde_json::json!({"type":"device-labels","v":2,"owner":owner,"device":device,"deviceLabel":null,"clientLabel":null,"updatedAtSecs":timestamp}).to_string();
    pair.b.receive_private_device_label_control(
        pair.owner.public_key(),
        Some(Keys::generate().public_key()),
        &clear,
    );
    assert!(label(&pair.b).is_some());
    pair.b.receive_private_device_label_control(
        pair.owner.public_key(),
        Some(pair.a_device.public_key()),
        &clear,
    );
    assert!(label(&pair.b).is_none());
    pair.b.persist_best_effort();
    let packets = pair.b.build_device_sync_packets_for_test(100, false);
    let json = String::from_utf8(packets.concat()).unwrap();
    assert!(json.contains("privateDeviceLabelsV2"));
    for packet in packets {
        let value: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        for roster in value["appKeys"].as_array().unwrap() {
            for entry in roster["devices"].as_array().unwrap() {
                assert!(entry.get("deviceLabel").is_none());
                assert!(entry.get("clientLabel").is_none());
                assert!(entry.get("labelUpdatedAt").is_none());
            }
        }
    }
}

#[test]
fn private_contacts_fact_storage_migrates_clears_and_pending_without_reclocking() {
    let mut pair = chat_read_sync_pair("private-fact-migration");
    let owner = pair.owner.public_key().to_hex();
    let contact = Keys::generate().public_key().to_hex();
    let state = create_private_contact_sync_v2(&owner, &"1".repeat(32)).unwrap();
    let state = edit_private_contact_v2(
        &state,
        &contact,
        &BTreeMap::from([
            ("nickname".into(), serde_json::json!("old")),
            ("note".into(), serde_json::json!("note")),
        ]),
    )
    .unwrap();
    let state = edit_private_contact_v2(
        &state,
        &contact,
        &BTreeMap::from([("nickname".into(), serde_json::Value::Null)]),
    )
    .unwrap();
    pair.a
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO app_meta(key,value) VALUES ('private_contact_sync_v2',?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
    pair.a.private_contacts.state = None;
    let migrated = pair.a.restore_private_contact_state(&owner).unwrap();
    assert_eq!(migrated, state);
    let saved: serde_json::Value = serde_json::from_str(
        &pair
            .a
            .app_store
            .load_private_contact_sync(&owner)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["version"], 4);
    assert!(saved["facts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|fact| fact.get("sig").is_none()));
    pair.a.private_contacts.state = None;
    assert_eq!(pair.a.restore_private_contact_state(&owner).unwrap(), state);
    for fact in pair.a.private_contact_facts() {
        assert!(pair.b.merge_private_contact_fact(&fact));
    }
    let right = pair.b.private_contact_state().unwrap();
    assert_eq!(right.contacts, state.contacts);
    let mut forged = pair.a.private_contact_facts()[0].clone();
    forged.pubkey = "f".repeat(64);
    assert!(!pair.b.merge_private_contact_fact(&forged));
}

#[test]
fn private_contacts_negentropy_repairs_dropped_snapshots_and_exports_interop() {
    for capable in [false, true] {
        let mut pair = chat_read_sync_pair("private-facts-negentropy");
        for known in pair.a.app_keys.values_mut() {
            known
                .devices
                .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
        }
        pair.b.app_keys = pair.a.app_keys.clone();
        let owner = pair.owner.public_key().to_hex();
        let contact = Keys::generate().public_key().to_hex();
        pair.a.edit_private_contact_fields(
            &contact,
            BTreeMap::from([
                ("nickname".into(), serde_json::json!("Original")),
                ("note".into(), serde_json::json!("Only my devices")),
            ]),
        );
        pair.a.edit_private_contact_fields(
            &contact,
            BTreeMap::from([("nickname".into(), serde_json::Value::Null)]),
        );
        pair.b.edit_private_contact_fields(
            &contact,
            BTreeMap::from([("favorite".into(), serde_json::json!(true))]),
        );
        let proof_owner = Keys::generate();
        let proof = AppKeys::new(vec![DeviceEntry::new(Keys::generate().public_key(), 100)])
            .get_event_at(proof_owner.public_key(), 100)
            .sign_with_keys(&proof_owner)
            .unwrap();
        assert!(pair.a.apply_app_keys_event(&proof).unwrap());
        let other = Keys::generate().public_key().to_hex();
        pair.a
            .merge_chat_pin(ChatPinState {
                chat_id: contact.clone(),
                pinned: false,
                updated_at_ms: 200,
            })
            .unwrap();
        pair.b
            .merge_chat_mute(ChatMuteState {
                chat_id: contact.clone(),
                until_secs: None,
                updated_at_ms: 300,
            })
            .unwrap();
        pair.a.apply_chat_deletion(&contact, 100);
        pair.b.apply_chat_deletion(&other, 100);

        let endpoint = Arc::new(
            pair.a
                .runtime
                .block_on(
                    fips_core::FipsEndpoint::builder()
                        .without_system_tun()
                        .bind(),
                )
                .unwrap(),
        );
        let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        pair.a.install_device_sync_sender_for_test(
            endpoint.clone(),
            left_tx,
            vec![test_fips_peer(&pair.b_device)],
        );
        pair.b.install_device_sync_sender_for_test(
            endpoint,
            right_tx,
            vec![test_fips_peer(&pair.a_device)],
        );
        pair.a.set_device_history_record_limit_for_test(1);
        pair.b.set_device_history_record_limit_for_test(1);
        let request = serde_json::to_vec(&serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1,"privateFacts":capable.then_some(2),"privateEvents":capable.then_some(2)})).unwrap();
        pair.a.handle_device_sync_packet(
            &pair.b_device.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &request,
        );
        pair.b.handle_device_sync_packet(
            &pair.a_device.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &request,
        );
        let mut facts_sent = 0;
        let mut trace = Vec::new();
        for round in 0..2048 {
            let mut progress = false;
            for direction in [false, true] {
                let (source, key, target, target_key, receiver) = if direction {
                    (
                        &mut pair.b,
                        &pair.b_device,
                        &mut pair.a,
                        &pair.a_device,
                        &right_rx,
                    )
                } else {
                    (
                        &mut pair.a,
                        &pair.a_device,
                        &mut pair.b,
                        &pair.b_device,
                        &left_rx,
                    )
                };
                let mut packets = Vec::new();
                while let Ok(batch) = receiver.try_recv() {
                    packets.extend(batch.records);
                }
                if let Some(packet) =
                    source.take_device_sync_control_for_test(test_fips_peer(target_key))
                {
                    packets.push(packet);
                }
                progress |= !packets.is_empty();
                for packet in packets {
                    let mut value: serde_json::Value = serde_json::from_slice(&packet).unwrap();
                    if value["type"] == "snapshot" {
                        for field in [
                            "appKeys",
                            "privateContactsV2",
                            "chatPins",
                            "chatMutes",
                            "deletedChats",
                            "privateDeviceLabelsV2",
                            "chats",
                        ] {
                            value.as_object_mut().unwrap().remove(field);
                        }
                    }
                    facts_sent += value["records"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|record| record["type"] == "privateContactFact")
                        .count();
                    trace.push(value.clone());
                    target.handle_device_sync_packet(
                        &key.public_key().to_hex(),
                        DEVICE_SYNC_PORT,
                        &serde_json::to_vec(&value).unwrap(),
                    );
                }
            }
            if !progress {
                break;
            }
            assert!(round < 2047);
        }
        if !capable {
            assert_eq!(facts_sent, 0);
            continue;
        }
        assert!(facts_sent > 0, "no private records: {trace:?}");
        assert!(pair
            .b
            .private_state_fixture_for_test()
            .iter()
            .any(|entry| entry["record"]["type"] == "authorization"
                && entry["id"] == proof.id.to_hex()));
        assert!(pair
            .b
            .app_keys
            .contains_key(&proof_owner.public_key().to_hex()));
        for core in [&pair.a, &pair.b] {
            assert_eq!(core.chat_deletions.get(&contact), Some(&100));
            assert_eq!(core.chat_deletions.get(&other), Some(&100));
            assert!(!core.chat_pin_states[&contact].pinned);
            assert_eq!(core.chat_mute_states[&contact].until_secs, None);
        }

        assert_eq!(
            pair.a.private_contact_state().unwrap().contacts,
            pair.b.private_contact_state().unwrap().contacts
        );
        assert_eq!(pair.b.owner_profiles[&contact].nickname, None);
        assert_eq!(
            pair.b.owner_profiles[&contact].contact_note.as_deref(),
            Some("Only my devices")
        );
        pair.a
            .write_name_approval(
                &contact,
                "Old name".into(),
                crate::contact_memory::AcceptedNameChange {
                    previous_name: "Old name".into(),
                    accepted_name: "New name".into(),
                    accepted_at_secs: unix_now().get(),
                },
                false,
            )
            .unwrap();
        pair.a.handle_action(AppAction::SetUserBlocked {
            owner_pubkey_hex: contact.clone(),
            blocked: true,
        });
        if let Ok(path) = std::env::var("IRIS_PRIVATE_STATE_FIXTURE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"owner":owner,"contact":contact,"records":pair.a.private_state_fixture_for_test()})).unwrap()).unwrap();
        }
    }
}
