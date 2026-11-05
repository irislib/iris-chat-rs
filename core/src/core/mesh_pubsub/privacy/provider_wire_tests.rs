use super::*;
use fips_tcp_endpoint::FipsTcpEndpoint;
use nostr_pubsub::{FipsPubsubWireCodec, FipsPubsubWireMessage};
use nostr_pubsub_fips::FIPS_NOSTR_PUBSUB_SERVICE_PORT;
use serde_json::{json, Value};

fn identity(keys: &Keys) -> fips_core::PeerIdentity {
    fips_core::PeerIdentity::from_npub(&keys.public_key().to_bech32().unwrap()).unwrap()
}

#[test]
fn private_provider_exclusions_preserve_public_url_seeds_and_replace_worker_policy() {
    let own = identity(&Keys::generate());
    let sibling = identity(&Keys::generate());
    let public = identity(&Keys::generate());
    let peers = [own, sibling, public]
        .into_iter()
        .map(|peer| PeerConfig::new(peer.npub(), "udp", "127.0.0.1:9"))
        .collect();
    let original = PrivateMeshProviders::new(
        peers,
        Some(WebSocketConfig {
            seed_urls: vec!["wss://provider.invalid/fips".into()],
            ..Default::default()
        }),
        vec![own.npub(), sibling.npub(), public.npub()],
        vec![],
    );
    let own_only = original.clone().with_excluded_peers(&own.npub(), &[]);
    let provider = original.with_excluded_peers(&own.npub(), &[sibling]);
    assert_ne!(own_only.key, provider.key);
    assert_eq!(provider.config.peers.len(), 1);
    assert_eq!(provider.config.peers[0].npub, public.npub());
    assert_eq!(provider.options.routed_peers, vec![public.npub()]);
    assert!(provider.available());
    assert!(!provider.config.node.peer_admission.configured_only);
    assert_eq!(
        provider
            .config
            .node
            .peer_admission
            .denied_npubs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([own.npub(), sibling.npub()])
    );
    assert_eq!(
        provider
            .config
            .transports
            .websocket
            .iter()
            .flat_map(|(_, config)| config.seed_urls.clone())
            .collect::<Vec<_>>(),
        vec!["wss://provider.invalid/fips"]
    );
    assert!(provider.config.node.strict_client_only);
    assert_eq!(
        provider.config.node.routing.mode,
        fips_core::config::RoutingMode::ReplyLearned
    );
    assert!(provider
        .config
        .transports
        .udp
        .iter()
        .all(|(_, config)| config.accept_connections == Some(false)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_provider_endpoint_binds_strict_without_accepting_udp() {
    let own = identity(&Keys::generate());
    let adapter = PrivateMeshProviders::new(vec![], None, vec![], vec![])
        .with_excluded_peers(&own.npub(), &[]);
    let endpoint = adapter.endpoint().await.unwrap();
    assert_ne!(endpoint.0.npub(), own.npub());
    assert!(endpoint.0.routing_status().await.unwrap().client_only);
    assert!(endpoint
        .0
        .bound_udp_listen_addrs()
        .await
        .unwrap()
        .is_empty());
    endpoint.0.shutdown().await.unwrap();
}

async fn accepting_endpoint(keys: &Keys) -> Arc<FipsEndpoint> {
    let mut config = Config::new();
    config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        accept_connections: Some(true),
        ..Default::default()
    });
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_provider_discovery_wire_is_single_author_anonymous_and_fresh_on_retry() {
    let own_keys = Keys::generate();
    let sibling_keys = Keys::generate();
    let subject_keys = Keys::generate();
    let own = accepting_endpoint(&own_keys).await;
    let sibling = accepting_endpoint(&sibling_keys).await;
    let provider = accepting_endpoint(&Keys::generate()).await;
    let mut hints = Vec::new();
    for endpoint in [&own, &sibling, &provider] {
        hints.push(PeerConfig::new(
            endpoint.npub(),
            "udp",
            endpoint.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
        ));
    }
    let mut adapter = PrivateMeshProviders::new(
        hints,
        None,
        vec![
            own.npub().into(),
            sibling.npub().into(),
            provider.npub().into(),
        ],
        vec![],
    )
    .with_excluded_peers(own.npub(), &[identity(&sibling_keys)]);
    // One actual response completes each query, with the production timeout as
    // its upper bound. No timer-only success or mocked network delivery.
    adapter.options.max_replay_events = 1;
    let tcp = FipsTcpEndpoint::bind(
        provider.clone(),
        FIPS_NOSTR_PUBSUB_SERVICE_PORT,
        fips_tcp::Config::default(),
        73,
    )
    .await
    .unwrap();
    let event = VerifiedEvent::try_from(
        EventBuilder::new(Kind::Metadata, "public fixture")
            .sign_with_keys(&subject_keys)
            .unwrap(),
    )
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let wire = tokio::spawn(discovery_wire(tcp, event.clone(), stopped));
    let (sender, incoming) = flume::unbounded();
    const LOOKUPS: usize = 2;
    for _ in 0..LOOKUPS {
        let mut observations = DiscoveryObservations::default();
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            adapter.discover(
                vec![Filter::new()
                    .kind(Kind::Metadata)
                    .author(subject_keys.public_key())],
                own.clone(),
                sender.clone(),
                &mut observations,
            ),
        )
        .await
        .unwrap();
        assert!(
            result.attempted && result.changed && !result.closed,
            "the unrelated provider must answer both the first lookup and retry"
        );
        let CoreMsg::Internal(message) = incoming.try_recv().unwrap() else {
            panic!("expected discovered event")
        };
        let InternalEvent::MeshEvent(delivered) = *message else {
            panic!("expected discovered event")
        };
        assert_eq!(delivered.id, event.as_event().id);
        assert!(
            incoming.is_empty(),
            "replayed requests deliver the logical event only once"
        );
    }
    stop.send(()).unwrap();
    let transcript = wire.await.unwrap();
    let serialized = serde_json::to_string(&transcript).unwrap();
    for keys in [&own_keys, &sibling_keys] {
        assert!(!serialized.contains(&keys.public_key().to_hex()));
        assert!(!serialized.contains(&keys.public_key().to_bech32().unwrap()));
    }
    // The released client reserves this exact bounded public bootstrap query
    // for every role. It has no subject; it is not an application query result.
    let bootstrap_filter = json!({"kinds": [37195], "#d": ["fips-overlay-v1"], "limit": 1});
    let (bootstrap, requests): (Vec<_>, Vec<_>) = transcript
        .iter()
        .filter(|(_, frame)| frame[0] == "REQ")
        .partition(|(_, frame)| frame[2] == bootstrap_filter);
    // Subscription replay may repeat an allowed request on the same source.
    // Count successful logical lookups separately from serialized frames.
    let sources = requests
        .iter()
        .map(|(source, _)| source.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sources.len(),
        2,
        "a retry must authenticate with a fresh anonymous key"
    );
    for keys in [&own_keys, &sibling_keys, &subject_keys] {
        assert!(!sources.contains(&keys.public_key().to_bech32().unwrap()));
    }
    let frame_counts = sources
        .iter()
        .map(|source| {
            let subject = requests.iter().filter(|(peer, _)| peer == source).count();
            let public_bootstrap = bootstrap.iter().filter(|(peer, _)| peer == source).count();
            assert!(
                subject >= 1,
                "each anonymous source sent the actual subject query"
            );
            (subject, public_bootstrap)
        })
        .collect::<Vec<_>>();
    eprintln!("provider_discovery_wire logical_lookups={LOOKUPS} anonymous_sources={} frame_counts_per_source(subject,bootstrap)={frame_counts:?}", sources.len());
    for (source, request) in bootstrap {
        assert_eq!(request.as_array().unwrap().len(), 3);
        assert!(
            sources.contains(source),
            "bootstrap stays on the same anonymous operation"
        );
    }
    for (_, request) in requests {
        assert_eq!(request.as_array().unwrap().len(), 3);
        assert_eq!(
            request[2],
            json!({"authors": [subject_keys.public_key().to_hex()], "kinds": [0], "limit": 1})
        );
    }
    // These real, accepting endpoints were explicitly offered as provider
    // hints. Neither may carry the anonymous query or learn its subject.
    assert!(own.peers().await.unwrap().is_empty());
    assert!(sibling.peers().await.unwrap().is_empty());
    for endpoint in [provider, sibling, own] {
        endpoint.shutdown().await.unwrap();
    }
}

/// Observe actual length-framed Nostr bytes and their authenticated TCP/FIPS
/// source, then answer over that same stream. No replay-store or codec-only mock.
async fn discovery_wire(
    mut tcp: FipsTcpEndpoint,
    event: VerifiedEvent,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
) -> Vec<(String, Value)> {
    let codec = FipsPubsubWireCodec::default();
    let start = std::time::Instant::now();
    let mut inputs = std::collections::HashMap::<fips_tcp::ConnectionId, Vec<u8>>::new();
    let mut transcript = Vec::new();
    tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            if stopped.try_recv().is_ok() {
                return transcript;
            }
            let now = u64::try_from(start.elapsed().as_millis()).unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(20), tcp.receive(now)).await;
            tcp.poll(now).await.unwrap();
            while let Some(id) = tcp.accept() {
                inputs.insert(id, Vec::new());
            }
            let connections = inputs.keys().copied().collect::<Vec<_>>();
            for id in connections {
                if tcp.state(id).is_none() {
                    inputs.remove(&id);
                    continue;
                }
                let source = tcp.peer(id).unwrap().npub();
                let input = inputs.get_mut(&id).unwrap();
                input.extend(tcp.read(id, 65_536, now).await.unwrap());
                while let Some(header) = input.get(..4) {
                    let length = u32::from_be_bytes(header.try_into().unwrap()) as usize;
                    assert!(length <= 65_536);
                    let Some(frame) = input.get(4..4 + length) else {
                        break;
                    };
                    let message = codec.decode_frame(frame).unwrap();
                    let request: Value = serde_json::from_slice(frame).unwrap();
                    transcript.push((source.clone(), request.clone()));
                    input.drain(..4 + length);
                    match message {
                        FipsPubsubWireMessage::Req {
                            subscription_id,
                            filters,
                        } => {
                            assert_eq!(filters.len(), 1);
                            assert_eq!(request.as_array().unwrap().len(), 3);
                            if request[2]
                                == json!({
                                    "kinds": [37195], "#d": ["fips-overlay-v1"], "limit": 1
                                })
                            {
                                // This reserved public stream has no events in
                                // the fixture. The FIPS codec has no EOSE record;
                                // leave it empty and await the actual subject REQ.
                                continue;
                            }
                            assert_eq!(
                                filters[0],
                                Filter::new()
                                    .kind(Kind::Metadata)
                                    .author(event.as_event().pubkey)
                                    .limit(1)
                            );
                            let frame = codec
                                .encode_frame(&FipsPubsubWireMessage::deliver(
                                    subscription_id,
                                    event.clone(),
                                ))
                                .unwrap();
                            let mut record =
                                u32::try_from(frame.len()).unwrap().to_be_bytes().to_vec();
                            record.extend(frame);
                            assert_eq!(tcp.write(id, &record, now).await.unwrap(), record.len());
                        }
                        FipsPubsubWireMessage::Close { .. } => {}
                        other => {
                            panic!("anonymous discovery emitted unexpected wire message: {other:?}")
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("both real anonymous discovery queries finish within their bounded timeouts")
}
