use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};

struct AdvertRelay {
    url: RelayUrl,
    events: flume::Receiver<Event>,
    wire: flume::Receiver<serde_json::Value>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl AdvertRelay {
    async fn stop(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}

async fn advert_relay(drop_first_event_before_ack: bool) -> AdvertRelay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let (sent, events) = flume::unbounded();
    let (trace, wire) = flume::unbounded();
    let (stop, mut stopped) = oneshot::channel();
    let drop_before_ack = Arc::new(AtomicBool::new(drop_first_event_before_ack));
    let task = tokio::spawn(async move {
        let mut sockets = JoinSet::new();
        let mut next_connection = 0;
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                result = sockets.join_next(), if !sockets.is_empty() => {
                    result.unwrap().unwrap();
                }
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap();
                    let sent = sent.clone();
                    let trace = trace.clone();
                    let drop_before_ack = drop_before_ack.clone();
                    let connection = next_connection;
                    next_connection += 1;
                    sockets.spawn(async move {
                        let mut socket = accept_async(stream).await.unwrap();
                        while let Some(message) = socket.next().await {
                            let text = match message {
                                Ok(Message::Text(text)) => text,
                                Ok(Message::Ping(payload)) => {
                                    trace.send(json!({"connection": connection, "action": "ping"})).unwrap();
                                    socket.send(Message::Pong(payload)).await.unwrap();
                                    continue;
                                }
                                Ok(Message::Pong(_)) => continue,
                                Ok(Message::Close(_)) => {
                                    let _ = socket.flush().await;
                                    break;
                                }
                                Ok(_) => continue,
                                Err(_) => break,
                            };
                            let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                            if frame[0] != "EVENT" {
                                continue;
                            }
                            let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                            event.verify().unwrap();
                            trace.send(json!({
                                "connection": connection, "action": "event", "id": event.id.to_hex(),
                                "event_json": serde_json::to_string(&event).unwrap(),
                            })).unwrap();
                            sent.send(event.clone()).unwrap();
                            if drop_before_ack.swap(false, Ordering::SeqCst) {
                                trace.send(json!({"connection": connection, "action": "drop-before-ack", "id": event.id.to_hex()})).unwrap();
                                // The relay received the event, but the publisher cannot know it did.
                                break;
                            }
                            socket.send(Message::Text(json!(["OK", event.id.to_hex(), true, ""]).to_string())).await.unwrap();
                            trace.send(json!({"connection": connection, "action": "ack", "id": event.id.to_hex()})).unwrap();
                        }
                    });
                }
            }
        }
        sockets.abort_all();
        while let Some(result) = sockets.join_next().await {
            if let Err(error) = result {
                assert!(error.is_cancelled(), "relay task failed: {error}");
            }
        }
    });
    AdvertRelay {
        url,
        events,
        wire,
        stop,
        task,
    }
}

fn signed_advert(keys: &Keys, now: u64, ttl: u64, endpoint: &str) -> Event {
    let event = EventBuilder::new(
        Kind::Custom(37195),
        json!({
            "identifier": "fips-overlay-v1",
            "version": 1,
            "endpoints": [{"transport": "udp", "addr": endpoint}],
        })
        .to_string(),
    )
    .custom_created_at(Timestamp::from(now))
    .tags([
        nostr::Tag::identifier("iris-chat"),
        nostr::Tag::parse(["protocol", "iris-chat"]).unwrap(),
        nostr::Tag::parse(["version", "1"]).unwrap(),
        nostr::Tag::expiration(Timestamp::from(now + ttl)),
    ])
    .sign_with_keys(keys)
    .unwrap();
    // Use the actual signed-event representation at the publication boundary.
    let event: Event = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
    event.verify().unwrap();
    event
}

async fn publish_on_wire(
    publication: &mut OwnAdvertPublication,
    event: &Event,
    now: Instant,
    client: &Client,
    relay: &RelayUrl,
    logical_calls: &mut Vec<String>,
) -> bool {
    publication
        .publish_if_due(event, now, event.created_at.as_secs(), || {
            logical_calls.push(event.id.to_hex());
            async {
                crate::core::publish_helpers::publish_event_once(
                    client,
                    std::slice::from_ref(relay),
                    event,
                )
                .await
                .unwrap();
            }
        })
        .await
}

#[tokio::test]
async fn own_adverts_publish_only_semantic_changes_and_periodic_refresh_on_wire() {
    let relay = advert_relay(false).await;
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    let start = Instant::now();
    let epoch = Timestamp::now().as_secs();
    let mut publication = OwnAdvertPublication::default();
    let mut logical_calls = Vec::new();
    let mut expected_calls = Vec::new();
    let mut legacy_fingerprints = HashSet::new();
    for tick in 0..30 {
        let elapsed = tick * 2;
        let event = signed_advert(&keys, epoch + elapsed, 120, "203.0.113.1:9000");
        if tick == 0 {
            expected_calls.push(event.id.to_hex());
        }
        legacy_fingerprints.insert(serde_json::to_string(&(&event.tags, &event.content)).unwrap());
        assert_eq!(
            publish_on_wire(
                &mut publication,
                &event,
                start + Duration::from_secs(elapsed),
                &client,
                &relay.url,
                &mut logical_calls,
            )
            .await,
            tick == 0,
            "unchanged advert at {elapsed}s",
        );
    }
    assert_eq!(
        legacy_fingerprints.len(),
        30,
        "the old fingerprint would publish every newly signed expiration"
    );
    let first = relay.events.try_iter().collect::<Vec<_>>();
    assert_eq!(first.len(), 1, "30 unchanged polls must send one EVENT");
    assert_eq!(first[0].created_at.as_secs(), epoch);

    for (elapsed, endpoint, expected) in [
        (60, "203.0.113.1:9000", true),
        (62, "203.0.113.2:9000", true),
        (64, "203.0.113.2:9000", false),
        (120, "203.0.113.2:9000", false),
        (122, "203.0.113.2:9000", true),
    ] {
        let event = signed_advert(&keys, epoch + elapsed, 120, endpoint);
        if expected {
            expected_calls.push(event.id.to_hex());
        }
        assert_eq!(
            publish_on_wire(
                &mut publication,
                &event,
                start + Duration::from_secs(elapsed),
                &client,
                &relay.url,
                &mut logical_calls,
            )
            .await,
            expected,
            "refresh or endpoint change at {elapsed}s",
        );
    }
    let refreshed = relay.events.try_iter().collect::<Vec<_>>();
    eprintln!(
        "ADVERT_WIRE_EVIDENCE={}",
        json!({"case": "normal", "logical_calls": logical_calls, "wire": relay.wire.try_iter().collect::<Vec<_>>()})
    );
    assert_eq!(logical_calls, expected_calls);
    assert_eq!(
        first
            .iter()
            .chain(&refreshed)
            .map(|event| event.id.to_hex())
            .collect::<Vec<_>>(),
        expected_calls
    );
    assert_eq!(
        refreshed
            .iter()
            .map(|event| event.created_at.as_secs() - epoch)
            .collect::<Vec<_>>(),
        [60, 62, 122]
    );
    assert!(refreshed[1].content.contains("203.0.113.2:9000"));
    client.shutdown().await;
    relay.stop().await;
}

#[tokio::test]
async fn advert_relay_survives_ping_and_pong_before_acknowledging_event() {
    let relay = advert_relay(false).await;
    let event = signed_advert(
        &Keys::generate(),
        Timestamp::now().as_secs(),
        120,
        "203.0.113.1:9000",
    );
    let (mut socket, _) = connect_async(relay.url.as_str()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        socket
            .send(Message::Pong(b"unsolicited".to_vec()))
            .await
            .unwrap();
        socket
            .send(Message::Ping(b"fixture-ping".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Pong(b"fixture-ping".to_vec())
        );
        socket
            .send(Message::Text(json!(["EVENT", event]).to_string()))
            .await
            .unwrap();
        let Message::Text(reply) = socket.next().await.unwrap().unwrap() else {
            panic!("expected event acknowledgement after Ping/Pong");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&reply).unwrap(),
            json!(["OK", event.id.to_hex(), true, ""])
        );
    })
    .await
    .unwrap();
    let events = relay.events.try_iter().collect::<Vec<_>>();
    let wire = relay.wire.try_iter().collect::<Vec<_>>();
    eprintln!(
        "ADVERT_WIRE_EVIDENCE={}",
        json!({"case": "ping-survival", "wire": wire})
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        serde_json::to_string(&events[0]).unwrap(),
        serde_json::to_string(&event).unwrap()
    );
    assert_eq!(wire.iter().filter(|row| row["action"] == "ping").count(), 1);
    assert_eq!(wire.iter().filter(|row| row["action"] == "ack").count(), 1);
    assert!(wire.iter().all(|row| row["connection"] == 0));
    socket.close(None).await.unwrap();
    relay.stop().await;
}

#[tokio::test]
async fn advert_transport_retry_preserves_one_logical_publish_and_signed_event() {
    let relay = advert_relay(true).await;
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    let event = signed_advert(&keys, Timestamp::now().as_secs(), 120, "203.0.113.1:9000");
    let start = Instant::now();
    let mut publication = OwnAdvertPublication::default();
    let mut logical_calls = Vec::new();
    assert!(tokio::time::timeout(
        crate::core::publish_helpers::RELAY_PUBLISH_ATTEMPT_TIMEOUT,
        publish_on_wire(
            &mut publication,
            &event,
            start,
            &client,
            &relay.url,
            &mut logical_calls
        ),
    )
    .await
    .unwrap());
    assert!(
        !publish_on_wire(
            &mut publication,
            &event,
            start + Duration::from_secs(2),
            &client,
            &relay.url,
            &mut logical_calls
        )
        .await
    );
    client.shutdown().await;
    let events = relay.events.try_iter().collect::<Vec<_>>();
    let wire = relay.wire.try_iter().collect::<Vec<_>>();
    eprintln!(
        "ADVERT_WIRE_EVIDENCE={}",
        json!({"case": "receive-before-ack-fallback", "logical_calls": logical_calls, "wire": wire})
    );
    assert_eq!(logical_calls, [event.id.to_hex()]);
    assert_eq!(
        events.len(),
        2,
        "one received event loses its ACK; the raw fallback retries it"
    );
    for received in &events {
        assert_eq!(
            serde_json::to_string(received).unwrap(),
            serde_json::to_string(&event).unwrap()
        );
    }
    let deliveries = wire
        .iter()
        .filter(|row| row["action"] == "event")
        .collect::<Vec<_>>();
    assert_eq!(deliveries.len(), 2);
    assert_ne!(deliveries[0]["connection"], deliveries[1]["connection"]);
    assert_eq!(deliveries[0]["event_json"], deliveries[1]["event_json"]);
    let dropped = wire
        .iter()
        .filter(|row| row["action"] == "drop-before-ack")
        .collect::<Vec<_>>();
    let acknowledged = wire
        .iter()
        .filter(|row| row["action"] == "ack")
        .collect::<Vec<_>>();
    assert_eq!(dropped.len(), 1);
    assert_eq!(acknowledged.len(), 1);
    assert_eq!(dropped[0]["connection"], deliveries[0]["connection"]);
    assert_eq!(acknowledged[0]["connection"], deliveries[1]["connection"]);
    assert_eq!(acknowledged[0]["id"], event.id.to_hex());
    relay.stop().await;
}

#[tokio::test]
async fn own_adverts_do_not_suppress_identity_protocol_or_other_semantic_changes() {
    let keys = Keys::generate();
    let epoch = Timestamp::now().as_secs();
    let original = signed_advert(&keys, epoch, 120, "203.0.113.1:9000");
    let changed = |kind, tags, signing_keys: &Keys| {
        EventBuilder::new(kind, original.content.clone())
            .custom_created_at(Timestamp::from(epoch + 2))
            .tags(tags)
            .sign_with_keys(signing_keys)
            .unwrap()
    };
    let tags = original.tags.iter().cloned().collect::<Vec<_>>();
    let mut cases = vec![
        (
            "author",
            changed(original.kind, tags.clone(), &Keys::generate()),
        ),
        ("kind", changed(Kind::Custom(37196), tags.clone(), &keys)),
    ];
    for (name, value) in [
        ("d", "new-app"),
        ("protocol", "new-protocol"),
        ("version", "2"),
    ] {
        let replaced = tags
            .iter()
            .filter(|tag| tag.as_slice()[0] != name)
            .cloned()
            .chain([nostr::Tag::parse([name, value]).unwrap()])
            .collect::<Vec<_>>();
        cases.push((name, changed(original.kind, replaced, &keys)));
    }
    let mut unknown = tags.clone();
    unknown.push(nostr::Tag::parse(["future-field", "preserve-me"]).unwrap());
    cases.push(("unknown tag", changed(original.kind, unknown, &keys)));
    let removed = tags
        .into_iter()
        .filter(|tag| tag.as_slice()[0] != "protocol")
        .collect::<Vec<_>>();
    cases.push(("removed tag", changed(original.kind, removed, &keys)));
    for (label, event) in cases {
        event.verify().unwrap();
        let start = Instant::now();
        let mut publication = OwnAdvertPublication::default();
        assert!(
            publication
                .publish_if_due(&original, start, epoch, || async {})
                .await
        );
        let mut calls = 0;
        assert!(
            publication
                .publish_if_due(&event, start + Duration::from_secs(2), epoch + 2, || {
                    calls += 1;
                    async {}
                })
                .await,
            "must publish changed {label} immediately"
        );
        assert_eq!(calls, 1, "changed {label} must reach the publication sink");
    }
}

#[tokio::test]
async fn own_adverts_refresh_before_the_previously_published_expiration() {
    let keys = Keys::generate();
    let epoch = Timestamp::now().as_secs();
    let start = Instant::now();
    let mut publication = OwnAdvertPublication::default();
    for (elapsed, expected) in [(0, true), (2, false), (6, false), (8, true), (10, false)] {
        let event = signed_advert(&keys, epoch + elapsed, 10, "203.0.113.1:9000");
        assert_eq!(
            publication
                .publish_if_due(
                    &event,
                    start + Duration::from_secs(elapsed),
                    epoch + elapsed,
                    || async {},
                )
                .await,
            expected,
            "refresh before the last published expiration, not the new poll's expiration"
        );
    }
}
