use std::{path::Path, process::Command};

use nostr::Keys;
use serde_json::Value;
use tempfile::TempDir;

fn run(data_dir: &Path, args: &[&str], success: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .env("IRIS_DEMO_RELAYS", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .args(["--json", "--no-background-sync", "--data-dir"])
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run chat setting command");
    assert_eq!(
        output.status.success(),
        success,
        "args={args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

#[test]
fn group_mention_notification_choice_survives_cli_restart() {
    let dir = TempDir::new().unwrap();
    run(dir.path(), &["account", "create", "--name", "Viewer"], true);
    let peer = Keys::generate().public_key().to_hex();
    let group = run(dir.path(), &["group", "create", "Friends", &peer], true);
    let chat = group["data"]["current_chat"]["chat_id"].as_str().unwrap();
    let initial = run(dir.path(), &["chat", "mention-notifications", chat], true);
    assert_eq!(initial["data"]["enabled"], true);
    for (value, expected) in [("false", false), ("true", true)] {
        let updated = run(
            dir.path(),
            &["chat", "mention-notifications", chat, value],
            true,
        );
        assert_eq!(updated["data"]["chat_id"], chat);
        assert_eq!(updated["data"]["enabled"], expected);
        let restored = run(
            dir.path(),
            &["chat", "mention-notifications", "Friends"],
            true,
        );
        assert_eq!(restored["data"]["enabled"], expected);
    }
    run(dir.path(), &["chat", "create", &peer], true);
    let error = run(
        dir.path(),
        &["chat", "mention-notifications", &peer, "false"],
        false,
    );
    assert_eq!(error["error"], "Choose a group chat.");
}
