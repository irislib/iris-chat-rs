use super::*;

#[test]
fn live_peer_advert_queries_have_one_anonymous_subject_and_no_own_receive_context() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let own = Keys::generate();
        let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
        let (relay, wire) =
            crate::core::publish_helpers::authenticated_relay(own.public_key(), false).await;
        let own_client = crate::core::publish_helpers::identity_receiver(&own, &relay, &wire).await;
        let advert = EventBuilder::new(Kind::Custom(37195), "own public endpoint advert")
            .sign_with_keys(&own)
            .unwrap();
        own_client.send_event(&advert).await.unwrap();
        let mut config = Config::new();
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.local.enabled = false;
        config.node.discovery.lan.enabled = false;
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .without_system_tun()
                .bind()
                .await
                .unwrap(),
        );
        let adapter = PrivateMeshProviders::new(Vec::new(), None, Vec::new(), vec![relay]);
        for peer in peers {
            adapter
                .discover(
                    vec![Filter::new().kind(Kind::Custom(37195)).author(peer)],
                    endpoint.clone(),
                    flume::unbounded().0,
                    &mut DiscoveryObservations::default(),
                )
                .await;
        }
        let rows = wire.lock().unwrap().clone();
        let identified = rows
            .iter()
            .filter(|(_, event)| event[0] == "EVENT" && event[1]["id"] == advert.id.to_hex())
            .map(|(connection, _)| *connection)
            .collect::<HashSet<_>>();
        assert!(!identified.is_empty());
        let mut subjects = BTreeMap::<usize, HashSet<PublicKey>>::new();
        for (connection, event) in rows {
            if event[0] == "REQ" {
                for peer in peers {
                    if event.to_string().contains(&peer.to_hex()) {
                        assert!(!identified.contains(&connection));
                        assert!(!event.to_string().contains(&own.public_key().to_hex()));
                        subjects.entry(connection).or_default().insert(peer);
                    }
                }
            }
            if event[0] == "AUTH" && !identified.contains(&connection) {
                assert_ne!(event[1]["pubkey"], own.public_key().to_hex());
            }
        }
        assert_eq!(subjects.len(), 2);
        assert!(subjects.values().all(|keys| keys.len() == 1));
        own_client.shutdown().await;
        endpoint.shutdown().await.unwrap();
    });
}

#[test]
fn mesh_advert_quiet_delay_respects_expiry_without_expired_spin() {
    let keys = Keys::generate();
    let now = unix_now().get();
    let event = EventBuilder::new(Kind::Custom(37195), "signed fixture")
        .custom_created_at(Timestamp::from(now))
        .tags([nostr::Tag::expiration(Timestamp::from(now + 75))])
        .sign_with_keys(&keys)
        .unwrap();
    let mut observations = DiscoveryObservations::default();
    assert!(observations.remember(&event, 3_600));
    assert!(!observations.remember(&event, 3_600));
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now),
        Duration::from_secs(75)
    );
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now + 70),
        Duration::from_secs(5)
    );
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now + 75),
        Duration::from_secs(120)
    );
    assert_eq!(observations.advert_valid_until, None);
    assert_eq!(
        observations.delay(Duration::from_secs(120), false, now),
        Duration::from_secs(120)
    );
    assert_eq!(
        DiscoveryObservations::default().delay(Duration::from_secs(120), true, now),
        Duration::from_secs(120)
    );

    let old = EventBuilder::new(Kind::Custom(37195), "old signed fixture")
        .custom_created_at(Timestamp::from(now.saturating_sub(7_195)))
        .tags([nostr::Tag::expiration(Timestamp::from(now + 1_000))])
        .sign_with_keys(&keys)
        .unwrap();
    observations.remember(&old, 3_600);
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now),
        Duration::from_secs(5)
    );
}

#[test]
fn mesh_quiet_cadence_changes_only_on_new_event_ids_and_explicit_demand() {
    let keys = Keys::generate();
    let first = EventBuilder::new(Kind::Metadata, "first")
        .sign_with_keys(&keys)
        .unwrap();
    let late = EventBuilder::new(Kind::Metadata, "late arrival")
        .custom_created_at(Timestamp::from(
            first.created_at.as_secs().saturating_sub(10),
        ))
        .sign_with_keys(&keys)
        .unwrap();
    let mut observations = DiscoveryObservations::default();
    let mut cadence = DiscoveryCadence::default();
    assert_eq!(
        cadence.after_poll(observations.remember(&first, 3_600)),
        Duration::from_secs(30)
    );
    assert_eq!(
        cadence.after_poll(observations.remember(&first, 3_600)),
        Duration::from_secs(60)
    );
    assert_eq!(cadence.after_poll(false), Duration::from_secs(120));
    assert_eq!(cadence.after_poll(false), Duration::from_secs(120));
    assert_eq!(
        cadence.after_poll(observations.remember(&late, 3_600)),
        Duration::from_secs(30)
    );
    cadence.after_poll(false);
    cadence.reset();
    assert_eq!(cadence.after_poll(false), Duration::from_secs(30));
}

type DiscoveryWire = Arc<std::sync::Mutex<Vec<(usize, serde_json::Value)>>>;

async fn discovery_relay(
    initially_released: bool,
) -> (RelayUrl, DiscoveryWire, tokio::sync::watch::Sender<bool>) {
    discovery_relay_with_auth(initially_released, true).await
}

async fn discovery_relay_with_auth(
    initially_released: bool,
    accept_auth: bool,
) -> (RelayUrl, DiscoveryWire, tokio::sync::watch::Sender<bool>) {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let wire = DiscoveryWire::default();
    let recorded = wire.clone();
    let (release, released) = tokio::sync::watch::channel(initially_released);
    tokio::spawn(async move {
        let mut connection = 0;
        while let Ok((stream, _)) = listener.accept().await {
            connection += 1;
            let wire = recorded.clone();
            let mut released = released.clone();
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(
                        json!(["AUTH", format!("fixture-{connection}")]).to_string(),
                    ))
                    .await
                    .unwrap();
                let mut authenticated = false;
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    wire.lock().unwrap().push((connection, value.clone()));
                    match value[0].as_str() {
                        Some("AUTH") => {
                            let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                            event.verify().unwrap();
                            authenticated = true;
                            if socket
                                .send(Message::Text(
                                    json!(["OK", event.id, accept_auth, ""]).to_string(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            if !accept_auth {
                                break;
                            }
                        }
                        Some("REQ") if authenticated => {
                            while !*released.borrow_and_update() {
                                if released.changed().await.is_err() {
                                    return;
                                }
                            }
                            if socket
                                .send(Message::Text(json!(["EOSE", value[1]]).to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    (relay, wire, release)
}

fn subject_connections(
    wire: &DiscoveryWire,
    author: PublicKey,
) -> std::collections::BTreeSet<usize> {
    let mut authenticated = HashSet::new();
    let mut requested = std::collections::BTreeSet::new();
    for (connection, value) in wire.lock().unwrap().iter() {
        if value[0] == "AUTH" {
            authenticated.insert(*connection);
        }
        if authenticated.contains(connection)
            && value[0] == "REQ"
            && value
                .as_array()
                .unwrap()
                .iter()
                .skip(2)
                .any(|filter| filter["authors"] == serde_json::json!([author.to_hex()]))
        {
            requested.insert(*connection);
        }
    }
    requested
}

async fn wait_subject(wire: &DiscoveryWire, author: PublicKey, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while subject_connections(wire, author).len() < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing anonymous subject query"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

async fn discovery_endpoint() -> Arc<FipsEndpoint> {
    let mut config = Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

#[test]
fn mesh_discovery_preserves_unrelated_workers_and_refreshes_only_requested_author() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (relay, wire, _release) = discovery_relay(true).await;
        let own = Keys::generate().public_key();
        let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
        let filter = |author| Filter::new().kind(Kind::Custom(37195)).author(author);
        let providers = Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            Vec::new(),
            vec![relay],
        ));
        let live = discovery_endpoint().await;
        let mut discovery = PrivateMeshDiscovery::default();
        discovery.update(
            providers.clone(),
            vec![filter(peers[0])],
            live.clone(),
            flume::unbounded().0,
            &runtime,
        );
        wait_subject(&wire, peers[0], 1).await;
        let original = discovery.workers.values().next().unwrap().wake.clone();
        discovery.update(
            providers.clone(),
            vec![filter(peers[1]), filter(peers[0])],
            live.clone(),
            flume::unbounded().0,
            &runtime,
        );
        wait_subject(&wire, peers[1], 1).await;
        let retained = discovery
            .workers
            .values()
            .find(|worker| worker.author == peers[0])
            .unwrap();
        assert!(Arc::ptr_eq(&original, &retained.wake));
        assert_eq!(subject_connections(&wire, peers[0]).len(), 1);
        discovery.refresh_author(peers[0]);
        wait_subject(&wire, peers[0], 2).await;
        assert_eq!(subject_connections(&wire, peers[1]).len(), 1);
        let rows = wire.lock().unwrap().clone();
        let mut authenticated = BTreeMap::new();
        for (connection, frame) in &rows {
            if frame[0] == "AUTH" {
                let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                event.verify().unwrap();
                assert_ne!(event.pubkey, own);
                assert!(authenticated.insert(*connection, event.pubkey).is_none());
            }
            if frame[0] == "REQ" {
                assert_eq!(frame.as_array().unwrap().len(), 3);
                let query = frame[2].as_object().unwrap();
                assert_eq!(query.len(), 2);
                assert_eq!(query["kinds"], serde_json::json!([37195]));
                assert_eq!(query["authors"].as_array().unwrap().len(), 1);
            }
            assert_ne!(frame[0], "EVENT");
        }
        assert_eq!(authenticated.len(), 3);
        assert_eq!(authenticated.values().collect::<HashSet<_>>().len(), 3);
        let stopped = discovery
            .workers
            .values()
            .map(|worker| worker.task.abort_handle())
            .collect::<Vec<_>>();
        discovery.update(
            providers,
            Vec::new(),
            live.clone(),
            flume::unbounded().0,
            &runtime,
        );
        assert!(discovery.workers.is_empty());
        tokio::time::timeout(Duration::from_secs(3), async {
            while !stopped.iter().all(tokio::task::AbortHandle::is_finished) {
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        live.shutdown().await.unwrap();
    });
}

#[test]
fn mesh_discovery_demand_waits_for_busy_query_and_coalesces() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (relay, wire, release) = discovery_relay(false).await;
        let author = Keys::generate().public_key();
        let providers = Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            Vec::new(),
            vec![relay],
        ));
        let live = discovery_endpoint().await;
        let mut discovery = PrivateMeshDiscovery::default();
        discovery.update(
            providers,
            vec![Filter::new().kind(Kind::Custom(37195)).author(author)],
            live.clone(),
            flume::unbounded().0,
            &runtime,
        );
        wait_subject(&wire, author, 1).await;
        discovery.refresh_author(author);
        discovery.refresh_author(author);
        sleep(Duration::from_millis(25)).await;
        assert_eq!(
            subject_connections(&wire, author).len(),
            1,
            "demand interrupted a busy read"
        );
        release.send(true).unwrap();
        wait_subject(&wire, author, 2).await;
        sleep(Duration::from_millis(25)).await;
        assert_eq!(
            subject_connections(&wire, author).len(),
            2,
            "demand failed to coalesce"
        );
        drop(discovery);
        live.shutdown().await.unwrap();
    });
}

#[test]
fn mesh_discovery_does_not_retain_unusable_metadata_workers() {
    // Configure without polling the runtime: this checks worker ownership, not
    // a synthetic provider's networking or any publicly configured service.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let live = runtime.block_on(discovery_endpoint());
    let a = Keys::generate().public_key();
    let b = Keys::generate().public_key();
    let relay = RelayUrl::parse("ws://127.0.0.1:9").unwrap();
    let filters = vec![
        Filter::new().kind(Kind::Custom(37195)).author(a),
        Filter::new().kind(Kind::Metadata).author(a),
        Filter::new().kind(Kind::Metadata).author(b),
    ];
    let mut discovery = PrivateMeshDiscovery::default();
    discovery.update(
        Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
        )),
        filters.clone(),
        live.clone(),
        flume::unbounded().0,
        &runtime,
    );
    assert!(discovery.workers.is_empty());
    discovery.update(
        Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            Vec::new(),
            vec![relay.clone()],
        )),
        filters.clone(),
        live.clone(),
        flume::unbounded().0,
        &runtime,
    );
    assert_eq!(discovery.workers.len(), 1);
    let retained = discovery.workers.values().next().unwrap();
    assert_eq!(retained.author, a);
    assert_eq!(retained.filters.len(), 1);
    assert!(is_peer_advert_filter(&retained.filters[0]));
    discovery.update(
        Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            vec![Keys::generate().public_key().to_bech32().unwrap()],
            vec![relay.clone()],
        )),
        filters.clone(),
        live.clone(),
        flume::unbounded().0,
        &runtime,
    );
    assert_eq!(discovery.workers.len(), 2);
    assert_eq!(
        discovery
            .workers
            .values()
            .map(|worker| worker.filters.len())
            .sum::<usize>(),
        3
    );
    discovery.update(
        Arc::new(PrivateMeshProviders::new(
            Vec::new(),
            None,
            Vec::new(),
            vec![relay],
        )),
        filters,
        live.clone(),
        flume::unbounded().0,
        &runtime,
    );
    assert_eq!(discovery.workers.len(), 1);
    assert!(discovery
        .workers
        .values()
        .all(|worker| worker.filters.iter().all(is_peer_advert_filter)));
    drop(discovery);
    runtime.block_on(async { live.shutdown().await.unwrap() });
}

#[test]
fn mesh_failed_anonymous_reads_back_off_and_explicit_demand_resets() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (relay, wire, _release) = discovery_relay_with_auth(true, false).await;
        let author = Keys::generate().public_key();
        let providers = PrivateMeshProviders::new(Vec::new(), None, Vec::new(), vec![relay]);
        let live = discovery_endpoint().await;
        let mut observations = DiscoveryObservations::default();
        let mut cadence = DiscoveryCadence::default();
        for expected in [30, 60, 120, 120] {
            let poll = providers
                .discover(
                    vec![Filter::new().kind(Kind::Custom(37195)).author(author)],
                    live.clone(),
                    flume::unbounded().0,
                    &mut observations,
                )
                .await;
            assert!(poll.attempted);
            assert!(!poll.changed);
            assert!(!poll.closed);
            assert!(
                observations.seen.is_empty(),
                "failed reads moved seen state"
            );
            assert_eq!(
                poll.next_delay(&mut cadence),
                Some(Duration::from_secs(expected))
            );
        }
        assert!(
            subject_connections(&wire, author).is_empty(),
            "rejected AUTH became a completed authenticated query"
        );
        let rows = wire.lock().unwrap().clone();
        let mut requests = 0;
        for (_, frame) in &rows {
            if frame[0] == "REQ" {
                requests += 1;
                assert_eq!(frame.as_array().unwrap().len(), 3);
                assert_eq!(
                    frame[2],
                    serde_json::json!({"authors": [author.to_hex()], "kinds": [37195]})
                );
            }
        }
        assert_eq!(requests, 4);
        let authentications = wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, frame)| frame[0] == "AUTH")
            .map(|(_, frame)| {
                serde_json::from_value::<Event>(frame[1].clone())
                    .unwrap()
                    .pubkey
            })
            .collect::<HashSet<_>>();
        assert_eq!(authentications.len(), 4);
        assert!(!authentications.contains(&author));
        cadence.reset();
        assert_eq!(
            DiscoveryPoll {
                attempted: true,
                ..Default::default()
            }
            .next_delay(&mut cadence),
            Some(Duration::from_secs(30))
        );
        assert_eq!(DiscoveryPoll::default().next_delay(&mut cadence), None);
        live.shutdown().await.unwrap();
    });
}

fn inject_route_need(core: &mut AppCore, author: PublicKey, generation: u64, guarded: bool) {
    core.handle_message(CoreMsg::Internal(Box::new(
        InternalEvent::FipsPeerDiscoveryNeeded {
            generation,
            author,
            requires_pending_work: guarded,
        },
    )));
}

async fn wait_subject_closed(wire: &DiscoveryWire, author: PublicKey, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let connections = subject_connections(wire, author);
        let closed = wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(connection, value)| connections.contains(connection) && value[0] == "CLOSE")
            .map(|(connection, _)| *connection)
            .collect::<HashSet<_>>();
        if closed.len() >= count {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing completed query"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn mesh_route_retry_core_guard_requires_exact_private_work_on_actual_wire() {
    let directory = tempfile::TempDir::new().unwrap();
    let (sender, _messages) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        sender,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(crate::state::AppState::empty())),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (relay, wire, _release) = discovery_relay(true).await;
        let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
        let [a, b] = peers;
        let a_npub = a.to_bech32().unwrap();
        let b_npub = b.to_bech32().unwrap();
        let live = discovery_endpoint().await;
        core.install_device_sync_endpoint_for_test(live.clone());
        core.fips_connection_generation = 7;
        let mesh = core.device_sync.as_mut().unwrap();
        let outbox = mesh.nearby_outbox.clone();
        mesh.protocol_subscriptions.discovery.update(
            Arc::new(PrivateMeshProviders::new(
                Vec::new(),
                None,
                Vec::new(),
                vec![relay],
            )),
            peers
                .map(|peer| Filter::new().kind(Kind::Custom(37195)).author(peer))
                .to_vec(),
            live.clone(),
            core.core_sender.clone(),
            &core.runtime,
        );
        wait_subject_closed(&wire, a, 1).await;
        wait_subject_closed(&wire, b, 1).await;

        for state in [
            "no work",
            "public",
            "empty targets",
            "other recipient",
            "acknowledged",
        ] {
            {
                let mut outbox = outbox.write().unwrap();
                *outbox = Default::default();
                match state {
                    "public" => outbox.insert_for_targets("public".into(), vec![1], None),
                    "empty targets" => {
                        outbox.insert_for_targets("empty".into(), vec![2], Some(Default::default()))
                    }
                    "other recipient" => outbox.insert_for_targets(
                        "other".into(),
                        vec![3],
                        Some([b_npub.clone()].into()),
                    ),
                    "acknowledged" => {
                        outbox.insert_for_targets(
                            "acked".into(),
                            vec![4],
                            Some([a_npub.clone()].into()),
                        );
                        assert!(outbox.acknowledge(&a_npub, "acked"));
                    }
                    _ => {}
                }
            }
            for _ in 0..128 {
                inject_route_need(&mut core, a, 7, true);
            }
            sleep(Duration::from_millis(100)).await;
            assert_eq!(
                subject_connections(&wire, a).len(),
                1,
                "idle retry: {state}"
            );
            assert_eq!(
                subject_connections(&wire, b).len(),
                1,
                "unrelated author: {state}"
            );
        }

        {
            let mut outbox = outbox.write().unwrap();
            *outbox = Default::default();
            outbox.insert_for_targets("pending".into(), vec![5], Some([a_npub.clone()].into()));
        }
        inject_route_need(&mut core, a, 6, true);
        inject_route_need(&mut core, a, 6, false);
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_connections(&wire, a).len(),
            1,
            "stale generation woke query"
        );
        inject_route_need(&mut core, a, 7, true);
        wait_subject_closed(&wire, a, 2).await;
        assert!(outbox.write().unwrap().acknowledge(&a_npub, "pending"));
        for _ in 0..128 {
            inject_route_need(&mut core, a, 7, true);
        }
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_connections(&wire, a).len(),
            2,
            "receipt failed to stop retry lookups"
        );
        inject_route_need(&mut core, a, 7, false);
        wait_subject_closed(&wire, a, 3).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_connections(&wire, a).len(),
            3,
            "one link loss must request one lookup"
        );
        assert_eq!(subject_connections(&wire, b).len(), 1);

        let mut authenticated = BTreeMap::new();
        let mut subjects = BTreeMap::<usize, HashSet<PublicKey>>::new();
        for (connection, frame) in wire.lock().unwrap().iter() {
            match frame[0].as_str() {
                Some("AUTH") => {
                    let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                    event.verify().unwrap();
                    assert!(!peers.contains(&event.pubkey));
                    assert!(authenticated.insert(*connection, event.pubkey).is_none());
                }
                Some("REQ") => {
                    assert_eq!(frame.as_array().unwrap().len(), 3);
                    let query = frame[2].as_object().unwrap();
                    assert_eq!(query.len(), 2);
                    assert_eq!(query["kinds"], serde_json::json!([37195]));
                    let authors = query["authors"].as_array().unwrap();
                    assert_eq!(authors.len(), 1);
                    let author = PublicKey::from_hex(authors[0].as_str().unwrap()).unwrap();
                    assert!(peers.contains(&author));
                    subjects.entry(*connection).or_default().insert(author);
                }
                Some("EVENT") => panic!("discovery context published an event"),
                _ => {}
            }
        }
        assert_eq!(authenticated.len(), 4);
        assert_eq!(authenticated.values().collect::<HashSet<_>>().len(), 4);
        assert_eq!(subjects.len(), 4);
        assert!(subjects.values().all(|authors| authors.len() == 1));
        drop(core.device_sync.take());
        live.shutdown().await.unwrap();
    });
}
