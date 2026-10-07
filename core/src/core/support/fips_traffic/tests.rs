use super::*;

fn peer(identity: &str, link_id: u64, transport: &'static str, packets: u64) -> ObservedPeer {
    ObservedPeer {
        key: PeerKey {
            identity: identity.to_owned(),
            link_id,
            transport,
        },
        counters: Counters {
            rx_packets: packets,
            tx_packets: packets,
            rx_bytes: packets * 100,
            tx_bytes: packets * 50,
        },
    }
}

#[test]
fn aggregates_only_and_exact_peer_interval() {
    let mut history = TrafficHistory::default();
    let configured = BTreeSet::from(["private-fixture-identity".to_owned()]);
    let now = Instant::now();
    let before = history.record(
        vec![peer("private-fixture-identity", 42, "udp", 10)],
        &configured,
        1,
        now,
    );
    assert_eq!(before["valid"], true);
    assert!(before["sample_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert_eq!(before["connected_peer_count"], 1);
    assert_eq!(before["connected_configured_direct_peer_count"], 1);
    assert_eq!(before["unexpected_connected_peer_count"], 0);
    assert_eq!(before["interval"]["reason"], "baseline");
    let after = history.record(
        vec![peer("private-fixture-identity", 42, "udp", 13)],
        &configured,
        1,
        now + Duration::from_secs(60),
    );
    assert_eq!(after["interval"]["valid"], true);
    assert_eq!(after["interval"]["since_sample_id"], before["sample_id"]);
    assert_eq!(after["interval"]["elapsed_ms"], 60_000);
    assert_eq!(
        after["interval"]["transport_deltas"]["udp"]["rx_packets"],
        3
    );
    assert_eq!(after["transports"]["udp"]["rx_bytes"], 1300);
    let encoded = after.to_string();
    assert!(!encoded.contains("private-fixture-identity"));
    assert!(!encoded.contains("link_id"));
    assert!(!encoded.contains("address"));
}

#[test]
fn endpoint_peer_link_and_counter_changes_invalidate_deltas() {
    let configured = BTreeSet::new();
    let now = Instant::now();
    for (generation, next, reason) in [
        (2, peer("first", 1, "udp", 20), "endpoint_changed"),
        (1, peer("second", 1, "udp", 20), "peer_changed"),
        (1, peer("first", 2, "udp", 20), "peer_changed"),
        (1, peer("first", 1, "websocket", 20), "peer_changed"),
        (1, peer("first", 1, "udp", 9), "counter_reset"),
    ] {
        let mut history = TrafficHistory::default();
        history.record(vec![peer("first", 1, "udp", 10)], &configured, 1, now);
        let after = history.record(
            vec![next],
            &configured,
            generation,
            now + Duration::from_secs(1),
        );
        assert_eq!(after["valid"], true);
        assert_eq!(after["interval"]["valid"], false);
        assert_eq!(after["interval"]["reason"], reason);
        assert!(after["interval"]["transport_deltas"].is_null());
    }
}

#[tokio::test]
async fn reply_is_once_on_success_error_unavailable_and_timeout() {
    for status in ["available", "query_error", "unavailable", "timeout"] {
        let (tx, rx) = flume::bounded(1);
        let query = async move {
            if status == "timeout" {
                std::future::pending::<()>().await;
            }
            if status == "available" {
                Ok(vec![peer("private-peer", 9, "udp", 1)])
            } else {
                Err(status)
            }
        };
        send_reply(
            json!({"preserved": "field"}),
            tx,
            query,
            Arc::default(),
            BTreeSet::new(),
            1,
            Duration::from_millis(10),
        )
        .await;
        let json: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
        assert_eq!(json["preserved"], "field");
        assert_eq!(json["fips_transport"]["status"], status);
        assert_eq!(json["fips_transport"]["valid"], status == "available");
        if status != "available" {
            assert!(json["fips_transport"]["connected_peer_count"].is_null());
        }
        assert_eq!(rx.try_recv(), Err(flume::TryRecvError::Disconnected));
    }
}

#[test]
fn unexpected_peers_overflow_and_zero_elapsed_never_look_like_clean_traffic() {
    let configured = BTreeSet::from(["expected".to_owned()]);
    let now = Instant::now();
    let mut history = TrafficHistory::default();
    let peers = || {
        vec![
            peer("expected", 1, "udp", 10),
            peer("unexpected", 2, "websocket", 20),
        ]
    };
    let first = history.record(peers(), &configured, 1, now);
    assert_eq!(first["connected_peer_count"], 2);
    assert_eq!(first["connected_configured_direct_peer_count"], 1);
    assert_eq!(first["unexpected_connected_peer_count"], 1);
    assert_eq!(first["transports"]["websocket"]["connected_peer_count"], 1);
    let second = history.record(peers(), &configured, 1, now);
    assert_eq!(second["interval"]["reason"], "no_elapsed_time");
    assert!(second["interval"]["transport_deltas"].is_null());
    let mut large = peer("expected", 1, "udp", 0);
    large.counters.rx_bytes = i64::MAX as u64;
    let overflow = history.record(
        vec![large, peer("unexpected", 2, "udp", 1)],
        &configured,
        1,
        now,
    );
    assert_eq!(overflow["valid"], false);
    assert_eq!(overflow["status"], "invalid_counters");
    assert!(overflow["transports"].is_null());
    assert!(history.baseline.is_none());
}

#[tokio::test]
async fn failed_query_discards_baseline_before_the_next_snapshot() {
    let history = Arc::new(Mutex::new(TrafficHistory::default()));
    history.lock().await.record(
        vec![peer("first", 1, "udp", 10)],
        &BTreeSet::new(),
        1,
        Instant::now(),
    );
    let (tx, rx) = flume::bounded(1);
    send_reply(
        json!({}),
        tx,
        std::future::ready(Err("query_error")),
        history.clone(),
        BTreeSet::new(),
        1,
        Duration::from_millis(10),
    )
    .await;
    let result: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
    assert_eq!(result["fips_transport"]["valid"], false);
    assert!(history.lock().await.baseline.is_none());
}

#[tokio::test]
async fn concurrent_queries_share_the_timeout_budget_and_cannot_overtake() {
    let history = Arc::new(Mutex::new(TrafficHistory::default()));
    let (started_tx, started_rx) = flume::bounded(1);
    let (release_tx, release_rx) = flume::bounded(1);
    let (first_tx, first_rx) = flume::bounded(1);
    let first = tokio::spawn(send_reply(
        json!({}),
        first_tx,
        async move {
            started_tx.send(()).unwrap();
            release_rx.recv_async().await.unwrap();
            Ok(vec![peer("first", 1, "udp", 10)])
        },
        history.clone(),
        BTreeSet::new(),
        1,
        Duration::from_secs(1),
    ));
    started_rx.recv_async().await.unwrap();
    let queried = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = queried.clone();
    let (second_tx, second_rx) = flume::bounded(1);
    send_reply(
        json!({}),
        second_tx,
        async move {
            observed.store(true, Ordering::Relaxed);
            Ok(vec![peer("first", 1, "udp", 20)])
        },
        history.clone(),
        BTreeSet::new(),
        1,
        Duration::from_millis(10),
    )
    .await;
    let second: Value = serde_json::from_str(&second_rx.recv().unwrap()).unwrap();
    assert_eq!(second["fips_transport"]["status"], "timeout");
    assert!(!queried.load(Ordering::Relaxed));
    release_tx.send(()).unwrap();
    first.await.unwrap();
    let first: Value = serde_json::from_str(&first_rx.recv().unwrap()).unwrap();
    assert_eq!(first["fips_transport"]["valid"], true);
    assert_eq!(
        history.lock().await.baseline.as_ref().unwrap().sample_id,
        first["fips_transport"]["sample_id"].as_str().unwrap()
    );
}

#[tokio::test]
async fn live_authenticated_endpoint_produces_private_aggregate_reply() {
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    let SocketAddr::V4(rendezvous) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
    else {
        panic!("expected IPv4");
    };
    async fn endpoint(rendezvous: std::net::SocketAddrV4) -> Arc<FipsEndpoint> {
        let mut config = fips_core::Config::new();
        config.node.control.enabled = false;
        config.node.discovery.local.rendezvous_addr = rendezvous;
        config.node.discovery.local.retry_interval_ms = 20;
        config.node.discovery.lan.enabled = false;
        config.node.discovery.nostr.enabled = false;
        Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .local_rendezvous()
                .without_system_tun()
                .bind()
                .await
                .unwrap(),
        )
    }
    let local = endpoint(rendezvous).await;
    let remote = endpoint(rendezvous).await;
    let peers = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let peers = query_endpoint(Some(local.clone())).await.unwrap();
            if peers.iter().any(|peer| peer.key.identity == remote.npub()) {
                break peers;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (tx, rx) = flume::bounded(1);
    send_reply(
        json!({"existing": 1}),
        tx,
        std::future::ready(Ok(peers)),
        Arc::default(),
        BTreeSet::from([remote.npub().to_owned()]),
        1,
        QUERY_TIMEOUT,
    )
    .await;
    let result: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
    assert_eq!(result["existing"], 1);
    assert_eq!(result["fips_transport"]["connected_peer_count"], 1);
    assert_eq!(
        result["fips_transport"]["connected_configured_direct_peer_count"],
        1
    );
    assert_eq!(
        result["fips_transport"]["unexpected_connected_peer_count"],
        0
    );
    assert!(!result.to_string().contains(remote.npub()));
    assert_eq!(rx.try_recv(), Err(flume::TryRecvError::Disconnected));
    local.shutdown().await.unwrap();
    remote.shutdown().await.unwrap();
}
