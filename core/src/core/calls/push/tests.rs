use super::*;

#[test]
fn wake_contains_only_recipient_bound_signed_ratchet_ciphertext() {
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let ratchet = Keys::generate();
    let encrypted = EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        "already-ratcheted-ciphertext",
    )
    .tag(Tag::parse(["v", "2"]).unwrap())
    .tag(Tag::public_key(recipient.public_key()))
    .sign_with_keys(&ratchet)
    .unwrap();
    let event = wake_event(&encrypted, None).unwrap();
    assert_eq!(
        wake_messages(&event, recipient.public_key())
            .unwrap()
            .events,
        vec![encrypted.clone()]
    );
    assert_ne!(event.pubkey, sender.public_key());
    assert_ne!(event.pubkey, ratchet.public_key());
    assert_ne!(event.pubkey, wake_event(&encrypted, None).unwrap().pubkey);
    assert!(!event.as_json().contains(&sender.public_key().to_hex()));
    assert!(wake_messages(&event, Keys::generate().public_key()).is_none());
    let mut tampered = event.clone();
    tampered.content.push('a');
    assert!(wake_messages(&tampered, recipient.public_key()).is_none());
    let stale = EventBuilder::new(event.kind, &event.content)
        .tags(event.tags.clone())
        .custom_created_at(Timestamp::from(unix_now().get() - 41))
        .sign_with_keys(&sender)
        .unwrap();
    assert!(wake_messages(&stale, recipient.public_key()).is_none());
    for content in [
        "legacy-static-ciphertext".to_string(),
        serde_json::json!({"type":"call-wake","v":2,"events":[],"call_id":"private"}).to_string(),
    ] {
        let old = EventBuilder::new(event.kind, content)
            .tags(event.tags.clone())
            .sign_with_keys(&sender)
            .unwrap();
        assert!(wake_messages(&old, recipient.public_key()).is_none());
    }
}

#[test]
fn call_wake_v3_oversize_bootstrap_uses_only_exact_id_and_rejects_mixed_forms() {
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let response = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "x".repeat(4096))
        .tag(Tag::parse(["v", "2"]).unwrap())
        .tag(Tag::public_key(Keys::generate().public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let encrypted = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "ratchet-ciphertext")
        .tag(Tag::parse(["v", "2"]).unwrap())
        .tag(Tag::public_key(recipient.public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let event = wake_event(&encrypted, Some(&response)).unwrap();
    let decoded = wake_messages(&event, recipient.public_key()).unwrap();
    assert_eq!(decoded.bootstrap_event_id, Some(response.id.to_hex()));
    assert_eq!(decoded.events, vec![encrypted.clone()]);
    assert!(event.as_json().len() <= MAX_WAKE_BYTES);
    for content in [
        serde_json::json!({"type":"call-wake","v":3,"events":[response,encrypted],"bootstrapEventId":response.id.to_hex()}),
        serde_json::json!({"type":"call-wake","v":3,"events":[encrypted],"bootstrapEventId":"not-an-event-id"}),
    ] {
        let invalid = EventBuilder::new(event.kind, content.to_string())
            .tags(event.tags.clone())
            .sign_with_keys(&sender)
            .unwrap();
        assert!(wake_messages(&invalid, recipient.public_key()).is_none());
    }
}

#[test]
fn call_push_subscription_uses_dedicated_device_filter_and_voip_topic() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let request = build_call_push_subscription_request(
        owner.secret_key().to_secret_hex(),
        device.public_key().to_hex(),
        None,
        "ios".into(),
        "test-token".into(),
        Some("to.iris.chat".into()),
        false,
        None,
    )
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&request.body_json.unwrap()).unwrap();
    assert_eq!(body["apns_topic"], "to.iris.chat.voip");
    assert_eq!(body["apns_environment"], "development");
    assert_eq!(body["filter"]["kinds"][0], CALL_WAKE_KIND);
    assert_eq!(body["filter"]["#p"][0], device.public_key().to_hex());
    assert!(
        body["filter"].get("authors").is_none(),
        "caller identities never leave the device"
    );
    assert_eq!(body["fcm_tokens"], serde_json::json!([]));
}

#[test]
fn call_wake_dispatch_is_anonymous_on_actual_relay_and_http_connections() {
    use crate::core::publish_helpers::{authenticated_relay, identity_receiver};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let mut fixture = super::super::tests::Fixture::new();
    let runtime = fixture.core.runtime.handle().clone();
    let device = fixture.core.logged_in.as_ref().unwrap().device_keys.clone();
    let owner = fixture.core.logged_in.as_ref().unwrap().owner_pubkey;
    let (relay, wire, receiver, http, requests, listener) = runtime.block_on(async {
        let (relay, wire) = authenticated_relay(device.public_key(), false).await;
        let receiver = identity_receiver(&device, &relay, &wire).await;
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", http.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let listener = tokio::spawn(async move {
            while let Ok((mut socket, peer)) = http.accept().await {
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")?
                                .trim()
                                .parse::<usize>()
                                .ok()
                        })
                        .unwrap();
                    if bytes.len() < end + 4 + length {
                        continue;
                    }
                    let event: Event =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    recorded.lock().unwrap().push((peer, headers, event));
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                        .await
                        .unwrap();
                    break;
                }
            }
        });
        (relay, wire, receiver, url, requests, listener)
    });
    let logged = fixture.core.logged_in.as_mut().unwrap();
    logged.client = receiver.clone();
    logged.relay_urls = vec![relay];
    fixture.core.preferences.mobile_push_server_url = http;
    let wakes = (0..2)
        .map(|_| {
            let encrypted =
                EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "ratchet ciphertext")
                    .tags([
                        Tag::parse(["v", "2"]).unwrap(),
                        Tag::public_key(Keys::generate().public_key()),
                    ])
                    .sign_with_keys(&Keys::generate())
                    .unwrap();
            wake_event(&encrypted, None).unwrap()
        })
        .collect::<Vec<_>>();
    // The same wake retried, followed by another recipient, needs three contexts.
    for event in [&wakes[0], &wakes[0], &wakes[1]] {
        fixture.core.publish_call_wake(event.clone());
    }
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let rows = wire.lock().unwrap().clone();
                if requests.lock().unwrap().len() == 3
                    && wakes.iter().all(|wake| {
                        rows.iter().any(|(connection, message)| {
                            message[0] == "EVENT"
                                && message[1]["id"] == wake.id.to_hex()
                                && rows
                                    .iter()
                                    .any(|(number, auth)| number == connection && auth[0] == "AUTH")
                        })
                    })
                {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        receiver.disconnect().await;
    });
    listener.abort();
    let rows = wire.lock().unwrap().clone();
    let deliveries = rows
        .iter()
        .filter(|(_, message)| {
            message[0] == "EVENT"
                && wakes
                    .iter()
                    .any(|wake| message[1]["id"] == wake.id.to_hex())
        })
        .map(|(connection, _)| *connection)
        .collect::<HashSet<_>>();
    assert_eq!(
        deliveries.len(),
        3,
        "each wake copy and retry needs a fresh connection"
    );
    let mut auth_keys = HashSet::new();
    for connection in deliveries {
        let messages = rows
            .iter()
            .filter(|(number, _)| *number == connection)
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        assert!(
            messages.iter().all(|value| value[0] != "REQ"),
            "call wake must never share the account receiving connection"
        );
        let auth = messages.iter().find(|value| value[0] == "AUTH").unwrap();
        let auth_key = auth[1]["pubkey"].as_str().unwrap();
        assert_ne!(auth_key, device.public_key().to_hex());
        assert_ne!(auth_key, owner.to_hex());
        assert!(wakes
            .iter()
            .all(|wake| !wake.tags.public_keys().any(|key| key.to_hex() == auth_key)));
        assert!(
            auth_keys.insert(auth_key.to_string()),
            "call wake authentication must not link recipients or retries"
        );
        assert!(!serde_json::to_string(&messages)
            .unwrap()
            .contains(&device.public_key().to_hex()));
        assert!(!serde_json::to_string(&messages)
            .unwrap()
            .contains(&owner.to_hex()));
    }
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|(peer, _, _)| peer)
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    for (_, headers, event) in requests.iter() {
        assert!(!headers.contains("authorization:") && !headers.contains("cookie:"));
        event.verify().unwrap();
        assert!(wakes.contains(event));
        assert!(!event.as_json().contains(&device.public_key().to_hex()));
    }
}
