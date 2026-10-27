use super::*;
use crate::core::protocol::anonymous_reads::tls_tests as fixture;
use rustls::{HandshakeKind, ProtocolVersion};

fn completed(relay: &fixture::Relay, author: PublicKey) -> usize {
    let rows = relay.wire.lock().unwrap();
    rows.iter()
        .filter(|(id, value)| {
            value[0] == "REQ"
                && value
                    .as_array()
                    .unwrap()
                    .iter()
                    .skip(2)
                    .any(|filter| filter["authors"] == serde_json::json!([author.to_hex()]))
                && rows
                    .iter()
                    .any(|(closed, value)| closed == id && value[0] == "CLOSE")
        })
        .map(|(id, _)| *id)
        .collect::<HashSet<_>>()
        .len()
}

async fn wait_completed(relay: &fixture::Relay, author: PublicKey, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while completed(relay, author) < expected {
            sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("discovery worker did not complete its TLS query");
    assert_eq!(completed(relay, author), expected);
}

async fn stop(worker: &PrivateDiscoveryClient) {
    worker.cancel();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !worker.task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled discovery worker retained its task and TLS state");
}

async fn wait_queries(relay: &fixture::Relay, author: PublicKey, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let count = {
                let rows = relay.wire.lock().unwrap();
                rows.iter()
                    .filter(|(_, value)| value[0] == "CLOSE")
                    .filter(|(connection, closed)| {
                        rows.iter().any(|(id, request)| {
                            id == connection
                                && request[0] == "REQ"
                                && request[1] == closed[1]
                                && request[2]["authors"] == serde_json::json!([author.to_hex()])
                        })
                    })
                    .count()
            };
            if count == expected {
                break;
            }
            assert!(count < expected, "unexpected extra query");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retained_worker_reuses_reads_and_discards_sockets_on_configuration_and_restart() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let relay = fixture::persistent_relay(vec![]).await;
        let alice = Keys::generate().public_key();
        let bob = Keys::generate().public_key();
        let filter = |author| Filter::new().author(author).kind(Kind::Metadata);
        let runtime = tokio::runtime::Handle::current();
        let (sender, _events) = flume::unbounded();
        let first = PrivateDiscoveryClient::new(&runtime, sender.clone());
        let second = PrivateDiscoveryClient::new(&runtime, sender.clone());
        configure(&first, &relay.url, vec![filter(alice)]);
        wait_queries(&relay, alice, 1).await;
        configure(&second, &relay.url, vec![filter(bob)]);
        wait_queries(&relay, bob, 1).await;
        for expected in 2..=4 {
            first.refresh();
            wait_queries(&relay, alice, expected).await;
        }
        assert_eq!(relay.handshakes.lock().unwrap().len(), 2);
        fixture::assert_wire(&relay, &[alice, bob], 2);
        first
            .configure(
                std::slice::from_ref(&relay.url),
                vec![filter(alice).limit(1)],
            )
            .unwrap();
        wait_queries(&relay, alice, 5).await;
        assert_eq!(relay.handshakes.lock().unwrap().len(), 3);
        stop(&first).await;
        let restarted = PrivateDiscoveryClient::new(&runtime, sender);
        configure(&restarted, &relay.url, vec![filter(alice)]);
        wait_queries(&relay, alice, 6).await;
        assert_eq!(relay.handshakes.lock().unwrap().len(), 4);
        fixture::assert_wire(&relay, &[alice, bob], 4);
        stop(&restarted).await;
        stop(&second).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn discovery_worker_services_retained_sibling_controls_during_pending_relay_read() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let idle = Arc::new(fixture::RelayControl {
            ping_after_close: true,
            ..Default::default()
        });
        let stalled = Arc::new(fixture::RelayControl {
            stall_queries: true,
            ..Default::default()
        });
        let first = fixture::controlled_persistent_relay(idle.clone()).await;
        let second = fixture::controlled_persistent_relay(stalled.clone()).await;
        let author = Keys::generate().public_key();
        let (sender, events) = flume::unbounded();
        let worker = PrivateDiscoveryClient::new(&tokio::runtime::Handle::current(), sender);
        worker
            .configure(
                &[first.url.clone(), second.url.clone()],
                vec![Filter::new().author(author).kind(Kind::Metadata)],
            )
            .unwrap();
        for relay in [&first, &second] {
            worker
                .set_test_tls_roots(&relay.url, fixture::certificates())
                .unwrap();
        }
        // The second authenticated REQ is on the wire, but its EOSE is held.
        // Thus the whole sweep cannot finish before the idle Pong is observed.
        stalled.query_waiting.notified().await;
        wait_queries(&first, author, 1).await;
        idle.send_ping.notify_one();
        tokio::time::timeout(Duration::from_secs(1), idle.pong.notified())
            .await
            .expect("a stalled sibling read starved the retained socket's Ping");
        let count = |relay: &fixture::Relay, kind: &str| {
            relay
                .wire
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, value)| value[0] == kind)
                .count()
        };
        assert_eq!(count(&second, "CLOSE"), 0);
        for relay in [&first, &second] {
            assert_eq!(
                count(relay, "REQ"),
                2,
                "initial REQ and anonymous AUTH retry only"
            );
            assert_eq!(count(relay, "AUTH"), 1);
            assert_eq!(relay.handshakes.lock().unwrap().len(), 1);
        }
        assert!(
            events.is_empty(),
            "idle controls must not create application events"
        );

        stalled.release_query.notify_one();
        wait_queries(&second, author, 1).await;
        // An explicit next poll must reuse each exact socket, proving that
        // servicing the Ping retained it rather than merely delaying closure.
        worker.refresh();
        stalled.release_query.notify_one();
        wait_queries(&first, author, 2).await;
        wait_queries(&second, author, 2).await;
        for relay in [&first, &second] {
            assert_eq!(count(relay, "REQ"), 3, "no extra query from idle handling");
            fixture::assert_wire(relay, &[author], 1);
            assert_eq!(relay.handshakes.lock().unwrap().len(), 1);
        }
        let auth_key = |relay: &fixture::Relay| {
            let rows = relay.wire.lock().unwrap();
            let auth = rows.iter().find(|(_, value)| value[0] == "AUTH").unwrap();
            serde_json::from_value::<Event>(auth.1[1].clone())
                .unwrap()
                .pubkey
        };
        assert_ne!(
            auth_key(&first),
            auth_key(&second),
            "relay scopes cannot share AUTH"
        );
        stop(&worker).await;
    })
    .await
    .expect("stalled-sibling idle-control regression timed out");
}

fn configure(worker: &PrivateDiscoveryClient, relay: &RelayUrl, filters: Vec<Filter>) {
    worker
        .configure(std::slice::from_ref(relay), filters)
        .unwrap();
    worker
        .set_test_tls_roots(relay, fixture::certificates())
        .unwrap();
}

#[tokio::test]
async fn discovery_worker_reconfiguration_and_shutdown_discard_scoped_tls_tickets() {
    tokio::time::timeout(Duration::from_secs(12), async {
        let relay = fixture::relay(true, false, vec![]).await;
        let alice = Keys::generate().public_key();
        let bob = Keys::generate().public_key();
        let filter = |author| Filter::new().author(author).kind(Kind::Metadata);
        let runtime = tokio::runtime::Handle::current();
        let (sender, _events) = flume::unbounded();
        let first = PrivateDiscoveryClient::new(&runtime, sender.clone());
        configure(&first, &relay.url, vec![filter(alice)]);
        wait_completed(&relay, alice, 1).await;
        first.refresh();
        wait_completed(&relay, alice, 2).await;

        let other = PrivateDiscoveryClient::new(&runtime, sender.clone());
        configure(&other, &relay.url, vec![filter(bob)]);
        wait_completed(&relay, bob, 1).await;

        // The actual watch/configuration path must drop tickets even when the
        // author and server name are unchanged but the query changes.
        first
            .configure(
                std::slice::from_ref(&relay.url),
                vec![filter(alice).limit(1)],
            )
            .unwrap();
        wait_completed(&relay, alice, 3).await;
        let changed_url = RelayUrl::parse(&format!(
            "{}/changed-discovery",
            relay.url.as_str().trim_end_matches('/')
        ))
        .unwrap();
        configure(&first, &changed_url, vec![filter(alice)]);
        wait_completed(&relay, alice, 4).await;

        // Await actual cancellation before starting a replacement, rather
        // than allowing the old worker to retain sockets or tickets in flight.
        stop(&first).await;
        first.refresh();
        assert!(first
            .configure(std::slice::from_ref(&changed_url), vec![filter(alice)])
            .is_err());
        drop(first);
        let restarted = PrivateDiscoveryClient::new(&runtime, sender);
        configure(&restarted, &changed_url, vec![filter(alice)]);
        wait_completed(&relay, alice, 5).await;
        other.refresh();
        wait_completed(&relay, bob, 2).await;
        stop(&restarted).await;
        stop(&other).await;

        let observed = relay.handshakes.lock().unwrap().clone();
        assert_eq!(
            observed.iter().map(|row| row.0).collect::<Vec<_>>(),
            [
                HandshakeKind::Full,
                HandshakeKind::Resumed,
                HandshakeKind::Full,
                HandshakeKind::Full,
                HandshakeKind::Full,
                HandshakeKind::Full,
                HandshakeKind::Resumed,
            ],
            "actual worker handshake sequence: {observed:?}"
        );
        assert!(observed
            .iter()
            .all(|(_, version, dhe)| *version == ProtocolVersion::TLSv1_3 && *dhe));
        fixture::assert_wire(&relay, &[alice, bob], 7);
        assert_eq!(completed(&relay, alice), 5);
        assert_eq!(completed(&relay, bob), 2);
    })
    .await
    .expect("discovery worker TLS lifecycle proof timed out");
}
