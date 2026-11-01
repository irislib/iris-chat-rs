//! Generated history with the phone's author count and observed handshake failures.
use super::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

struct Relay {
    url: RelayUrl,
    mode: Arc<AtomicUsize>,
    attempts: Arc<AtomicUsize>,
    wire: Arc<Mutex<Vec<(usize, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn relay(mode: usize, events: Arc<Vec<Event>>) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
    let modes = Arc::new(AtomicUsize::new(mode));
    let attempts = Arc::new(AtomicUsize::new(0));
    let wire = Arc::new(Mutex::new(Vec::new()));
    let (state, count, recorded) = (modes.clone(), attempts.clone(), wire.clone());
    let task = tokio::spawn(async move {
        let acceptor = tls_tests::acceptor(true, false);
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                incoming = listener.accept() => {
                    let Ok((stream, _)) = incoming else { break };
                    let id = count.fetch_add(1, Ordering::SeqCst);
                    let mode = state.load(Ordering::SeqCst);
                    let (acceptor, wire, events) = (acceptor.clone(), recorded.clone(), events.clone());
                    connections.spawn(async move {
                        let Ok(mut stream) = acceptor.accept(stream).await else { return };
                        if mode == 1 {
                            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                            return;
                        }
                        if mode == 2 {
                            // Keep the TLS connection alive, but never complete the upgrade.
                            let _stream = stream;
                            std::future::pending::<()>().await;
                            return;
                        }
                        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return };
                        if socket.send(Message::Text(json!(["AUTH", format!("fixture-{id}")]).to_string())).await.is_err() { return; }
                        let mut authenticated = false;
                        while let Some(Ok(Message::Text(text))) = socket.next().await {
                            let frame: Value = serde_json::from_str(&text).unwrap();
                            wire.lock().unwrap().push((id, frame.clone()));
                            let mut replies = Vec::new();
                            match frame[0].as_str() {
                                Some("AUTH") => {
                                    let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
                                    auth.verify().unwrap();
                                    assert_eq!(auth.kind, Kind::Authentication);
                                    authenticated = true;
                                    replies.push(json!(["OK", auth.id, true, ""]));
                                }
                                Some("REQ") if authenticated => {
                                    let filter: Filter = serde_json::from_value(frame[2].clone()).unwrap();
                                    for event in events.iter().filter(|event| filter.match_event(event, nostr::filter::MatchEventOptions::new())) {
                                        replies.push(json!(["EVENT", frame[1], event]));
                                    }
                                    replies.push(json!(["EOSE", frame[1]]));
                                }
                                Some("REQ") => replies.push(json!(["CLOSED", frame[1], "auth-required: fixture"])),
                                Some("CLOSE") => break,
                                _ => {}
                            }
                            for reply in replies {
                                if socket.send(Message::Text(reply.to_string())).await.is_err() { return; }
                            }
                        }
                    });
                }
                result = connections.join_next(), if !connections.is_empty() => { result.unwrap().unwrap(); }
            }
        }
    });
    Relay {
        url,
        mode: modes,
        attempts,
        wire,
        task,
    }
}

async fn history(relay: &Relay, author: PublicKey) -> Result<Vec<Event>, ReadFailure> {
    let filters = [Filter::new().author(author).kind(Kind::Custom(1060))];
    let tls = ScopedTlsConnector::new_with_roots(&relay.url, &filters, tls_tests::roots()).unwrap();
    read_one_with_connector(
        &relay.url,
        &filters,
        Duration::from_millis(250),
        Some(tls.connector(&relay.url, &filters).unwrap()),
        None,
        None,
    )
    .await
}

#[tokio::test]
async fn failed_servers_do_not_multiply_handshakes_by_history_author_count() {
    if super::tests::capacity_experiment_with_watchdog("core::protocol::anonymous_reads::failed_relay_tests::failed_servers_do_not_multiply_handshakes_by_history_author_count", Duration::from_secs(40)) { return; }
    let events = Arc::new(
        (0..78)
            .map(|_| {
                EventBuilder::new(Kind::Custom(1060), "synthetic encrypted history")
                    .sign_with_keys(&Keys::generate())
                    .unwrap()
            })
            .collect::<Vec<_>>(),
    );
    let mut relays = Vec::new();
    for mode in [1, 2, 0, 0, 0] {
        relays.push(relay(mode, events.clone()).await);
    }
    for event in events.iter() {
        let returned =
            futures_util::future::join_all(relays.iter().map(|relay| history(relay, event.pubkey)))
                .await;
        for result in &returned[2..] {
            assert_eq!(
                result
                    .as_ref()
                    .ok()
                    .map(|events| events.iter().map(|e| e.id).collect::<Vec<_>>()),
                Some(vec![event.id])
            );
        }
    }
    let attempts = relays
        .iter()
        .map(|relay| relay.attempts.load(Ordering::SeqCst))
        .collect::<Vec<_>>();
    println!("78 isolated authors, five TLS servers: handshake attempts={attempts:?}");
    assert!(
        attempts[0] <= 2 && attempts[1] <= 2,
        "failed servers must cool down across authors: {attempts:?}"
    );
    for relay in &relays[..2] {
        relay.mode.store(0, Ordering::SeqCst);
    }
    let recovery_deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    for relay in &relays[..2] {
        loop {
            if history(relay, events[0].pubkey).await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < recovery_deadline,
                "recovered server must become readable after its bounded cooldown"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for event in events.iter() {
            assert_eq!(
                history(relay, event.pubkey).await.ok().unwrap()[0].id,
                event.id
            );
        }
    }
    let authors = events
        .iter()
        .map(|event| event.pubkey)
        .collect::<HashSet<_>>();
    let mut auth_keys = HashSet::new();
    for relay in &relays {
        let wire = relay.wire.lock().unwrap();
        let mut subjects = HashMap::<usize, HashSet<String>>::new();
        for (connection, frame) in wire.iter() {
            if frame[0] == "AUTH" {
                let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                assert!(!authors.contains(&event.pubkey));
                assert!(
                    auth_keys.insert(event.pubkey),
                    "fresh anonymous AUTH per connection"
                );
            }
            if frame[0] == "REQ" {
                assert_eq!(frame.as_array().unwrap().len(), 3);
                let filter = &frame[2];
                assert_eq!(filter["authors"].as_array().unwrap().len(), 1);
                subjects
                    .entry(*connection)
                    .or_default()
                    .insert(filter["authors"][0].as_str().unwrap().to_owned());
                assert!(filter.get("#p").is_none());
            }
            assert_ne!(frame[0], "EVENT", "read connections never publish");
        }
        assert!(subjects.values().all(|authors| authors.len() == 1));
    }
}
