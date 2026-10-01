fn saved_sibling_rumors(core: &AppCore, owner: &Keys, device: &Keys) -> Vec<UnsignedEvent> {
    use base64::Engine;
    let storage = SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    );
    let raw = storage
        .get("appcore/protocol-engine-state-v1")
        .unwrap()
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    state["pending_local_sibling_sends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pending| {
            let bytes: Vec<u8> = serde_json::from_value(pending["payload"].clone()).unwrap();
            let wrapper: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let payload = base64::engine::general_purpose::STANDARD
                .decode(wrapper["payload"].as_str().unwrap())
                .unwrap();
            serde_json::from_slice(&payload).unwrap()
        })
        .collect()
}

#[test]
fn private_contact_upgrade_retires_plaintext_intents_only_after_durable_migration() {
    use crate::private_contact_sync as old;
    for known_roster in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
        let (mut core, _, _dir) =
            logged_in_test_core_with_updates("contact-upgrade", &owner, &device);
        core.app_store.bind_account(owner.public_key()).unwrap();
        let contact = Keys::generate().public_key().to_hex();
        let legacy =
            old::create_private_contact_sync(&owner.public_key().to_hex(), &"a".repeat(32))
                .unwrap();
        let legacy = old::edit_private_contact(
            &legacy,
            &contact,
            &private_contact_patch(serde_json::json!({"favorite":true,"note":"retained locally"})),
            Some(&"b".repeat(32)),
        )
        .unwrap();
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_meta(key,value) VALUES('private_contact_sync_v1',?1)",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();
        let roster = AppKeys::new(vec![
            DeviceEntry::new(device.public_key(), 1),
            DeviceEntry::new(sibling.public_key(), 1),
        ])
        .get_event_at(owner.public_key(), 1)
        .sign_with_keys(&owner)
        .unwrap();
        let engine = core.protocol_engine.as_mut().unwrap();
        if known_roster {
            engine.ingest_app_keys_event(&roster).unwrap();
        }
        let controls = [
            serde_json::json!({"type":"private-contact-sync","v":1,"request":true}),
            serde_json::json!({"type":"private-contact-sync","v":1,"document":legacy.records[&contact].document}),
            serde_json::json!({"type":"unrelated","v":1,"request":true}),
            serde_json::json!({"type":"private-contact-sync","v":3,"request":true}),
        ];
        for body in controls {
            let rumor =
                EventBuilder::new(Kind::from(10451), body.to_string()).build(owner.public_key());
            let sent = engine
                .send_local_sibling_unsigned_event(
                    owner.public_key(),
                    &owner.public_key().to_hex(),
                    rumor,
                    unix_now(),
                )
                .unwrap();
            assert!(sent.effects.is_empty());
        }
        assert_eq!(saved_sibling_rumors(&core, &owner, &device).len(), 4);
        // Fail only the retirement write, not engine loading or V2 data migration.
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_contact_retirement BEFORE UPDATE ON ndr_kv
             WHEN NEW.key = 'appcore/protocol-engine-state-v1'
             AND json_array_length(json_extract(NEW.value, '$.pending_local_sibling_sends')) < 4
             BEGIN SELECT RAISE(FAIL, 'injected retirement failure'); END;",
            )
            .unwrap();
        assert!(core
            .start_session(
                owner.public_key(),
                Some(owner.clone()),
                device.clone(),
                true,
                true,
            )
            .is_err());
        assert!(
            core.protocol_engine.is_none(),
            "failed barrier exposes no sending engine"
        );
        assert!(core.logged_in.is_none());
        assert!(core.pending_relay_publishes.is_empty());
        assert_eq!(saved_sibling_rumors(&core, &owner, &device).len(), 4);
        let saved = core
            .app_store
            .load_private_contact_sync(&owner.public_key().to_hex())
            .unwrap()
            .unwrap();
        let migrated = restore_private_contact_sync_v2(
            &serde_json::from_str(&saved).unwrap(),
            &owner.public_key().to_hex(),
        )
        .unwrap();
        assert_eq!(
            migrated.pending.len(),
            1,
            "migration survives failed retirement"
        );
        assert_eq!(migrated.clock, legacy.clock);
        assert_eq!(
            private_contact_values_v2(&migrated, &contact)
                .note
                .as_deref(),
            Some("retained locally")
        );
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_contact_retirement;")
            .unwrap();

        // Reload the real persisted engine and execute the same startup barrier.
        let storage = Arc::new(SqliteStorageAdapter::new(
            core.app_store.shared(),
            owner.public_key().to_hex(),
            device.public_key().to_hex(),
        ));
        let mut restored = ProtocolEngine::load_or_create_for_local_device(
            storage.clone(),
            owner.public_key(),
            &device,
        )
        .unwrap();
        core.retire_legacy_private_contact_intents(&mut restored, owner.public_key())
            .unwrap();
        assert_eq!(saved_sibling_rumors(&core, &owner, &device).len(), 2);
        core.protocol_engine = Some(restored);
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });
        core.start_private_contact_sync();
        let queued = saved_sibling_rumors(&core, &owner, &device);
        assert_eq!(queued.len(), 4);
        assert_eq!(
            queued
                .iter()
                .filter(|event| event.kind.as_u16() == 10452)
                .count(),
            2,
            "startup queues the V2 recovery request and migrated document"
        );
        assert!(
            core.private_contact_state().unwrap().pending.is_empty(),
            "durable handoff acknowledged"
        );
        let mut restored =
            ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
                .unwrap();
        restored
            .authenticate_local_owner_for_sending(&owner)
            .unwrap();
        let receiver_store = Arc::new(iris_chat_protocol::InMemoryStorage::new());
        let mut receiver = ProtocolEngine::load_or_create_for_local_device(
            receiver_store,
            owner.public_key(),
            &sibling,
        )
        .unwrap();
        restored.ingest_app_keys_event(&roster).unwrap();
        receiver.ingest_app_keys_event(&roster).unwrap();
        let invite = nostr_double_ratchet::invite_unsigned_event(&receiver.local_invite().unwrap())
            .unwrap()
            .sign_with_keys(&sibling)
            .unwrap();
        let batch = restored.observe_invite_event(&invite).unwrap();
        let mut messages = Vec::new();
        for ProtocolEffect::Publish(publish) in batch.effects {
            if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
                messages.extend(
                    receiver
                        .observe_invite_response_event(&publish.event)
                        .unwrap()
                        .direct_messages,
                );
            } else {
                messages.extend(
                    receiver
                        .process_direct_message_event(&publish.event)
                        .unwrap(),
                );
            }
        }
        let rumors: Vec<UnsignedEvent> = messages
            .iter()
            .map(|message| serde_json::from_str(&message.content).unwrap())
            .collect();
        assert_eq!(
            rumors.len(),
            4,
            "only V2 recovery/data and unrelated intents are delivered"
        );
        let controls = rumors
            .iter()
            .filter(|event| event.kind.as_u16() == 10452)
            .map(|event| {
                parse_private_contact_control_v2(
                    &serde_json::from_str(&event.content).unwrap(),
                    &owner.public_key().to_hex(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(controls
            .iter()
            .any(|control| matches!(control, PrivateContactControlV2::Request { .. })));
        let document = controls
            .into_iter()
            .find_map(|control| match control {
                PrivateContactControlV2::Sync { document, .. } => Some(document),
                PrivateContactControlV2::Request { .. } => None,
            })
            .expect("migrated document delivered");
        assert_eq!(document.fields, migrated.contacts[&contact]);
    }
}

#[test]
fn private_contact_decrypted_journal_waits_for_both_devices_after_restart() {
    let mut pair = chat_read_receipt_pair("private-contact-journal-roster");
    let contact = Keys::generate().public_key().to_hex();
    pair.a.edit_private_contact_fields(
        &contact,
        private_contact_patch(serde_json::json!({"favorite":true,"note":"retry after roster"})),
    );
    for event in pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND) {
        pair.b
            .protocol_engine
            .as_mut()
            .unwrap()
            .process_direct_message_event(&event)
            .unwrap();
    }
    let journal_len = pair
        .b
        .protocol_engine
        .as_ref()
        .unwrap()
        .pending_decrypted_deliveries_len_for_test();
    assert!(journal_len > 0);
    let storage = Arc::new(SqliteStorageAdapter::new(
        pair.b.app_store.shared(),
        pair.owner.public_key().to_hex(),
        pair.b_device.public_key().to_hex(),
    ));
    pair.b.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(
            storage,
            pair.owner.public_key(),
            &pair.b_device,
        )
        .unwrap(),
    );
    let owner = pair.owner.public_key().to_hex();
    let roster = pair.b.app_keys[&owner].clone();
    for missing in [pair.a_device.public_key(), pair.b_device.public_key()] {
        let mut stale = roster.clone();
        stale
            .devices
            .retain(|device| device.identity_pubkey_hex != missing.to_hex());
        pair.b.app_keys.insert(owner.clone(), stale);
        for kind in [10449, 10450, 10452, 10453] {
            assert!(pair.b.private_sibling_control_waits_for_roster(
                pair.owner.public_key(),
                Some(pair.a_device.public_key()),
                kind,
            ));
        }
        pair.b
            .retry_protocol_engine_pending_work("stale_private_roster");
        assert_eq!(
            pair.b
                .protocol_engine
                .as_ref()
                .unwrap()
                .pending_decrypted_deliveries_len_for_test(),
            journal_len
        );
        assert!(!pair.b.owner_profiles.contains_key(&contact));
        assert!(pair.b.pending_decrypted_delivery_acks.is_empty());
    }
    pair.b.app_keys.insert(owner, roster);
    pair.b
        .retry_protocol_engine_pending_work("private_roster_caught_up");
    assert!(pair.b.owner_profiles[&contact].contact_memory.favorite);
    assert_eq!(
        pair.b.owner_profiles[&contact].contact_note.as_deref(),
        Some("retry after roster")
    );
    assert_eq!(
        pair.b
            .protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        0
    );
    let saved = pair
        .b
        .app_store
        .load_private_contact_sync(&pair.owner.public_key().to_hex())
        .unwrap()
        .unwrap();
    assert!(restore_private_contact_sync_v2(
        &serde_json::from_str(&saved).unwrap(),
        &pair.owner.public_key().to_hex()
    )
    .unwrap()
    .contacts
    .contains_key(&contact));
    assert!(
        !pair.b.private_sibling_control_waits_for_roster(
            Keys::generate().public_key(),
            Some(pair.a_device.public_key()),
            10452,
        ),
        "foreign owner remains a permanent rejection, never a roster wait"
    );
}
