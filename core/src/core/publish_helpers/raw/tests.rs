use super::*;
use serde_json::{json, Value};
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

pub(in crate::core) type Wire = Arc<Mutex<Vec<(usize, Value)>>>;

pub(in crate::core) async fn authenticated_relay(
    owner: PublicKey,
    reject_anonymous: bool,
) -> (RelayUrl, Wire) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let wire: Wire = Arc::new(Mutex::new(Vec::new()));
    let recorded = wire.clone();
    tokio::spawn(async move {
        let mut connection = 0;
        while let Ok((stream, _)) = listener.accept().await {
            connection += 1;
            let wire = recorded.clone();
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(json!(["AUTH", "challenge"]).to_string()))
                    .await
                    .unwrap();
                let mut authenticated = false;
                while let Some(Ok(incoming)) = socket.next().await {
                    let Message::Text(text) = incoming else {
                        continue;
                    };
                    let value: Value = serde_json::from_str(&text).unwrap();
                    wire.lock().unwrap().push((connection, value.clone()));
                    match value[0].as_str() {
                        Some("AUTH") => {
                            let auth: Event = serde_json::from_value(value[1].clone()).unwrap();
                            auth.verify().unwrap();
                            assert_eq!(auth.kind, Kind::Authentication);
                            authenticated = !reject_anonymous || auth.pubkey == owner;
                            if socket
                                .send(Message::Text(
                                    json!(["OK", auth.id.to_hex(), authenticated, "auth policy"])
                                        .to_string(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some("EVENT") => {
                            let id = &value[1]["id"];
                            let reason = if authenticated {
                                ""
                            } else {
                                "auth-required: authenticate first"
                            };
                            if socket
                                .send(Message::Text(
                                    json!(["OK", id, authenticated, reason]).to_string(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some("REQ") => {
                            let response = json!(["EOSE", value[1]]).to_string();
                            if socket.send(Message::Text(response)).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    (relay, wire)
}

pub(in crate::core) async fn identity_receiver(
    owner: &Keys,
    relay: &RelayUrl,
    wire: &Wire,
) -> Client {
    let client = Client::new(owner.clone());
    client.add_relay(relay.clone()).await.unwrap();
    client.connect().await;
    client
        .subscribe(Filter::new().pubkey(owner.public_key()), None)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let rows = wire.lock().unwrap().clone();
            if rows.iter().any(|(_, event)| event[0] == "REQ")
                && rows.iter().any(|(_, event)| {
                    event[0] == "AUTH" && event[1]["pubkey"] == owner.public_key().to_hex()
                })
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    client
}

#[tokio::test]
async fn private_publishing_never_reuses_identity_auth_or_receiver_subscriptions() {
    let owner = Keys::generate();
    let recipient = Keys::generate().public_key();
    let (relay, wire) = authenticated_relay(owner.public_key(), false).await;
    let client = identity_receiver(&owner, &relay, &wire).await;
    let mut private_ids = Vec::new();
    for connected in [false, true] {
        for kind in [INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND, 21111] {
            let event = EventBuilder::new(Kind::from(kind as u16), "private ciphertext")
                .tags([
                    nostr::Tag::public_key(recipient),
                    nostr::Tag::parse(["v", "2"]).unwrap(),
                ])
                .sign_with_keys(&Keys::generate())
                .unwrap();
            let result = if connected {
                publish_event_to_any_connected_relay(
                    &client,
                    std::slice::from_ref(&relay),
                    &event,
                    "private",
                )
                .await
            } else {
                publish_event_to_any_relay(&client, std::slice::from_ref(&relay), &event, "private")
                    .await
            };
            assert!(result.is_ok(), "{result:?}");
            private_ids.push(event.id.to_hex());
        }
    }
    let rows = wire.lock().unwrap().clone();
    let mut auth_keys = HashSet::new();
    for id in private_ids {
        let connection = rows
            .iter()
            .find(|(_, event)| event[0] == "EVENT" && event[1]["id"] == id)
            .unwrap()
            .0;
        let messages = rows
            .iter()
            .filter(|(number, _)| *number == connection)
            .map(|(_, event)| event)
            .collect::<Vec<_>>();
        assert!(messages.iter().all(|event| event[0] != "REQ"));
        let auth = messages.iter().find(|event| event[0] == "AUTH").unwrap();
        let key = auth[1]["pubkey"].as_str().unwrap();
        assert_ne!(key, owner.public_key().to_hex());
        assert_ne!(key, recipient.to_hex());
        assert!(
            auth_keys.insert(key.to_string()),
            "authentication keys must not correlate separate private deliveries"
        );
        assert!(!serde_json::to_string(&messages)
            .unwrap()
            .contains(&owner.public_key().to_hex()));
    }
    client.disconnect().await;
}

#[tokio::test]
async fn private_publish_rejected_anonymous_auth_never_falls_back_to_account_client() {
    let owner = Keys::generate();
    let (relay, wire) = authenticated_relay(owner.public_key(), true).await;
    let client = identity_receiver(&owner, &relay, &wire).await;
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "private ciphertext")
        .tag(nostr::Tag::public_key(Keys::generate().public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let result = publish_event_to_any_connected_relay(&client, &[relay], &event, "private").await;
    assert!(result.is_err());
    let rows = wire.lock().unwrap().clone();
    let owner_connections = rows
        .iter()
        .filter(|(_, value)| {
            value[0] == "AUTH" && value[1]["pubkey"] == owner.public_key().to_hex()
        })
        .map(|(number, _)| *number)
        .collect::<HashSet<_>>();
    assert!(!owner_connections.is_empty());
    assert!(!rows
        .iter()
        .any(|(number, value)| owner_connections.contains(number) && value[0] == "EVENT"));
    client.disconnect().await;
}
