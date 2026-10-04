use crate::private_contact_sync_v2::*;

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
fn private_contacts_failed_durable_write_does_not_mutate_visible_favorite() {
    let mut pair = chat_read_sync_pair("private-v2-failed-write");
    let id = Keys::generate().public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.private_contact_state().unwrap();
    pair.a.app_store.shared().lock().unwrap().execute_batch("CREATE TRIGGER reject_private_contact BEFORE UPDATE ON app_meta WHEN NEW.key = 'private_contact_sync_v2' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
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
