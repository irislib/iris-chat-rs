use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::{io::BufRead, io::BufReader};

use iris_chat_core::local_relay::TestRelay;
use iris_chat_core::{AppAction, AppState, DeviceAuthorizationState, FfiApp};
use nostr::{Event, Keys};
use nostr_double_ratchet::{
    invite_url, parse_message_event, process_invite_response_event, APP_KEYS_EVENT_KIND,
    INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND,
};
use nostr_double_ratchet::{Invite, ProtocolContext, Session, UnixSeconds};
use serde_json::Value;
use tempfile::TempDir;

fn new_app(data_dir: &Path) -> Arc<FfiApp> {
    FfiApp::new(
        data_dir.to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    )
}

fn wait_for_app_state(
    app: &FfiApp,
    label: &str,
    timeout: Duration,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    let started = Instant::now();
    let mut last = app.state();
    while started.elapsed() < timeout {
        last = app.state();
        if predicate(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {label}; last_state={last:?}");
}

fn dispatch_and_wait_state(
    app: &FfiApp,
    action: AppAction,
    label: &str,
    timeout: Duration,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    app.dispatch(action);
    wait_for_app_state(app, label, timeout, predicate)
}

fn iris_binary() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(path) = option_env!("CARGO_BIN_EXE_iris") {
            let path = PathBuf::from(path);
            if path.exists() {
                return path;
            }
        }

        let mut fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fallback.push("target");
        fallback.push("debug");
        fallback.push("iris");
        #[cfg(windows)]
        fallback.set_extension("exe");
        if fallback.exists() {
            return fallback;
        }

        let status = Command::new("cargo")
            .args(["build", "--bin", "iris"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("build iris binary");
        assert!(status.success(), "cargo build --bin iris failed");
        fallback
    })
}

fn run_iris(data_dir: &Path, args: &[&str]) -> Value {
    let output = Command::new(iris_binary())
        .arg("--json")
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run iris");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "iris failed args={:?} status={}\nstdout={}\nstderr={}\ndebug={}",
        args,
        output.status,
        stdout,
        stderr,
        debug_snapshot(data_dir)
    );
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("invalid json: {error}\nstdout={stdout}\nstderr={stderr}"))
}

fn run_iris_capture(data_dir: &Path, args: &[&str]) -> String {
    let output = Command::new(iris_binary())
        .arg("--json")
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run iris");
    format!(
        "args={:?} status={}\nstdout={}\nstderr={}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn debug_snapshot(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join("iris_chat_runtime_debug.json"))
        .unwrap_or_else(|error| format!("debug snapshot unavailable: {error}"))
}

fn relay_events(relay: &TestRelay) -> Vec<Value> {
    relay
        .events()
        .into_iter()
        .filter_map(|event| {
            serde_json::to_string(&event)
                .ok()
                .and_then(|json| serde_json::from_str(&json).ok())
        })
        .collect()
}

fn relay_event_summary(relay: &TestRelay) -> Value {
    Value::Array(
        relay_events(relay)
            .into_iter()
            .filter(|event| {
                matches!(
                    event.get("kind").and_then(Value::as_u64),
                    Some(1059 | 1060 | 30078)
                )
            })
            .map(|event| {
                serde_json::json!({
                    "id": event.get("id").cloned().unwrap_or(Value::Null),
                    "kind": event.get("kind").cloned().unwrap_or(Value::Null),
                    "pubkey": event.get("pubkey").cloned().unwrap_or(Value::Null),
                    "tags": event.get("tags").cloned().unwrap_or(Value::Null),
                })
            })
            .collect(),
    )
}

fn state_contains_group(value: &Value, group_id: &str) -> bool {
    let group_chat_id = format!("group:{group_id}");
    value
        .pointer("/data/groups")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("group_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == group_id)
                    || group
                        .get("chat_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id == group_chat_id)
            })
        })
}

fn start_iris(data_dir: &Path, args: &[&str]) -> std::process::Child {
    Command::new(iris_binary())
        .arg("--json")
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start iris")
}

fn spawn_json_reader(stdout: std::process::ChildStdout) -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let value = serde_json::from_str(line.trim())
                        .unwrap_or_else(|error| panic!("invalid json line: {error}\nline={line}"));
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                Err(error) => panic!("read iris stdout: {error}"),
            }
        }
    });
    rx
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr) -> Arc<Mutex<String>> {
    let output = Arc::new(Mutex::new(String::new()));
    let output_for_thread = output.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Ok(mut output) = output_for_thread.lock() {
                        output.push_str(&line);
                    }
                }
                Err(_) => break,
            }
        }
    });
    output
}

fn read_owner_nsec(data_dir: &Path) -> String {
    let path = data_dir.join("cli-account.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let bundle: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    bundle
        .get("owner_nsec")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("{} missing owner_nsec", path.display()))
        .to_string()
}

fn wait_for_listener_ready(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<Value>,
    stderr: &Arc<Mutex<String>>,
) {
    // `iris listen` may restore an existing account, foreground the app, and
    // wait through its full network-runtime readiness window before printing.
    let ready = receiver
        .recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|error| {
            let status = child.try_wait().expect("child status");
            let stderr = stderr.lock().map(|text| text.clone()).unwrap_or_default();
            panic!(
                "timed out waiting for iris listen ready: {error}; status={status:?}; stderr={stderr}"
            );
        });
    assert_eq!(ready["command"], "listen");
    assert_eq!(ready["data"]["ready"], true);
    assert_eq!(ready["data"]["network"], true);
}

fn wait_for_relay_event(relay: &TestRelay, kind: u64) -> Event {
    let started = Instant::now();
    let mut last_events = Vec::new();
    while started.elapsed() < Duration::from_secs(10) {
        last_events = relay_events(relay);
        for event in &last_events {
            if event.get("kind").and_then(Value::as_u64) == Some(kind) {
                return serde_json::from_value(event.clone()).expect("relay event json");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let kinds = last_events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_u64))
        .collect::<Vec<_>>();
    panic!("timed out waiting for relay event kind {kind}; saw kinds {kinds:?}");
}

fn wait_for_relay_events(relay: &TestRelay, kind: u64, count: usize) -> Vec<Event> {
    let started = Instant::now();
    let mut matched = Vec::new();
    while started.elapsed() < Duration::from_secs(10) {
        matched = relay_events(relay)
            .into_iter()
            .filter(|event| event.get("kind").and_then(Value::as_u64) == Some(kind))
            .filter_map(|event| serde_json::from_value(event).ok())
            .collect::<Vec<_>>();
        if matched.len() >= count {
            return matched;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "timed out waiting for {count} relay events kind {kind}; saw {}; relay_events={}",
        matched.len(),
        relay_event_summary(relay)
    );
}

fn read_stream_message(
    relay: &TestRelay,
    receiver: &mpsc::Receiver<Value>,
    expected_body: &str,
) -> Option<Value> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        relay.replay_stored();
        let line = match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(error) => panic!("iris listen stream closed: {error}"),
        };
        if line.get("command").and_then(Value::as_str) != Some("message") {
            continue;
        }
        if line
            .get("data")
            .and_then(|data| data.get("body"))
            .and_then(Value::as_str)
            == Some(expected_body)
        {
            return Some(line);
        }
    }
    None
}

fn sync_until_message(
    relay: &TestRelay,
    data_dir: &Path,
    chat_id: &str,
    expected_body: &str,
) -> Value {
    let started = Instant::now();
    let mut last_sync = Value::Null;
    let mut last_read = Value::Null;
    while started.elapsed() < Duration::from_secs(30) {
        relay.replay_stored();
        last_sync = run_iris(data_dir, &["sync", "--wait-ms", "1500"]);
        last_read = run_iris(data_dir, &["read", chat_id, "--limit", "20"]);
        if let Some(messages) = last_read["data"]["messages"].as_array() {
            if let Some(message) = messages
                .iter()
                .find(|message| message["body"] == expected_body)
            {
                return message.clone();
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!(
        "timed out waiting for {expected_body}; last_sync={last_sync}; last_read={last_read}; relay_events={}",
        relay_event_summary(relay)
    );
}

fn wait_for_decrypted_message(relay: &TestRelay, session: &mut Session, expected: &str) -> Value {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        for event in relay_events(relay) {
            if event.get("kind").and_then(Value::as_u64) != Some(MESSAGE_EVENT_KIND as u64) {
                continue;
            }
            let event: Event = serde_json::from_value(event).expect("message event json");
            let Ok(envelope) = parse_message_event(&event) else {
                continue;
            };
            if !session.matches_sender(envelope.sender) {
                continue;
            }
            let mut rng = rand::rngs::OsRng;
            let mut ctx = ProtocolContext::new(UnixSeconds(event.created_at.as_secs()), &mut rng);
            let Ok(plan) = session.plan_receive(&mut ctx, &envelope) else {
                continue;
            };
            let plaintext =
                String::from_utf8(session.apply_receive(plan).payload).expect("decrypted utf8");
            let rumor: Value = serde_json::from_str(&plaintext).expect("inner event json");
            if rumor.get("content").and_then(Value::as_str) == Some(expected) {
                return rumor;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for decrypted message {expected}");
}

#[test]
fn device_linking_e2e_installs_local_sibling_session() {
    let relay = TestRelay::start();
    let owner_dir = TempDir::new().unwrap();
    let linked_dir = TempDir::new().unwrap();
    let owner = new_app(owner_dir.path());
    let linked = new_app(linked_dir.path());

    owner.dispatch(AppAction::SetNostrRelays {
        relay_urls: vec![relay.url().to_string()],
    });
    let owner_state = dispatch_and_wait_state(
        &owner,
        AppAction::CreateAccount {
            name: "Alice".to_string(),
        },
        "owner account",
        Duration::from_secs(5),
        |state| state.account.is_some(),
    );
    let owner_id = owner_state
        .account
        .as_ref()
        .expect("owner account")
        .public_key_hex
        .clone();
    wait_for_app_state(
        &owner,
        "owner relay startup",
        Duration::from_secs(30),
        |state| {
            state.network_status.as_ref().is_some_and(|network| {
                network.connected_relay_count > 0
                    && !network.syncing
                    && network.pending_outbound_count == 0
            })
        },
    );

    linked.dispatch(AppAction::SetNostrRelays {
        relay_urls: vec![relay.url().to_string()],
    });
    let linked_state = dispatch_and_wait_state(
        &linked,
        AppAction::StartLinkedDevice {
            owner_input: String::new(),
        },
        "linked device link code",
        Duration::from_secs(5),
        |state| state.link_device.is_some(),
    );
    let link = linked_state
        .link_device
        .as_ref()
        .expect("link snapshot")
        .clone();

    dispatch_and_wait_state(
        &owner,
        AppAction::AddAuthorizedDevice {
            device_input: link.url.clone(),
        },
        "owner roster includes linked device",
        Duration::from_secs(10),
        |state| {
            state.device_roster.as_ref().is_some_and(|roster| {
                roster
                    .devices
                    .iter()
                    .any(|device| device.device_npub == link.device_input)
            })
        },
    );

    let linked_authorized = wait_for_app_state(
        &linked,
        "linked device authorization",
        Duration::from_secs(10),
        |state| {
            state.account.as_ref().is_some_and(|account| {
                account.public_key_hex == owner_id
                    && account.authorization_state == DeviceAuthorizationState::Authorized
            })
        },
    );
    assert!(
        linked_authorized.link_device.is_none(),
        "fully linked device should not keep showing link UI"
    );

    owner.shutdown();
    linked.shutdown();
}

#[test]
fn cli_account_create_publishes_app_keys_snapshot() {
    let relay = TestRelay::start();
    let data_dir = TempDir::new().unwrap();

    run_iris(data_dir.path(), &["relay", "set", relay.url()]);
    let account = run_iris(data_dir.path(), &["account", "create", "--name", "Alice"]);
    let owner_id = account
        .pointer("/data/user_id")
        .and_then(Value::as_str)
        .expect("account create user_id");
    run_iris(data_dir.path(), &["relay", "set", relay.url()]);

    let app_keys_events = wait_for_relay_events(&relay, APP_KEYS_EVENT_KIND as u64, 1);
    assert!(
        app_keys_events
            .iter()
            .any(|event| event.pubkey.to_hex() == owner_id),
        "missing owner-signed app-keys snapshot for {owner_id}; saw {}",
        relay_event_summary(&relay)
    );
}

#[test]
fn iris_cli_sends_to_protocol_client() {
    let relay = TestRelay::start();
    let iris_dir = TempDir::new().unwrap();
    let alice_keys = Keys::generate();
    let alice_secret = alice_keys.secret_key().to_secret_bytes();
    let mut invite = Invite::create_new(alice_keys.public_key(), Some("interop".to_string()), None)
        .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());
    let invite_url = invite_url(&invite, "https://chat.iris.to/").expect("invite url");

    run_iris(iris_dir.path(), &["relay", "set", relay.url()]);
    let iris_account = run_iris(iris_dir.path(), &["account", "create", "--name", "Iris"]);
    run_iris(iris_dir.path(), &["relay", "set", relay.url()]);
    let accepted = run_iris(iris_dir.path(), &["invite", "accept", &invite_url]);
    let chat_id = accepted["data"]["current_chat"]["chat_id"]
        .as_str()
        .expect("chat id");

    let sent = run_iris(iris_dir.path(), &["send", chat_id, "hello from iris cli"]);
    assert_eq!(sent["data"]["body"], "hello from iris cli");
    // Send is fire-and-forget; delivery flips to "sent" asynchronously
    // after the background publish task acks the relay. We verify
    // eventual delivery by waiting for the relay to receive the event
    // and decrypting it below.

    let response_event = wait_for_relay_event(&relay, INVITE_RESPONSE_KIND as u64);
    let response = process_invite_response_event(&invite, &response_event, alice_secret)
        .expect("process invite response")
        .expect("invite response");
    assert_eq!(
        response.resolved_owner_pubkey().to_hex(),
        iris_account["data"]["user_id"].as_str().unwrap()
    );
    let mut protocol_session = response.session;
    wait_for_decrypted_message(&relay, &mut protocol_session, "hello from iris cli");
}

#[test]
fn iris_listen_receives_from_another_iris_client() {
    let relay = TestRelay::start();
    let bob = TempDir::new().unwrap();
    let alice = TempDir::new().unwrap();

    run_iris(bob.path(), &["relay", "set", relay.url()]);
    run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let invite_created = run_iris(bob.path(), &["invite", "create"]);
    let invite_url = invite_created["data"]["url"].as_str().expect("invite url");

    let mut child = start_iris(bob.path(), &["listen", "--interval-ms", "100"]);
    let stdout = child.stdout.take().expect("stdout");
    let stderr = spawn_stderr_reader(child.stderr.take().expect("stderr"));
    let receiver = spawn_json_reader(stdout);
    let ready = receiver
        .recv_timeout(Duration::from_secs(25))
        .unwrap_or_else(|error| {
            let status = child.try_wait().expect("child status");
            let stderr = stderr.lock().map(|text| text.clone()).unwrap_or_default();
            panic!("timed out waiting for iris listen ready: {error}; status={status:?}; stderr={stderr}");
        });
    assert_eq!(ready["command"], "listen");
    assert_eq!(ready["data"]["ready"], true);
    assert_eq!(ready["data"]["network"], true);
    assert_eq!(ready["data"]["chat"], Value::Null);

    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_account = run_iris(alice.path(), &["account", "create", "--name", "Alice"]);
    let alice_user_id = alice_account["data"]["user_id"].as_str().unwrap();
    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let accepted = run_iris(alice.path(), &["invite", "accept", invite_url]);
    let alice_chat_id = accepted["data"]["current_chat"]["chat_id"]
        .as_str()
        .expect("alice chat id");
    let sent = run_iris(
        alice.path(),
        &["send", alice_chat_id, "hello from alice cli"],
    );
    assert_eq!(sent["data"]["body"], "hello from alice cli");

    let message = match read_stream_message(&relay, &receiver, "hello from alice cli") {
        Some(message) => message,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let sync = run_iris(bob.path(), &["sync", "--wait-ms", "5000"]);
            let read = run_iris(bob.path(), &["read", alice_user_id]);
            let relay_kinds = relay_events(&relay)
                .into_iter()
                .filter_map(|event| event.get("kind").and_then(Value::as_u64))
                .collect::<Vec<_>>();
            panic!(
                "timed out waiting for streamed message; sync={sync}; read={read}; relay_kinds={relay_kinds:?}"
            );
        }
    };
    assert_eq!(message["data"]["chat_id"], alice_user_id);
    assert_eq!(message["data"]["is_outgoing"], false);

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn iris_listen_receives_first_contact_sent_to_user_id() {
    let relay = TestRelay::start();
    let bob = TempDir::new().unwrap();
    let alice = TempDir::new().unwrap();

    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();

    let mut child = start_iris(bob.path(), &["listen", "--interval-ms", "100"]);
    let stdout = child.stdout.take().expect("stdout");
    let stderr = spawn_stderr_reader(child.stderr.take().expect("stderr"));
    let receiver = spawn_json_reader(stdout);
    wait_for_listener_ready(&mut child, &receiver, &stderr);

    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_account = run_iris(alice.path(), &["account", "create", "--name", "Alice"]);
    let alice_user_id = alice_account["data"]["user_id"].as_str().unwrap();
    run_iris(alice.path(), &["relay", "set", relay.url()]);

    let body = "first contact by user id";
    let sent = run_iris(alice.path(), &["send", bob_user_id, body]);
    assert_eq!(sent["data"]["chat_id"], bob_user_id);
    assert_eq!(sent["data"]["is_outgoing"], true);

    let message = match read_stream_message(&relay, &receiver, body) {
        Some(message) => message,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let bob_sync = run_iris(bob.path(), &["sync", "--wait-ms", "5000"]);
            let bob_read = run_iris(bob.path(), &["read", alice_user_id]);
            let bob_debug = debug_snapshot(bob.path());
            let relay_events = relay_event_summary(&relay);
            panic!(
                "bob did not receive first-contact send; sent={}; sync={bob_sync}; read={bob_read}; debug={bob_debug}; relay_events={relay_events}",
                sent["data"]
            );
        }
    };
    assert_eq!(message["data"]["chat_id"], alice_user_id);
    assert_eq!(message["data"]["is_outgoing"], false);

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn short_lived_cli_processes_exchange_direct_messages_by_user_id() {
    let relay = TestRelay::start();
    let alice = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();

    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_account = run_iris(alice.path(), &["account", "create", "--name", "Alice"]);
    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_user_id = alice_account["data"]["user_id"].as_str().unwrap();

    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();

    run_iris(alice.path(), &["chat", "create", bob_user_id]);
    run_iris(bob.path(), &["chat", "create", alice_user_id]);

    let alice_body = "short lived alice to bob";
    let alice_sent = run_iris(alice.path(), &["send", bob_user_id, alice_body]);
    assert_eq!(alice_sent["data"]["body"], alice_body);
    let bob_received = sync_until_message(&relay, bob.path(), alice_user_id, alice_body);
    assert_eq!(bob_received["is_outgoing"], false);

    let bob_body = "short lived bob to alice";
    let bob_sent = run_iris(bob.path(), &["send", alice_user_id, bob_body]);
    assert_eq!(bob_sent["data"]["body"], bob_body);
    let alice_received = sync_until_message(&relay, alice.path(), bob_user_id, bob_body);
    assert_eq!(alice_received["is_outgoing"], false);
}

#[test]
fn restored_same_nsec_cli_send_reaches_peer_and_self_syncs_to_existing_session() {
    let relay = TestRelay::start();
    let alice_old = TempDir::new().unwrap();
    let alice_fresh = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();

    run_iris(alice_old.path(), &["relay", "set", relay.url()]);
    let alice_account = run_iris(alice_old.path(), &["account", "create", "--name", "Alice"]);
    run_iris(alice_old.path(), &["relay", "set", relay.url()]);
    let alice_user_id = alice_account["data"]["user_id"].as_str().unwrap();
    let alice_nsec = read_owner_nsec(alice_old.path());

    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();
    let bob_npub = bob_account["data"]["npub"].as_str().unwrap();
    let bob_invite = run_iris(bob.path(), &["invite", "create"]);
    let bob_invite_url = bob_invite["data"]["url"].as_str().expect("bob invite url");

    run_iris(alice_old.path(), &["invite", "accept", bob_invite_url]);
    run_iris(
        alice_old.path(),
        &["send", bob_user_id, "initial old alice"],
    );
    let alice_read = run_iris(alice_old.path(), &["read", bob_user_id]);
    assert!(alice_read["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| {
            message["body"] == "initial old alice" && message["is_outgoing"] == true
        }));

    let mut alice_child = start_iris(alice_old.path(), &["listen", "--interval-ms", "100"]);
    let alice_stdout = alice_child.stdout.take().expect("alice stdout");
    let alice_stderr = spawn_stderr_reader(alice_child.stderr.take().expect("alice stderr"));
    let alice_receiver = spawn_json_reader(alice_stdout);
    wait_for_listener_ready(&mut alice_child, &alice_receiver, &alice_stderr);

    let mut bob_child = start_iris(bob.path(), &["listen", "--interval-ms", "100"]);
    let bob_stdout = bob_child.stdout.take().expect("bob stdout");
    let bob_stderr = spawn_stderr_reader(bob_child.stderr.take().expect("bob stderr"));
    let bob_receiver = spawn_json_reader(bob_stdout);
    wait_for_listener_ready(&mut bob_child, &bob_receiver, &bob_stderr);

    run_iris(alice_fresh.path(), &["relay", "set", relay.url()]);
    let fresh_account = run_iris(alice_fresh.path(), &["restore", &alice_nsec]);
    run_iris(alice_fresh.path(), &["relay", "set", relay.url()]);
    assert_eq!(fresh_account["data"]["user_id"], alice_user_id);
    assert_ne!(
        fresh_account["data"]["device_id"], alice_account["data"]["device_id"],
        "fresh restore should create a new local device for the same owner"
    );
    run_iris(alice_fresh.path(), &["sync", "--wait-ms", "5000"]);

    let body = "fresh restored same nsec send";
    let sent = run_iris(alice_fresh.path(), &["send", bob_npub, body]);
    assert_eq!(sent["data"]["chat_id"], bob_user_id);
    assert_eq!(sent["data"]["is_outgoing"], true);
    // Fire-and-forget send; the relay-arrival assertions below verify
    // the publish task actually flushed the event.

    let bob_message = match read_stream_message(&relay, &bob_receiver, body) {
        Some(message) => message,
        None => {
            let relay_kinds = relay_events(&relay)
                .into_iter()
                .filter_map(|event| event.get("kind").and_then(Value::as_u64))
                .collect::<Vec<_>>();
            let _ = alice_child.kill();
            let _ = alice_child.wait();
            let _ = bob_child.kill();
            let _ = bob_child.wait();
            let bob_sync = run_iris(bob.path(), &["sync", "--wait-ms", "2000"]);
            let bob_read = run_iris(bob.path(), &["read", alice_user_id]);
            let fresh_debug = debug_snapshot(alice_fresh.path());
            let bob_debug = debug_snapshot(bob.path());
            let relay_events = relay_event_summary(&relay);
            panic!(
                "bob did not receive fresh restored send; trace={}; fresh_debug={fresh_debug}; bob_sync={bob_sync}; bob_read={bob_read}; bob_debug={bob_debug}; relay_events={relay_events}; relay_kinds={relay_kinds:?}",
                sent["data"]["delivery_trace"],
            );
        }
    };
    assert_eq!(bob_message["data"]["chat_id"], alice_user_id);
    assert_eq!(bob_message["data"]["is_outgoing"], false);

    let old_alice_message = match read_stream_message(&relay, &alice_receiver, body) {
        Some(message) => message,
        None => {
            let relay_kinds = relay_events(&relay)
                .into_iter()
                .filter_map(|event| event.get("kind").and_then(Value::as_u64))
                .collect::<Vec<_>>();
            let _ = alice_child.kill();
            let _ = alice_child.wait();
            let _ = bob_child.kill();
            let _ = bob_child.wait();
            let alice_sync = run_iris(alice_old.path(), &["sync", "--wait-ms", "2000"]);
            let alice_read = run_iris(alice_old.path(), &["read", bob_user_id]);
            let fresh_read = run_iris(alice_fresh.path(), &["read", bob_user_id]);
            let fresh_debug = debug_snapshot(alice_fresh.path());
            let alice_debug = debug_snapshot(alice_old.path());
            panic!(
                "old alice did not receive sender copy; trace={}; alice_sync={alice_sync}; alice_read={alice_read}; fresh_read={fresh_read}; fresh_debug={fresh_debug}; alice_debug={alice_debug}; relay_kinds={relay_kinds:?}",
                sent["data"]["delivery_trace"],
            );
        }
    };
    assert_eq!(old_alice_message["data"]["chat_id"], bob_user_id);
    assert_eq!(old_alice_message["data"]["is_outgoing"], true);

    let _ = alice_child.kill();
    let _ = alice_child.wait();
    let _ = bob_child.kill();
    let _ = bob_child.wait();
}

#[test]
fn sender_key_cli_group_interop_three_members_restart_and_restored_owner_device() {
    let relay = TestRelay::start();
    let alice = TempDir::new().unwrap();
    let alice_linked = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    let charlie = TempDir::new().unwrap();

    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_account = run_iris(alice.path(), &["account", "create", "--name", "Alice"]);
    run_iris(alice.path(), &["relay", "set", relay.url()]);
    let alice_nsec = read_owner_nsec(alice.path());

    run_iris(alice_linked.path(), &["relay", "set", relay.url()]);
    let linked_account = run_iris(alice_linked.path(), &["restore", &alice_nsec]);
    run_iris(alice_linked.path(), &["relay", "set", relay.url()]);
    assert_eq!(
        linked_account["data"]["user_id"],
        alice_account["data"]["user_id"]
    );
    assert_ne!(
        linked_account["data"]["device_id"],
        alice_account["data"]["device_id"]
    );
    run_iris(alice.path(), &["sync", "--wait-ms", "12000"]);
    run_iris(alice_linked.path(), &["sync", "--wait-ms", "12000"]);

    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    run_iris(bob.path(), &["relay", "set", relay.url()]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();
    let bob_invite = run_iris(bob.path(), &["invite", "create"]);

    run_iris(charlie.path(), &["relay", "set", relay.url()]);
    let charlie_account = run_iris(charlie.path(), &["account", "create", "--name", "Charlie"]);
    run_iris(charlie.path(), &["relay", "set", relay.url()]);
    let charlie_user_id = charlie_account["data"]["user_id"].as_str().unwrap();
    let charlie_invite = run_iris(charlie.path(), &["invite", "create"]);

    run_iris(
        alice_linked.path(),
        &[
            "invite",
            "accept",
            bob_invite["data"]["url"].as_str().expect("bob invite url"),
        ],
    );
    run_iris(
        alice_linked.path(),
        &[
            "invite",
            "accept",
            charlie_invite["data"]["url"]
                .as_str()
                .expect("charlie invite url"),
        ],
    );

    let group = run_iris(
        alice_linked.path(),
        &[
            "group",
            "create",
            "SenderKey CLI",
            bob_user_id,
            charlie_user_id,
        ],
    );
    let group_id = group["data"]["current_chat"]["group_id"]
        .as_str()
        .expect("group id");
    let mut bob_group_sync = run_iris(bob.path(), &["sync", "--wait-ms", "15000"]);
    for _ in 0..3 {
        if state_contains_group(&bob_group_sync, group_id) {
            break;
        }
        bob_group_sync = run_iris(bob.path(), &["sync", "--wait-ms", "15000"]);
    }
    if !state_contains_group(&bob_group_sync, group_id) {
        let bob_live_debug = run_iris_capture(bob.path(), &["debug", "--wait-ms", "30000"]);
        panic!(
            "bob did not learn cli-created group after sync; sync={bob_group_sync}; bob_debug={}; bob_live_debug={bob_live_debug}; relay_events={}",
            debug_snapshot(bob.path()),
            relay_event_summary(&relay),
        );
    }
    run_iris(charlie.path(), &["sync", "--wait-ms", "15000"]);
    run_iris(alice.path(), &["sync", "--wait-ms", "15000"]);

    let mut bob_child = start_iris(bob.path(), &["listen", "--interval-ms", "100"]);
    let bob_stdout = bob_child.stdout.take().expect("bob stdout");
    let bob_stderr = spawn_stderr_reader(bob_child.stderr.take().expect("bob stderr"));
    let bob_receiver = spawn_json_reader(bob_stdout);
    wait_for_listener_ready(&mut bob_child, &bob_receiver, &bob_stderr);

    let mut charlie_child = start_iris(charlie.path(), &["listen", "--interval-ms", "100"]);
    let charlie_stdout = charlie_child.stdout.take().expect("charlie stdout");
    let charlie_stderr = spawn_stderr_reader(charlie_child.stderr.take().expect("charlie stderr"));
    let charlie_receiver = spawn_json_reader(charlie_stdout);
    wait_for_listener_ready(&mut charlie_child, &charlie_receiver, &charlie_stderr);

    let mut alice_child = start_iris(alice.path(), &["listen", "--interval-ms", "100"]);
    let alice_stdout = alice_child.stdout.take().expect("alice stdout");
    let alice_stderr = spawn_stderr_reader(alice_child.stderr.take().expect("alice stderr"));
    let alice_receiver = spawn_json_reader(alice_stdout);
    wait_for_listener_ready(&mut alice_child, &alice_receiver, &alice_stderr);

    let alice_body = "linked alice sender-key cli group";
    let sent = run_iris(
        alice_linked.path(),
        &["group", "send", group_id, alice_body],
    );
    assert_eq!(sent["data"]["body"], alice_body);
    run_iris(alice_linked.path(), &["sync", "--wait-ms", "15000"]);

    let bob_message = read_stream_message(&relay, &bob_receiver, alice_body);
    let charlie_message = read_stream_message(&relay, &charlie_receiver, alice_body);
    let alice_primary_message = read_stream_message(&relay, &alice_receiver, alice_body);
    if bob_message.is_none() || charlie_message.is_none() || alice_primary_message.is_none() {
        let _ = alice_child.kill();
        let _ = alice_child.wait();
        let _ = bob_child.kill();
        let _ = bob_child.wait();
        let _ = charlie_child.kill();
        let _ = charlie_child.wait();
        let bob_read = run_iris_capture(bob.path(), &["read", group_id]);
        let charlie_read = run_iris_capture(charlie.path(), &["read", group_id]);
        panic!(
            "three-member sender-key group did not converge; bob_message={bob_message:?}; charlie_message={charlie_message:?}; alice_primary_message={alice_primary_message:?}; bob_read={bob_read}; charlie_read={charlie_read}; linked_debug={}; bob_debug={}; charlie_debug={}; alice_debug={}; relay_events={}",
            debug_snapshot(alice_linked.path()),
            debug_snapshot(bob.path()),
            debug_snapshot(charlie.path()),
            debug_snapshot(alice.path()),
            relay_event_summary(&relay),
        );
    }

    let _ = bob_child.kill();
    let _ = bob_child.wait();
    let mut bob_child = start_iris(bob.path(), &["listen", "--interval-ms", "100"]);
    let bob_stdout = bob_child.stdout.take().expect("bob stdout after restart");
    let bob_stderr =
        spawn_stderr_reader(bob_child.stderr.take().expect("bob stderr after restart"));
    let bob_receiver = spawn_json_reader(bob_stdout);
    wait_for_listener_ready(&mut bob_child, &bob_receiver, &bob_stderr);

    let _ = charlie_child.kill();
    let _ = charlie_child.wait();
    let charlie_body = "charlie sender-key cli group after restart";
    let charlie_sent = run_iris(charlie.path(), &["group", "send", group_id, charlie_body]);
    assert_eq!(charlie_sent["data"]["body"], charlie_body);
    run_iris(charlie.path(), &["sync", "--wait-ms", "15000"]);
    if read_stream_message(&relay, &bob_receiver, charlie_body).is_none() {
        let _ = bob_child.kill();
        let _ = bob_child.wait();
        panic!(
            "restarted bob did not receive charlie group send; bob_debug={}; charlie_debug={}; relay_events={}",
            debug_snapshot(bob.path()),
            debug_snapshot(charlie.path()),
            relay_event_summary(&relay),
        );
    }

    let _ = alice_child.kill();
    let _ = alice_child.wait();
    let _ = bob_child.kill();
    let _ = bob_child.wait();
}
