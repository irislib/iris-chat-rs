fn block_carrier_test_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    let mut core = logged_in_test_core(label, owner, device);
    // Publication tasks cannot run until the test explicitly polls this runtime.
    core.runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    core
}

async fn block_carrier_endpoint(
    keys: &Keys,
    bind: &str,
    peer: Option<(&Keys, &str)>,
) -> Arc<fips_core::FipsEndpoint> {
    use fips_core::config::{PeerConfig, TransportInstances, UdpConfig};
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some(bind.to_owned()),
        advertise_on_nostr: Some(false),
        public: Some(false),
        accept_connections: Some(true),
        ..Default::default()
    });
    if let Some((peer, address)) = peer {
        config.peers = vec![PeerConfig::new(test_fips_peer(peer).npub(), "udp", address)];
    }
    Arc::new(
        fips_core::FipsEndpoint::builder()
            .config(config)
            .identity_nsec(keys.secret_key().to_secret_hex())
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

fn queue_block_carrier_events(core: &mut AppCore, device: &Keys, target: &str) -> Vec<Event> {
    let other = Keys::generate().public_key().to_hex();
    [target, &other, "group:shared"]
        .into_iter()
        .enumerate()
        .map(|(index, chat)| {
            let event = EventBuilder::new(Kind::Custom(1060), format!("queued {index}"))
                .sign_with_keys(device)
                .unwrap();
            assert!(core.publish_runtime_event(
                event.clone(),
                "carrier-test",
                Some((format!("inner-{index}"), chat.to_owned())),
            ));
            event
        })
        .collect()
}

#[test]
fn private_block_cancels_unpolled_mesh_publication_and_preserves_other_chats() {
    use nostr_pubsub::EventBus;
    let owner = Keys::generate();
    let device = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = block_carrier_test_core("block-mesh-task", &owner, &device);
    let endpoint = core
        .runtime
        .block_on(block_carrier_endpoint(&device, "127.0.0.1:0", None));
    let client = Arc::new(
        core.runtime
            .block_on(nostr_pubsub_fips::FipsPubsubClient::start(
                endpoint.clone(),
                nostr_pubsub_fips::FipsPubsubClientOptions {
                    query_timeout: Duration::from_millis(100),
                    ..Default::default()
                },
            ))
            .unwrap(),
    );
    let (tcp, _records) = DeviceSyncTcpSender::test_channel(4, 1024);
    core.install_device_sync_sender_for_test(endpoint.clone(), tcp, Vec::new());
    core.device_sync.as_mut().unwrap().pubsub = Some(client.clone());
    let events = queue_block_carrier_events(&mut core, &device, &target);
    core.set_user_blocked(&target, true);
    let seen = core.runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        client
            .query(
                vec![Filter::new().ids(events.iter().map(|event| event.id))],
                Default::default(),
            )
            .await
            .unwrap()
            .events
            .into_iter()
            .map(|event| event.event.as_event().id)
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert_eq!(
        seen,
        [events[1].id, events[2].id].into_iter().collect(),
        "blocked task cannot reach the mesh cache; unrelated and group work still publishes"
    );
    core.runtime.block_on(async {
        client.shutdown_shared().await;
        endpoint.shutdown().await.unwrap();
    });
}

#[test]
fn private_block_cancels_unpolled_nearby_publication_and_preserves_other_chats() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let recipient = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = block_carrier_test_core("block-nearby-task", &owner, &device);
    let reserve = || std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let left_socket = reserve();
    let right_socket = reserve();
    let left_address = left_socket.local_addr().unwrap().to_string();
    let right_address = right_socket.local_addr().unwrap().to_string();
    drop((left_socket, right_socket));
    let (endpoint, receiver_endpoint, receiver) = core.runtime.block_on(async {
        let left =
            block_carrier_endpoint(&device, &left_address, Some((&recipient, &right_address)))
                .await;
        let right =
            block_carrier_endpoint(&recipient, &right_address, Some((&device, &left_address)))
                .await;
        let receiver = right
            .register_service_receiver(fips_nearby::FIPS_NEARBY_PORT)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if device_sync_pair_is_connected([
                    (&left, &test_fips_peer(&recipient)),
                    (&right, &test_fips_peer(&device)),
                ])
                .await
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("isolated authenticated UDP peers connect");
        (left, right, receiver)
    });
    let (tcp, _records) = DeviceSyncTcpSender::test_channel(4, 1024);
    core.install_device_sync_sender_for_test(endpoint.clone(), tcp, Vec::new());
    core.device_sync.as_mut().unwrap().nearby_enabled = true;
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_mailbag_enabled = true;
    let events = queue_block_carrier_events(&mut core, &device, &target);
    core.set_user_blocked(&target, true);
    let seen = core.runtime.block_on(async {
        let mut seen = std::collections::BTreeSet::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let mut datagrams = Vec::new();
            if tokio::time::timeout_at(deadline, receiver.recv_batch_into(&mut datagrams, 16))
                .await
                .is_err()
            {
                break;
            }
            for datagram in datagrams {
                if let Some(fips_nearby::FipsNearbyPacket::Event { event_id, .. }) =
                    fips_nearby::FipsNearbyPacket::decode(datagram.data.as_slice())
                {
                    seen.insert(event_id);
                }
            }
        }
        seen
    });
    assert_eq!(
        seen,
        [events[1].id.to_hex(), events[2].id.to_hex()]
            .into_iter()
            .collect(),
        "forgotten outbox entries cannot escape through a cloned nearby task"
    );
    core.runtime.block_on(async {
        endpoint.shutdown().await.unwrap();
        receiver_endpoint.shutdown().await.unwrap();
    });
}
