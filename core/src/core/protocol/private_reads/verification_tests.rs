use super::*;
use crate::core::protocol::anonymous_reads::tls_tests as fixture;
use std::sync::atomic::Ordering;

async fn wait_completed(relay: &fixture::Relay, author: PublicKey, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let complete = {
                let rows = relay.wire.lock().unwrap();
                rows.iter()
                    .filter(|(id, value)| {
                        value[0] == "REQ"
                            && value[2]["authors"] == serde_json::json!([author.to_hex()])
                            && rows
                                .iter()
                                .any(|(closed, value)| closed == id && value[0] == "CLOSE")
                    })
                    .map(|(id, _)| *id)
                    .collect::<HashSet<_>>()
                    .len()
            };
            if complete == expected {
                break;
            }
            assert!(complete < expected, "unexpected extra discovery poll");
            sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("retained verification worker did not finish its query");
}

async fn delivered(receiver: &flume::Receiver<CoreMsg>, expected: &[Event]) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut ids = HashSet::new();
        for _ in expected {
            let CoreMsg::Internal(message) = receiver.recv_async().await.unwrap() else {
                panic!("unexpected core message");
            };
            let InternalEvent::RelayEvent(event) = *message else {
                panic!("unexpected internal message");
            };
            assert!(expected.contains(&event));
            assert!(ids.insert(event.id));
        }
    })
    .await
    .expect("verified events were not delivered");
}

fn configure(worker: &PrivateDiscoveryClient, url: &RelayUrl, filters: Vec<Filter>) {
    worker
        .configure(std::slice::from_ref(url), filters)
        .unwrap();
    worker
        .set_test_tls_roots(url, fixture::certificates())
        .unwrap();
}

async fn stop(worker: &PrivateDiscoveryClient) {
    worker.cancel();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !worker.task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("verification worker did not release its scoped state");
}

#[tokio::test]
async fn retained_signature_cache_redelivers_and_resets_with_actual_worker_config() {
    tokio::time::timeout(Duration::from_secs(12), async {
        let alice = Keys::generate();
        let bob = Keys::generate();
        let events = (0..32)
            .map(|index| {
                EventBuilder::new(Kind::Metadata, index.to_string())
                    .sign_with_keys(&alice)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let bob_event = EventBuilder::new(Kind::Metadata, "bob")
            .sign_with_keys(&bob)
            .unwrap();
        let mut all = events.clone();
        all.push(bob_event.clone());
        let relay = fixture::relay(true, false, all).await;
        let filters = vec![Filter::new()
            .author(alice.public_key())
            .kind(Kind::Metadata)];
        let runtime = tokio::runtime::Handle::current();
        let (sender, incoming) = flume::unbounded();
        let worker = PrivateDiscoveryClient::new(&runtime, sender.clone());
        configure(&worker, &relay.url, filters.clone());
        wait_completed(&relay, alice.public_key(), 1).await;
        delivered(&incoming, &events).await;
        assert_eq!(worker.signature_calls.load(Ordering::Relaxed), 32);

        worker.refresh();
        wait_completed(&relay, alice.public_key(), 2).await;
        delivered(&incoming, &events).await;
        assert_eq!(
            worker.signature_calls.load(Ordering::Relaxed),
            32,
            "demand must redeliver cache hits"
        );

        worker
            .configure(
                std::slice::from_ref(&relay.url),
                vec![filters[0].clone().limit(64)],
            )
            .unwrap();
        wait_completed(&relay, alice.public_key(), 3).await;
        assert_eq!(
            worker.signature_calls.load(Ordering::Relaxed),
            64,
            "filter change must discard positives"
        );
        assert!(
            incoming.is_empty(),
            "a config change must not alter normal ID deduplication"
        );

        let changed_url = RelayUrl::parse(&format!(
            "{}/changed-discovery",
            relay.url.as_str().trim_end_matches('/')
        ))
        .unwrap();
        configure(&worker, &changed_url, filters.clone());
        wait_completed(&relay, alice.public_key(), 4).await;
        assert_eq!(
            worker.signature_calls.load(Ordering::Relaxed),
            96,
            "relay-index reuse must discard old positives"
        );

        let (other_sender, other_incoming) = flume::unbounded();
        let other = PrivateDiscoveryClient::new(&runtime, other_sender);
        configure(
            &other,
            &relay.url,
            vec![Filter::new().author(bob.public_key()).kind(Kind::Metadata)],
        );
        wait_completed(&relay, bob.public_key(), 1).await;
        delivered(&other_incoming, std::slice::from_ref(&bob_event)).await;
        other.refresh();
        wait_completed(&relay, bob.public_key(), 2).await;
        delivered(&other_incoming, std::slice::from_ref(&bob_event)).await;
        assert_eq!(other.signature_calls.load(Ordering::Relaxed), 1);

        stop(&worker).await;
        worker.refresh();
        assert!(worker
            .configure(std::slice::from_ref(&changed_url), filters.clone())
            .is_err());
        let restarted = PrivateDiscoveryClient::new(&runtime, sender);
        configure(&restarted, &changed_url, filters);
        wait_completed(&relay, alice.public_key(), 5).await;
        delivered(&incoming, &events).await;
        assert_eq!(restarted.signature_calls.load(Ordering::Relaxed), 32);
        assert_eq!(worker.signature_calls.load(Ordering::Relaxed), 96);
        stop(&restarted).await;
        stop(&other).await;
        fixture::assert_wire(&relay, &[alice.public_key(), bob.public_key()], 7);
    })
    .await
    .expect("retained verification wire regression timed out");
}

#[tokio::test]
async fn retained_signature_cache_relay_reordering_reverifies_each_partition() {
    tokio::time::timeout(Duration::from_secs(12), async {
        let author = Keys::generate();
        let event = EventBuilder::new(Kind::Metadata, "same public event on both relays")
            .sign_with_keys(&author)
            .unwrap();
        let first = fixture::relay(true, false, vec![event.clone()]).await;
        let second = fixture::relay(true, false, vec![event.clone()]).await;
        let filters = vec![Filter::new()
            .author(author.public_key())
            .kind(Kind::Metadata)];
        let (sender, incoming) = flume::unbounded();
        let worker = PrivateDiscoveryClient::new(&tokio::runtime::Handle::current(), sender);
        worker
            .configure(&[first.url.clone(), second.url.clone()], filters.clone())
            .unwrap();
        for relay in [&first, &second] {
            worker
                .set_test_tls_roots(&relay.url, fixture::certificates())
                .unwrap();
        }
        wait_completed(&first, author.public_key(), 1).await;
        wait_completed(&second, author.public_key(), 1).await;
        delivered(&incoming, std::slice::from_ref(&event)).await;
        assert_eq!(worker.signature_calls.load(Ordering::Relaxed), 2);
        worker.refresh();
        wait_completed(&first, author.public_key(), 2).await;
        wait_completed(&second, author.public_key(), 2).await;
        // Explicit recovery redelivers one response from each relay, preserving
        // the original delivery behavior even though both are signature hits.
        delivered(&incoming, std::slice::from_ref(&event)).await;
        delivered(&incoming, std::slice::from_ref(&event)).await;
        assert_eq!(worker.signature_calls.load(Ordering::Relaxed), 2);
        worker
            .configure(&[second.url.clone(), first.url.clone()], filters)
            .unwrap();
        wait_completed(&first, author.public_key(), 3).await;
        wait_completed(&second, author.public_key(), 3).await;
        assert_eq!(
            worker.signature_calls.load(Ordering::Relaxed),
            4,
            "relay indices cannot survive config reordering"
        );
        assert!(incoming.is_empty());
        stop(&worker).await;
        fixture::assert_wire(&first, &[author.public_key()], 3);
        fixture::assert_wire(&second, &[author.public_key()], 3);
    })
    .await
    .expect("relay partition reconfiguration timed out");
}
