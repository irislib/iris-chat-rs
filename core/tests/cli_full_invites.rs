//! Normal CLI/service coverage; all network fixtures are loopback-only.
use iris_chat_core::local_relay::TestRelay;
use nostr::PublicKey;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct OwnedProcess(Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn wait(child: &mut Child, seconds: u64) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "owned CLI process exceeded its deadline"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn command(profile: &Path, config: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_iris"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("IRIS_")
            || name.to_string_lossy().starts_with("HTREE_")
        {
            command.env_remove(name);
        }
    }
    command
        .env_remove("NOSTR_RELAYS")
        .env("IRIS_DEMO_RELAYS", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .env("IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR", "")
        .env("IRIS_CHAT_FIPS_UDP_BIND_ADDR", "")
        .env("IRIS_CHAT_FIPS_ENABLE_WEBRTC", "0")
        .env("IRIS_CHAT_FIPS_STATIC_PEERS", "")
        .env("IRIS_CHAT_FIPS_ROUTED_PEERS", "")
        .env("IRIS_CHAT_FIPS_TRUSTED_RATERS", "")
        .env("IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR", "")
        .env("IRIS_CHAT_SAME_HOST_HASHTREE", "0")
        .env("IRIS_UPDATE_RELAYS", "")
        .env("IRIS_UPDATE_BLOSSOM_SERVERS", "http://127.0.0.1:9")
        .env("HTREE_CONFIG_DIR", config)
        .env("HTREE_DATA_DIR", profile.join("hashtree-data"))
        .env("XDG_DATA_HOME", profile.join("xdg-data"))
        .args(["--json", "--no-background-sync", "--data-dir"])
        .arg(profile)
        .args(args);
    command
}

fn assert_offline_isolation(profile: &Path, config: &Path) -> Value {
    let service = run(profile, config, &["service", "status"], true);
    assert_eq!(service["data"]["running"], true);
    assert_eq!(service["data"]["nearby_lan"], false);
    assert_eq!(service["data"]["connected_servers"], 0);
    let debug = run(profile, config, &["debug"], true);
    assert_eq!(debug["data"]["relay_urls"], serde_json::json!([]));
    assert_eq!(debug["data"]["relay_transport"]["connected_relay_count"], 0);
    // Fresh defaults keep LAN/Bluetooth off, with no same-account/contact peers.
    // Assert the effective normal runtime has no FIPS endpoint: this also rules
    // out LAN, WebRTC/STUN, provider, seed, and same-host traffic during creation.
    assert_eq!(debug["data"]["fips_transport"]["status"], "unavailable");
    assert_eq!(debug["data"]["fips_services"]["status"], "unavailable");
    debug
}

fn run(profile: &Path, config: &Path, args: &[&str], success: bool) -> Value {
    let output = TempDir::new().unwrap();
    let stdout = output.path().join("stdout");
    let stderr = output.path().join("stderr");
    let mut child = OwnedProcess(
        command(profile, config, args)
            .env("IRIS_REQUIRE_SERVICE", "1")
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let status = wait(&mut child.0, 45);
    // Invite URLs contain secrets. Failure diagnostics must not print arguments/output.
    assert_eq!(status.success(), success, "CLI status={status}");
    serde_json::from_slice(&fs::read(stdout).unwrap()).expect("CLI JSON response")
}

fn service(profile: &Path, config: &Path) -> OwnedProcess {
    let log = tempfile::NamedTempFile::new().unwrap();
    let mut child = OwnedProcess(
        command(profile, config, &["service", "run"])
            .stdin(Stdio::null())
            .stdout(log.reopen().unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "service exited before readiness"
        );
        if fs::read_to_string(log.path()).unwrap().lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|value| value["data"]["ready"] == true)
        }) {
            return child;
        }
        assert!(Instant::now() < deadline, "service readiness deadline");
        thread::sleep(Duration::from_millis(20));
    }
}

struct UnavailableFiles {
    config: TempDir,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl UnavailableFiles {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, stopped) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        seen.fetch_add(1, Ordering::SeqCst);
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut wire = [0; 1024];
                        let _ = stream.read(&mut wire);
                        let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback fixture: {error}"),
                }
            }
        });
        let config = TempDir::new().unwrap();
        fs::write(config.path().join("config.toml"), format!(
            "[server]\nbind_address = \"127.0.0.1:9\"\n[blossom]\nservers = []\nread_servers = [\"{url}\"]\nwrite_servers = [\"{url}\"]\n",
        )).unwrap();
        Self {
            config,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for UnavailableFiles {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn cli_full_invite_is_offline_durable_and_accepted_by_normal_cli() {
    let files = UnavailableFiles::new();
    let config = files.config.path();
    let alice = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    let mut alice_service = service(alice.path(), config);
    let alice_account = run(
        alice.path(),
        config,
        &["account", "create", "--name", "Alice"],
        true,
    );
    let before = assert_offline_isolation(alice.path(), config);
    assert_eq!(files.requests.load(Ordering::SeqCst), 0);

    let full = run(alice.path(), config, &["invite", "create", "--full"], true);
    let full_url = full["data"]["url"].as_str().expect("full invite URL");
    assert!(full_url.starts_with("https://chat.iris.to/#/invite/"));
    let payload = full_url
        .strip_prefix("https://chat.iris.to/#/invite/")
        .unwrap();
    let invite =
        nostr_double_ratchet::parse_invite_url(&format!("https://chat.iris.to/#{payload}"))
            .unwrap();
    assert_eq!(invite.purpose.as_deref(), Some("private"));
    let after = assert_offline_isolation(alice.path(), config);
    let pending_ids = |debug: &Value| {
        debug["data"]["pending_relay_publishes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["event_id"].as_str().unwrap().to_string())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(pending_ids(&after), pending_ids(&before));
    assert_eq!(
        files.requests.load(Ordering::SeqCst),
        0,
        "full creation attempted a file-server request"
    );

    run(alice.path(), config, &["service", "stop"], true);
    assert!(wait(&mut alice_service.0, 10).success());
    drop(alice_service);
    let _alice_restarted = service(alice.path(), config);
    assert_offline_isolation(alice.path(), config);
    // A failed short-link upload must still fail, even with a usable full link saved.
    let short = run(alice.path(), config, &["invite", "create"], false);
    assert!(short["error"]
        .as_str()
        .unwrap()
        .contains("Could not create invite"));
    assert!(
        files.requests.load(Ordering::SeqCst) > 0,
        "negative control never attempted upload"
    );
    let file_requests = files.requests.load(Ordering::SeqCst);

    // Bootstrap authorization through a real loopback message server. This phase
    // proves normal CLI acceptance and the restored invite subscription, not LAN.
    let relay = TestRelay::start();
    assert!(!relay.url().is_empty());
    run(alice.path(), config, &["relay", "set", relay.url()], true);
    let _bob_service = service(bob.path(), config);
    let bob_account = run(
        bob.path(),
        config,
        &["account", "create", "--name", "Bob"],
        true,
    );
    run(bob.path(), config, &["relay", "set", relay.url()], true);
    let accepted = run(bob.path(), config, &["invite", "accept", full_url], true);
    let owner = PublicKey::parse(alice_account["data"]["user_id"].as_str().unwrap()).unwrap();
    assert_eq!(accepted["data"]["current_chat"]["chat_id"], owner.to_hex());
    run(
        bob.path(),
        config,
        &["send", &owner.to_hex(), "full invite after restart"],
        true,
    );
    let peer = bob_account["data"]["user_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let read = run(alice.path(), config, &["read", peer], true);
        if read["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["body"] == "full invite after restart")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "restored full invite did not receive first-contact message"
        );
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        files.requests.load(Ordering::SeqCst),
        file_requests,
        "inline acceptance fetched a file-server record"
    );
}
