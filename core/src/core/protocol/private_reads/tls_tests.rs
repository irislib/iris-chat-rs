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
