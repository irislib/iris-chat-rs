use nostr::nips::nip44;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum RemoteSignerFault {
    None,
    ChangedEvent,
    Reject,
    UnsupportedSwitch,
    IgnoreSwitch,
    InvalidSwitch,
}

async fn remote_signer_response(
    client: &Client,
    transport: &Keys,
    recipient: PublicKey,
    response: Value,
) {
    let content = nip44::encrypt(
        transport.secret_key(),
        &recipient,
        response.to_string(),
        nip44::Version::V2,
    )
    .unwrap();
    let event = EventBuilder::new(Kind::NostrConnect, content)
        .tag(nostr::Tag::public_key(recipient))
        .sign_with_keys(transport)
        .unwrap();
    client.send_event(&event).await.unwrap();
}

fn launch_remote_signer_fixture(
    core: &AppCore,
    relay: &crate::local_relay::TestRelay,
    transport: &Keys,
    owner: &Keys,
    client_uri: Option<String>,
    fault: RemoteSignerFault,
    switched_relay: Option<String>,
) -> tokio::task::JoinHandle<()> {
    let transport = transport.clone();
    let owner = owner.clone();
    let client = Client::new(transport.clone());
    let mut notifications = client.notifications();
    core.runtime.block_on(async {
        client.add_relay(relay.url()).await.unwrap();
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(3)).await;
        client
            .subscribe(
                Filter::new()
                    .kind(Kind::NostrConnect)
                    .pubkey(transport.public_key()),
                None,
            )
            .await
            .unwrap();
    });
    core.runtime.spawn(async move {
        if let Some(uri) = client_uri {
            let uri = url::Url::parse(&uri).unwrap();
            let recipient = PublicKey::from_hex(uri.host_str().unwrap()).unwrap();
            let secret = uri.query_pairs().find(|(key, _)| key == "secret").unwrap().1.to_string();
            // A spoofed connect response with the wrong challenge must not pin its author.
            remote_signer_response(&client, &Keys::generate(), recipient, json!({"id":"spoof", "result":"wrong-secret"})).await;
            remote_signer_response(&client, &transport, recipient, json!({"id":"connect", "result":secret})).await;
        }
        let loop_result = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let RelayPoolNotification::Event { event, .. } = notifications.recv().await.unwrap() else { continue };
                if event.pubkey == transport.public_key() { continue; }
                let Ok(decrypted) = nip44::decrypt(transport.secret_key(), &event.pubkey, &event.content) else { continue };
                let request: Value = serde_json::from_str(&decrypted).unwrap();
                let Some(method) = request["method"].as_str() else { continue };
                let id = request["id"].as_str().unwrap();
                let recipient = event.pubkey;
                let result = match method {
                    "connect" => {
                        assert_eq!(request["params"][0], transport.public_key().to_hex());
                        assert_eq!(request["params"][1], "fixture-secret");
                        json!("ack")
                    }
                    "switch_relays" => {
                        if matches!(fault, RemoteSignerFault::IgnoreSwitch) { continue; }
                        if matches!(fault, RemoteSignerFault::UnsupportedSwitch) {
                            remote_signer_response(&client, &transport, recipient, json!({"id":id,"error":"unknown method"})).await;
                            continue;
                        }
                        if matches!(fault, RemoteSignerFault::InvalidSwitch) {
                            remote_signer_response(&client, &transport, recipient, json!({"id":id,"result":["https://invalid.example"]})).await;
                            return;
                        }
                        if let Some(next) = &switched_relay {
                            remote_signer_response(&client, &transport, recipient, json!({"id":id,"result":[next]})).await;
                            client.unsubscribe_all().await;
                            client.remove_all_relays().await;
                            client.add_relay(next.as_str()).await.unwrap();
                            client.connect().await;
                            client.wait_for_connection(Duration::from_secs(3)).await;
                            client.subscribe(Filter::new().kind(Kind::NostrConnect).pubkey(transport.public_key()), None).await.unwrap();
                            continue;
                        }
                        Value::Null
                    }
                    "get_public_key" => {
                        // Wrong request IDs and other authors are ignored after connection pinning.
                        remote_signer_response(&client, &transport, recipient, json!({"id":"wrong-id","result":Keys::generate().public_key().to_hex()})).await;
                        remote_signer_response(&client, &Keys::generate(), recipient, json!({"id":id,"result":Keys::generate().public_key().to_hex()})).await;
                        json!(owner.public_key().to_hex())
                    }
                    "sign_event" => {
                        if matches!(fault, RemoteSignerFault::Reject) {
                            remote_signer_response(&client, &transport, recipient, json!({"id":id,"error":"user rejected"})).await;
                            return;
                        }
                        let mut unsigned: UnsignedEvent = serde_json::from_str(request["params"][0].as_str().unwrap()).unwrap();
                        assert_eq!(unsigned.pubkey, owner.public_key());
                        assert_eq!(unsigned.kind.as_u16(), APP_KEYS_EVENT_KIND as u16);
                        if matches!(fault, RemoteSignerFault::ChangedEvent) {
                            unsigned.content = "changed".into(); unsigned.id = None;
                        }
                        let signed = unsigned.sign_with_keys(&owner).unwrap();
                        remote_signer_response(&client, &transport, recipient, json!({"id":id,"result":"auth_url","error":"https://signer.example/approve"})).await;
                        remote_signer_response(&client, &transport, recipient, json!({"id":id,"result":serde_json::to_string(&signed).unwrap()})).await;
                        return;
                    }
                    _ => panic!("unexpected signer method"),
                };
                remote_signer_response(&client, &transport, recipient, json!({"id":id,"result":result})).await;
            }
        }).await;
        client.shutdown().await;
        assert!(loop_result.is_ok(), "fixture timed out");
    })
}
