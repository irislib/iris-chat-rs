use super::*;
use std::sync::Mutex;
use tokio::net::TcpListener;

#[derive(Clone, Copy)]
pub(super) enum Behavior {
    Keep,
    Ping,
    Close,
    Reauthenticate,
    Flood,
    NoEose,
}

pub(super) struct Relay {
    pub(super) url: RelayUrl,
    pub(super) author: PublicKey,
    pub(super) wire: tls::Wire,
    pub(super) pong: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    pub(super) fn auth_keys(&self) -> HashSet<PublicKey> {
        let rows = self.wire.lock().unwrap();
        let mut keys = HashSet::new();
        let mut authors = std::collections::BTreeMap::<usize, HashSet<String>>::new();
        let mut authentications = 0;
        for (id, value) in rows.iter() {
            match value[0].as_str() {
                Some("AUTH") => {
                    let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                    event.verify().unwrap();
                    assert_eq!(event.kind, Kind::Authentication);
                    keys.insert(event.pubkey);
                    authentications += 1;
                }
                Some("REQ") => {
                    for filter in &value.as_array().unwrap()[2..] {
                        assert!(filter.get("#p").is_none());
                        let selected = filter["authors"].as_array().unwrap();
                        assert_eq!(selected.len(), 1);
                        authors
                            .entry(*id)
                            .or_default()
                            .insert(selected[0].as_str().unwrap().into());
                    }
                }
                Some("EVENT") => panic!("a read connection published an event"),
                _ => {}
            }
        }
        assert_eq!(
            keys.len(),
            authentications,
            "anonymous authentication keys must be fresh for every socket"
        );
        assert!(authors.values().all(|authors| authors.len() == 1));
        assert!(keys.iter().all(|key| authors
            .values()
            .all(|authors| !authors.contains(&key.to_hex()))));
        keys
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn relay(behavior: Behavior) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let wire = Arc::new(Mutex::new(Vec::new()));
    let recorded = wire.clone();
    let pong = Arc::new(tokio::sync::Notify::new());
    let observed_pong = pong.clone();
    let keys = Keys::generate();
    let author = keys.public_key();
    let event = EventBuilder::new(Kind::Metadata, "valid partial result")
        .sign_with_keys(&keys)
        .unwrap();
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        let mut id = 0;
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap();
                    id += 1;
                    let wire = recorded.clone();
                    let pong = observed_pong.clone();
                    let event = event.clone();
                    children.spawn(async move {
                        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                        socket.send(Message::Text(json!(["AUTH", format!("challenge-{id}")]).to_string())).await.unwrap();
                        let mut authenticated = false;
                        let mut reads = 0;
                        while let Some(Ok(message)) = socket.next().await {
                            if matches!(message, Message::Pong(_)) {
                                pong.notify_one();
                                continue;
                            }
                            let Message::Text(text) = message else { break; };
                            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                            wire.lock().unwrap().push((id, value.clone()));
                            match value[0].as_str() {
                                Some("AUTH") => {
                                    let auth: Event = serde_json::from_value(value[1].clone()).unwrap();
                                    auth.verify().unwrap();
                                    authenticated = true;
                                    if socket.send(Message::Text(json!(["OK", auth.id, true, ""]).to_string())).await.is_err() { break; }
                                }
                                Some("REQ") if authenticated => {
                                    reads += 1;
                                    if matches!(behavior, Behavior::Reauthenticate) && reads > 1 {
                                        let _ = socket.send(Message::Text(json!(["AUTH", "second-challenge"]).to_string())).await;
                                        continue;
                                    }
                                    if matches!(behavior, Behavior::NoEose) {
                                        let _ = socket.send(Message::Text(json!(["EVENT", value[1], event]).to_string())).await;
                                        continue;
                                    }
                                    if socket.send(Message::Text(json!(["EOSE", value[1]]).to_string())).await.is_err() { break; }
                                }
                                Some("CLOSE") => match behavior {
                                    Behavior::Close => { let _ = socket.close(None).await; break; },
                                    Behavior::Ping => { let _ = socket.send(Message::Ping(vec![1])).await; },
                                    Behavior::Flood => {
                                        for _ in 0..MAX_IDLE_CONTROL_FRAMES + 1 {
                                            if socket.send(Message::Ping(vec![1])).await.is_err() { break; }
                                        }
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    });
                },
                child = children.join_next(), if !children.is_empty() => {
                    child.unwrap().expect("retained discovery relay failed");
                }
            }
        }
    });
    Relay {
        url,
        author,
        wire,
        pong,
        task,
    }
}
