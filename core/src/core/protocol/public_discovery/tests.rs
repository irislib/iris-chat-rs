use super::test_support::{pool, relay, until};
use super::*;
use std::sync::atomic::Ordering;

fn metadata(keys: &Keys, content: &str) -> Event {
    EventBuilder::new(Kind::Metadata, content)
        .sign_with_keys(keys)
        .unwrap()
}
async fn receive(receiver: &flume::Receiver<CoreMsg>) -> Event {
    match tokio::time::timeout(Duration::from_secs(30), receiver.recv_async())
        .await
        .unwrap()
        .unwrap()
    {
        CoreMsg::Internal(event) => match *event {
            InternalEvent::RelayEvent(event) => event,
            _ => panic!("unexpected event"),
        },
        _ => panic!("unexpected message"),
    }
}

#[test]
fn merges_only_compatible_author_selectors_without_weakening_limits_or_tags() {
    let a = Keys::generate().public_key();
    let b = Keys::generate().public_key();
    let base = Filter::new().kind(Kind::Metadata);
    let merged = merge_filters(vec![base.clone().author(a), base.clone().author(b)]);
    assert_eq!(merged, vec![base.clone().authors([a, b])]);
    for changed in [
        base.clone().author(b).limit(1),
        base.clone().author(b).since(Timestamp::from(1)),
        base.clone().author(b).pubkey(a),
    ] {
        assert_eq!(
            merge_filters(vec![base.clone().author(a), changed]).len(),
            2
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn populated_live_and_cold_public_discovery_share_one_connection_and_auth_identity() {
    let keys = (0..512).map(|_| Keys::generate()).collect::<Vec<_>>();
    let events = keys
        .iter()
        .map(|key| metadata(key, "initial"))
        .collect::<Vec<_>>();
    let relay = relay(events.clone()).await;
    let pool = pool();
    let (send, receive_queue) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool.clone());
    worker
        .configure(
            std::slice::from_ref(&relay.url),
            keys.iter()
                .map(|key| Filter::new().kind(Kind::Metadata).author(key.public_key()))
                .collect(),
        )
        .unwrap();
    let mut delivered = HashSet::new();
    while delivered.len() < keys.len() {
        delivered.insert(receive(&receive_queue).await.id);
    }
    assert_eq!(delivered, events.iter().map(|event| event.id).collect());
    let before = relay.requests();
    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        relay.requests(),
        before,
        "healthy subscriptions must not poll"
    );
    let cold = pool
        .fetch(
            std::slice::from_ref(&relay.url),
            vec![Filter::new()
                .kind(Kind::Metadata)
                .authors([keys[0].public_key(), keys[1].public_key()])],
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert_eq!(cold.len(), 2);
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
    let rows = relay.wire.lock().unwrap().clone();
    let auth = rows
        .iter()
        .filter(|(_, frame)| frame[0] == "AUTH")
        .collect::<Vec<_>>();
    assert_eq!(auth.len(), 1);
    let event: Event = serde_json::from_value(auth[0].1[1].clone()).unwrap();
    assert!(keys.iter().all(|key| key.public_key() != event.pubkey));
    assert!(rows.iter().all(|(_, frame)| frame[0] != "EVENT"));
    drop(worker);
    until(|| relay.active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_updates_verify_signatures_and_targeted_recovery_redelivers_only_requested_author() {
    let a = Keys::generate();
    let b = Keys::generate();
    let initial = [metadata(&a, "a"), metadata(&b, "b")];
    let relay = relay(initial.to_vec()).await;
    let (send, incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool());
    worker
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new()
                .kind(Kind::Metadata)
                .authors([a.public_key(), b.public_key()])],
        )
        .unwrap();
    for _ in &initial {
        receive(&incoming).await;
    }
    let changed = metadata(&a, "changed");
    relay.publish(changed.clone());
    assert_eq!(receive(&incoming).await, changed);
    let mut forged = metadata(&a, "forged");
    forged.sig = metadata(&b, "signature").sig;
    relay.publish(forged);
    sleep(Duration::from_millis(200)).await;
    assert!(incoming.is_empty());
    let before = relay.wire.lock().unwrap().len();
    worker.refresh(Some(b.public_key()));
    assert_eq!(receive(&incoming).await, initial[1]);
    let rows = relay.wire.lock().unwrap().clone();
    for (_, frame) in rows
        .iter()
        .skip(before)
        .filter(|(_, frame)| frame[0] == "REQ")
    {
        assert_eq!(
            frame[2]["authors"],
            serde_json::json!([b.public_key().to_hex()])
        );
    }
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
    assert!(incoming.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconfiguration_and_drop_close_only_their_subscriptions_on_shared_transport() {
    let a = Keys::generate();
    let b = Keys::generate();
    let relay = relay(vec![metadata(&a, "a"), metadata(&b, "b")]).await;
    let pool = pool();
    let (first_send, first_events) = flume::unbounded();
    let (second_send, second_events) = flume::unbounded();
    let first = PublicDiscovery::new(&tokio::runtime::Handle::current(), first_send, pool.clone());
    let second = PublicDiscovery::new(&tokio::runtime::Handle::current(), second_send, pool);
    first
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().author(a.public_key()).kind(Kind::Metadata)],
        )
        .unwrap();
    second
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().author(b.public_key()).kind(Kind::Metadata)],
        )
        .unwrap();
    receive(&first_events).await;
    receive(&second_events).await;
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
    let before = relay
        .wire
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, frame)| frame[0] == "CLOSE")
        .count();
    drop(first);
    until(|| {
        relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, frame)| frame[0] == "CLOSE")
            .count()
            > before
    })
    .await;
    let changed = metadata(&b, "still live");
    relay.publish(changed.clone());
    assert_eq!(receive(&second_events).await, changed);
    assert_eq!(relay.active.load(Ordering::SeqCst), 1);
    second
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().author(a.public_key()).kind(Kind::Metadata)],
        )
        .unwrap();
    assert_eq!(receive(&second_events).await.pubkey, a.public_key());
    relay.publish(metadata(&b, "removed"));
    sleep(Duration::from_millis(200)).await;
    assert!(second_events.is_empty());
    drop(second);
    until(|| relay.active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_restores_live_filters_and_recovers_late_stored_metadata() {
    let peer = Keys::generate();
    let relay = relay(vec![metadata(&peer, "initial")]).await;
    let (send, incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool());
    worker
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().kind(Kind::Metadata).author(peer.public_key())],
        )
        .unwrap();
    receive(&incoming).await;
    relay.disconnect();
    until(|| relay.active.load(Ordering::SeqCst) == 0).await;
    let late = EventBuilder::new(Kind::Metadata, "late older metadata")
        .custom_created_at(Timestamp::from(unix_now().get() - 60))
        .sign_with_keys(&peer)
        .unwrap();
    relay.events.lock().unwrap().push(late.clone());
    assert_eq!(receive(&incoming).await, late);
    assert_eq!(relay.connections.load(Ordering::SeqCst), 2);
    assert_eq!(relay.active.load(Ordering::SeqCst), 1);
}
