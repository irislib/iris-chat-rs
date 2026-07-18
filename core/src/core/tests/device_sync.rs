fn configure_test_device_sync_profile(
    core: &mut AppCore,
    owner: &Keys,
    local_device: &Keys,
    sibling_device: &Keys,
    relay_url: &str,
) {
    core.logged_in.as_mut().expect("logged in").relay_urls =
        relay_urls_from_strings(&[relay_url.to_string()]);
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
}

fn test_fips_peer(keys: &Keys) -> fips_core::PeerIdentity {
    fips_core::PeerIdentity::from_npub(
        &keys
            .public_key()
            .to_bech32()
            .expect("encode test device npub"),
    )
    .expect("test FIPS identity")
}

fn test_keys_with_compressed_prefix(prefix: u8) -> Keys {
    loop {
        let keys = Keys::generate();
        let identity = fips_core::Identity::from_secret_str(&keys.secret_key().to_secret_hex())
            .expect("test FIPS identity from Nostr secret");
        if identity.pubkey_full().serialize()[0] == prefix {
            return keys;
        }
    }
}

fn reserve_tcp_addr() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve TCP address");
    listener.local_addr().expect("reserved TCP address")
}

async fn device_sync_peer_transport(
    endpoint: &fips_core::FipsEndpoint,
    peer: &fips_core::PeerIdentity,
) -> Option<String> {
    endpoint
        .peers()
        .await
        .expect("device-sync peer health")
        .into_iter()
        .find(|candidate| candidate.connected && candidate.npub == peer.npub())
        .and_then(|candidate| candidate.transport_type)
}

async fn device_sync_pair_is_connected(
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
) -> bool {
    device_sync_peer_transport(link[0].0, link[0].1)
        .await
        .is_some()
        && device_sync_peer_transport(link[1].0, link[1].1)
            .await
            .is_some()
}

async fn device_sync_pair_uses_websocket(
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
) -> bool {
    device_sync_peer_transport(link[0].0, link[0].1)
        .await
        .as_deref()
        == Some("websocket")
        && device_sync_peer_transport(link[1].0, link[1].1)
            .await
            .as_deref()
            == Some("websocket")
}

fn has_device_sync_message(core: &AppCore, chat_id: &str, message_id: &str) -> bool {
    core.threads.get(chat_id).is_some_and(|thread| {
        thread
            .messages
            .iter()
            .any(|message| message.id == message_id)
    })
}

fn wait_for_device_sync_message(
    sender: &AppCore,
    receiver: &mut AppCore,
    messages: &flume::Receiver<CoreMsg>,
    link: [(&fips_core::FipsEndpoint, &fips_core::PeerIdentity); 2],
    chat_id: &str,
    message_id: &str,
    stable_for: Duration,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10) + stable_for;
    let mut first_seen = None;
    loop {
        while let Ok(message) = messages.try_recv() {
            receiver.handle_message(message);
        }
        let now = std::time::Instant::now();
        if has_device_sync_message(receiver, chat_id, message_id) {
            first_seen.get_or_insert(now);
        }
        assert!(
            sender.runtime.block_on(device_sync_pair_is_connected(link)),
            "production device sync must retain an authenticated FIPS route"
        );
        if first_seen.is_some_and(|seen| now.duration_since(seen) >= stable_for) {
            return;
        }
        assert!(
            now < deadline,
            "production fips-tcp device sync should converge: message={message_id}",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn device_sync_websocket_authenticates_siblings_and_rejects_non_siblings() {
    const UNAUTHORIZED_PORT: u16 = 47_002;
    const SOURCE_PORT: u16 = 47_000;
    let relay = crate::local_relay::TestRelay::start();
    assert!(!relay.url().is_empty(), "test relay should start");
    let owner = Keys::generate();
    let alice = test_keys_with_compressed_prefix(0x03);
    let bob = test_keys_with_compressed_prefix(0x02);
    let (mut alice_core, _alice_updates, _alice_temp) =
        logged_in_test_core_with_updates("device-sync-relay-alice", &owner, &alice);
    let (mut bob_core, _bob_updates, _bob_temp) =
        logged_in_test_core_with_updates("device-sync-relay-bob", &owner, &bob);
    let (alice_core_tx, _alice_core_rx) = flume::unbounded();
    alice_core.core_sender = alice_core_tx.clone();
    alice_core.priority_sender = alice_core_tx;
    let (bob_core_tx, bob_core_rx) = flume::unbounded();
    bob_core.core_sender = bob_core_tx.clone();
    bob_core.priority_sender = bob_core_tx;
    configure_test_device_sync_profile(&mut alice_core, &owner, &alice, &bob, relay.url());
    configure_test_device_sync_profile(&mut bob_core, &owner, &bob, &alice, relay.url());

    let websocket_addr = reserve_tcp_addr();
    alice_core.reconcile_device_sync_with_websocket_for_test(
        fips_core::config::WebSocketConfig {
            bind_addr: Some(websocket_addr.to_string()),
            ..Default::default()
        },
    );
    bob_core.reconcile_device_sync_with_websocket_for_test(
        fips_core::config::WebSocketConfig {
            seed_urls: vec![format!("ws://{websocket_addr}/fips")],
            ..Default::default()
        },
    );
    let alice_endpoint = alice_core
        .device_sync_endpoint_for_test()
        .expect("Alice FIPS endpoint");
    let bob_endpoint = bob_core
        .device_sync_endpoint_for_test()
        .expect("Bob FIPS endpoint");
    let alice_peer = test_fips_peer(&alice);
    let bob_peer = test_fips_peer(&bob);
    let link = [
        (alice_endpoint.as_ref(), &bob_peer),
        (bob_endpoint.as_ref(), &alice_peer),
    ];
    alice_core.runtime.block_on(async {
        let advertised = alice_endpoint
            .local_advertised_endpoints()
            .await
            .expect("Alice advertised endpoints");
        assert!(advertised.iter().any(|endpoint| {
            endpoint.transport
                == fips_core::discovery::nostr::OverlayTransportKind::WebRtc
        }));
        let promotion = tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if device_sync_pair_uses_websocket(link).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        if promotion.is_err() {
            let alice_status = alice_endpoint.peers().await.expect("Alice diagnostics");
            let bob_status = bob_endpoint.peers().await.expect("Bob diagnostics");
            panic!(
                "roster siblings did not establish the configured WebSocket seed route; \
                 Alice={alice_status:#?}, Bob={bob_status:#?}"
            );
        }

    });

    let sync_peer = Keys::generate();
    let sync_chat_id = sync_peer.public_key().to_hex();
    alice_core.apply_runtime_text_message(
        sync_peer.public_key(),
        Some(sync_chat_id.clone()),
        "production device sync over authenticated WebSocket FIPS".to_string(),
        200,
        None,
        Some("websocket-device-sync-message".to_string()),
        None,
    );
    wait_for_device_sync_message(
        &alice_core,
        &mut bob_core,
        &bob_core_rx,
        link,
        &sync_chat_id,
        "websocket-device-sync-message",
        Duration::ZERO,
    );
    alice_core.apply_runtime_text_message(
        sync_peer.public_key(),
        Some(sync_chat_id.clone()),
        "production device sync stays on authenticated FIPS".to_string(),
        201,
        None,
        Some("stable-websocket-device-sync-message".to_string()),
        None,
    );
    wait_for_device_sync_message(
        &alice_core,
        &mut bob_core,
        &bob_core_rx,
        link,
        &sync_chat_id,
        "stable-websocket-device-sync-message",
        Duration::from_secs(1),
    );

    let attacker_owner = Keys::generate();
    let attacker = Keys::generate();
    let (mut attacker_core, _attacker_updates, _attacker_temp) =
        logged_in_test_core_with_updates("device-sync-relay-attacker", &attacker_owner, &attacker);
    configure_test_device_sync_profile(
        &mut attacker_core,
        &attacker_owner,
        &attacker,
        &alice,
        relay.url(),
    );
    attacker_core.reconcile_device_sync();
    let attacker_endpoint = attacker_core
        .device_sync_endpoint_for_test()
        .expect("attacker FIPS endpoint");
    let attacker_peer = test_fips_peer(&attacker);
    let alice_service = alice_core
        .runtime
        .block_on(alice_endpoint.register_service_receiver(UNAUTHORIZED_PORT))
        .expect("Alice rejection service");
    alice_core.runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!alice_endpoint
            .peers()
            .await
            .expect("Alice peers after attack")
            .iter()
            .any(|peer| peer.npub == attacker_peer.npub() && peer.connected));
        assert!(!attacker_endpoint
            .peers()
            .await
            .expect("attacker peers")
            .iter()
            .any(|peer| peer.npub == alice_peer.npub() && peer.connected));
        let _ = attacker_endpoint
            .send_datagram(
                alice_peer,
                SOURCE_PORT,
                UNAUTHORIZED_PORT,
                b"unauthorized".to_vec(),
            )
            .await;
        let mut datagrams = Vec::new();
        assert!(tokio::time::timeout(
            Duration::from_millis(500),
            alice_service.recv_batch_into(&mut datagrams, 1),
        )
        .await
        .is_err());
    });

    attacker_core.stop_device_sync();
    bob_core.stop_device_sync();
    alice_core.stop_device_sync();
    assert!(alice_core.device_sync_endpoint_for_test().is_none());
    alice_core.runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
}

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
                "body": "b2xk",
                "author": owner_hex,
                "createdAt": 100
            },
            {
                "chatId": peer_hex,
                "id": "after-cutoff",
                "body": "c3RpbGwgb2xk",
                "author": owner_hex,
                "createdAt": 101
            },
            {
                "chatId": peer_hex,
                "id": "after-both-cutoffs",
                "body": "bmV3",
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
    assert!(!core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_device_roster_entry_for_owner(peer.public_key(), peer_device.public_key()));
    assert!(core
        .compute_protocol_subscription_plan()
        .expect("protocol subscription plan")
        .roster_authors
        .contains(&peer_hex));

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
    core.batch_depth = 1;
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "survives a sibling relay miss".to_string(),
        100,
        None,
        Some("live-message-id".to_string()),
        Some("live-event-id".to_string()),
    );
    core.batch_depth = 0;

    let queued = records
        .try_recv()
        .expect("the accepted message should be queued for the sibling stream");
    assert_eq!(queued.peer, sibling);
    let incoming_record = queued.records.into_iter().next().unwrap();
    let packet = serde_json::from_slice::<serde_json::Value>(&incoming_record).unwrap();
    assert_eq!(packet["type"], "snapshot");
    assert_eq!(packet["chats"], serde_json::json!([]));
    assert_eq!(packet["appKeys"], serde_json::json!([]));
    assert_eq!(packet["groups"], serde_json::json!([]));
    let messages = packet["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["chatId"], chat_id);
    assert_eq!(messages[0]["id"], "live-message-id");
    assert!(core
        .build_device_sync_packets_for_test(100, true)
        .iter()
        .any(|packet| packet.windows("live-message-id".len()).any(|window| {
            window == "live-message-id".as_bytes()
        })));

    core.push_outgoing_message_with_id(
        "live-outgoing-id".to_string(),
        &chat_id,
        "linked-device reply".to_string(),
        102,
        None,
        DeliveryState::Pending,
    );
    assert!(records.try_recv().is_err());
    core.update_message_delivery(&chat_id, "live-outgoing-id", DeliveryState::Sent);
    let queued = records
        .try_recv()
        .expect("the sibling reply should be queued only after it becomes sent");
    let packet = serde_json::from_slice::<serde_json::Value>(&queued.records[0]).unwrap();
    let messages = packet["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["chatId"], chat_id);
    assert_eq!(messages[0]["id"], "live-outgoing-id");
    assert_eq!(messages[0]["author"], owner.public_key().to_hex());
    core.update_message_delivery(&chat_id, "live-outgoing-id", DeliveryState::Failed);
    assert!(!core
        .build_device_sync_packets_for_test(100, true)
        .iter()
        .any(|packet| packet.windows("live-outgoing-id".len()).any(|window| {
            window == "live-outgoing-id".as_bytes()
        })));

    let (mut linked, _linked_updates, _linked_temp_dir) =
        logged_in_test_core_with_updates("device-sync-live-linked", &owner, &sibling_device);
    linked.app_keys.insert(
        owner.public_key().to_hex(),
        core.app_keys[&owner.public_key().to_hex()].clone(),
    );
    let source = local_device.public_key().to_hex();
    linked.batch_depth = 1;
    linked.handle_device_sync_packet(&source, DEVICE_SYNC_PORT, &incoming_record);
    linked.handle_device_sync_packet(&source, DEVICE_SYNC_PORT, &incoming_record);
    linked.batch_depth = 0;
    let linked_messages = &linked.threads[&chat_id].messages;
    assert_eq!(linked_messages.len(), 1);
    assert_eq!(linked_messages[0].id, "live-message-id");
    assert_eq!(linked_messages[0].body, "survives a sibling relay miss");
    assert!(!linked_messages[0].is_outgoing);

    let resync = br#"{"type":"resyncRequired","v":1}"#;
    core.handle_device_sync_packet(
        &sibling_device.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        resync,
    );
    let request_record = core
        .take_device_sync_control_for_test(sibling)
        .expect("overflow notice should elicit a lossless snapshot request");
    let request = serde_json::from_slice::<serde_json::Value>(&request_record).unwrap();
    assert_eq!(request["type"], "request");
    assert_eq!(request["rosterAt"], 100);

    core.device_sync = None;
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "x".repeat(32 * 1024 + 1),
        150,
        None,
        Some("oversized-message".to_string()),
        None,
    );
    assert!(!core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.id == "oversized-message"));
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "y".repeat(32 * 1024),
        151,
        None,
        Some("maximum-size-message".to_string()),
        None,
    );
    assert!(core.threads[&chat_id]
        .messages
        .iter()
        .any(|message| message.id == "maximum-size-message"));
    for index in 0..140 {
        core.push_incoming_message_from(
            &chat_id,
            Some(format!("page-{index:03}")),
            format!("page body {index}"),
            200 + index,
            None,
            None,
            Some(sender.public_key().to_hex()),
            None,
        );
    }
    core.persist_best_effort_inner();
    for index in 0..11 {
        core.update_message_delivery(
            &chat_id,
            &format!("page-{index:03}"),
            DeliveryState::Failed,
        );
    }
    let mut cursor = None;
    let mut paged_ids = Vec::new();
    loop {
        let (ids, next) = core.device_sync_message_page_for_test(100, cursor, 32);
        paged_ids.extend(ids);
        let Some(next) = next else { break };
        cursor = Some(next);
    }
    assert_eq!(paged_ids.len(), 131);
    assert_eq!(paged_ids.iter().filter(|id| *id == "live-message-id").count(), 1);
    assert_eq!(paged_ids.iter().filter(|id| *id == "page-139").count(), 1);
    assert_eq!(
        paged_ids
            .iter()
            .filter(|id| *id == "maximum-size-message")
            .count(),
        1
    );

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
