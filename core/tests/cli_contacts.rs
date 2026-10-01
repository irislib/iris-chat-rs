use nostr::Keys;
use serde_json::Value;
use std::{path::Path, process::Command};
use tempfile::TempDir;

fn run(data_dir: &Path, args: &[&str], success: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .env("IRIS_DEMO_RELAYS", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .args(["--json", "--no-background-sync", "--data-dir"])
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run iris contact command");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.success(),
        success,
        "args={args:?}\nstdout={stdout}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn set_public_name(data_dir: &Path, contact: &str, name: &str, timestamp: u64) {
    let db = rusqlite::Connection::open(data_dir.join("core.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO owner_profiles(owner_pubkey_hex, name, updated_at_secs)
                VALUES (?1, ?2, ?3) ON CONFLICT(owner_pubkey_hex)
                DO UPDATE SET name = excluded.name, updated_at_secs = excluded.updated_at_secs",
        rusqlite::params![contact, name, timestamp],
    )
    .unwrap();
}

#[test]
fn cli_contact_favorite_and_exact_name_approval_persist_without_changing_public_follows() {
    let dir = TempDir::new().unwrap();
    run(dir.path(), &["account", "create", "--name", "Viewer"], true);
    let peer = Keys::generate().public_key().to_hex();
    set_public_name(dir.path(), &peer, "Alice", 1);
    run(dir.path(), &["chat", "create", &peer], true);
    set_public_name(dir.path(), &peer, "Alicia", 2);

    let shown = run(dir.path(), &["contact", "show", &peer], true);
    assert_eq!(shown["data"]["name"], "Alice");
    assert_eq!(
        shown["data"]["contact_identity"]["first_seen_name"],
        "Alice"
    );
    assert_eq!(shown["data"]["contact_identity"]["pending_name"], "Alicia");
    let favorite = run(dir.path(), &["contact", "favorite", &peer], true);
    assert_eq!(favorite["data"]["contact_identity"]["is_favorite"], true);
    assert_eq!(favorite["data"]["contact_identity"]["is_following"], false);
    let nickname = run(
        dir.path(),
        &["contact", "nickname", &peer, "Tea friend"],
        true,
    );
    assert_eq!(nickname["data"]["nickname"], "Tea friend");
    assert_eq!(nickname["data"]["private_contact_sync"], "queued");
    let note = run(
        dir.path(),
        &["contact", "note", &peer, "Met at lunch"],
        true,
    );
    assert_eq!(note["data"]["nickname"], "Tea friend");
    assert_eq!(note["data"]["note"], "Met at lunch");
    run(dir.path(), &["contact", "nickname", &peer, ""], true);
    run(dir.path(), &["contact", "note", &peer, ""], true);
    let stale = run(
        dir.path(),
        &["contact", "approve-name", &peer, "Unseen name"],
        false,
    );
    assert!(stale["error"].as_str().unwrap().contains("not pending"));
    let approved = run(
        dir.path(),
        &["contact", "approve-name", &peer, "Alicia"],
        true,
    );
    assert_eq!(approved["data"]["name"], "Alicia");
    assert_eq!(approved["data"]["contact_identity"]["saved_name"], "Alicia");
    assert_eq!(
        approved["data"]["contact_identity"]["first_seen_name"],
        "Alice"
    );
    assert!(approved["data"]["contact_identity"]["pending_name"].is_null());
    let read = run(dir.path(), &["read", &peer], true);
    assert_eq!(
        read["data"]["chat"]["contact_identity"]["is_favorite"],
        true
    );
    assert!(read["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["body"] == "Name change approved: Alice → Alicia"));
    let unfavorite = run(dir.path(), &["contact", "unfavorite", &peer], true);
    assert_eq!(unfavorite["data"]["contact_identity"]["is_favorite"], false);
    let follow = run(dir.path(), &["contact", "follow", &peer], false);
    assert!(follow["error"].as_str().unwrap().contains("message server"));
    let db = rusqlite::Connection::open(dir.path().join("core.sqlite3")).unwrap();
    let raw: String = db
        .query_row(
            "SELECT contact_memory_json FROM owner_profiles WHERE owner_pubkey_hex = ?1",
            [&peer],
            |row| row.get(0),
        )
        .unwrap();
    let memory: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(memory["first_seen_name"], "Alice");
    assert_eq!(memory["accepted_name"], "Alicia");
    assert_eq!(memory["name_changes"].as_array().unwrap().len(), 1);
    let count: u64 = db
        .query_row("SELECT COUNT(*) FROM user_discovery_users", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn cli_contact_commands_require_an_existing_direct_contact() {
    let dir = TempDir::new().unwrap();
    run(dir.path(), &["account", "create", "--name", "Viewer"], true);
    let peer = Keys::generate().public_key().to_hex();
    let error = run(dir.path(), &["contact", "favorite", &peer], false);
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("Start a chat first"));
    let list = run(dir.path(), &["chat", "list"], true);
    assert_eq!(list["data"]["chats"].as_array().unwrap().len(), 0);
}

#[test]
fn cli_contact_follow_publishes_before_one_shot_exit_and_preserves_existing_contacts() {
    use iris_chat_core::local_relay::TestRelay;
    use nostr::{Event, EventBuilder, Kind, Tag, Timestamp, ToBech32};
    use std::time::{Duration, Instant};

    let relay = TestRelay::start();
    assert!(!relay.url().is_empty(), "local relay started");
    let owner = Keys::generate();
    let existing = Keys::generate().public_key();
    let peer = Keys::generate().public_key();
    let original = EventBuilder::new(Kind::ContactList, "legacy relay preferences")
        .tags([
            Tag::parse(["p", &existing.to_hex(), "wss://example.com", "Old friend"]).unwrap(),
            Tag::parse(["alt", "Keep this extra tag"]).unwrap(),
        ])
        .custom_created_at(Timestamp::from_secs(
            Timestamp::now().as_secs().saturating_sub(60),
        ))
        .sign_with_keys(&owner)
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client = nostr_sdk::Client::default();
        client.add_relay(relay.url()).await.unwrap();
        client.connect().await;
        client.send_event(&original).await.unwrap();
        client.disconnect().await;
    });
    let dir = TempDir::new().unwrap();
    run(
        dir.path(),
        &["restore", &owner.secret_key().to_bech32().unwrap()],
        true,
    );
    run(dir.path(), &["relay", "set", relay.url()], true);
    run(dir.path(), &["chat", "create", &peer.to_hex()], true);
    let mut previous_id = original.id;
    for (command, following) in [("follow", true), ("unfollow", false)] {
        let result = run(dir.path(), &["contact", command, &peer.to_hex()], true);
        assert_eq!(
            result["data"]["contact_identity"]["is_following"],
            following
        );
        assert_eq!(result["data"]["network_publication"], "queue_drained");
        // No service, extra sync command, or profile reopen is allowed to deliver it.
        let started = Instant::now();
        let published = loop {
            if let Some(event) = relay
                .events()
                .into_iter()
                .filter_map(|event| serde_json::from_value::<Event>(event).ok())
                .find(|event| {
                    event.kind == Kind::ContactList
                        && event.pubkey == owner.public_key()
                        && event.id != previous_id
                        && event.created_at > original.created_at
                        && event.tags.iter().any(|tag| {
                            tag.as_slice().first().map(String::as_str) == Some("p")
                                && tag.as_slice().get(1) == Some(&peer.to_hex())
                        }) == following
                })
            {
                break event;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "public follow never reached relay"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        published.verify().unwrap();
        assert_eq!(published.content, original.content);
        for tag in original.tags.iter() {
            assert!(published.tags.iter().any(|actual| actual == tag));
        }
        assert_eq!(
            published.tags.len(),
            original.tags.len() + usize::from(following)
        );
        previous_id = published.id;
    }
}
