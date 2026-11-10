// Reuse the generated account/history fixture, never an exported user account.

use super::*;
use nostr::{EventId, RelayMessage};

async fn received_eose(
    notifications: &mut tokio::sync::broadcast::Receiver<RelayPoolNotification>,
    expected_relay: &nostr::RelayUrl,
) -> Vec<EventId> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut events = Vec::new();
        loop {
            let RelayPoolNotification::Message { relay_url, message } =
                notifications.recv().await.unwrap()
            else {
                continue;
            };
            if &relay_url != expected_relay {
                continue;
            }
            match message {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } if subscription_id.as_str() == "ndr-protocol" => events.push(event.id),
                RelayMessage::EndOfStoredEvents(id) if id.as_str() == "ndr-protocol" => {
                    return events
                }
                _ => {}
            }
        }
    })
    .await
    .expect("protocol subscription EOSE")
}

#[tokio::test]
async fn another_relays_eose_does_not_complete_the_observed_subscription() {
    use futures_util::FutureExt;

    let first = nostr::RelayUrl::parse("ws://127.0.0.1:18001").unwrap();
    let second = nostr::RelayUrl::parse("ws://127.0.0.1:18002").unwrap();
    let (sender, mut receiver) = tokio::sync::broadcast::channel(2);
    let eose = |relay_url| RelayPoolNotification::Message {
        relay_url,
        message: RelayMessage::eose(SubscriptionId::new("ndr-protocol")),
    };
    sender.send(eose(second)).unwrap();
    let completion = received_eose(&mut receiver, &first);
    tokio::pin!(completion);
    assert!(
        completion.as_mut().now_or_never().is_none(),
        "another relay cannot acknowledge the observed subscription"
    );
    sender.send(eose(first.clone())).unwrap();
    assert!(completion.await.is_empty());
}

fn tick(core: &mut AppCore, incoming: &flume::Receiver<CoreMsg>) {
    core.handle_protocol_subscription_liveness_check(core.timers.protocol_liveness);
    reconcile(core, incoming);
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < deadline, "relay readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn protocol_requests(relay: &crate::local_relay::TestRelay) -> usize {
    relay
        .wire()
        .iter()
        .filter(|(_, frame)| frame[0] == "REQ" && frame[1] == "ndr-protocol")
        .count()
}

fn reconcile(core: &mut AppCore, incoming: &flume::Receiver<CoreMsg>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while core.protocol_subscription_runtime.refresh_in_flight {
        assert!(
            Instant::now() < deadline,
            "subscription reconciliation stalled"
        );
        if let Ok(CoreMsg::Internal(event)) = incoming.recv_timeout(Duration::from_millis(20)) {
            core.handle_internal(*event);
        }
    }
}

#[test]
fn populated_history_liveness_does_not_replay_healthy_subscriptions() {
    let mut fixture = synthetic_history_fixture_with_devices(1, 54, 2, 3).0;
    let core = &mut fixture.core;
    let sender = Keys::generate();
    let event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(),
        &sender,
        "Synthetic retained message",
        200,
    );
    let owner = core.logged_in.as_ref().unwrap().owner_pubkey;
    let device = core.logged_in.as_ref().unwrap().device_keys.public_key();
    assert!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .has_pending_retry_work(),
        "fixture must retain queued delivery work while idle"
    );
    let relay = crate::local_relay::TestRelay::start();
    assert!(relay.wire().is_empty());
    let urls = relay_urls_from_strings(&[relay.url().to_owned()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_owned()];
    core.logged_in.as_mut().unwrap().relay_urls = urls.clone();
    let client = core.logged_in.as_ref().unwrap().client.clone();
    core.runtime.block_on(async {
        ensure_session_relays_configured(&client, &urls).await;
        connect_client_with_timeout(&client, Duration::from_secs(2)).await;
        let publisher = Client::new(Keys::generate());
        ensure_session_relays_configured(&publisher, &urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher.send_event(&event).await.unwrap();
        publisher.disconnect().await;
    });
    let mut notifications = client.notifications();
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    core.protocol_subscription_runtime.desired_plan =
        Some(protocol_plan_for_test(vec![event.pubkey], Vec::new()));
    core.refresh_relay_connection_status();
    core.reconcile_protocol_subscriptions("fixture_start", false);
    reconcile(core, &rx);
    let received = core
        .runtime
        .block_on(received_eose(&mut notifications, &urls[0]));
    assert!(
        received.contains(&event.id),
        "retained encrypted message must arrive"
    );
    assert_eq!(protocol_requests(&relay), 1);

    for _ in 0..5 {
        tick(core, &rx);
    }
    // A finite read on the same socket is a wire barrier: earlier
    // REQ/CLOSE frames have reached the relay before its EOSE response.
    core.runtime.block_on(async {
        client
            .fetch_events(Filter::new().id(event.id), Duration::from_secs(2))
            .await
            .unwrap();
    });
    let wire = relay.wire();
    for (_, frame) in &wire {
        let serialized = frame.to_string();
        assert!(!serialized.contains(&owner.to_hex()));
        assert!(!serialized.contains(&device.to_hex()));
        assert!(!serialized.contains(&sender.public_key().to_hex()));
    }
    assert_eq!(
        protocol_requests(&relay),
        1,
        "idle liveness must not re-request retained history on healthy subscriptions"
    );
    assert_eq!(
        wire.iter()
            .filter(|(_, f)| f[0] == "CLOSE" && f[1] == "ndr-protocol")
            .count(),
        0
    );
    assert!(core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_pending_retry_work());
    core.shutdown();
}

#[test]
fn liveness_repairs_closed_and_missing_subscriptions_without_repeated_replay() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let author = Keys::generate();
    let mut core = logged_in_test_core("subscription-recovery", &owner, &device);
    let relay = crate::local_relay::TestRelay::start();
    assert!(relay.wire().is_empty());
    let urls = relay_urls_from_strings(&[relay.url().to_owned()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_owned()];
    core.logged_in.as_mut().unwrap().relay_urls = urls.clone();
    let client = core.logged_in.as_ref().unwrap().client.clone();
    core.runtime.block_on(async {
        client
            .pool()
            .add_relay(
                urls[0].clone(),
                nostr_sdk::RelayOptions::default()
                    .retry_interval(Duration::from_millis(10))
                    .adjust_retry_interval(false),
            )
            .await
            .unwrap();
        connect_client_with_timeout(&client, Duration::from_secs(2)).await;
    });
    let sdk_relay = core.runtime.block_on(client.relay(&urls[0])).unwrap();
    let id = SubscriptionId::new("ndr-protocol");
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    core.refresh_relay_connection_status();
    core.start_relay_status_watchers();
    let CoreMsg::Internal(initial) = rx.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!("initial relay status");
    };
    core.handle_internal(*initial);
    let mut notifications = client.notifications();
    core.protocol_subscription_runtime.desired_plan = Some(protocol_plan_for_test(
        vec![author.public_key()],
        Vec::new(),
    ));
    core.reconcile_protocol_subscriptions("fixture_start", false);
    reconcile(&mut core, &rx);
    core.runtime
        .block_on(received_eose(&mut notifications, &urls[0]));
    let mut requests = 1;
    for reason in [
        "error: try later",
        "auth-required: authenticate",
        "rate-limited: try later",
    ] {
        assert_eq!(relay.close_subscription("ndr-protocol", reason), 1);
        wait_until(|| core.runtime.block_on(sdk_relay.subscription(&id)).is_none());
        tick(&mut core, &rx);
        core.runtime
            .block_on(received_eose(&mut notifications, &urls[0]));
        requests += 1;
        assert_eq!(protocol_requests(&relay), requests);
        for _ in 0..3 {
            tick(&mut core, &rx);
        }
        assert_eq!(protocol_requests(&relay), requests);
    }
    // A missing local SDK entry must also be repaired with an unchanged plan.
    core.runtime.block_on(sdk_relay.unsubscribe(&id)).unwrap();
    tick(&mut core, &rx);
    core.runtime
        .block_on(received_eose(&mut notifications, &urls[0]));
    requests += 1;
    assert_eq!(protocol_requests(&relay), requests);

    // Adding a server repairs only that server, even with an unchanged plan.
    let second = crate::local_relay::TestRelay::start();
    assert!(second.wire().is_empty());
    let both_urls = relay_urls_from_strings(&[relay.url().into(), second.url().into()]);
    core.preferences.nostr_relay_urls = vec![relay.url().into(), second.url().into()];
    core.logged_in.as_mut().unwrap().relay_urls = both_urls.clone();
    core.runtime.block_on(async {
        ensure_session_relays_configured(&client, &both_urls).await;
        client.connect().await;
        client
            .relay(&both_urls[1])
            .await
            .unwrap()
            .wait_for_connection(Duration::from_secs(2))
            .await;
    });
    core.refresh_relay_connection_status();
    tick(&mut core, &rx);
    core.runtime
        .block_on(received_eose(&mut notifications, &both_urls[1]));
    assert_eq!(protocol_requests(&second), 1);
    assert_eq!(protocol_requests(&relay), requests);

    // Exercise a rapid reconnect while the other server stays connected.
    // Drive the real status handler, including the SDK's same-second case.
    assert_eq!(relay.disconnect_clients(), 1);
    wait_until(|| {
        if let Ok(CoreMsg::Internal(event)) = rx.recv_timeout(Duration::from_millis(10)) {
            core.handle_internal(*event);
        }
        protocol_requests(&relay) > requests
            && !core.protocol_subscription_runtime.refresh_in_flight
            && core.runtime.block_on(sdk_relay.subscription(&id)).is_some()
    });
    core.runtime
        .block_on(received_eose(&mut notifications, &urls[0]));
    // Some reconnects also trigger the SDK's own catch-up; neither path may
    // re-request history on subsequent healthy liveness checks.
    requests = protocol_requests(&relay);
    for _ in 0..3 {
        tick(&mut core, &rx);
    }
    assert_eq!(protocol_requests(&relay), requests);
    assert_eq!(protocol_requests(&second), 1);

    let next_author = Keys::generate();
    core.protocol_subscription_runtime.desired_plan = Some(protocol_plan_for_test(
        vec![next_author.public_key()],
        Vec::new(),
    ));
    core.reconcile_protocol_subscriptions("fixture_changed", false);
    reconcile(&mut core, &rx);
    core.runtime
        .block_on(received_eose(&mut notifications, &urls[0]));
    // Pool notifications share a subscription ID across relays. Require each
    // relay's newest wire request to contain the changed plan before counting.
    let expected_request = serde_json::json!([
        "REQ",
        "ndr-protocol",
        { "authors": [next_author.public_key().to_hex()], "kinds": [MESSAGE_EVENT_KIND] }
    ]);
    wait_until(|| {
        [&relay, &second].into_iter().all(|server| {
            server
                .wire()
                .into_iter()
                .rev()
                .find(|(_, frame)| frame[0] == "REQ" && frame[1] == "ndr-protocol")
                .is_some_and(|(_, frame)| frame == expected_request)
        })
    });
    requests += 1;
    assert_eq!(protocol_requests(&relay), requests);
    assert_eq!(protocol_requests(&second), 2);

    // New live traffic still crosses the unchanged subscription after idle.
    for _ in 0..3 {
        tick(&mut core, &rx);
    }
    let live = EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        "synthetic transport payload",
    )
    .sign_with_keys(&next_author)
    .unwrap();
    core.runtime.block_on(async {
        let publisher = Client::new(Keys::generate());
        ensure_session_relays_configured(&publisher, &urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher.send_event(&live).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let RelayPoolNotification::Event { event, .. } =
                    notifications.recv().await.unwrap()
                {
                    if event.id == live.id {
                        break;
                    }
                }
            }
        })
        .await
        .expect("live event after quiet liveness");
        publisher.disconnect().await;
    });
    assert_eq!(protocol_requests(&relay), requests);

    // Removing the plan closes the durable subscription too.
    core.protocol_subscription_runtime.desired_plan = None;
    core.reconcile_protocol_subscriptions("fixture_removed", false);
    reconcile(&mut core, &rx);
    assert!(core.runtime.block_on(sdk_relay.subscription(&id)).is_none());
    let wire = relay.wire();
    let receiving_connections = wire
        .iter()
        .filter(|(_, frame)| frame[0] == "REQ" && frame[1] == "ndr-protocol")
        .map(|(connection, _)| *connection)
        .collect::<HashSet<_>>();
    for (_, frame) in wire
        .iter()
        .filter(|(connection, _)| receiving_connections.contains(connection))
    {
        let serialized = frame.to_string();
        assert!(!serialized.contains(&owner.public_key().to_hex()));
        assert!(!serialized.contains(&device.public_key().to_hex()));
    }
    core.shutdown();
}
