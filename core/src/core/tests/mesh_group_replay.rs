// Offline members recover their own recipient-specific ratchet carrier from
// the sender's durable queue. Another group member never serves shared group traffic.
#[test]
fn private_group_recipient_delivery_survives_sender_restart_while_member_offline() {
    let mut farm = GroupScaleFarm::new(&[1, 1, 1]);
    farm.pump();
    let members = farm.owners[1..]
        .iter()
        .map(|owner| owner.public_key().to_hex())
        .collect::<Vec<_>>();
    farm.devices[0]
        .core
        .create_group("offline private group", &members);
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    let chat_id = group_chat_id(&group_id);
    farm.pump();
    for device in &mut farm.devices {
        device.core.accept_message_request(&chat_id);
    }
    farm.send(0, &chat_id, "private group bootstrap");
    farm.pump();
    farm.devices[2].online = false;
    let body = "attachment message while C is offline";
    let (message_id, _) = farm.send(0, &chat_id, body);
    let c_key = farm.devices[2].keys.public_key();
    let pending = farm.devices[0]
        .core
        .pending_relay_publishes
        .values()
        .filter(|pending| pending.inner_event_id.as_deref() == Some(message_id.as_str()))
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .collect::<Vec<_>>();
    assert_eq!(
        pending.len(),
        2,
        "each other member has its own private carrier"
    );
    let carrier = pending
        .iter()
        .find(|event| event.tags.public_keys().any(|key| *key == c_key))
        .expect("C's private ratchet carrier")
        .clone();
    let sender = &farm.devices[0].core;
    assert!(pending.iter().all(|event| !sender
        .protocol_engine
        .as_ref()
        .unwrap()
        .is_known_group_sender_event_author(event.pubkey)));
    let wire = serde_json::to_string(&carrier).unwrap();
    assert!(!wire.contains(&group_id));
    assert!(!wire.contains(&farm.owners[0].public_key().to_hex()));
    assert!(!wire.contains(&farm.devices[0].keys.public_key().to_hex()));
    let preview = mobile_push::preview_direct_messages(
        &farm.devices[2]._dir.path().to_string_lossy(),
        farm.owners[2].public_key(),
        &farm.devices[2].keys,
        std::slice::from_ref(&carrier),
    )
    .expect("inspect C's carrier without advancing the live ratchet");
    let rumor = parse_runtime_rumor(&preview.content).unwrap();
    let nested = private_group_wake::decode(&rumor.content).unwrap();
    assert!(!farm.devices[2].core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.id == message_id));
    assert!(
        !farm.history.iter().any(|event| event.id == carrier.id),
        "restart occurs before the recipient carrier has reached any relay"
    );

    let owner = farm.owners[0].clone();
    let keys = farm.devices[0].keys.clone();
    let path = farm.devices[0].core.data_dir.to_string_lossy().into_owned();
    farm.devices[0].core.preferences.nostr_relay_urls.clear();
    farm.devices[0].core.preferences.nearby_lan_enabled = false;
    farm.devices[0].core.preferences.nearby_bluetooth_enabled = false;
    farm.devices[0].core.persist_app_state().unwrap();
    farm.devices[0].core.stop_device_sync_now();
    let placeholder = logged_in_test_core("private-group-restart-placeholder", &owner, &keys);
    let previous = std::mem::replace(&mut farm.devices[0].core, placeholder);
    drop(previous);
    let mut restored = logged_in_test_core_at_data_dir(&owner, &keys, path);
    restored
        .start_session_inner(
            owner.public_key(),
            Some(owner.clone()),
            keys,
            true,
            true,
            true,
        )
        .unwrap();
    restored.stop_device_sync_now();
    let recovered = restored
        .pending_relay_publishes
        .get(&carrier.id.to_hex())
        .expect("sender restart retains C's exact queued private carrier");
    assert_eq!(
        serde_json::from_str::<Event>(&recovered.event_json).unwrap(),
        carrier
    );
    farm.devices[0].core = restored;
    farm.devices[2].online = true;
    farm.pump();
    for _ in 0..2 {
        farm.devices[2].core.handle_relay_event(carrier.clone());
    }
    let received = farm.devices[2].core.threads[&chat_id]
        .messages
        .iter()
        .filter(|message| message.id == message_id)
        .collect::<Vec<_>>();
    assert_eq!(
        received.len(),
        1,
        "restarted queue and duplicate delivery display exactly once"
    );
    assert_eq!(received[0].body, body);
    assert_eq!(
        received[0].source_event_id.as_deref(),
        Some(nested.id.to_hex().as_str())
    );
    assert_eq!(
        received[0].author_owner_pubkey_hex.as_deref(),
        Some(owner.public_key().to_hex().as_str())
    );
    assert!(!received[0].is_outgoing);
}
