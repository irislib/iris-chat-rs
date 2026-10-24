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
    relay_with_ready(events, hold, None).await
}

async fn relay_with_ready(
    events: Vec<Event>,
    hold: Option<tokio::sync::watch::Receiver<bool>>,
    event_sent_without_eose: Option<Arc<tokio::sync::Notify>>,
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
            let event_sent_without_eose = event_sent_without_eose.clone();
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
                                if let Some(ready) = &event_sent_without_eose {
                                    ready.notify_one();
                                }
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
async fn retained_signature_cache_still_matches_current_filters_and_uses_fresh_auth() {
    let peer = Keys::generate();
    let event = profile(&peer, "{};");
    let mut tampered = event.clone();
    tampered.content.push('!');
    let (url, wire, _) = relay(vec![event.clone(), tampered], None).await;
    let mut cache = VerificationCache::default();
    for (kind, expected) in [
        (Kind::Metadata, vec![event.clone()]),
        (Kind::TextNote, vec![]),
        (Kind::Metadata, vec![event.clone()]),
    ] {
        // Deliberately retain the low-level cache while changing the filter:
        // a valid signature hit must never bypass matching the current request.
        assert_eq!(
            poll_retained(
                &url,
                vec![Filter::new().author(peer.public_key()).kind(kind)],
                None,
                cache.relay(0),
                None,
            )
            .await
            .unwrap(),
            expected
        );
    }
    assert_eq!(cache.signature_calls.load(Ordering::Relaxed), 1);
    // Cold reads keep their existing full verification path and reject the
    // same tampered event even after the retained worker has a positive hit.
    assert_eq!(
        fetch(
            &[url],
            vec![filter(peer.public_key())],
            Duration::from_secs(2)
        )
        .await
        .unwrap(),
        vec![event]
    );
    let rows = wire.lock().unwrap();
    let mut signers = HashSet::new();
    let mut connections = HashSet::new();
    for (connection, value) in rows.iter() {
        match value[0].as_str() {
            Some("AUTH") => {
                let auth: Event = serde_json::from_value(value[1].clone()).unwrap();
                auth.verify().unwrap();
                assert_ne!(auth.pubkey, peer.public_key());
                assert!(signers.insert(auth.pubkey));
            }
            Some("REQ") => {
                assert_eq!(value[2]["authors"], json!([peer.public_key().to_hex()]));
                assert!(value[2].get("#p").is_none());
                connections.insert(*connection);
            }
            Some("EVENT") => panic!("discovery read published an event"),
            _ => {}
        }
    }
    assert_eq!(signers.len(), 4);
    assert_eq!(connections.len(), 4);
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

/// These experiments each need to occupy the entire process-global budget.
/// Separate exact-test children let them run concurrently under the parallel
/// harness without sharing that deliberately saturated application process.
fn capacity_experiment_in_child(test: &str) -> bool {
    use std::process::{Command, Stdio};
    const CHILD: &str = "IRIS_TEST_ANONYMOUS_CAPACITY_CHILD";
    let name = format!("core::protocol::anonymous_reads::tests::{test}");
    if std::env::var(CHILD).as_deref() == Ok(name.as_str()) {
        return false;
    }
    let executable = std::env::current_exe().expect("test executable");
    let mut command = if cfg!(unix) {
        let mut shell = Command::new("/bin/sh");
        shell
            .args([
                "-c",
                "ulimit -c 0 && exec \"$@\"",
                "iris-anonymous-capacity-child",
            ])
            .arg(&executable);
        shell
    } else {
        Command::new(&executable)
    };
    let mut child = command
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env(CHILD, &name)
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start owned capacity experiment");
    let deadline = Instant::now() + Duration::from_secs(20);
    let timed_out = loop {
        if child
            .try_wait()
            .expect("poll capacity experiment")
            .is_some()
        {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop overdue owned experiment");
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child
        .wait_with_output()
        .expect("collect capacity experiment");
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!(
        "capacity experiment={name} status={}\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!timed_out, "capacity experiment exceeded its watchdog");
    assert!(output.status.success(), "capacity experiment failed");
    assert!(
        stdout.contains("test result: ok. 1 passed; 0 failed"),
        "exact child must execute its assertion-bearing test"
    );
    true
}

#[tokio::test]
async fn anonymous_reads_global_budget_covers_concurrent_callers_before_socket_open() {
    if capacity_experiment_in_child(
        "anonymous_reads_global_budget_covers_concurrent_callers_before_socket_open",
    ) {
        return;
    }

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
        assert!(
            poll_relays(&relays, &[filter(peer.public_key())], &[None, None])
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn anonymous_background_relay_polls_union_events_and_reject_mixed_authors() {
    let peer = Keys::generate();
    let filters = [filter(peer.public_key())];
    let events = [
        profile(&peer, "first"),
        profile(&peer, "shared"),
        profile(&peer, "second"),
    ];
    let (first, first_wire, _) = relay(events[..2].to_vec(), None).await;
    let (second, second_wire, _) = relay(events[1..].to_vec(), None).await;
    let relays = [first, second];
    let received = poll_relays(&relays, &filters, &[None, None])
        .await
        .expect("small complete relay results");
    assert_eq!(received.len(), events.len());
    assert_eq!(
        received
            .iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>(),
        events.iter().map(|event| event.id).collect::<HashSet<_>>()
    );
    let wires = [first_wire, second_wire];
    // A late CLOSE is harmless. Count only AUTH/REQ when checking that an
    // invalid multi-author scope never emits either frame on either relay.
    let request_count = |wire: &Wire| {
        wire.lock()
            .unwrap()
            .iter()
            .filter(|(_, value)| matches!(value[0].as_str(), Some("AUTH" | "REQ")))
            .count()
    };
    let before = wires.each_ref().map(request_count);
    assert!(poll_relays(
        &relays,
        &[filters[0].clone(), filter(Keys::generate().public_key())],
        &[None, None],
    )
    .await
    .is_err());
    assert_eq!(
        wires.each_ref().map(request_count),
        before,
        "invalid scope emitted AUTH or REQ"
    );
    let mut auths = HashSet::new();
    for wire in &wires {
        for (_, value) in wire.lock().unwrap().iter() {
            match value[0].as_str() {
                Some("AUTH") => {
                    let key = value[1]["pubkey"].as_str().unwrap();
                    assert_ne!(key, peer.public_key().to_hex());
                    assert!(auths.insert(key.to_string()), "relay reused anonymous AUTH");
                }
                Some("REQ") => {
                    assert_eq!(value.as_array().unwrap().len(), 3);
                    assert_eq!(value[2]["authors"], json!([peer.public_key().to_hex()]));
                    assert!(value[2].get("#p").is_none());
                }
                Some("CLOSE") => {}
                _ => panic!("unexpected outgoing discovery frame"),
            }
        }
    }
    assert_eq!(auths.len(), 2);
}

#[tokio::test]
async fn anonymous_reads_complete_relay_results_enforce_aggregate_byte_bound() {
    let peer = Keys::generate();
    let payload = "x".repeat(MAX_FRAME_BYTES - 2048);
    let per_relay = MAX_RESULT_BYTES / payload.len() / 2 + 1;
    let events = (0..per_relay * 2)
        .map(|index| profile(&peer, &format!("{index}:{payload}")))
        .collect::<Vec<_>>();
    let serialized_bytes = |events: &[Event]| {
        events
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap().len())
            .sum::<usize>()
    };
    assert_eq!(
        events
            .iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>()
            .len(),
        events.len(),
        "aggregate fixture events must be unique"
    );
    for event in &events {
        event.verify().expect("valid signed fixture event");
        assert!(json!(["EVENT", "fixture", event]).to_string().len() < MAX_FRAME_BYTES);
    }
    let (first, second) = events.split_at(per_relay);
    for batch in [first, second] {
        assert!(serialized_bytes(batch) < MAX_RESULT_BYTES);
        let received = collect_reads(stream::iter([Ok(batch.to_vec())]), false)
            .await
            .expect("each complete relay result fits independently");
        assert_eq!(received.len(), batch.len());
    }
    assert!(serialized_bytes(&events) > MAX_RESULT_BYTES);
    // Feed complete validated per-relay results through the production
    // aggregator. Network deadlines deliberately permit partial delivery and
    // must not decide whether this deterministic byte-bound assertion runs.
    match collect_reads(
        stream::iter([Ok(first.to_vec()), Ok(second.to_vec())]),
        false,
    )
    .await
    {
        Err(error) => assert!(error.contains("result exceeds safety bound"), "{error}"),
        Ok(received) => panic!(
            "aggregate byte bound accepted {} events / {} serialized bytes",
            received.len(),
            serialized_bytes(&received)
        ),
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
    if capacity_experiment_in_child(
        "anonymous_reads_blackout_cancellation_drops_active_and_queued_jobs",
    ) {
        return;
    }

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
    let ready = Arc::new(tokio::sync::Notify::new());
    let (url, _, _) = relay_with_ready(vec![event.clone()], None, Some(ready.clone())).await;
    let author = peer.public_key();
    let budget = Duration::from_secs(2);
    let started = Instant::now();
    // This case isolates the actual socket deadline from admission, which is
    // separately tested with concurrent callers saturating the global budget.
    // Parallel tests may deliberately hold all four cold permits.
    let read = tokio::spawn(async move {
        read_one(&url, &[filter(author).limit(2)], budget)
            .await
            .map_err(ReadFailure::message)
    });
    tokio::time::timeout(Duration::from_secs(3), ready.notified())
        .await
        .expect("relay sent the event and is deliberately withholding EOSE");
    let received = tokio::time::timeout(Duration::from_secs(3), read)
        .await
        .expect("finite socket read ended")
        .unwrap()
        .unwrap();
    assert!(
        started.elapsed() >= budget,
        "read ended before its missing-EOSE deadline"
    );
    assert_eq!(received, vec![event]);
}
