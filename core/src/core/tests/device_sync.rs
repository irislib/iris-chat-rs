#[test]
fn device_sync_bootstraps_missing_chats_groups_and_post_roster_messages_once() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let group_member = Keys::generate();
    let group_member_device = Keys::generate();
    let (mut core, _updates, _temp_dir) =
        logged_in_test_core_with_updates("device-sync", &owner, &local_device);
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling_device.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    let group_member_hex = group_member.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex.clone(),
            created_at_secs: 100,
            devices: vec![
                KnownAppKeyDevice {
                    identity_pubkey_hex: local_device.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
                KnownAppKeyDevice {
                    identity_pubkey_hex: sibling_hex.clone(),
                    created_at_secs: 100,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
            ],
        },
    );

    let data = serde_json::to_vec(&serde_json::json!({
        "type": "snapshot",
        "v": 1,
        "rosterAt": 102,
        "chats": [{ "id": peer_hex, "updatedAt": 90 }],
        "appKeys": [
            {
                "ownerPubkey": peer_hex,
                "createdAt": 80,
                "devices": [{
                    "identityPubkey": peer_device.public_key().to_hex(),
                    "createdAt": 70
                }]
            },
            {
                "ownerPubkey": group_member_hex,
                "createdAt": 81,
                "devices": [{
                    "identityPubkey": group_member_device.public_key().to_hex(),
                    "createdAt": 71
                }]
            }
        ],
        "groups": [{
            "id": "friends",
            "name": "Friends",
            "description": "Good people",
            "createdBy": owner_hex,
            "members": [owner_hex, peer_hex, group_member_hex],
            "admins": [owner_hex],
            "protocol": "pairwise_fanout_v1",
            "revision": 4,
            "createdAt": 50,
            "updatedAt": 90,
            "accepted": true
        }],
        "messages": [
            {
                "chatId": peer_hex,
                "id": "at-cutoff",
                "body": "old",
                "author": owner_hex,
                "createdAt": 100
            },
            {
                "chatId": peer_hex,
                "id": "after-cutoff",
                "body": "still old",
                "author": owner_hex,
                "createdAt": 101
            },
            {
                "chatId": peer_hex,
                "id": "after-both-cutoffs",
                "body": "new",
                "author": owner_hex,
                "createdAt": 103
            }
        ]
    }))
    .unwrap();

    core.handle_device_sync_packet(&Keys::generate().public_key().to_hex(), 7369, &data);
    assert!(
        core.threads.is_empty(),
        "unregistered devices cannot inject state"
    );
    assert!(!core.app_keys.contains_key(&peer_hex));
    assert!(!core.app_keys.contains_key(&group_member_hex));

    core.handle_device_sync_packet(&sibling_hex, 7369, &data);
    core.handle_device_sync_packet(&sibling_hex, 7369, &data);

    let newer_group = serde_json::to_vec(&serde_json::json!({
        "type": "snapshot",
        "v": 1,
        "rosterAt": 100,
        "chats": [],
        "groups": [{
            "id": "friends",
            "name": "Best friends",
            "createdBy": owner_hex,
            "members": [owner_hex, peer_hex, group_member_hex],
            "admins": [owner_hex],
            "revision": 5,
            "createdAt": 50,
            "updatedAt": 102
        }],
        "messages": []
    }))
    .unwrap();
    core.handle_device_sync_packet(&sibling_hex, 7369, &newer_group);

    assert!(core.threads.contains_key(&peer_hex));
    assert!(core.threads.contains_key("group:friends"));
    assert_eq!(core.groups.get("friends").unwrap().name, "Best friends");
    let messages = &core.threads.get(&peer_hex).unwrap().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "after-both-cutoffs");
    assert!(core.preferences.accepted_owner_pubkeys.contains(&peer_hex));
    assert_eq!(core.app_keys[&peer_hex].created_at_secs, 80);
    assert_eq!(core.app_keys[&group_member_hex].created_at_secs, 81);
    assert!(core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_device_roster_entry_for_owner(peer.public_key(), peer_device.public_key()));

    let unrelated = Keys::generate();
    let unrelated_hex = unrelated.public_key().to_hex();
    core.app_keys.insert(
        unrelated_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: unrelated_hex.clone(),
            created_at_secs: 82,
            devices: vec![KnownAppKeyDevice {
                identity_pubkey_hex: Keys::generate().public_key().to_hex(),
                created_at_secs: 72,
                device_label: None,
                client_label: None,
                label_updated_at_secs: 0,
            }],
        },
    );
    let packets = core.build_device_sync_packets_for_test(100, false);
    let mut synced_owners = packets
        .iter()
        .flat_map(|packet| {
            serde_json::from_slice::<serde_json::Value>(packet).unwrap()["appKeys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|roster| roster["ownerPubkey"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    synced_owners.sort();
    let mut expected_owners = vec![
        owner_hex.clone(),
        peer_hex.clone(),
        group_member_hex.clone(),
    ];
    expected_owners.sort();
    assert_eq!(synced_owners, expected_owners);
    assert!(packets.iter().all(|packet| {
        serde_json::from_slice::<serde_json::Value>(packet).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    }));

    let (mut linked, _updates, _temp) =
        logged_in_test_core_with_updates("device-sync-linked", &owner, &sibling_device);
    linked
        .app_keys
        .insert(owner_hex.clone(), core.app_keys[&owner_hex].clone());
    for packet in core.build_device_sync_packets_for_test(100, true) {
        linked.handle_device_sync_packet(&local_device.public_key().to_hex(), 7369, &packet);
    }
    assert_eq!(linked.threads[&peer_hex].messages[0].id, "after-both-cutoffs");
    assert_eq!(linked.groups["friends"].name, "Best friends");
    assert_eq!(linked.groups["friends"].members.len(), 3);
    assert!(linked.threads.contains_key("group:friends"));
    assert_eq!(linked.app_keys[&peer_hex].created_at_secs, 80);
    assert_eq!(linked.app_keys[&group_member_hex].created_at_secs, 81);
    assert!(!linked.app_keys.contains_key(&unrelated_hex));
}

#[tokio::test]
async fn newly_received_message_is_queued_for_an_authorized_sibling() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let sibling_device = Keys::generate();
    let sender = Keys::generate();
    let (mut core, _updates, _temp_dir) =
        logged_in_test_core_with_updates("device-sync-live-message", &owner, &local_device);
    let owner_hex = owner.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex,
            created_at_secs: 100,
            devices: vec![
                KnownAppKeyDevice {
                    identity_pubkey_hex: local_device.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
                KnownAppKeyDevice {
                    identity_pubkey_hex: sibling_device.public_key().to_hex(),
                    created_at_secs: 100,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
            ],
        },
    );

    let endpoint = Arc::new(
        fips_core::FipsEndpoint::builder()
            .without_system_tun()
            .bind()
            .await
            .expect("bind embedded endpoint"),
    );
    let (tcp, records) = DeviceSyncTcpSender::test_channel(4, 1024);
    let sibling = fips_core::PeerIdentity::from_npub(
        &sibling_device
            .public_key()
            .to_bech32()
            .expect("encode sibling npub"),
    )
    .expect("valid sibling identity");
    core.install_device_sync_sender_for_test(endpoint.clone(), tcp, vec![sibling]);

    let chat_id = sender.public_key().to_hex();
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "survives a sibling relay miss".to_string(),
        101,
        None,
        Some("live-message-id".to_string()),
        Some("live-event-id".to_string()),
    );

    let queued = records
        .try_recv()
        .expect("the accepted message should be queued for the sibling stream");
    assert_eq!(queued.peer, sibling);
    let incoming_record = queued.record;
    let packet = serde_json::from_slice::<serde_json::Value>(&incoming_record).unwrap();
    assert_eq!(packet["type"], "snapshot");
    assert_eq!(packet["chats"], serde_json::json!([]));
    assert_eq!(packet["appKeys"], serde_json::json!([]));
    assert_eq!(packet["groups"], serde_json::json!([]));
    let messages = packet["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["chatId"], chat_id);
    assert_eq!(messages[0]["id"], "live-message-id");
    assert_eq!(messages[0]["body"], "survives a sibling relay miss");

    core.apply_runtime_text_message(
        owner.public_key(),
        Some(chat_id.clone()),
        "linked-device reply".to_string(),
        102,
        None,
        Some("live-outgoing-id".to_string()),
        Some("live-outgoing-event-id".to_string()),
    );
    let queued = records
        .try_recv()
        .expect("the sent sibling reply should be queued for the primary stream");
    let packet = serde_json::from_slice::<serde_json::Value>(&queued.record).unwrap();
    let messages = packet["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["chatId"], chat_id);
    assert_eq!(messages[0]["id"], "live-outgoing-id");
    assert_eq!(messages[0]["author"], owner.public_key().to_hex());

    let (mut linked, _linked_updates, _linked_temp_dir) =
        logged_in_test_core_with_updates("device-sync-live-linked", &owner, &sibling_device);
    linked.app_keys.insert(
        owner.public_key().to_hex(),
        core.app_keys[&owner.public_key().to_hex()].clone(),
    );
    let source = local_device.public_key().to_hex();
    linked.handle_device_sync_packet(&source, DEVICE_SYNC_PORT, &incoming_record);
    linked.handle_device_sync_packet(&source, DEVICE_SYNC_PORT, &incoming_record);
    let linked_messages = &linked.threads[&chat_id].messages;
    assert_eq!(linked_messages.len(), 1);
    assert_eq!(linked_messages[0].id, "live-message-id");
    assert_eq!(linked_messages[0].body, "survives a sibling relay miss");
    assert!(!linked_messages[0].is_outgoing);

    core.device_sync = None;
    endpoint.shutdown().await.expect("shutdown endpoint");
    tokio::task::spawn_blocking(move || {
        drop(core);
        drop(linked);
    })
        .await
        .expect("drop test core outside async runtime");
}

#[test]
fn device_sync_app_keys_use_canonical_freshness_and_preserve_retained_labels() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let device_c = Keys::generate();
    let (mut core, _updates, _temp_dir) =
        logged_in_test_core_with_updates("device-sync-app-keys", &owner, &local_device);
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling_device.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex,
            created_at_secs: 100,
            devices: vec![
                KnownAppKeyDevice {
                    identity_pubkey_hex: local_device.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
                KnownAppKeyDevice {
                    identity_pubkey_hex: sibling_hex.clone(),
                    created_at_secs: 2,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
            ],
        },
    );

    let snapshot = |created_at: u64, devices: &[&Keys]| {
        serde_json::to_vec(&serde_json::json!({
            "type": "snapshot",
            "v": 1,
            "rosterAt": 100,
            "chats": [{ "id": peer_hex, "updatedAt": 90 }],
            "appKeys": [{
                "ownerPubkey": peer_hex,
                "createdAt": created_at,
                "devices": devices.iter().map(|device| serde_json::json!({
                    "identityPubkey": device.public_key().to_hex(),
                    "createdAt": 10
                })).collect::<Vec<_>>()
            }],
            "groups": [],
            "messages": []
        }))
        .unwrap()
    };

    core.handle_device_sync_packet(&sibling_hex, 7369, &snapshot(20, &[&device_a]));
    core.app_keys
        .get_mut(&peer_hex)
        .unwrap()
        .devices[0]
        .device_label = Some("Kept locally".to_string());
    core.handle_device_sync_packet(&sibling_hex, 7369, &snapshot(19, &[&device_b]));
    assert_eq!(core.app_keys[&peer_hex].devices.len(), 1);
    assert_eq!(
        core.app_keys[&peer_hex].devices[0].identity_pubkey_hex,
        device_a.public_key().to_hex()
    );

    core.handle_device_sync_packet(&sibling_hex, 7369, &snapshot(20, &[&device_b]));
    assert_eq!(core.app_keys[&peer_hex].devices.len(), 2);

    core.handle_device_sync_packet(
        &sibling_hex,
        7369,
        &snapshot(21, &[&device_a, &device_c]),
    );
    let roster = &core.app_keys[&peer_hex];
    assert_eq!(roster.created_at_secs, 21);
    assert_eq!(roster.devices.len(), 2);
    assert!(roster
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == device_c.public_key().to_hex()));
    assert_eq!(
        roster
            .devices
            .iter()
            .find(|device| device.identity_pubkey_hex == device_a.public_key().to_hex())
            .and_then(|device| device.device_label.as_deref()),
        Some("Kept locally")
    );
    assert!(!roster
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == device_b.public_key().to_hex()));
}

#[test]
fn device_sync_rejects_malformed_app_keys_rosters() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _updates, _temp_dir) =
        logged_in_test_core_with_updates("device-sync-invalid-app-keys", &owner, &local_device);
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling_device.public_key().to_hex();
    let peer_hex = peer.public_key().to_hex();
    core.app_keys.insert(
        owner_hex.clone(),
        KnownAppKeys {
            owner_pubkey_hex: owner_hex,
            created_at_secs: 100,
            devices: vec![
                KnownAppKeyDevice {
                    identity_pubkey_hex: local_device.public_key().to_hex(),
                    created_at_secs: 1,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
                KnownAppKeyDevice {
                    identity_pubkey_hex: sibling_hex.clone(),
                    created_at_secs: 2,
                    device_label: None,
                    client_label: None,
                    label_updated_at_secs: 0,
                },
            ],
        },
    );
    let data = serde_json::to_vec(&serde_json::json!({
        "type": "snapshot",
        "v": 1,
        "rosterAt": 100,
        "chats": [],
        "appKeys": [{
            "ownerPubkey": peer_hex,
            "createdAt": 20,
            "devices": [{ "identityPubkey": "not-a-key", "createdAt": 10 }]
        }],
        "groups": [],
        "messages": []
    }))
    .unwrap();

    core.handle_device_sync_packet(&sibling_hex, 7369, &data);
    assert!(!core.app_keys.contains_key(&peer_hex));

    let duplicate_owner = Keys::generate();
    let duplicate_owner_hex = duplicate_owner.public_key().to_hex();
    let duplicate_device_hex = Keys::generate().public_key().to_hex();
    let duplicates = serde_json::to_vec(&serde_json::json!({
        "type": "snapshot",
        "v": 1,
        "rosterAt": 100,
        "chats": [],
        "appKeys": [{
            "ownerPubkey": duplicate_owner_hex,
            "createdAt": 20,
            "devices": [
                { "identityPubkey": duplicate_device_hex, "createdAt": 10 },
                { "identityPubkey": duplicate_device_hex, "createdAt": 11 }
            ]
        }],
        "groups": [],
        "messages": []
    }))
    .unwrap();
    core.handle_device_sync_packet(&sibling_hex, 7369, &duplicates);
    assert!(!core.app_keys.contains_key(&duplicate_owner_hex));
}
#[test]
fn device_sync_uses_single_original_service_port() {
    assert_eq!(DEVICE_SYNC_PORT, 7369);
}
