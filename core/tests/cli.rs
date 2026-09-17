use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{io::BufRead, io::BufReader, process::Stdio};

use iris_chat_core::FfiApp;
use serde_json::Value;
use tempfile::TempDir;

fn run_iris(data_dir: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
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
        "iris failed args={args:?} status={}\nstdout={}\nstderr={}",
        output.status,
        stdout,
        stderr
    );
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("invalid json: {error}\nstdout={stdout}\nstderr={stderr}"))
}

fn run_iris_error(data_dir: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
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
    Command::new(env!("CARGO_BIN_EXE_iris"))
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
fn help_groups_top_level_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .arg("--help")
        .output()
        .expect("run iris help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "iris help failed status={}\nstdout={}\nstderr={}",
        output.status,
        stdout,
        stderr
    );

    for heading in [
        "Account:",
        "Messages:",
        "Groups:",
        "Invites and Devices:",
        "Message Servers:",
        "Maintenance:",
    ] {
        assert!(
            stdout.contains(heading),
            "missing heading {heading}\nstdout={stdout}"
        );
    }
    assert!(
        stdout.find("Account:").unwrap() < stdout.find("Messages:").unwrap(),
        "help headings are out of order\nstdout={stdout}"
    );
}

#[test]
fn whoami_does_not_emit_perf_logs_by_default() {
    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);

    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .env_remove("IRIS_PERF_LOG")
        .arg("--data-dir")
        .arg(dir.path())
        .arg("whoami")
        .output()
        .expect("run iris whoami");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "iris whoami failed status={}\nstdout={}\nstderr={}",
        output.status,
        stdout,
        stderr
    );
    assert!(stdout.contains("\"user_id\""), "unexpected stdout={stdout}");
    assert!(
        !stdout.contains("IrisPerf") && !stderr.contains("IrisPerf"),
        "perf logs leaked into cli output\nstdout={stdout}\nstderr={stderr}"
    );
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

    run_iris(dir.path(), &["relay", "set"]);
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
fn account_storage_rejects_other_keys_without_changing_history_or_credentials() {
    use nostr::{Keys, ToBech32};

    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["relay", "set"]);
    let alice = run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    let peer = Keys::generate().public_key().to_hex();
    run_iris(dir.path(), &["send", &peer, "Alice private history"]);
    let bundle_path = dir.path().join("cli-account.json");
    let original_bundle = std::fs::read(&bundle_path).unwrap();
    let other_secret = Keys::generate().secret_key().to_bech32().unwrap();

    for args in [
        vec!["restore", other_secret.as_str()],
        vec!["account", "create", "--name", "Bot"],
    ] {
        let error = run_iris_error(dir.path(), &args);
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("different account"));
        assert_eq!(std::fs::read(&bundle_path).unwrap(), original_bundle);
        let whoami = run_iris(dir.path(), &["whoami"]);
        assert_eq!(whoami["data"]["user_id"], alice["data"]["user_id"]);
        let read = run_iris(dir.path(), &["read", &peer]);
        assert_eq!(read["data"]["messages"][0]["body"], "Alice private history");
    }

    // A fresh device for the same account must still retain its history.
    let bundle: Value = serde_json::from_slice(&original_bundle).unwrap();
    let restored = run_iris(
        dir.path(),
        &["restore", bundle["owner_nsec"].as_str().unwrap()],
    );
    assert_eq!(restored["data"]["user_id"], alice["data"]["user_id"]);
    let read = run_iris(dir.path(), &["read", &peer]);
    assert_eq!(read["data"]["messages"][0]["body"], "Alice private history");
}

#[test]
fn account_storage_checks_replaced_cli_credentials_even_for_direct_database_reads() {
    use nostr::{Keys, ToBech32};

    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["relay", "set"]);
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    let peer = Keys::generate().public_key().to_hex();
    run_iris(dir.path(), &["send", &peer, "Alice private history"]);
    let bundle_path = dir.path().join("cli-account.json");
    let original_bundle = std::fs::read(&bundle_path).unwrap();
    let bot = Keys::generate();
    let device = Keys::generate();
    std::fs::write(
        &bundle_path,
        serde_json::to_vec(&serde_json::json!({
            "owner_nsec": bot.secret_key().to_bech32().unwrap(),
            "owner_pubkey_hex": bot.public_key().to_hex(),
            "device_nsec": device.secret_key().to_bech32().unwrap(),
        }))
        .unwrap(),
    )
    .unwrap();

    for args in [
        vec!["whoami"],
        vec!["read", peer.as_str()],
        vec!["search", "private"],
        vec!["tail"],
        vec!["tail", "--follow"],
    ] {
        let error = run_iris_error(dir.path(), &args);
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("different account"));
        assert!(!error.to_string().contains("Alice private history"));
    }

    std::fs::write(&bundle_path, original_bundle).unwrap();
    let read = run_iris(dir.path(), &["read", &peer]);
    assert_eq!(read["data"]["messages"][0]["body"], "Alice private history");
}

#[test]
fn account_storage_follow_keeps_the_account_it_started_with() {
    use nostr::{Keys, ToBech32};

    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["relay", "set"]);
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    let mut child = start_iris(dir.path(), &["tail", "--follow", "--interval-ms", "1000"]);
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    assert_eq!(read_json_line(&mut reader)["data"]["ready"], true);

    // Simulate the folder being reassigned between polls. Both the new account
    // bundle and database agree, but this reader still belongs to Alice.
    let bot = Keys::generate();
    let device = Keys::generate();
    std::fs::write(
        dir.path().join("cli-account.json"),
        serde_json::to_vec(&serde_json::json!({
            "owner_nsec": bot.secret_key().to_bech32().unwrap(),
            "owner_pubkey_hex": bot.public_key().to_hex(),
            "device_nsec": device.secret_key().to_bech32().unwrap(),
        }))
        .unwrap(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("core.sqlite3")).unwrap();
    conn.execute(
        "UPDATE app_meta SET value = ?1 WHERE key = 'account_owner_pubkey_hex'",
        [bot.public_key().to_hex()],
    )
    .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status
        .expect("reader must stop on account change")
        .success());
    let error = read_json_line(&mut reader);
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("different account"));
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
    let read_bodies = read["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["body"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(read_bodies.len(), 2);
    assert!(read_bodies.contains(&"queued offline"));
    assert!(read_bodies.contains(&"short lived"));

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
    assert!(read["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["body"] == "group note"));

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
fn tail_follow_streams_new_sqlite_messages_for_agents() {
    let dir = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);
    run_iris(dir.path(), &["relay", "set"]);
    let bob_account = run_iris(bob.path(), &["account", "create", "--name", "Bob"]);
    let bob_user_id = bob_account["data"]["user_id"].as_str().unwrap();
    let chat = run_iris(dir.path(), &["chat", "create", bob_user_id]);
    let chat_id = chat["data"]["chat"]["chat_id"].as_str().unwrap();

    let mut child = start_iris(dir.path(), &["tail", "--follow", "--interval-ms", "100"]);
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let ready = read_json_line(&mut reader);
    assert_eq!(ready["command"], "tail");
    assert_eq!(ready["data"]["ready"], true);
    assert_eq!(ready["data"]["network"], false);

    run_iris(dir.path(), &["send", chat_id, "from another process"]);
    let message = read_json_line(&mut reader);
    assert_eq!(message["command"], "message");
    assert_eq!(message["data"]["body"], "from another process");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn listen_owns_the_core_lock_for_the_data_dir() {
    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["account", "create", "--name", "Alice"]);

    let mut child = start_iris(dir.path(), &["listen", "--interval-ms", "100"]);
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let ready = read_json_line(&mut reader);
    assert_eq!(ready["command"], "listen");
    assert_eq!(ready["data"]["ready"], true);
    assert_eq!(ready["data"]["network"], true);

    let error = run_iris_error(dir.path(), &["whoami"]);
    assert_eq!(error["status"], "error");
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("already using this data folder"),
        "unexpected error: {error}"
    );

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
fn link_create_outputs_compact_device_approval_bootstrap() {
    let dir = TempDir::new().unwrap();
    run_iris(dir.path(), &["relay", "set"]);

    let link = run_iris(dir.path(), &["link", "create"]);
    let url = link["data"]["url"].as_str().unwrap();
    let bootstrap = nostr_identity::parse_nostr_identity_device_approval_bootstrap(url, &[])
        .expect("parse approval bootstrap")
        .expect("approval bootstrap");
    assert!(url.starts_with("nostr-identity://device-approval/"));
    assert_eq!(
        serde_json::to_value(&bootstrap)
            .expect("bootstrap JSON")
            .as_object()
            .expect("bootstrap object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["deviceAppKeyNpub", "requestNpub", "requestSecret"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    assert_eq!(
        link["data"]["device_input"].as_str(),
        Some(bootstrap.device_app_key_npub.as_str())
    );
}

#[test]
fn account_profile_preserves_omitted_fields_across_processes() {
    let dir = TempDir::new().unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_iris"))
            .env("IRIS_DEMO_RELAYS", "")
            .arg("--json")
            .arg("--data-dir")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    run(&["account", "create", "--name", "Alice"]);
    let extras = r#"{"website":"https://example.com","custom":{"keep":true}}"#;
    {
        let db = rusqlite::Connection::open(dir.path().join("core.sqlite3")).unwrap();
        db.execute(
            "UPDATE owner_profiles SET extra_metadata_json = ?1",
            [extras],
        )
        .unwrap();
    }
    let edited = run(&[
        "account",
        "profile",
        "--picture-url",
        "https://example.com/avatar.png",
        "--about",
        "Hello",
    ]);
    assert_eq!(edited["data"]["network_publication"], "not_verified");
    run(&["account", "profile", "--name", "Alicia"]);
    let inspected = run(&["account", "profile"]);
    assert_eq!(inspected["data"]["edited"], false);
    assert_eq!(inspected["data"]["local_save"], "not_requested");
    assert_eq!(inspected["data"]["profile"]["name"], "Alicia");
    assert_eq!(
        inspected["data"]["profile"]["picture_url"],
        "https://example.com/avatar.png"
    );
    assert_eq!(inspected["data"]["profile"]["about"], "Hello");
    run(&["account", "profile", "--about", ""]);
    let cleared = run(&["account", "profile"]);
    assert!(cleared["data"]["profile"]["about"].is_null());
    assert_eq!(
        cleared["data"]["profile"]["picture_url"],
        "https://example.com/avatar.png"
    );
    let db = rusqlite::Connection::open(dir.path().join("core.sqlite3")).unwrap();
    let retained: String = db
        .query_row(
            "SELECT extra_metadata_json FROM owner_profiles LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&retained).unwrap(),
        serde_json::from_str::<Value>(extras).unwrap()
    );
}
