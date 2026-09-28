use serde_json::Value;
#[cfg(unix)]
use std::fs;
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use tempfile::TempDir;

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn cmd(path: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_iris"));
    c.arg("--json").arg("--data-dir").arg(path).args(args);
    c
}
fn run(path: &Path, args: &[&str]) -> Value {
    let o = cmd(path, args).output().unwrap();
    assert!(
        o.status.success(),
        "args={args:?}: {} {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
fn lines(child: &mut Child) -> mpsc::Receiver<Value> {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str(&line) else {
                continue;
            };
            if tx.send(v).is_err() {
                break;
            }
        }
    });
    rx
}
fn start(path: &Path) -> Service {
    let mut s = Service(
        cmd(path, &["service", "run"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let rx = lines(&mut s.0);
    let ready = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!(ready["data"]["ready"], true, "{ready}");
    s
}

#[test]
fn service_keeps_profile_owned_and_serves_commands_while_listening() {
    let dir = TempDir::new().unwrap();
    run(dir.path(), &["relay", "set"]);
    let account = run(dir.path(), &["account", "create", "--name", "Alice"]);
    run(dir.path(), &["relay", "set"]);
    let mut service = start(dir.path());
    let socket = dir.path().join("cli.sock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let who = run(dir.path(), &["whoami"]);
    assert_eq!(who["data"]["npub"], account["data"]["npub"]);
    let relative = Command::new(env!("CARGO_BIN_EXE_iris"))
        .current_dir(dir.path().parent().unwrap())
        .args(["--json", "--data-dir"])
        .arg(dir.path().file_name().unwrap())
        .arg("whoami")
        .output()
        .unwrap();
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stdout)
    );
    let from_env = Command::new(env!("CARGO_BIN_EXE_iris"))
        .env("IRIS_DATA_DIR", dir.path())
        .args(["--json", "whoami"])
        .output()
        .unwrap();
    assert!(
        from_env.status.success(),
        "{}",
        String::from_utf8_lossy(&from_env.stdout)
    );
    let mut listener = Service(
        cmd(dir.path(), &["listen"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let rx = lines(&mut listener.0);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).unwrap()["data"]["ready"],
        true
    );
    run(
        dir.path(),
        &["account", "profile", "--name", "Service Alice"],
    );
    assert_eq!(
        run(dir.path(), &["whoami"])["data"]["name"],
        "Service Alice"
    );
    let blocked = cmd(dir.path(), &["service", "run"]).output().unwrap();
    assert!(!blocked.status.success());
    // Malformed clients cannot kill the owner or consume its command loop forever.
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::net::UnixStream};
        let mut malformed = UnixStream::connect(&socket).unwrap();
        malformed.write_all(b"not json\n").unwrap();
    }
    assert_eq!(
        run(dir.path(), &["service", "status"])["data"]["running"],
        true
    );
    run(dir.path(), &["service", "stop"]);
    assert!(service.0.wait().unwrap().success());
    assert!(!socket.exists());
    let closed = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(closed["status"], "error");
    run(dir.path(), &["whoami"]); // standalone mode remains available after clean stop
}

#[test]
fn killed_service_reclaims_stale_socket_without_weakening_profile_lock() {
    let dir = TempDir::new().unwrap();
    run(dir.path(), &["relay", "set"]);
    let mut s = start(dir.path());
    s.0.kill().unwrap();
    s.0.wait().unwrap();
    #[cfg(unix)]
    assert!(dir.path().join("cli.sock").exists());
    let _restarted = start(dir.path());
    assert_eq!(
        run(dir.path(), &["service", "status"])["data"]["running"],
        true
    );
}

#[test]
fn required_service_fails_closed_and_never_replaces_regular_files() {
    let dir = TempDir::new().unwrap();
    let missing = cmd(dir.path(), &["whoami"])
        .env("IRIS_REQUIRE_SERVICE", "1")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(!dir.path().join("core.sqlite3").exists());
    #[cfg(unix)]
    {
        fs::write(dir.path().join("cli.sock"), b"preserve").unwrap();
        let bad = cmd(dir.path(), &["service", "run"]).output().unwrap();
        assert!(!bad.status.success());
        assert_eq!(fs::read(dir.path().join("cli.sock")).unwrap(), b"preserve");
    }
}

#[test]
fn service_receives_and_replies_without_stopping_listener() {
    use iris_chat_core::local_relay::TestRelay;
    let relay = TestRelay::start();
    let alice = TempDir::new().unwrap();
    let bob = TempDir::new().unwrap();
    run(alice.path(), &["relay", "set", relay.url()]);
    let a = run(alice.path(), &["account", "create", "--name", "Alice"]);
    run(alice.path(), &["relay", "set", relay.url()]);
    run(bob.path(), &["relay", "set", relay.url()]);
    let b = run(bob.path(), &["account", "create", "--name", "Bob"]);
    run(bob.path(), &["relay", "set", relay.url()]);
    let aid = a["data"]["user_id"].as_str().unwrap();
    let bid = b["data"]["user_id"].as_str().unwrap();
    let _aservice = start(alice.path());
    let _bservice = start(bob.path());
    let mut listener = Service(
        cmd(bob.path(), &["listen", "--interval-ms", "100"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let rx = lines(&mut listener.0);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).unwrap()["data"]["ready"],
        true
    );
    // Exercise frames larger than the Windows named-pipe buffer in both directions.
    let body = "live service request ".repeat(100).trim_end().to_string();
    run(alice.path(), &["send", bid, &body]);
    let incoming = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(incoming["data"]["body"], body);
    assert_eq!(incoming["data"]["is_outgoing"], false);
    let response = run(bob.path(), &["send", aid, "live service reply"]);
    assert_eq!(response["data"]["body"], "live service reply");
    assert!(listener.0.try_wait().unwrap().is_none());
    let end = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let read = run(alice.path(), &["read", bid]);
        if read["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "live service reply" && m["is_outgoing"] == false)
        {
            break;
        }
        assert!(std::time::Instant::now() < end, "reply missing");
        std::thread::sleep(Duration::from_millis(100));
    }
}
