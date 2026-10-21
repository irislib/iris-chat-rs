use super::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

type Wire = Arc<Mutex<Vec<(usize, Value)>>>;

/// Real wire fixture: challenged queries only receive data after anonymous AUTH.
async fn relay(
    events: Vec<Event>,
    hold: Option<tokio::sync::watch::Receiver<bool>>,
) -> (RelayUrl, Wire, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let wire: Wire = Arc::default();
    let recorded = wire.clone();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let maximum = peak.clone();
    tokio::spawn(async move {
        let mut number = 0;
        while let Ok((stream, _)) = listener.accept().await {
            number += 1;
            let wire = recorded.clone();
            let events = events.clone();
            let active = active.clone();
            let peak = maximum.clone();
            let mut hold = hold.clone();
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                struct Active(Arc<AtomicUsize>);
                impl Drop for Active {
                    fn drop(&mut self) {
                        self.0.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                let _active = Active(active);
                socket
                    .send(Message::Text(json!(["AUTH", "fixture"]).to_string()))
                    .await
                    .unwrap();
                let mut authenticated = false;
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    wire.lock().unwrap().push((number, value.clone()));
                    match value[0].as_str() {
                        Some("AUTH") => {
                            let auth: Event = serde_json::from_value(value[1].clone()).unwrap();
                            auth.verify().unwrap();
                            assert_eq!(auth.kind, Kind::Authentication);
                            authenticated = true;
                            if socket
                                .send(Message::Text(json!(["OK", auth.id, true, ""]).to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some("REQ") if authenticated => {
                            if value[2]["limit"] == 1 {
                                let _ = socket
                                    .send(Message::Text(
                                        json!(["CLOSED", value[1], "fixture unavailable"])
                                            .to_string(),
                                    ))
                                    .await;
                                continue;
                            }
                            if let Some(hold) = hold.as_mut() {
                                while !*hold.borrow_and_update() {
                                    if hold.changed().await.is_err() {
                                        return;
                                    }
                                }
                            }
                            for event in &events {
                                if socket
                                    .send(Message::Text(
                                        json!(["EVENT", value[1], event]).to_string(),
                                    ))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            if value[2]["limit"] == 2 {
                                while socket.next().await.is_some() {}
                                return;
                            }
                            if socket
                                .send(Message::Text(json!(["EOSE", value[1]]).to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some("CLOSE") => break,
                        _ => {}
                    }
                }
            });
        }
    });
    (url, wire, peak)
}

fn profile(keys: &Keys, content: &str) -> Event {
    EventBuilder::new(Kind::Metadata, content)
        .sign_with_keys(keys)
        .unwrap()
}

fn filter(author: PublicKey) -> Filter {
    Filter::new().author(author).kind(Kind::Metadata)
}

#[tokio::test]
async fn anonymous_reads_wire_isolates_authors_retries_and_validates_delivery() {
    let peer = Keys::generate();
    let other = Keys::generate();
    let event = profile(&peer, "{};");
    let mut invalid = event.clone();
    invalid.content = "tampered".into();
    let (url, wire, _) = relay(
        vec![profile(&other, "wrong author"), invalid, event.clone()],
        None,
    )
    .await;
    for _ in 0..2 {
        assert_eq!(
            fetch(
                std::slice::from_ref(&url),
                vec![filter(peer.public_key())],
                Duration::from_secs(2)
            )
            .await
            .unwrap(),
            vec![event.clone()]
        );
    }
    // An unrelated correctly signed event and a tampered event did not starve
    // the valid event behind them. A separate author uses a separate context.
    assert_eq!(
        fetch(
            &[url],
            vec![filter(other.public_key())],
            Duration::from_secs(2)
        )
        .await
        .unwrap()
        .len(),
        1
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, value)| value[0] == "CLOSE")
            .count()
            < 3
        {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("relay observed every completed subscription close");
    let rows = wire.lock().unwrap();
    let mut auths = HashSet::new();
    let mut scopes = std::collections::HashMap::<usize, HashSet<String>>::new();
    for (connection, value) in rows.iter() {
        match value[0].as_str() {
            Some("AUTH") => {
                let key = value[1]["pubkey"].as_str().unwrap();
                assert_ne!(key, peer.public_key().to_hex());
                assert_ne!(key, other.public_key().to_hex());
                assert!(
                    auths.insert(key.to_string()),
                    "retry reused anonymous AUTH identity"
                );
            }
            Some("REQ") => {
                assert_eq!(value.as_array().unwrap().len(), 3);
                let authors = value[2]["authors"].as_array().unwrap();
                assert_eq!(authors.len(), 1);
                scopes
                    .entry(*connection)
                    .or_default()
                    .insert(authors.first().unwrap().as_str().unwrap().to_string());
                assert!(value[2].get("#p").is_none());
            }
            Some("CLOSE") => {}
            _ => panic!("unexpected outgoing discovery frame"),
        }
    }
    assert_eq!(auths.len(), 3);
    assert!(scopes.values().all(|authors| authors.len() == 1));
    assert_eq!(
        rows.iter().filter(|(_, value)| value[0] == "CLOSE").count(),
        3
    );
}

#[tokio::test]
async fn anonymous_reads_global_budget_covers_concurrent_callers_before_socket_open() {
    let peer = Keys::generate();
    let event = profile(&peer, "{}");
    let (release, hold) = tokio::sync::watch::channel(false);
    let (url, _, peak) = relay(vec![event], Some(hold)).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..24 {
        let url = url.clone();
        let author = peer.public_key();
        tasks.spawn(
            async move { fetch(&[url], vec![filter(author)], Duration::from_secs(3)).await },
        );
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while peak.load(Ordering::SeqCst) < COLD_READERS {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    sleep(Duration::from_millis(30)).await;
    assert_eq!(peak.load(Ordering::SeqCst), COLD_READERS);
    release.send(true).unwrap();
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap().unwrap().len(), 1);
    }
    assert!(peak.load(Ordering::SeqCst) <= COLD_READERS);
}

#[tokio::test]
async fn anonymous_reads_timeout_and_oversized_frames_fail_without_partial_success() {
    let peer = Keys::generate();
    let (_release, hold) = tokio::sync::watch::channel(false);
    let (url, _, _) = relay(vec![profile(&peer, "{}")], Some(hold)).await;
    assert!(fetch(
        &[url],
        vec![filter(peer.public_key())],
        Duration::from_millis(40)
    )
    .await
    .unwrap_err()
    .contains("timed out"));
    let (oversized, _, _) = relay(vec![profile(&peer, &"x".repeat(MAX_FRAME_BYTES))], None).await;
    let (empty, _, _) = relay(vec![], None).await;
    // A healthy relay's empty EOSE cannot hide a safety-limit failure on a
    // different relay and turn truncated history into apparent success.
    for relays in [
        vec![oversized.clone(), empty.clone()],
        vec![empty, oversized],
    ] {
        assert!(fetch(
            &relays,
            vec![filter(peer.public_key())],
            Duration::from_secs(2)
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn anonymous_reads_session_history_preserves_multiple_authors_without_shared_context() {
    let first = Keys::generate();
    let second = Keys::generate();
    let events = [&first, &second]
        .into_iter()
        .map(|keys| {
            EventBuilder::new(Kind::Custom(1060), "encrypted")
                .sign_with_keys(keys)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let (url, wire, _) = relay(events.clone(), None).await;
    let returned = fetch(
        &[url],
        vec![Filter::new()
            .kind(Kind::Custom(1060))
            .authors([first.public_key(), second.public_key()])],
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!(
        returned
            .into_iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>(),
        events
            .into_iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>()
    );
    let rows = wire.lock().unwrap();
    let mut authors = std::collections::HashMap::<usize, HashSet<String>>::new();
    for (connection, value) in rows.iter().filter(|(_, value)| value[0] == "REQ") {
        assert_eq!(value[2]["authors"].as_array().unwrap().len(), 1);
        authors
            .entry(*connection)
            .or_default()
            .insert(value[2]["authors"][0].as_str().unwrap().into());
    }
    assert_eq!(authors.len(), 2);
    assert!(authors.values().all(|scope| scope.len() == 1));
}

#[tokio::test]
async fn anonymous_reads_blackout_cancellation_drops_active_and_queued_jobs() {
    let peer = Keys::generate();
    let (release, hold) = tokio::sync::watch::channel(false);
    let (url, wire, _) = relay(vec![profile(&peer, "{}")], Some(hold)).await;
    let relays = vec![url.clone(); 24];
    let author = peer.public_key();
    let operation =
        tokio::spawn(
            async move { fetch(&relays, vec![filter(author)], Duration::from_secs(3)).await },
        );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let authenticated = wire
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, frame)| frame[0] == "AUTH")
                .count();
            if authenticated == COLD_READERS {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    release.send(true).unwrap();
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        wire.lock()
            .unwrap()
            .iter()
            .filter(|(_, frame)| frame[0] == "AUTH")
            .count(),
        COLD_READERS,
        "cancelled queued sockets opened"
    );
    assert_eq!(
        fetch(&[url], vec![filter(author)], Duration::from_secs(2))
            .await
            .unwrap()
            .len(),
        1,
        "cancelled jobs retained admission permits"
    );
}

#[tokio::test]
async fn anonymous_reads_retains_successful_scope_when_independent_scope_unavailable() {
    let peer = Keys::generate();
    let event = profile(&peer, "{}");
    let (url, _, _) = relay(vec![event.clone()], None).await;
    let unavailable = filter(Keys::generate().public_key()).limit(1);
    assert_eq!(
        fetch(
            &[url],
            vec![filter(peer.public_key()), unavailable],
            Duration::from_secs(2)
        )
        .await
        .unwrap(),
        vec![event]
    );
}

#[tokio::test]
async fn anonymous_reads_preserves_valid_message_history_above_metadata_result_cap() {
    let peer = Keys::generate();
    let content = "x".repeat(64 * 1024);
    let events = (0..257)
        .map(|index| {
            EventBuilder::new(
                Kind::Custom(MESSAGE_EVENT_KIND as u16),
                format!("{index}:{content}"),
            )
            .sign_with_keys(&peer)
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap().len())
            .sum::<usize>()
            > MAX_RESULT_BYTES
    );
    let expected = events.iter().map(|event| event.id).collect::<HashSet<_>>();
    let (url, _, _) = relay(events, None).await;
    let received = fetch(
        &[url],
        vec![Filter::new()
            .author(peer.public_key())
            .kind(Kind::Custom(MESSAGE_EVENT_KIND as u16))],
        Duration::from_secs(15),
    )
    .await
    .unwrap();
    assert_eq!(
        received
            .into_iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>(),
        expected,
        "valid history was truncated or discarded by the metadata bound"
    );
}

#[tokio::test]
async fn anonymous_reads_keeps_validated_metadata_at_deadline_when_relay_omits_eose() {
    let peer = Keys::generate();
    let event = profile(&peer, "{}");
    let (url, _, _) = relay(vec![event.clone()], None).await;
    assert_eq!(
        fetch(
            &[url],
            vec![filter(peer.public_key()).limit(2)],
            Duration::from_millis(40)
        )
        .await
        .unwrap(),
        vec![event]
    );
}
