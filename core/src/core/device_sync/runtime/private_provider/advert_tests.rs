use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

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
) -> bool {
    publication
        .publish_if_due(event, now, event.created_at.as_secs(), || async {
            crate::core::publish_helpers::publish_event_once(
                client,
                std::slice::from_ref(relay),
                event,
            )
            .await
            .unwrap();
        })
        .await
}

#[tokio::test]
async fn own_adverts_publish_only_semantic_changes_and_periodic_refresh_on_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let (sent, received) = flume::unbounded();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sent = sent.clone();
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if frame[0] == "EVENT" {
                        let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                        event.verify().unwrap();
                        sent.send(event.clone()).unwrap();
                        socket
                            .send(Message::Text(
                                json!(["OK", event.id.to_hex(), true, ""]).to_string(),
                            ))
                            .await
                            .unwrap();
                    }
                }
            });
        }
    });
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    let start = Instant::now();
    let epoch = Timestamp::now().as_secs();
    let mut publication = OwnAdvertPublication::default();
    let mut legacy_fingerprints = HashSet::new();
    for tick in 0..30 {
        let elapsed = tick * 2;
        let event = signed_advert(&keys, epoch + elapsed, 120, "203.0.113.1:9000");
        legacy_fingerprints.insert(serde_json::to_string(&(&event.tags, &event.content)).unwrap());
        assert_eq!(
            publish_on_wire(
                &mut publication,
                &event,
                start + Duration::from_secs(elapsed),
                &client,
                &relay,
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
    let first = received.try_iter().collect::<Vec<_>>();
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
        assert_eq!(
            publish_on_wire(
                &mut publication,
                &event,
                start + Duration::from_secs(elapsed),
                &client,
                &relay,
            )
            .await,
            expected,
            "refresh or endpoint change at {elapsed}s",
        );
    }
    let refreshed = received.try_iter().collect::<Vec<_>>();
    assert_eq!(
        refreshed
            .iter()
            .map(|event| event.created_at.as_secs() - epoch)
            .collect::<Vec<_>>(),
        [60, 62, 122]
    );
    assert!(refreshed[1].content.contains("203.0.113.2:9000"));
    client.shutdown().await;
    server.abort();
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
