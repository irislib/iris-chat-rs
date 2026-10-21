//! Keep the populated-account failure in a capped process, not the test runner.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::core::{AppCore, LocalAuthorizationState, LoggedInState};
use crate::state::AppState;
use crate::updates::{CoreMsg, InternalEvent};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, RelayUrl, Timestamp};
use nostr_sdk::Client;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

const CHILD: &str = "IRIS_TEST_DISCOVERY_MEMORY_CHILD";
const TEST: &str = if cfg!(target_os = "linux") {
    "core::protocol::anonymous_reads::memory_tests::populated_account_discovery_under_memory_limit"
} else {
    "core::protocol::anonymous_reads::memory_tests::populated_account_discovery_host_progress"
};
const PEERS: usize = 512;

#[test]
#[cfg(target_os = "linux")]
fn populated_account_discovery_under_memory_limit() {
    if let Ok(mode) = std::env::var(CHILD) {
        match mode.as_str() {
            "baseline" => baseline(),
            "legacy-sdk" => legacy_sdk_allocation(),
            "production" => populated_startup(),
            _ => panic!("unknown memory-test child mode"),
        }
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(60);
    let baseline = capped_child("baseline", deadline);
    assert!(baseline.status.success(), "baseline infrastructure failed");
    assert!(String::from_utf8_lossy(&baseline.stdout).contains("BASELINE_OK"));

    // This is the actual SDK allocation that caused startup to abort. Never
    // run it in-process, even when running only this regression locally.
    let legacy = capped_child("legacy-sdk", deadline);
    let diagnostic = String::from_utf8_lossy(&legacy.stderr);
    assert!(
        !legacy.status.success()
            && diagnostic.contains("memory allocation of ")
            && diagnostic.contains("bytes failed")
            && String::from_utf8_lossy(&legacy.stdout).contains("LEGACY_SDK_BEGIN"),
        "calibration must fail from the Rust allocator, not unrelated setup"
    );

    let production = capped_child("production", deadline);
    assert!(
        production.status.success(),
        "populated startup exceeded its cap or failed"
    );
    let stdout = String::from_utf8_lossy(&production.stdout);
    assert!(stdout.contains("PRODUCTION_PROGRESS peers=512"));
    assert!(stdout.contains("PRODUCTION_CANCELLED"));
}

#[test]
#[cfg(not(target_os = "linux"))]
fn populated_account_discovery_host_progress() {
    // Functional host proof only. The Linux test is the memory-cap gate.
    if std::env::var(CHILD).as_deref() == Ok("production") {
        populated_startup();
        return;
    }
    let output = capped_child("production", Instant::now() + Duration::from_secs(60));
    assert!(output.status.success(), "host discovery child failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("PRODUCTION_CANCELLED"));
}

fn capped_child(mode: &str, deadline: Instant) -> Output {
    // Static shell text, positional arguments only. The address-space limit is
    // in effect before the real test executable and its runtime are loaded.
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            if cfg!(target_os = "linux") {
                "ulimit -c 0 && ulimit -v 786432 && exec \"$@\""
            } else {
                "ulimit -c 0 && exec \"$@\""
            },
            "iris-discovery-memory-child",
        ])
        .arg(std::env::current_exe().expect("test executable"))
        .args(["--exact", TEST, "--nocapture", "--test-threads", "1"])
        .env(CHILD, mode)
        // Avoid scaling 8 MiB worker stacks with a developer machine's CPU
        // count. This is Tokio's existing setting, not an app test switch.
        .env("TOKIO_WORKER_THREADS", "2")
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start capped child");
    let mut peak_kib = 0;
    let timed_out = loop {
        peak_kib = peak_kib.max(process_peak_rss(child.id()).unwrap_or(0));
        if child.try_wait().expect("poll capped child").is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill overdue capped child");
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().expect("collect capped child");
    eprintln!(
        "memory child={mode} status={} observed_peak_rss_kib={peak_kib}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !timed_out,
        "capped child exceeded the absolute parent deadline"
    );
    output
}

fn process_peak_rss(pid: u32) -> Option<u64> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

fn fixture() -> (tempfile::TempDir, AppCore, flume::Receiver<CoreMsg>) {
    let temp = tempfile::tempdir().expect("isolated app data");
    let (updates, _) = flume::unbounded();
    let (sender, incoming) = flume::unbounded();
    let mut core = AppCore::new(
        updates,
        sender,
        temp.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let owner = Keys::generate();
    let device = Keys::generate();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    (temp, core, incoming)
}

fn baseline() {
    let (_temp, _core, _incoming) = fixture();
    println!(
        "BASELINE_OK peak_rss_kib={:?}",
        process_peak_rss(std::process::id())
    );
}

fn legacy_sdk_allocation() {
    let (_temp, core, _incoming) = fixture();
    let _entered = core.runtime.enter();
    println!("LEGACY_SDK_BEGIN clients={PEERS}");
    std::io::stdout().flush().expect("flush calibration marker");
    let mut clients = Vec::with_capacity(PEERS);
    for _ in 0..PEERS {
        clients.push(Client::new(Keys::generate()));
    }
    // Completing this calibration means the pinned SDK's allocation behavior
    // changed and the regression's negative control needs explicit review.
    println!("LEGACY_SDK_COMPLETED clients={}", clients.len());
}

#[derive(Default)]
struct RelayState {
    active: AtomicUsize,
    requests: AtomicUsize,
    parked: AtomicUsize,
    phase: AtomicUsize,
    seen: Mutex<HashSet<String>>,
}

struct Connection(Arc<RelayState>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve_connection(
    stream: TcpStream,
    state: Arc<RelayState>,
    events: Arc<HashMap<String, Event>>,
    mut release: watch::Receiver<bool>,
) {
    // Include accepted sockets whose handshake is still in flight in the
    // cancellation proof. Cancelling a reader can close TCP before it sends
    // the complete WebSocket handshake.
    state.active.fetch_add(1, Ordering::SeqCst);
    let _connection = Connection(state.clone());
    let mut socket = match tokio_tungstenite::accept_async(stream).await {
        Ok(socket) => socket,
        Err(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::HandshakeIncomplete,
        )) if state.phase.load(Ordering::SeqCst) == 1 => return,
        Err(error) => panic!("local handshake: {error}"),
    };
    while let Some(Ok(Message::Text(text))) = socket.next().await {
        let frame: serde_json::Value = serde_json::from_str(&text).expect("serialized request");
        if frame[0] != "REQ" {
            continue;
        }
        let author = frame[2]["authors"].as_array().expect("author-scoped read");
        assert_eq!(author.len(), 1, "private interests must not be batched");
        let author = author[0].as_str().expect("author key");
        let index = state.requests.fetch_add(1, Ordering::SeqCst);
        state.seen.lock().expect("seen lock").insert(author.into());
        if state.phase.load(Ordering::SeqCst) == 1 {
            // Cancellation must close both active reads and queued scope work.
            while socket.next().await.is_some() {}
            return;
        }
        if index < 4 {
            state.parked.fetch_add(1, Ordering::SeqCst);
            loop {
                let released = *release.borrow();
                if released {
                    break;
                }
                tokio::select! {
                    changed = release.changed() => if changed.is_err() { return; },
                    _ = socket.next() => return,
                }
            }
        }
        let event = events.get(author).expect("requested fixture author");
        socket
            .send(Message::Text(
                serde_json::json!(["EVENT", frame[1], event]).to_string(),
            ))
            .await
            .expect("local event");
        socket
            .send(Message::Text(
                serde_json::json!(["EOSE", frame[1]]).to_string(),
            ))
            .await
            .expect("local end of stored events");
    }
}

fn populated_startup() {
    let (_temp, mut core, incoming) = fixture();
    let handle = core.runtime.handle().clone();
    handle.block_on(async {
        let events = Arc::new(
            (0..PEERS)
                .map(|_| {
                    let keys = Keys::generate();
                    let event = EventBuilder::new(Kind::Metadata, "{\"name\":\"memory fixture\"}")
                        .sign_with_keys(&keys)
                        .expect("signed profile");
                    (keys.public_key().to_hex(), event)
                })
                .collect::<HashMap<_, _>>(),
        );
        assert_eq!(events.len(), PEERS);
        let filters = events
            .values()
            .map(|event| Filter::new().kind(Kind::Metadata).author(event.pubkey))
            .collect::<Vec<_>>();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("local relay");
        let relay = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let state = Arc::new(RelayState::default());
        let (release, release_rx) = watch::channel(false);
        let server_state = state.clone();
        let server_events = events.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.expect("local accept");
                        connections.spawn(serve_connection(stream, server_state.clone(),
                            server_events.clone(), release_rx.clone()));
                    },
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.expect("connection result").expect("local connection task");
                    },
                }
            }
        });

        // This is the same registration and apply path used when reconciling
        // protocol subscriptions after a populated account is opened.
        let scopes = core
            .private_discovery_subscriptions(filters.clone())
            .expect("register peers");
        super::super::private_reads::apply_private_discovery_subscriptions(
            scopes,
            &[relay.clone()],
        )
        .await
        .expect("apply production scopes");
        let mut delivered = HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while delivered.len() < PEERS {
            assert!(
                Instant::now() < deadline,
                "not every retained author made progress"
            );
            if let Ok(Ok(CoreMsg::Internal(event))) =
                tokio::time::timeout(Duration::from_millis(100), incoming.recv_async()).await
            {
                if let InternalEvent::RelayEvent(event) = *event {
                    assert!(events.contains_key(&event.pubkey.to_hex()));
                    delivered.insert(event.pubkey);
                }
            }
            let released = *release.borrow();
            if delivered.len() >= 64 && !released {
                assert_eq!(state.parked.load(Ordering::SeqCst), 4);
                release.send(true).expect("release stalled reads");
            }
        }
        assert_eq!(state.seen.lock().unwrap().len(), PEERS);
        println!("PRODUCTION_PROGRESS peers={}", delivered.len());

        // Cold startup/backfill goes through the other production reader too;
        // retaining all results must not recreate one SDK cache per author.
        let cold = tokio::time::timeout(
            Duration::from_secs(15),
            super::fetch(&[relay.clone()], filters.clone(), Duration::from_secs(3)),
        )
        .await
        .expect("cold populated discovery deadline")
        .expect("cold populated discovery");
        assert_eq!(cold.len(), PEERS);
        assert!(cold.iter().all(|event| events
            .get(&event.pubkey.to_hex())
            .is_some_and(|expected| expected.id == event.id)));
        println!("PRODUCTION_COLD peers={}", cold.len());

        state.phase.store(1, Ordering::SeqCst);
        for reset in [false, true] {
            assert!(
                !server.is_finished(),
                "local relay exited before cancellation"
            );
            let changed = filters
                .iter()
                .cloned()
                .map(|filter| filter.since(Timestamp::from(if reset { 2 } else { 1 })))
                .collect();
            let scopes = core.private_discovery_subscriptions(changed).unwrap();
            let stale_apply = scopes.clone();
            let before = state.requests.load(Ordering::SeqCst);
            super::super::private_reads::apply_private_discovery_subscriptions(
                scopes,
                &[relay.clone()],
            )
            .await
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while state.requests.load(Ordering::SeqCst) == before {
                assert!(Instant::now() < deadline, "no active read to cancel");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(
                state.requests.load(Ordering::SeqCst) - before < PEERS,
                "no queued cancellation case"
            );
            if reset {
                core.protocol_subscription_runtime = Default::default();
            } else {
                core.private_discovery_subscriptions(Vec::new())
                    .expect("remove interests");
            }
            assert!(
                super::super::private_reads::apply_private_discovery_subscriptions(
                    stale_apply,
                    &[relay.clone()]
                )
                .await
                .is_err(),
                "an obsolete apply must not revive removed interests"
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while state.active.load(Ordering::SeqCst) != 0 {
                assert!(Instant::now() < deadline, "cancelled sockets remained open");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let cancelled_at = state.requests.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(
                state.requests.load(Ordering::SeqCst),
                cancelled_at,
                "queued work opened after its lifetime ended"
            );
            assert_eq!(
                state.active.load(Ordering::SeqCst),
                0,
                "cancelled handshakes or sockets remained open"
            );
            assert!(
                !server.is_finished(),
                "local relay exited during cancellation"
            );
        }
        println!(
            "PRODUCTION_CANCELLED peak_rss_kib={:?}",
            process_peak_rss(std::process::id())
        );
        server.abort();
        let _ = server.await;
    });
}
