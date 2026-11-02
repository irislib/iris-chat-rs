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
    assert_eq!(
        delivered,
        events.iter().map(|event| event.id).collect::<HashSet<_>>()
    );
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
    relay.publish(metadata(&a, &"x".repeat(128 * 1024)));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_setup_closes_all_its_subscriptions_without_stopping_shared_peer() {
    let a = Keys::generate();
    let b = Keys::generate();
    let relay = relay(vec![metadata(&a, "a"), metadata(&b, "b")]).await;
    let pool = pool();
    let (send, incoming) = flume::unbounded();
    let survivor = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool.clone());
    survivor
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().author(a.public_key()).kind(Kind::Metadata)],
        )
        .unwrap();
    receive(&incoming).await;
    let worker = PublicDiscovery::new(
        &tokio::runtime::Handle::current(),
        flume::unbounded().0,
        pool.clone(),
    );
    let filters = (1..=512)
        .map(|limit| {
            Filter::new()
                .author(b.public_key())
                .kind(Kind::Metadata)
                .limit(limit)
        })
        .collect();
    worker
        .configure(std::slice::from_ref(&relay.url), filters)
        .unwrap();
    until(|| {
        relay.wire.lock().unwrap().iter().any(|(_, row)| {
            row[0] == "REQ" && row[2]["authors"] == serde_json::json!([b.public_key().to_hex()])
        })
    })
    .await;
    worker.cancel();
    assert!(worker
        .configure(
            std::slice::from_ref(&relay.url),
            vec![Filter::new().author(b.public_key())]
        )
        .is_err());
    until(|| {
        let rows = relay.wire.lock().unwrap();
        let ids = rows
            .iter()
            .filter(|(_, row)| {
                row[0] == "REQ" && row[2]["authors"] == serde_json::json!([b.public_key().to_hex()])
            })
            .map(|(_, row)| row[1].clone())
            .collect::<Vec<_>>();
        !ids.is_empty()
            && ids.iter().all(|id| {
                rows.iter()
                    .any(|(_, row)| row[0] == "CLOSE" && &row[1] == id)
            })
    })
    .await;
    // Holding the cancelled worker must not retain a lease or revive filters.
    until(|| {
        pool.users
            .try_lock()
            .is_ok_and(|users| users.get(&relay.url) == Some(&1))
    })
    .await;
    let changed = metadata(&a, "survives cancellation");
    relay.publish(changed.clone());
    assert_eq!(receive(&incoming).await, changed);
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
    drop(survivor);
    until(|| relay.active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_and_cold_queries_enforce_byte_budget_and_release_admission_after_flood() {
    let peer = Keys::generate();
    let relay = relay(Vec::new()).await;
    let pool = pool();
    let filter = Filter::new().author(peer.public_key()).kind(Kind::Metadata);
    let (send, incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool.clone());
    worker
        .configure(std::slice::from_ref(&relay.url), vec![filter.clone()])
        .unwrap();
    until(|| relay.ready_subjects() == HashSet::from([peer.public_key().to_hex()])).await;
    let body = "x".repeat(70 * 1024);
    *relay.events.lock().unwrap() = (0..256)
        .map(|index| metadata(&peer, &format!("{index}:{body}")))
        .collect();
    let error = pool
        .fetch(
            std::slice::from_ref(&relay.url),
            vec![filter.clone()],
            Duration::from_secs(3),
        )
        .await
        .unwrap_err();
    assert!(error.contains("safety bound"), "{error}");
    worker.refresh(Some(peer.public_key()));
    until(|| {
        relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, row)| row[0] == "CLOSE")
            .count()
            >= 2
    })
    .await;
    assert!(
        incoming.is_empty(),
        "overflow must not masquerade as complete metadata"
    );
    let before = relay.requests();
    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        relay.requests(),
        before,
        "resource rejection must not start a retry storm"
    );
    let small = metadata(&peer, "small recovery");
    *relay.events.lock().unwrap() = vec![small.clone()];
    assert_eq!(
        pool.fetch(
            std::slice::from_ref(&relay.url),
            vec![filter],
            Duration::from_secs(3)
        )
        .await
        .unwrap(),
        vec![small.clone()]
    );
    worker.refresh(Some(peer.public_key()));
    assert_eq!(receive(&incoming).await, small);
    assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readiness_requires_every_relay_to_accept_and_finish_its_current_subscription() {
    let peer = Keys::generate();
    let event = metadata(&peer, "initial");
    let subjects = HashSet::from([peer.public_key().to_hex()]);
    let mut relays = Vec::new();
    for _ in 0..5 {
        relays.push(relay(vec![event.clone()]).await);
    }
    relays[4].reject_subscriptions.store(true, Ordering::SeqCst);
    let (send, incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool());
    worker
        .configure(
            &relays
                .iter()
                .map(|relay| relay.url.clone())
                .collect::<Vec<_>>(),
            vec![Filter::new().author(peer.public_key()).kind(Kind::Metadata)],
        )
        .unwrap();
    assert_eq!(
        receive(&incoming).await,
        event,
        "one relay can deliver all data"
    );
    until(|| {
        relays[..4]
            .iter()
            .all(|relay| relay.ready_subjects() == subjects)
            && relays[4].requests() > 0
    })
    .await;
    assert!(
        relays[4].ready_subjects().is_empty(),
        "rejected requests cannot establish readiness"
    );
    assert!(!relays
        .iter()
        .all(|relay| relay.ready_subjects() == subjects));
    drop(worker);
    for relay in &relays {
        until(|| relay.active.load(Ordering::SeqCst) == 0).await;
        assert!(
            relay.ready_subjects().is_empty(),
            "closed/disconnected subscriptions remain unready"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_query_closes_only_requested_relay_and_releases_admission() {
    let peer = Keys::generate();
    let event = metadata(&peer, "initial");
    let a = relay(vec![event.clone()]).await;
    let b = relay(vec![event.clone()]).await;
    let pool = pool();
    let (send, incoming) = flume::unbounded();
    let worker = PublicDiscovery::new(&tokio::runtime::Handle::current(), send, pool.clone());
    let filter = Filter::new().author(peer.public_key()).kind(Kind::Metadata);
    worker
        .configure(&[a.url.clone(), b.url.clone()], vec![filter.clone()])
        .unwrap();
    receive(&incoming).await;
    until(|| a.ready_subjects().len() == 1 && b.ready_subjects().len() == 1).await;
    a.hold_eose.store(true, Ordering::SeqCst);
    let before_a = a.requests();
    let before_b = b.requests();
    let querying = {
        let pool = pool.clone();
        let url = a.url.clone();
        let filter = filter.clone();
        tokio::spawn(async move {
            pool.fetch(&[url], vec![filter], Duration::from_secs(30))
                .await
        })
    };
    until(|| a.requests() > before_a).await;
    let id = a
        .wire
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(_, row)| row[0] == "REQ")
        .unwrap()
        .1[1]
        .clone();
    querying.abort();
    assert!(querying.await.unwrap_err().is_cancelled());
    until(|| {
        a.wire
            .lock()
            .unwrap()
            .iter()
            .any(|(_, row)| row[0] == "CLOSE" && row[1] == id)
    })
    .await;
    assert_eq!(pool.queries.available_permits(), 1);
    assert_eq!(
        b.requests(),
        before_b,
        "query escaped requested relay subset"
    );
    assert_eq!(a.active.load(Ordering::SeqCst), 1);
    a.hold_eose.store(false, Ordering::SeqCst);
    assert_eq!(
        pool.fetch(
            std::slice::from_ref(&a.url),
            vec![filter.clone()],
            Duration::from_secs(3)
        )
        .await
        .unwrap(),
        vec![event.clone()]
    );
    b.hold_eose.store(true, Ordering::SeqCst);
    assert_eq!(
        pool.fetch(
            &[a.url.clone(), b.url.clone()],
            vec![filter],
            Duration::from_millis(300)
        )
        .await
        .unwrap(),
        vec![event],
        "a stalled relay must preserve verified results from its healthy peer"
    );
    b.hold_eose.store(false, Ordering::SeqCst);
    let changed = metadata(&peer, "live after cancellation");
    b.publish(changed.clone());
    assert_eq!(receive(&incoming).await, changed);
    assert_eq!(a.connections.load(Ordering::SeqCst), 1);
    assert_eq!(b.connections.load(Ordering::SeqCst), 1);
}

#[test]
fn account_switch_suspend_and_logout_close_old_discovery_and_rotate_anonymous_auth() {
    use crate::core::tests::logged_in_test_core_with_updates;
    let owner = Keys::generate();
    let device = Keys::generate();
    let next_owner = Keys::generate();
    let next_device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("discovery-lifecycle", &owner, &device);
    core.preferences.nostr_relay_urls.clear();
    let relay = core
        .runtime
        .block_on(test_support::plain_relay(vec![metadata(&peer, "peer")]));
    let (send, incoming) = flume::unbounded();
    core.core_sender = send;
    let mut signers = HashSet::new();
    for stage in 0..3 {
        let filters = vec![Filter::new().author(peer.public_key()).kind(Kind::Metadata)];
        let subscriptions = core.private_discovery_subscriptions(filters).unwrap();
        core.runtime
            .block_on(
                super::super::private_reads::apply_private_discovery_subscriptions(
                    subscriptions,
                    std::slice::from_ref(&relay.url),
                ),
            )
            .unwrap();
        core.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let CoreMsg::Internal(message) = incoming.recv_async().await.unwrap() {
                        if let InternalEvent::RelayEvent(event) = *message {
                            assert_eq!(event.pubkey, peer.public_key());
                            break;
                        }
                    }
                }
            })
            .await
            .unwrap();
        });
        let rows = relay.wire.lock().unwrap().clone();
        let (connection, auth) = rows.iter().rev().find(|(_, row)| row[0] == "AUTH").unwrap();
        let auth: Event = serde_json::from_value(auth[1].clone()).unwrap();
        assert!(
            signers.insert(auth.pubkey),
            "replacement pool reused anonymous identity"
        );
        assert!([&owner, &device, &next_owner, &next_device, &peer]
            .iter()
            .all(|key| key.public_key() != auth.pubkey));
        assert!(rows
            .iter()
            .filter(|(id, _)| id == connection)
            .all(|(_, row)| row[0] != "EVENT" && !row.to_string().contains("#p")));
        match stage {
            0 => core
                .start_session(
                    next_owner.public_key(),
                    Some(next_owner.clone()),
                    next_device.clone(),
                    false,
                    false,
                )
                .unwrap(),
            1 => {
                assert!(core.prepare_for_suspend());
            }
            _ => core.logout(),
        }
        core.runtime
            .block_on(until(|| relay.active.load(Ordering::SeqCst) == 0));
        assert!(core.protocol_subscription_runtime.discovery.is_none());
        if stage == 1 {
            core.handle_app_foregrounded();
        }
        while incoming.try_recv().is_ok() {}
    }
    assert_eq!(relay.connections.load(Ordering::SeqCst), 3);
    core.shutdown();
}
