use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

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
