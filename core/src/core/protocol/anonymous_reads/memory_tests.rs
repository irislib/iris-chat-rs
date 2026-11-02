//! Keep the populated-account failure in a capped process, not the test runner.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::core::{AppCore, LocalAuthorizationState, LoggedInState};
use crate::state::AppState;
use crate::updates::{CoreMsg, InternalEvent};
use nostr::{EventBuilder, Filter, Keys, Kind, Timestamp};
use nostr_sdk::Client;
use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

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
    let executable = std::env::current_exe().expect("test executable");
    let mut command = if cfg!(unix) {
        let mut shell = Command::new("/bin/sh");
        shell
            .args([
                "-c",
                if cfg!(target_os = "linux") {
                    "ulimit -c 0 && ulimit -v 786432 && exec \"$@\""
                } else {
                    "ulimit -c 0 && exec \"$@\""
                },
                "iris-discovery-memory-child",
            ])
            .arg(&executable);
        shell
    } else {
        Command::new(&executable)
    };
    let mut child = command
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

fn populated_startup() {
    use crate::core::protocol::public_discovery::test_support;
    use std::sync::atomic::Ordering;
    let (_temp, mut core, incoming) = fixture();
    core.discovery_relays = test_support::pool();
    let handle = core.runtime.handle().clone();
    handle.block_on(async {
        let events = (0..PEERS)
            .map(|_| {
                EventBuilder::new(Kind::Metadata, "{\"name\":\"memory fixture\"}")
                    .sign_with_keys(&Keys::generate())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let relay = test_support::relay(events.clone()).await;
        let filters = events
            .iter()
            .map(|event| Filter::new().kind(Kind::Metadata).author(event.pubkey))
            .collect::<Vec<_>>();
        let subscriptions = core
            .private_discovery_subscriptions(filters.clone())
            .unwrap();
        assert_eq!(
            subscriptions.len(),
            1,
            "contact count must not create more workers"
        );
        super::super::private_reads::apply_private_discovery_subscriptions(
            subscriptions,
            std::slice::from_ref(&relay.url),
        )
        .await
        .unwrap();
        let mut delivered = HashSet::new();
        tokio::time::timeout(Duration::from_secs(15), async {
            while delivered.len() < PEERS {
                if let CoreMsg::Internal(event) = incoming.recv_async().await.unwrap() {
                    if let InternalEvent::RelayEvent(event) = *event {
                        delivered.insert(event.pubkey);
                    }
                }
            }
        })
        .await
        .expect("populated public discovery did not progress");
        assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
        println!("PRODUCTION_PROGRESS peers={}", delivered.len());
        let cold = core
            .discovery_relays
            .fetch(
                std::slice::from_ref(&relay.url),
                filters.clone(),
                Duration::from_secs(3),
            )
            .await
            .unwrap();
        assert_eq!(cold.len(), PEERS);
        assert_eq!(
            relay.connections.load(Ordering::SeqCst),
            1,
            "cold lookup opened a duplicate connection"
        );
        println!("PRODUCTION_COLD peers={}", cold.len());
        for reset in [false, true] {
            let changed = filters
                .iter()
                .cloned()
                .map(|filter| filter.since(Timestamp::from(if reset { 2 } else { 1 })))
                .collect();
            let subscriptions = core.private_discovery_subscriptions(changed).unwrap();
            let stale = subscriptions.clone();
            let before = relay.requests();
            super::super::private_reads::apply_private_discovery_subscriptions(
                subscriptions,
                std::slice::from_ref(&relay.url),
            )
            .await
            .unwrap();
            test_support::until(|| relay.requests() > before).await;
            if reset {
                core.protocol_subscription_runtime = Default::default();
            } else {
                core.private_discovery_subscriptions(Vec::new()).unwrap();
            }
            assert!(
                super::super::private_reads::apply_private_discovery_subscriptions(
                    stale,
                    std::slice::from_ref(&relay.url)
                )
                .await
                .is_err()
            );
            test_support::until(|| relay.active.load(Ordering::SeqCst) == 0).await;
            let stopped = relay.requests();
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(relay.requests(), stopped, "removed discovery restarted");
        }
        println!(
            "PRODUCTION_CANCELLED peak_rss_kib={:?}",
            process_peak_rss(std::process::id())
        );
    });
}
