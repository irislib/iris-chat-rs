#[test]
fn encrypted_chat_crosses_uninterested_fips_transit_without_relays_or_sibling_sync() {
    use fips_core::config::{TransportInstances, WebSocketConfig};
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let (mut alice, _, _alice_dir) =
        logged_in_test_core_with_updates("mesh-alice", &alice_owner, &alice_device);
    let (mut bob, _, _bob_dir) =
        logged_in_test_core_with_updates("mesh-bob", &bob_owner, &bob_device);
    let alice_invite = nostr_double_ratchet::invite_unsigned_event(
        &alice
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap(),
    )
    .unwrap()
    .sign_with_keys(&alice_device)
    .unwrap();
    let bob_invite = nostr_double_ratchet::invite_unsigned_event(
        &bob.protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap(),
    )
    .unwrap()
    .sign_with_keys(&bob_device)
    .unwrap();
    let now = unix_now().get();
    for (core, owner, device, peer_owner, peer_device, invite) in [
        (
            &mut alice,
            &alice_owner,
            &alice_device,
            &bob_owner,
            &bob_device,
            bob_invite,
        ),
        (
            &mut bob,
            &bob_owner,
            &bob_device,
            &alice_owner,
            &alice_device,
            alice_invite,
        ),
    ] {
        let engine = core.protocol_engine.as_mut().unwrap();
        observe_current_device_appkeys_for_test(engine, owner, device);
        observe_peer_appkeys_for_test(engine, peer_owner, &[peer_device.public_key()], now);
        engine.observe_invite_event(&invite).unwrap();
        for (owner, device) in [(owner, device), (peer_owner, peer_device)] {
            let hex = owner.public_key().to_hex();
            core.app_keys.insert(
                hex.clone(),
                KnownAppKeys {
                    owner_pubkey_hex: hex,
                    created_at_secs: now,
                    devices: vec![KnownAppKeyDevice {
                        identity_pubkey_hex: device.public_key().to_hex(),
                        created_at_secs: now,
                        device_label: None,
                        client_label: None,
                        label_updated_at_secs: 0,
                    }],
                },
            );
        }
    }
    let (alice_tx, alice_rx) = flume::unbounded();
    alice.core_sender = alice_tx.clone();
    alice.priority_sender = alice_tx;
    let (bob_tx, bob_rx) = flume::unbounded();
    bob.core_sender = bob_tx.clone();
    bob.priority_sender = bob_tx;
    let transit_addr = reserve_tcp_addr();
    let transit_keys = Keys::generate();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.transports.websocket = TransportInstances::Single(WebSocketConfig {
        bind_addr: Some(transit_addr.to_string()),
        ..WebSocketConfig::default()
    });
    let transit = runtime
        .block_on(
            fips_core::FipsEndpoint::builder()
                .config(config.clone())
                .identity_nsec(transit_keys.secret_key().to_secret_hex())
                .without_system_tun()
                .bind(),
        )
        .unwrap();
    for core in [&mut alice, &mut bob] {
        core.reconcile_device_sync_with_websocket_for_test(WebSocketConfig {
            seed_urls: vec![format!("ws://{transit_addr}/fips")],
            ..WebSocketConfig::default()
        });
        assert!(!core.device_sync_has_sibling_tcp_for_test());
        assert!(core.logged_in.as_ref().unwrap().relay_urls.is_empty());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        for (core, receiver) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        let ready = [&alice, &bob].into_iter().all(|core| {
            core.device_sync
                .as_ref()
                .and_then(|mesh| mesh.pubsub.as_ref())
                .is_some_and(|client| client.connected_peer_count().unwrap_or(0) > 0)
        });
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "routed Chat pubsub did not connect"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    alice.send_direct_message(
        &bob_owner.public_key().to_hex(),
        "encrypted across the mesh",
        UnixSeconds(now),
        None,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        for (core, receiver) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        if bob
            .threads
            .get(&alice_owner.public_key().to_hex())
            .is_some_and(|thread| {
                thread
                    .messages
                    .iter()
                    .any(|message| message.body == "encrypted across the mesh")
            })
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "encrypted Chat message did not decrypt through pubsub"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(bob
        .event_transport_channels
        .values()
        .any(|channel| channel.contains("FIPS mesh")));
    for core in [&alice, &bob] {
        let endpoint = core.device_sync_endpoint_for_test().unwrap();
        let direct = runtime
            .block_on(endpoint.peers())
            .unwrap()
            .into_iter()
            .filter(|peer| peer.connected)
            .map(|peer| peer.npub)
            .collect::<Vec<_>>();
        assert_eq!(direct, vec![transit.npub().to_string()]);
    }
    runtime.block_on(transit.shutdown()).unwrap();
    let burst = 70;
    // Exceed both the 64-event live cache and the 16-event outbox batch.
    for index in 0..burst {
        alice.send_direct_message(
            &bob_owner.public_key().to_hex(),
            &format!("offline mesh message {index}"),
            UnixSeconds(now + index + 1),
            None,
        );
    }
    assert!(alice.pending_relay_publishes.len() >= burst as usize);
    assert!(
        alice
            .device_sync
            .as_ref()
            .unwrap()
            .nearby_outbox
            .read()
            .unwrap()
            .pending_for_link("unregistered", 0)
            .is_empty(),
        "inactive Nearby service must not retain or broadcast duplicate payloads"
    );
    assert!(alice
        .protocol_subscription_runtime
        .liveness_due_at
        .is_some());
    let transit = runtime
        .block_on(
            fips_core::FipsEndpoint::builder()
                .config(config)
                .identity_nsec(transit_keys.secret_key().to_secret_hex())
                .without_system_tun()
                .bind(),
        )
        .unwrap();
    let recovery_started = std::time::Instant::now();
    let deadline = recovery_started + Duration::from_secs(60);
    loop {
        for (core, receiver) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        let received = bob
            .threads
            .get(&alice_owner.public_key().to_hex())
            .map_or(0, |thread| {
                thread
                    .messages
                    .iter()
                    .filter(|message| message.body.starts_with("offline mesh message "))
                    .count()
            });
        if received == burst as usize {
            eprintln!(
                "mesh_chat.recovered messages={burst} elapsed_ms={}",
                recovery_started.elapsed().as_millis()
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            for (name, core) in [("alice", &alice), ("bob", &bob)] {
                let mesh = core.device_sync.as_ref().unwrap();
                let peers = runtime.block_on(mesh.endpoint.peers()).unwrap();
                eprintln!(
                    "{name}: connected={} pubsub={} pending={} meshwork={} timer={:?}",
                    peers.iter().filter(|p| p.connected).count(),
                    mesh.pubsub
                        .as_ref()
                        .unwrap()
                        .connected_peer_count()
                        .unwrap(),
                    core.pending_relay_publishes.len(),
                    core.has_mesh_outbox_work(),
                    core.protocol_subscription_runtime.liveness_due_at
                );
                let client = mesh.pubsub.as_ref().unwrap();
                eprintln!(
                    "{name} delivery={:?} subscriptions={:?} transport_errors={} peers={peers:?}",
                    client.delivery_snapshot(),
                    client.peer_subscription_snapshot(),
                    client.transport_error_count()
                );
                eprintln!(
                    "{name} local_subs={} peer_subs={}",
                    client.active_subscription_count().unwrap(),
                    client.peer_subscription_count().unwrap()
                );
                for entry in core.debug_log.iter().rev().take(18) {
                    eprintln!("{} {}", entry.category, entry.detail);
                }
            }
            eprintln!("transit peers={:?}", runtime.block_on(transit.peers()));
            panic!("outage recovery delivered only {received}/{burst} messages");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // The normal read action returns authenticated receipts through the same mesh.
    let alice_chat = alice_owner.public_key().to_hex();
    let read_ids = bob.threads[&alice_chat]
        .messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    bob.preferences.send_read_receipts = true;
    bob.accept_direct_peer(&alice_chat);
    bob.mark_messages_seen(&alice_chat, &read_ids);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while alice.has_mesh_outbox_work() {
        for (core, receiver) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "authenticated receipts did not stop mesh retries"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        alice.pending_relay_publishes.len() >= burst as usize,
        "optional relay persistence must retain its records"
    );
    // Allow the last scheduled fast retry and in-flight control frames to settle.
    for _ in 0..150 {
        for (core, receiver) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)] {
            while let Ok(message) = receiver.try_recv() {
                core.handle_message(message);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let snapshots = [&alice, &bob].map(|core| {
        core.device_sync
            .as_ref()
            .unwrap()
            .pubsub
            .as_ref()
            .unwrap()
            .delivery_snapshot()
    });
    let transit_bytes = || {
        runtime
            .block_on(transit.peers())
            .unwrap()
            .iter()
            .map(|peer| peer.bytes_sent + peer.bytes_recv)
            .sum::<u64>()
    };
    let bytes_before = transit_bytes();
    let quiet_started = std::time::Instant::now();
    let mut liveness_wakes = [0usize; 2];
    eprintln!("mesh_chat.quiet pid={}", std::process::id());
    for _ in 0..750 {
        for (index, (core, receiver)) in [(&mut alice, &alice_rx), (&mut bob, &bob_rx)]
            .into_iter()
            .enumerate()
        {
            while let Ok(message) = receiver.try_recv() {
                if let CoreMsg::Internal(event) = &message {
                    if matches!(event.as_ref(), InternalEvent::ProtocolSubscriptionLivenessCheck { token } if *token == core.protocol_liveness_token)
                    {
                        liveness_wakes[index] += 1;
                    }
                }
                core.handle_message(message);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let idle_bytes = transit_bytes().saturating_sub(bytes_before);
    let quiet_seconds = quiet_started.elapsed().as_secs_f64();
    eprintln!("mesh_chat.quiet transit_bytes={idle_bytes} sample_seconds={quiet_seconds:.3}");
    assert!(
        idle_bytes as f64 / quiet_seconds < 8192.0,
        "idle mesh exceeded 8 KiB/s across both transit links: {idle_bytes}"
    );
    for ((core, before), wakes) in [&alice, &bob]
        .into_iter()
        .zip(snapshots)
        .zip(liveness_wakes)
    {
        let after = core
            .device_sync
            .as_ref()
            .unwrap()
            .pubsub
            .as_ref()
            .unwrap()
            .delivery_snapshot();
        assert!(!core.has_mesh_outbox_work());
        assert_eq!(
            (
                after.event_frames_received,
                after.inv_frames_received,
                after.want_frames_received,
                after.req_frames_received
            ),
            (
                before.event_frames_received,
                before.inv_frames_received,
                before.want_frames_received,
                before.req_frames_received
            ),
            "acknowledged conversations must stop application retransmission"
        );
        assert!(
            wakes <= 1,
            "quiet conversations must not keep the fast retry timer active: {wakes} wakes"
        );
    }
    alice.stop_device_sync_now();
    bob.stop_device_sync_now();
    runtime.block_on(transit.shutdown()).unwrap();
}
