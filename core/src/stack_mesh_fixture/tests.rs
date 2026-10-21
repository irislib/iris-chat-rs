use super::*;
use fips_core::config::{PeerConfig, TransportInstances, UdpConfig};
use fips_tcp_endpoint::FipsTcpEndpoint;
use nostr_pubsub::{FipsPubsubWireCodec, FipsPubsubWireMessage, SubscriptionId};
use nostr_pubsub_fips::{FipsPubsubClientOptions, FIPS_NOSTR_PUBSUB_SERVICE_PORT};
use std::time::{Duration, Instant};

async fn endpoint(keys: &Keys, peers: Vec<PeerConfig>) -> Arc<FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        accept_connections: Some(true),
        ..Default::default()
    });
    config.peers = peers;
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .identity_nsec(keys.secret_key().to_secret_hex())
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

async fn fixture(keys: Keys, endpoint: Arc<FipsEndpoint>, peers: Vec<String>) -> StackMeshFixture {
    // Match AppCore's production receiver role and own public-author permission.
    let client = FipsPubsubClient::start_receive_only_with_public_authors(
        endpoint.clone(),
        FipsPubsubClientOptions {
            routed_peers: peers,
            ..Default::default()
        },
        vec![keys.public_key()],
    )
    .await
    .unwrap();
    StackMeshFixture {
        endpoint,
        client: Arc::new(client),
        keys,
        lan_discovery_enabled: false,
        relay_count: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_stack_fixture_keeps_scoped_subscription_and_publication_on_actual_wire() {
    let peer_keys = Keys::generate();
    let peer = endpoint(&peer_keys, vec![]).await;
    let address = peer.bound_udp_listen_addrs().await.unwrap()[0];
    let tcp = FipsTcpEndpoint::bind(
        peer.clone(),
        FIPS_NOSTR_PUBSUB_SERVICE_PORT,
        fips_tcp::Config::default(),
        73,
    )
    .await
    .unwrap();
    let keys = Keys::generate();
    let own = endpoint(
        &keys,
        vec![PeerConfig::new(peer.npub(), "udp", address.to_string())],
    )
    .await;
    let fixture = fixture(keys, own.clone(), vec![peer.npub().into()]).await;
    let public_file = VerifiedEvent::try_from(
        EventBuilder::new(Kind::FileMetadata, "synthetic public Drive probe")
            .sign_with_keys(&peer_keys)
            .unwrap(),
    )
    .unwrap();
    let wire = tokio::spawn(public_peer_wire(tcp, public_file.clone()));
    assert!(fixture.client.subscribe(vec![Filter::new()]).await.is_err());
    let mut incoming = fixture.subscribe_public_events().await.unwrap();
    let published = fixture
        .publish(9, "synthetic public Chat probe")
        .await
        .unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(15), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.event, public_file);
    assert_eq!(delivered.source.id.as_str(), peer.npub());
    let (request, event) = wire.await.unwrap();
    assert_eq!(request, json!({"kinds": [1063], "limit": 8}));
    assert_eq!(event["id"], published["id"]);
    assert_eq!(event["kind"], 9);
    assert_eq!(event["pubkey"], fixture.keys.public_key().to_hex());
    assert_eq!(event["tags"], json!([]));
    let signed: nostr::Event = serde_json::from_value(event).unwrap();
    signed.verify().unwrap();
    drop(incoming);
    fixture.client.shutdown_shared().await;
    own.shutdown().await.unwrap();
    peer.shutdown().await.unwrap();
}

#[tokio::test]
async fn public_stack_fixture_rejects_arbitrary_and_private_publication_kinds() {
    let keys = Keys::generate();
    let own = endpoint(&keys, vec![]).await;
    let fixture = fixture(keys, own.clone(), vec![]).await;
    for kind in [
        0, 1, 1059, 1060, 1063, 10445, 10446, 10447, 21111, 21113, 37368,
    ] {
        assert!(
            fixture.publish(kind, "must not publish").await.is_err(),
            "fixture must reject undeclared kind {kind} before the network"
        );
    }
    fixture
        .publish(9, "synthetic public Chat probe")
        .await
        .unwrap();
    fixture.client.shutdown_shared().await;
    own.shutdown().await.unwrap();
}

// Decode actual length-framed Nostr bytes received over the production TCP/FIPS
// service. This peer is deliberately a wire observer, not another fixture helper.
async fn public_peer_wire(mut tcp: FipsTcpEndpoint, file: VerifiedEvent) -> (Value, Value) {
    let codec = FipsPubsubWireCodec::default();
    let start = Instant::now();
    let mut connection = None;
    let mut input = Vec::new();
    let mut request = None;
    let mut event = None;
    let mut peerfinding_seen = false;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let now = u64::try_from(start.elapsed().as_millis()).unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(20), tcp.receive(now)).await;
            tcp.poll(now).await.unwrap();
            if let Some(id) = tcp.accept() {
                assert!(connection.replace(id).is_none());
                send(&mut tcp, id, &codec, FipsPubsubWireMessage::req(
                    SubscriptionId::new("public-chat-probes"),
                    vec![Filter::new().kind(Kind::Custom(9))],
                ), now).await;
            }
            let Some(id) = connection else { continue };
            input.extend(tcp.read(id, 65_536, now).await.unwrap());
            while let Some(header) = input.get(..4) {
                let length = u32::from_be_bytes(header.try_into().unwrap()) as usize;
                assert!(length <= 65_536);
                let Some(frame) = input.get(4..4 + length) else { break };
                let message = codec.decode_frame(frame).unwrap();
                let json: Value = serde_json::from_slice(frame).unwrap();
                input.drain(..4 + length);
                match message {
                    FipsPubsubWireMessage::Req { subscription_id, filters }
                        if filters.iter().any(|filter| filter.kinds.as_ref().is_some_and(
                            |kinds| kinds.contains(&Kind::FileMetadata))) => {
                        assert_eq!(filters.len(), 1);
                        request = Some(json[2].clone());
                        send(&mut tcp, id, &codec,
                            FipsPubsubWireMessage::deliver(subscription_id, file.clone()), now).await;
                    }
                    FipsPubsubWireMessage::Req { filters, .. } => {
                        assert_eq!(filters.len(), 1);
                        eprintln!("stack fixture public peerfinding filter: {}", json[2]);
                        assert_eq!(json[2], json!({
                            "kinds": [37195], "#d": ["fips-overlay-v1"], "limit": 8
                        }), "only generic bounded public peerfinding may accompany the fixture request");
                        peerfinding_seen = true;
                    }
                    FipsPubsubWireMessage::Inv { event_id, event_kind: 9, .. } => {
                        send(&mut tcp, id, &codec, FipsPubsubWireMessage::want(event_id), now).await;
                    }
                    FipsPubsubWireMessage::Event { event: delivered, .. } => {
                        assert_eq!(delivered.as_event().kind.as_u16(), 9);
                        event = Some(serde_json::to_value(delivered.as_event()).unwrap());
                    }
                    _ => {}
                }
            }
            if let (Some(request), Some(event), true) = (&request, &event, peerfinding_seen) {
                return (request.clone(), event.clone());
            }
        }
    }).await.expect("scoped request and signed public probe must cross the actual wire")
}

async fn send(
    tcp: &mut FipsTcpEndpoint,
    id: fips_tcp::ConnectionId,
    codec: &FipsPubsubWireCodec,
    message: FipsPubsubWireMessage,
    now: u64,
) {
    let frame = codec.encode_frame(&message).unwrap();
    let mut record = u32::try_from(frame.len()).unwrap().to_be_bytes().to_vec();
    record.extend(frame);
    assert_eq!(tcp.write(id, &record, now).await.unwrap(), record.len());
}
