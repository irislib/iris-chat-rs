// Match the physical LAN failure: all three members already have the group
// and A's sender key. C misses one later message; only B can supply it after
// restart. The deterministic bootstrap is never used as a catch-up source.
#[test]
fn mesh_group_received_ciphertext_survives_provider_restart_with_sender_offline() {
    use fips_core::config::WebSocketConfig;

    let mut farm = GroupScaleFarm::new(&[1, 1, 1]);
    farm.pump();
    let members = farm.owners[1..]
        .iter()
        .map(|owner| owner.public_key().to_hex())
        .collect::<Vec<_>>();
    farm.devices[0].core.create_group("offline group replay", &members);
    let group = farm.devices[0].core.groups.keys().next().unwrap().clone();
    let chat = group_chat_id(&group);
    farm.pump();
    for device in &mut farm.devices {
        device.core.accept_message_request(&chat);
    }
    let (bootstrap_id, _) = farm.send(0, &chat, "sender key bootstrap");
    farm.pump();
    farm.assert_messages(&chat, &[(bootstrap_id, "sender key bootstrap".into())]);

    farm.devices[2].online = false;
    let body = "attachment message while C is offline";
    let (message_id, _) = farm.send(0, &chat, body);
    let outer = farm.devices[0]
        .core
        .pending_relay_publishes
        .values()
        .filter(|pending| pending.inner_event_id.as_deref() == Some(message_id.as_str()))
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .find(|event| parse_group_sender_key_message_event_unchecked(event).is_ok())
        .expect("the original signed group ciphertext");
    farm.devices[1]
        .core
        .handle_internal(InternalEvent::MeshEvent(outer.clone()));
    assert!(farm.devices[1].core.threads[&chat]
        .messages.iter().any(|message| message.id == message_id));
    assert!(!farm.devices[2].core.threads[&chat]
        .messages.iter().any(|message| message.id == message_id));
    assert!(!farm.devices[1].core.pending_relay_publishes.contains_key(&outer.id.to_hex()),
        "B must not become the author or use A's outbox");

    for device in &mut farm.devices {
        device.core.preferences.nostr_relay_urls.clear();
        device.core.preferences.nearby_lan_enabled = false;
        device.core.preferences.nearby_bluetooth_enabled = false;
        device.core.persist_app_state().unwrap();
        device.core.stop_device_sync_now();
    }
    let c = farm.devices.pop().unwrap();
    let b = farm.devices.pop().unwrap();
    let b_path = b.core.data_dir.to_string_lossy().into_owned();
    let c_path = c.core.data_dir.to_string_lossy().into_owned();
    let b_keys = b.keys.clone();
    let c_keys = c.keys.clone();
    let owners = farm.owners.clone();
    // Preserve only B/C's directories; A and every bootstrap/relay history
    // source are gone before either receiving runtime is restarted.
    let (_b_dir, _c_dir) = (b._dir, c._dir);
    drop(b.core);
    drop(c.core);
    drop(farm);

    let restart = |index: usize, keys: &Keys, path: String| {
        let mut core = logged_in_test_core_at_data_dir(&owners[index], keys, path);
        let (sender, receiver) = flume::unbounded();
        core.core_sender = sender;
        core.start_session_inner(owners[index].public_key(), Some(owners[index].clone()),
            keys.clone(), true, true, true).unwrap();
        (core, receiver)
    };
    let (mut b, b_rx) = restart(1, &b_keys, b_path);
    let (mut c, c_rx) = restart(2, &c_keys, c_path);
    assert!(b.threads[&chat].messages.iter().any(|message| message.id == message_id));
    assert!(!c.threads[&chat].messages.iter().any(|message| message.id == message_id));
    let address = reserve_tcp_addr();
    b.reconcile_device_sync_with_isolated_websocket_for_test(WebSocketConfig {
        bind_addr: Some(address.to_string()), ..Default::default()
    });
    c.reconcile_device_sync_with_isolated_websocket_for_test(WebSocketConfig {
        seed_urls: vec![format!("ws://{address}/fips")], ..Default::default()
    });
    assert!(!b.device_sync_has_sibling_tcp_for_test());
    assert!(!c.device_sync_has_sibling_tcp_for_test());
    let deadline = Instant::now() + Duration::from_secs(20);
    let received = loop {
        for (core, receiver) in [(&mut b, &b_rx), (&mut c, &c_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        if let Some(message) = c.threads[&chat].messages.iter()
            .find(|message| message.id == message_id) {
            break Some(message.clone());
        }
        if Instant::now() >= deadline { break None; }
        std::thread::sleep(Duration::from_millis(20));
    };
    b.stop_device_sync_now();
    c.stop_device_sync_now();
    let received = received.expect("cold C must receive A's original ciphertext from restarted B");
    assert_eq!(received.body, body);
    assert_eq!(received.source_event_id.as_deref(), Some(outer.id.to_hex().as_str()));
    assert_eq!(received.author_owner_pubkey_hex.as_deref(), Some(owners[0].public_key().to_hex().as_str()));
    assert!(!received.is_outgoing);
    assert!(c.event_transport_channels[&outer.id.to_hex()].contains("FIPS mesh"));
}
