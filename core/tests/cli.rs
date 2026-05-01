use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{io::BufRead, io::BufReader, process::Stdio};

use iris_chat_core::FfiApp;
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

fn run_iris_error(data_dir: &Path, args: &[&str]) -> Value {
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
        !output.status.success(),
        "iris unexpectedly succeeded\nstdout={}\nstderr={}",
        stdout,
        stderr
    );
    serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!("invalid error json: {error}\nstdout={stdout}\nstderr={stderr}")
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

fn read_json_line(reader: &mut BufReader<std::process::ChildStdout>) -> Value {
    let started = Instant::now();
    let mut line = String::new();
    while started.elapsed() < Duration::from_secs(5) {
        line.clear();
        if reader.read_line(&mut line).expect("read iris stdout") > 0 {
            return serde_json::from_str(line.trim())
                .unwrap_or_else(|error| panic!("invalid json line: {error}\nline={line}"));
        }
    }
    panic!("timed out waiting for iris stdout");
}

#[test]
fn account_create_persists_and_restores_for_next_process() {
    let dir = TempDir::new().unwrap();

    let created = run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    assert_eq!(created["status"], "ok");
    assert_eq!(created["data"]["name"], "Alice");
    assert!(created["data"]["user_id"].as_str().unwrap().len() >= 64);

    let whoami = run_iris(dir.path(), &["whoami"]);
    assert_eq!(whoami["data"]["user_id"], created["data"]["user_id"]);
    assert_eq!(whoami["data"]["device_state"], "authorized");

    let synced = run_iris(dir.path(), &["sync", "--wait-ms", "100"]);
    assert_eq!(
        synced["data"]["account"]["user_id"],
        created["data"]["user_id"]
    );

    let bundle = run_iris(dir.path(), &["account", "bundle"]);
    assert_eq!(bundle["data"]["has_owner_secret"], true);
    assert_eq!(bundle["data"]["has_device_secret"], true);
}

#[test]
fn direct_chat_send_read_search_and_tail_work_offline() {
    let alice = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();

    let alice_account = run_iris(alice.path(), &["account", "create", "--name", "Alice"]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    let bob_npub = bob_account["data"]["npub"].as_str().unwrap();

    run_iris(alice.path(), &["relay", "set"]);
    let sent = run_iris(alice.path(), &["send", bob_npub, "queued offline"]);
    assert_eq!(sent["data"]["body"], "queued offline");
    assert_eq!(sent["data"]["delivery"], "queued");
    let chat_id = sent["data"]["chat_id"].as_str().unwrap();
    let message_id = sent["data"]["id"].as_str().unwrap();

    let reacted = run_iris(alice.path(), &["react", chat_id, message_id, "+1"]);
    assert_eq!(reacted["data"]["reactions"][0]["emoji"], "+1");
    assert_eq!(reacted["data"]["reactions"][0]["reacted_by_me"], true);

    let expiring = run_iris(
        alice.path(),
        &["send", chat_id, "short lived", "--ttl", "60"],
    );
    assert_eq!(expiring["data"]["body"], "short lived");
    assert!(expiring["data"]["expires_at_secs"].as_u64().unwrap() > 0);

    let typing = run_iris(alice.path(), &["typing", chat_id]);
    assert_eq!(typing["data"]["typing"], true);

    let read = run_iris(alice.path(), &["read", chat_id]);
    assert_eq!(read["data"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(read["data"]["messages"][0]["body"], "queued offline");

    let found = run_iris(alice.path(), &["search", "offline"]);
    assert_eq!(found["data"]["messages"][0]["body"], "queued offline");

    let tail = run_iris(alice.path(), &["tail", "--limit", "1"]);
    assert_eq!(tail["data"]["messages"][0]["body"], "short lived");

    let list = run_iris(alice.path(), &["chat", "list"]);
    assert_eq!(list["data"]["chats"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"]["chats"][0]["last_message"], "short lived");

    assert_ne!(
        alice_account["data"]["user_id"],
        bob_account["data"]["user_id"]
    );
}

#[test]
fn invite_create_group_create_relays_and_logout_are_scriptable() {
    let dir = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();
    run_iris(dir.path(), &["relay", "set"]);

    let invite = run_iris(dir.path(), &["invite", "create"]);
    assert!(invite["data"]["url"].as_str().unwrap().contains("iris"));

    let group = run_iris(dir.path(), &["group", "create", "Notes", bob_user_id]);
    let chat_id = group["data"]["current_chat"]["chat_id"].as_str().unwrap();
    assert!(chat_id.starts_with("group:"));

    let group_id = group["data"]["current_chat"]["group_id"].as_str().unwrap();
    let sent = run_iris(dir.path(), &["group", "send", group_id, "group note"]);
    assert_eq!(sent["data"]["body"], "group note");
    let message_id = sent["data"]["id"].as_str().unwrap();

    let reacted = run_iris(dir.path(), &["group", "react", group_id, message_id, "+1"]);
    assert_eq!(reacted["data"]["reactions"][0]["emoji"], "+1");

    let read = run_iris(dir.path(), &["group", "read", group_id]);
    assert_eq!(read["data"]["messages"][0]["body"], "group note");

    let renamed = run_iris(dir.path(), &["group", "rename", group_id, "Renamed"]);
    assert_eq!(renamed["data"]["name"], "Renamed");

    let admin = run_iris(dir.path(), &["group", "add-admin", group_id, bob_user_id]);
    assert!(admin["data"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|member| member["user_id"] == bob_user_id && member["admin"] == true));

    let member = run_iris(
        dir.path(),
        &["group", "remove-admin", group_id, bob_user_id],
    );
    assert!(member["data"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|member| member["user_id"] == bob_user_id && member["admin"] == false));

    let removed = run_iris(dir.path(), &["group", "remove", group_id, bob_user_id]);
    assert!(!removed["data"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|member| member["user_id"] == bob_user_id));

    let relays = run_iris(
        dir.path(),
        &[
            "relay",
            "set",
            "wss://relay-one.example",
            "wss://relay-two.example",
        ],
    );
    assert_eq!(
        relays["data"]["message_servers"].as_array().unwrap().len(),
        2
    );

    let deleted = run_iris(dir.path(), &["group", "delete", group_id]);
    assert_eq!(deleted["data"]["deleted"], true);

    let logout = run_iris(dir.path(), &["logout"]);
    assert_eq!(logout["data"]["logged_out"], true);
}

#[test]
fn listen_streams_new_sqlite_messages_for_agents() {
    let dir = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    run_iris(dir.path(), &["relay", "set"]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();
    let chat = run_iris(dir.path(), &["chat", "create", bob_user_id]);
    let chat_id = chat["data"]["chat"]["chat_id"].as_str().unwrap();

    let mut child = start_iris(dir.path(), &["listen", "--interval-ms", "100"]);
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let ready = read_json_line(&mut reader);
    assert_eq!(ready["command"], "listen");
    assert_eq!(ready["data"]["ready"], true);

    run_iris(dir.path(), &["send", chat_id, "from another process"]);
    let message = read_json_line(&mut reader);
    assert_eq!(message["command"], "message");
    assert_eq!(message["data"]["body"], "from another process");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn second_core_process_fails_while_data_dir_is_locked() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );

    let error = run_iris_error(dir.path(), &["whoami"]);
    assert_eq!(error["status"], "error");
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("already using this data folder"),
        "unexpected error: {error}"
    );

    app.shutdown();
    let after = run_iris_error(dir.path(), &["whoami"]);
    assert_eq!(after["error"], "Create or restore a profile first.");
}

#[test]
fn link_create_outputs_device_invite() {
    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["relay", "set"]);

    let link = run_iris(dir.path(), &["link", "create"]);
    assert!(link["data"]["url"].as_str().unwrap().contains("iris"));
    assert!(link["data"]["device_input"]
        .as_str()
        .unwrap()
        .starts_with("npub"));
}
