use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::{io::BufRead, io::BufReader};

use iris_chat_core::local_relay::TestRelay;
use nostr::{Event, Keys};
use nostr_double_ratchet::{Invite, INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND};
use serde_json::Value;
use tempfile::TempDir;

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
        "iris failed status={}\nstdout={}\nstderr={}",
        output.status,
        stdout,
        stderr
    );
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("invalid json: {error}\nstdout={stdout}\nstderr={stderr}"))
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

fn wait_for_relay_event(relay: &TestRelay, kind: u64) -> Event {
    let started = Instant::now();
    let mut last_events = Vec::new();
    while started.elapsed() < Duration::from_secs(10) {
        last_events = relay.events();
        for event in &last_events {
            if event.get("kind").and_then(Value::as_u64) == Some(kind) {
                return serde_json::from_value(event.clone()).expect("relay event json");
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let kinds = last_events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_u64))
        .collect::<Vec<_>>();
    panic!("timed out waiting for relay event kind {kind}; saw kinds {kinds:?}");
}

fn read_stream_message(
    relay: &TestRelay,
    receiver: &mpsc::Receiver<Value>,
    expected_body: &str,
) -> Option<Value> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
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

fn wait_for_decrypted_message(
    relay: &TestRelay,
    session: &mut nostr_double_ratchet::Session,
    expected: &str,
) -> Value {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        for event in relay.events() {
            if event.get("kind").and_then(Value::as_u64) != Some(MESSAGE_EVENT_KIND as u64) {
                continue;
            }
            let event: Event = serde_json::from_value(event).expect("message event json");
            let Ok(Some(plaintext)) = session.receive(&event) else {
                continue;
            };
            let rumor: Value = serde_json::from_str(&plaintext).expect("inner event json");
            if rumor.get("content").and_then(Value::as_str) == Some(expected) {
                return rumor;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for decrypted message {expected}");
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
    let invite_url = invite.get_url("https://chat.iris.to/").expect("invite url");

    run_iris(iris_dir.path(), &["relay", "set", relay.url()]);
    let iris_account = run_iris(iris_dir.path(), &["account", "create", "--name", "Iris"]);
    run_iris(iris_dir.path(), &["relay", "set", relay.url()]);
    let accepted = run_iris(iris_dir.path(), &["invite", "accept", &invite_url]);
    let chat_id = accepted["data"]["current_chat"]["chat_id"]
        .as_str()
        .expect("chat id");

    let response_event = wait_for_relay_event(&relay, INVITE_RESPONSE_KIND as u64);
    let response = invite
        .process_invite_response(&response_event, alice_secret)
        .expect("process invite response")
        .expect("invite response");
    assert_eq!(
        response.resolved_owner_pubkey().to_hex(),
        iris_account["data"]["user_id"].as_str().unwrap()
    );
    let mut protocol_session = response.session;

    let sent = run_iris(iris_dir.path(), &["send", chat_id, "hello from iris cli"]);
    assert_eq!(sent["data"]["body"], "hello from iris cli");
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
        .recv_timeout(Duration::from_secs(10))
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
            let relay_kinds = relay
                .events()
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
