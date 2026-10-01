use crate::private_contact_sync::{
    create_private_contact_sync, edit_private_contact, open_private_contact_event,
    prepare_private_contact_event, private_contact_values,
};

fn private_contact_patch(
    value: serde_json::Value,
) -> crate::private_contact_sync::PrivateContactPatch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn private_contacts_encrypt_favorites_and_restore_exact_pending_bytes() {
    let mut pair = chat_read_sync_pair("private-favorite");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    let state = pair.a.private_contacts.state.as_ref().unwrap().clone();
    let record = &state.records[&id];
    let event = record.event.as_ref().unwrap();
    assert_eq!(event.kind, Kind::from(30078));
    assert!(!event.content.contains(&id));
    assert!(!event
        .tags
        .iter()
        .any(|tag| tag.as_slice().iter().any(|value| value == &id)));
    assert!(pair
        .a
        .pending_relay_publishes
        .contains_key(&event.id.to_hex()));
    assert_eq!(
        open_private_contact_event(event, &pair.owner.public_key().to_hex(), &pair.owner)
            .unwrap()
            .fields["favorite"]
            .value,
        true
    );
    let first = pair.a.owner_profiles[&id]
        .contact_memory
        .first_seen_name
        .clone();
    pair.a.private_contacts.state = None;
    let restored = pair.a.private_contact_state().unwrap();
    assert_eq!(restored.records[&id].event, Some(event.clone()));
    assert_eq!(
        pair.a.owner_profiles[&id].contact_memory.first_seen_name,
        first
    );
    pair.a
        .handle_relay_publish_finished(event.id.to_hex(), true, vec![], "accepted".into());
    assert!(!pair.a.private_contacts.state.as_ref().unwrap().records[&id].pending);
}

#[test]
fn private_contacts_linked_device_snapshot_and_control_converge_without_owner_key() {
    let mut pair = chat_read_sync_pair("private-linked");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    for core in [&mut pair.a, &mut pair.b] {
        core.handle_action(AppAction::CreateChat {
            peer_input: id.clone(),
        });
    }
    pair.b.logged_in.as_mut().unwrap().owner_keys = None;
    pair.b.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    assert!(pair.b.private_contacts.state.as_ref().unwrap().records[&id]
        .event
        .is_none());
    let snapshots = pair.b.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.a, &Keys::generate(), &snapshots);
    assert!(!pair.a.owner_profiles[&id].contact_memory.favorite);
    deliver_chat_read_packets(&mut pair.a, &pair.b_device, &snapshots);
    assert!(pair.a.owner_profiles[&id].contact_memory.favorite);
    assert!(pair.a.private_contacts.state.as_ref().unwrap().records[&id]
        .event
        .is_some());
    save_contact_details(&mut pair.a, &peer, "Tea friend", "Met at the park");
    let document = pair
        .a
        .private_contact_snapshot()
        .into_iter()
        .find(|document| document.contact == id)
        .unwrap();
    let content =
        serde_json::json!({"type":"private-contact-sync", "v":1, "document":document}).to_string();
    pair.b.receive_private_contact_control(
        pair.owner.public_key(),
        Some(Keys::generate().public_key()),
        &content,
    );
    assert!(pair.b.owner_profiles[&id].nickname.is_none());
    pair.b.receive_private_contact_control(
        pair.owner.public_key(),
        Some(pair.a_device.public_key()),
        &content,
    );
    assert_eq!(
        pair.b.owner_profiles[&id].nickname.as_deref(),
        Some("Tea friend")
    );
    assert!(pair.b.owner_profiles[&id].contact_memory.favorite);
}

#[test]
fn private_contacts_cross_app_signed_event_preserves_independent_edits_and_tombstones() {
    let mut pair = chat_read_sync_pair("private-cross-app");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    save_contact_details(&mut pair.a, &peer, "Native name", "Native note");
    let other =
        create_private_contact_sync(&pair.owner.public_key().to_hex(), &"b".repeat(32)).unwrap();
    let other = edit_private_contact(
        &other,
        &id,
        &private_contact_patch(serde_json::json!({"favorite":true})),
        Some(&"c".repeat(32)),
    )
    .unwrap();
    let prepared =
        prepare_private_contact_event(&other, &id, &pair.owner, unix_now().get()).unwrap();
    pair.a.handle_relay_event(prepared.event.unwrap());
    assert!(pair.a.owner_profiles[&id].contact_memory.favorite);
    assert_eq!(
        pair.a.owner_profiles[&id].nickname.as_deref(),
        Some("Native name")
    );
    let old = pair.a.private_contact_snapshot().remove(0);
    save_contact_details(&mut pair.a, &peer, "", "");
    pair.a.merge_private_contact_from_sibling(&old);
    assert!(pair.a.owner_profiles[&id].nickname.is_none());
    assert!(pair.a.owner_profiles[&id].contact_note.is_none());
    let state = pair.a.private_contacts.state.as_ref().unwrap();
    assert!(private_contact_values(state, &id).favorite);
}

#[test]
fn private_contacts_failed_durable_write_does_not_mutate_the_visible_favorite() {
    let mut pair = chat_read_sync_pair("private-failed-write");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.private_contact_state().unwrap();
    pair.a.app_store.shared().lock().unwrap().execute_batch("CREATE TRIGGER reject_private_contact BEFORE UPDATE ON app_meta WHEN NEW.key = 'private_contact_sync_v1' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
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
fn private_contacts_relay_subscription_recovers_another_app_and_acknowledges_local_edits() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("private-relay", &owner, &Keys::generate());
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    core.logged_in.as_mut().unwrap().relay_urls = relay_urls_from_strings(&[relay.url().into()]);
    core.preferences.nostr_relay_urls = vec![relay.url().into()];
    core.start_notifications_loop(core.logged_in.as_ref().unwrap().client.clone());
    let state = create_private_contact_sync(&owner.public_key().to_hex(), &"8".repeat(32)).unwrap();
    let state = edit_private_contact(
        &state,
        &peer,
        &private_contact_patch(
            serde_json::json!({"favorite":true,"note":"Private cross-app note"}),
        ),
        Some(&"9".repeat(32)),
    )
    .unwrap();
    let event = prepare_private_contact_event(&state, &peer, &owner, unix_now().get())
        .unwrap()
        .event
        .unwrap();
    publish_signer_test_event(&core, &relay, &event);
    core.start_private_contact_sync();
    core.schedule_session_connect();
    pump_signer_core_until(&mut core, &rx, |core| {
        core.owner_profiles
            .get(&peer)
            .is_some_and(|profile| profile.contact_memory.favorite)
    });
    assert_eq!(
        core.owner_profiles[&peer].contact_note.as_deref(),
        Some("Private cross-app note")
    );
    core.handle_action(AppAction::CreateChat {
        peer_input: peer.clone(),
    });
    core.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: peer.clone(),
        favorite: false,
    });
    pump_signer_core_until(&mut core, &rx, |core| {
        core.private_contacts.state.as_ref().is_some_and(|state| {
            state
                .records
                .get(&peer)
                .is_some_and(|record| !record.pending)
        })
    });
    let local = core.private_contacts.state.as_ref().unwrap().records[&peer]
        .event
        .as_ref()
        .unwrap();
    assert!(relay
        .events()
        .iter()
        .any(|event| event["id"] == local.id.to_hex()));
    assert!(!local.content.contains("Private cross-app note"));
    let restored = crate::private_contact_sync::restore_private_contact_sync(
        &core
            .app_store
            .load_private_contact_sync(&owner.public_key().to_hex())
            .unwrap()
            .unwrap(),
        &owner.public_key().to_hex(),
    )
    .unwrap();
    assert!(!private_contact_values(&restored, &peer).favorite);
    assert_eq!(
        private_contact_values(&restored, &peer).note.as_deref(),
        Some("Private cross-app note")
    );
}

#[test]
fn private_contacts_reject_other_owner_ciphertext_and_keep_large_imported_notes() {
    let mut pair = chat_read_sync_pair("private-owner-boundary");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    let other_owner = Keys::generate();
    let make = |owner: &Keys| {
        let state =
            create_private_contact_sync(&owner.public_key().to_hex(), &"8".repeat(32)).unwrap();
        let state = edit_private_contact(
            &state,
            &id,
            &private_contact_patch(serde_json::json!({"note":"n".repeat(4096)})),
            Some(&"9".repeat(32)),
        )
        .unwrap();
        prepare_private_contact_event(&state, &id, owner, unix_now().get())
            .unwrap()
            .event
            .unwrap()
    };
    pair.a.handle_relay_event(make(&other_owner));
    assert!(pair.a.owner_profiles[&id].contact_note.is_none());
    pair.a.handle_relay_event(make(&pair.owner));
    assert_eq!(
        pair.a.owner_profiles[&id]
            .contact_note
            .as_ref()
            .unwrap()
            .len(),
        4096
    );
    pair.a.handle_action(AppAction::SetContactNickname {
        owner_pubkey_hex: id.clone(),
        nickname: "Friend".into(),
    });
    assert_eq!(
        pair.a.owner_profiles[&id].nickname.as_deref(),
        Some("Friend")
    );
    assert_eq!(
        pair.a.owner_profiles[&id]
            .contact_note
            .as_ref()
            .unwrap()
            .len(),
        4096
    );
}
