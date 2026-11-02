use super::*;
use crate::core::protocol::public_discovery::{test_support, DiscoveryRelays, PublicDiscovery};
use std::sync::atomic::Ordering;

fn reported_account_failure(receiver: &flume::Receiver<CoreMsg>) -> bool {
    receiver.try_iter().any(|message| {
        let CoreMsg::Internal(event) = message else {
            return false;
        };
        matches!(event.as_ref(), InternalEvent::DebugLog { category, detail }
            if category == "protocol.read.error" && detail.starts_with("scope=account "))
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_reads_reuse_account_receiver_and_keep_peer_and_private_scopes_separate() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let oversized = Keys::generate();
    let peer = Keys::generate();
    let session = Keys::generate();
    let (failures, failed_reads) = flume::unbounded();
    let scope = ReadScope {
        account_authors: [
            owner.public_key(),
            device.public_key(),
            sibling.public_key(),
            oversized.public_key(),
        ]
        .into_iter()
        .collect(),
        failure_sender: Some(failures),
    };
    let mut events = [&owner, &device, &sibling, &peer]
        .into_iter()
        .map(|key| {
            EventBuilder::new(Kind::Metadata, "verified fixture")
                .sign_with_keys(key)
                .unwrap()
        })
        .collect::<Vec<_>>();
    events.push(
        EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "ratchet fixture")
            .tags([Tag::public_key(device.public_key())])
            .sign_with_keys(&session)
            .unwrap(),
    );
    let expected = events.iter().map(|event| event.id).collect::<HashSet<_>>();
    events.push(
        EventBuilder::new(Kind::Metadata, "x".repeat(128 * 1024))
            .sign_with_keys(&oversized)
            .unwrap(),
    );
    let mut forged = EventBuilder::new(Kind::Metadata, "forged own metadata")
        .sign_with_keys(&owner)
        .unwrap();
    forged.sig = events.last().unwrap().sig;
    events.push(forged);
    let relay = test_support::plain_relay(events).await;
    let client = Client::new(device.clone());
    client.add_relay(relay.url.clone()).await.unwrap();
    client.connect().await;
    let receiver = client.relay(relay.url.clone()).await.unwrap();
    let retained = SubscriptionId::new("existing-account-receiver");
    receiver
        .subscribe_with_id(
            retained.clone(),
            vec![Filter::new()
                .kind(Kind::Metadata)
                .author(owner.public_key())],
            SubscribeOptions::default(),
        )
        .await
        .unwrap();
    test_support::until(|| {
        relay
            .ready_subjects()
            .contains(&owner.public_key().to_hex())
    })
    .await;
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);

    let pool = Arc::new(DiscoveryRelays::default());
    let (sender, _incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), sender, pool.clone());
    worker
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().kind(Kind::Metadata).author(peer.public_key())],
        )
        .unwrap();
    test_support::until(|| relay.ready_subjects().contains(&peer.public_key().to_hex())).await;
    assert_eq!(relay.connections.load(Ordering::SeqCst), 2);
    let public = Filter::new().kind(Kind::Metadata).authors([
        owner.public_key(),
        device.public_key(),
        sibling.public_key(),
        oversized.public_key(),
        peer.public_key(),
    ]);
    let private = Filter::new()
        .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
        .author(session.public_key())
        .pubkey(device.public_key());
    for _ in 0..2 {
        let fetched = fetch_events_for_filters(
            &client,
            &pool,
            &scope,
            vec![public.clone(), private.clone()],
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(
            fetched.iter().map(|event| event.id).collect::<HashSet<_>>(),
            expected
        );
    }
    // Only private reads get fresh anonymous connections. Public reads reuse
    // the original account receiver and the one anonymous peer connection.
    test_support::until(|| relay.active.load(Ordering::SeqCst) == 2).await;
    assert_eq!(relay.connections.load(Ordering::SeqCst), 4);

    let own_filter = Filter::new()
        .kind(Kind::Metadata)
        .author(owner.public_key());
    let before = relay.requests();
    relay.hold_eose.store(true, Ordering::SeqCst);
    let pending = tokio::spawn({
        let client = client.clone();
        let pool = pool.clone();
        let filter = own_filter.clone();
        let scope = ReadScope {
            account_authors: scope.account_authors.clone(),
            failure_sender: scope.failure_sender.clone(),
        };
        async move {
            fetch_events_for_filters(
                &client,
                &pool,
                &scope,
                vec![filter],
                Duration::from_secs(30),
            )
            .await
        }
    });
    test_support::until(|| relay.requests() > before).await;
    let pending_id = relay
        .wire
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(_, frame)| frame[0] == "REQ")
        .unwrap()
        .1[1]
        .clone();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    test_support::until(|| {
        relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .any(|(_, frame)| frame[0] == "CLOSE" && frame[1] == pending_id)
    })
    .await;
    assert!(receiver
        .subscription(&SubscriptionId::new(pending_id.as_str().unwrap()))
        .await
        .is_none());
    relay.hold_eose.store(false, Ordering::SeqCst);
    assert!(receiver.subscription(&retained).await.is_some());

    relay.reject_subscriptions.store(true, Ordering::SeqCst);
    assert!(fetch_events_for_filters(
        &client,
        &pool,
        &scope,
        vec![own_filter.clone()],
        Duration::from_secs(3)
    )
    .await
    .is_err());
    relay.reject_subscriptions.store(false, Ordering::SeqCst);
    // Rejection and cancellation must not send the own-author read to the peer
    // pool, open a fallback socket, or close the account's retained receiver.
    assert!(receiver.subscription(&retained).await.is_some());
    assert_eq!(relay.connections.load(Ordering::SeqCst), 4);
    let recovered = fetch_events_for_filters(
        &client,
        &pool,
        &scope,
        vec![own_filter],
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].pubkey, owner.public_key());
    assert!(reported_account_failure(&failed_reads));

    let rows = relay.wire.lock().unwrap().clone();
    let mut authentication = BTreeMap::new();
    for (connection, frame) in rows.iter().filter(|(_, frame)| frame[0] == "AUTH") {
        let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
        auth.verify().unwrap();
        authentication.insert(*connection, auth.pubkey);
    }
    assert_eq!(authentication.len(), 4);
    let account_connections = authentication
        .iter()
        .filter(|(_, signer)| **signer == device.public_key())
        .map(|(connection, _)| *connection)
        .collect::<HashSet<_>>();
    assert_eq!(account_connections.len(), 1);
    let mut peers = HashSet::new();
    let mut private_connections = HashSet::new();
    for (connection, frame) in rows.iter().filter(|(_, frame)| frame[0] == "REQ") {
        for value in &frame.as_array().unwrap()[2..] {
            let filter: Filter = serde_json::from_value(value.clone()).unwrap();
            let authors = filter.authors.as_ref().unwrap();
            if account_connections.contains(connection) {
                assert!(authors
                    .iter()
                    .all(|author| scope.account_authors.contains(author)));
                assert_eq!(
                    filter.kinds.as_ref().unwrap(),
                    &std::collections::BTreeSet::from([Kind::Metadata])
                );
                assert!(value.get("#p").is_none());
            } else {
                let signer = authentication[connection];
                assert!(!scope.account_authors.contains(&signer));
                assert_ne!(signer, peer.public_key());
                assert_ne!(signer, session.public_key());
                if value.get("#p").is_some() {
                    private_connections.insert(*connection);
                    assert_eq!(
                        authors,
                        &std::collections::BTreeSet::from([session.public_key()])
                    );
                    assert_eq!(
                        value["#p"],
                        serde_json::json!([device.public_key().to_hex()])
                    );
                    assert_eq!(
                        filter.kinds.as_ref().unwrap(),
                        &std::collections::BTreeSet::from([Kind::from(MESSAGE_EVENT_KIND as u16)])
                    );
                } else {
                    peers.insert(*connection);
                    assert_eq!(
                        authors,
                        &std::collections::BTreeSet::from([peer.public_key()])
                    );
                    assert_eq!(
                        filter.kinds.as_ref().unwrap(),
                        &std::collections::BTreeSet::from([Kind::Metadata])
                    );
                }
            }
        }
    }
    assert_eq!(peers.len(), 1);
    assert_eq!(private_connections.len(), 2);
    assert!(peers.is_disjoint(&private_connections));
    assert_eq!(
        private_connections
            .iter()
            .map(|connection| authentication[connection])
            .collect::<HashSet<_>>()
            .len(),
        2,
        "private retries must use fresh anonymous AUTH"
    );
    assert!(rows.iter().all(|(_, frame)| frame[0] != "EVENT"));
    drop(worker);
    client.shutdown().await;
    pool.stop(&tokio::runtime::Handle::current());
    test_support::until(|| relay.active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_auth_or_query_failure_preserves_successful_peer_and_private_reads() {
    for reject_auth in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let session = Keys::generate();
        let own_event = EventBuilder::new(Kind::Metadata, "own")
            .sign_with_keys(&owner)
            .unwrap();
        let peer_event = EventBuilder::new(Kind::Metadata, "peer")
            .sign_with_keys(&peer)
            .unwrap();
        let private_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "private")
            .tags([Tag::public_key(device.public_key())])
            .sign_with_keys(&session)
            .unwrap();
        let relay =
            test_support::plain_relay(vec![own_event, peer_event.clone(), private_event.clone()])
                .await;
        let rejected = if reject_auth {
            &relay.reject_authentication_for
        } else {
            &relay.reject_requests_for
        };
        rejected.lock().unwrap().insert(device.public_key());
        let client = Client::new(device.clone());
        client.add_relay(relay.url.clone()).await.unwrap();
        client.connect().await;
        let pool = Arc::new(DiscoveryRelays::default());
        let (sender, _incoming) = flume::unbounded();
        let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), sender, pool.clone());
        worker
            .configure(
                std::slice::from_ref(&relay.url),
                vec![Filter::new().kind(Kind::Metadata).author(peer.public_key())],
            )
            .unwrap();
        test_support::until(|| relay.ready_subjects().contains(&peer.public_key().to_hex())).await;
        let (sender, failures) = flume::unbounded();
        let scope = ReadScope {
            account_authors: HashSet::from([owner.public_key(), device.public_key()]),
            failure_sender: Some(sender),
        };
        let filters = vec![
            Filter::new()
                .kind(Kind::Metadata)
                .authors([owner.public_key(), peer.public_key()]),
            Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .author(session.public_key())
                .pubkey(device.public_key()),
        ];
        let result =
            fetch_events_for_filters(&client, &pool, &scope, filters, Duration::from_secs(3))
                .await
                .unwrap();
        assert_eq!(
            result.iter().map(|event| event.id).collect::<HashSet<_>>(),
            HashSet::from([peer_event.id, private_event.id])
        );
        assert!(reported_account_failure(&failures));
        assert!(fetch_events_for_filters(
            &client,
            &pool,
            &scope,
            vec![Filter::new()
                .kind(Kind::Metadata)
                .author(owner.public_key())],
            Duration::from_secs(3)
        )
        .await
        .is_err());
        assert!(reported_account_failure(&failures));

        let rows = relay.wire.lock().unwrap().clone();
        let authentication = rows
            .iter()
            .filter(|(_, frame)| frame[0] == "AUTH")
            .map(|(connection, frame)| {
                let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                event.verify().unwrap();
                (*connection, event.pubkey)
            })
            .collect::<BTreeMap<_, _>>();
        let mut peer_connections = HashSet::new();
        let mut private_connections = HashSet::new();
        for (connection, frame) in rows.iter().filter(|(_, frame)| frame[0] == "REQ") {
            for value in &frame.as_array().unwrap()[2..] {
                let filter: Filter = serde_json::from_value(value.clone()).unwrap();
                let authors = filter.authors.as_ref().unwrap();
                let signer = authentication[connection];
                if signer != device.public_key() {
                    assert!(!scope.account_authors.contains(&signer));
                    assert_ne!(signer, peer.public_key());
                    assert_ne!(signer, session.public_key());
                }
                if authentication[connection] == device.public_key() {
                    assert_eq!(
                        authors,
                        &std::collections::BTreeSet::from([owner.public_key()])
                    );
                    assert_eq!(value["kinds"], serde_json::json!([0]));
                    assert!(value.get("#p").is_none());
                } else if value.get("#p").is_some() {
                    private_connections.insert(*connection);
                    assert_eq!(
                        authors,
                        &std::collections::BTreeSet::from([session.public_key()])
                    );
                    assert_eq!(
                        value["#p"],
                        serde_json::json!([device.public_key().to_hex()])
                    );
                } else {
                    peer_connections.insert(*connection);
                    assert_eq!(
                        authors,
                        &std::collections::BTreeSet::from([peer.public_key()])
                    );
                    assert_eq!(value["kinds"], serde_json::json!([0]));
                }
            }
        }
        assert_eq!(peer_connections.len(), 1);
        assert_eq!(private_connections.len(), 1);
        assert!(peer_connections.is_disjoint(&private_connections));
        assert!(rows.iter().all(|(_, frame)| frame[0] != "EVENT"));
        drop(worker);
        client.shutdown().await;
        pool.stop(&tokio::runtime::Handle::current());
        test_support::until(|| relay.active.load(Ordering::SeqCst) == 0).await;
    }
}
