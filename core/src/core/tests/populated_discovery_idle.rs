// Generated populated storage, real TLS/pub-sub and process CPU. Never opens
// an existing account or dials public infrastructure.
#[cfg(target_os = "linux")]
mod populated_discovery_idle {
    use super::super::mesh_pubsub::{
        discovery_test_support, PrivateMeshDiscovery, PrivateMeshProviders,
    };
    use super::super::protocol::public_discovery::test_support;
    use super::*;
    use sha2::Digest;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const TEST: &str = "core::tests::populated_discovery_idle::populated_account_discovery_stays_quiet_and_targeted_demand_progresses";
    const FIVE_RELAY_TEST: &str = "core::tests::populated_discovery_idle::populated_account_five_relay_discovery_stays_quiet_and_targeted_demand_progresses";
    const CHILD: &str = "IRIS_TEST_POPULATED_DISCOVERY_IDLE_CHILD";
    const CONTACTS: usize = 96;
    const DEVICES: usize = 3;
    const MESH_DEVICES: usize = 96;
    include!("populated_discovery_idle_support.rs");
    fn cpu_ticks() -> u64 {
        let raw = std::fs::read_to_string("/proc/self/stat").unwrap();
        let fields = raw[raw.rfind(')').unwrap() + 2..]
            .split_whitespace()
            .collect::<Vec<_>>();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    }

    fn tick_rate() -> f64 {
        let result = std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Linux CPU clock calibration unavailable"
        );
        let rate = std::str::from_utf8(&result.stdout)
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap();
        assert!(rate.is_finite() && rate > 0.0);
        rate
    }

    fn restored_fixture() -> (SyntheticHistory, Vec<Event>) {
        let (mut fixture, events) =
            synthetic_history_fixture_with_devices(100, CONTACTS, 8, DEVICES);
        assert!(fixture.database_bytes >= 100 * 1024 * 1024);
        assert!(fixture.rows >= (CONTACTS + 8) as u64 * 200);
        let logged_in = fixture.core.logged_in.as_ref().unwrap();
        let owner = logged_in.owner_keys.clone().unwrap();
        let local = logged_in.device_keys.clone();
        fixture.core.persist_app_state().unwrap();
        fixture.core.shutdown();
        drop(fixture.core);
        // Reopen both real SQLite projections and the verified protocol checkpoint.
        // Never reconstruct session/roster snapshots by hand in the regression.
        let mut core = logged_in_test_core_at_data_dir(
            &owner,
            &local,
            fixture.directory.path().to_string_lossy().into_owned(),
        );
        let persisted = core.load_persisted().unwrap().unwrap();
        assert_eq!(persisted.threads.len(), CONTACTS + 8);
        super::super::persistence::apply_persisted_preferences(
            &mut core.preferences,
            &persisted.preferences,
        );
        core.app_keys = persisted
            .app_keys
            .into_iter()
            .map(|keys| (keys.owner_pubkey_hex.clone(), keys))
            .collect();
        core.groups = persisted
            .groups
            .into_iter()
            .map(|group| (group.group_id.clone(), group))
            .collect();
        core.owner_profiles = persisted.owner_profiles;
        core.threads = persisted
            .threads
            .into_iter()
            .map(|thread| {
                (
                    thread.chat_id.clone(),
                    ThreadRecord {
                        chat_id: thread.chat_id,
                        unread_count: thread.unread_count,
                        updated_at_secs: thread.updated_at_secs,
                        draft: thread.draft,
                        messages: thread
                            .messages
                            .iter()
                            .map(chats::chat_message_from_persisted)
                            .collect(),
                    },
                )
            })
            .collect();
        let engine = core.protocol_engine.as_ref().unwrap();
        let checkpoint = engine.session_manager_snapshot();
        assert!(
            checkpoint
                .users
                .iter()
                .flat_map(|u| &u.devices)
                .filter(|d| !d.authorized && d.is_stale)
                .count()
                >= CONTACTS,
            "revoked/stale durable records must survive a real reopen"
        );
        assert_eq!(core.app_keys.len(), CONTACTS + 1);
        for roster in core.app_keys.values() {
            assert_eq!(roster.devices.len(), DEVICES);
            let owner = PublicKey::from_hex(&roster.owner_pubkey_hex).unwrap();
            for device in &roster.devices {
                assert!(engine.has_device_roster_entry_for_owner(
                    owner,
                    PublicKey::from_hex(&device.identity_pubkey_hex).unwrap()
                ));
            }
        }
        fixture.core = core;
        (fixture, events)
    }

    fn child(relay_count: usize) {
        let (mut fixture, mut events) = restored_fixture();
        let proof_index = events
            .iter()
            .position(|event| {
                event.kind.as_u16() as u32 == APP_KEYS_EVENT_KIND && event.created_at.as_secs() == 6
            })
            .unwrap();
        let proof = events.remove(proof_index);
        let replay_events = events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind.as_u16() as u32,
                    APP_KEYS_EVENT_KIND | INVITE_EVENT_KIND
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(replay_events.len() >= CONTACTS * 2);
        let (app_writes, protocol_writes) = measure_core_writes(&mut fixture.core);
        let (_, discovery) = fixture.core.split_private_discovery_filters(
            fixture.core.recent_protocol_metadata_filters(unix_now()),
        );
        let mut subjects = discovery
            .iter()
            .flat_map(|filter| filter.authors.iter().flatten())
            .map(PublicKey::to_hex)
            .collect::<HashSet<_>>();
        assert_eq!(subjects.len(), 384);
        let own = fixture.core.logged_in.as_ref().unwrap();
        let mut forbidden = HashSet::from([own.owner_pubkey, own.device_keys.public_key()]);
        for roster in fixture.core.app_keys.values() {
            forbidden.insert(PublicKey::from_hex(&roster.owner_pubkey_hex).unwrap());
            forbidden.extend(
                roster
                    .devices
                    .iter()
                    .map(|device| PublicKey::from_hex(&device.identity_pubkey_hex).unwrap()),
            );
        }
        for user in &fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot()
            .users
        {
            forbidden.extend(
                user.devices
                    .iter()
                    .map(|device| PublicKey::from_slice(&device.device_pubkey.to_bytes()).unwrap()),
            );
        }
        let now = unix_now().get();
        let mut mesh_filters = Vec::new();
        for _ in 0..MESH_DEVICES {
            let key = Keys::generate();
            subjects.insert(key.public_key().to_hex());
            mesh_filters.push(
                Filter::new()
                    .kind(Kind::Custom(37195))
                    .author(key.public_key()),
            );
            events.push(discovery_test_support::advert(
                &key,
                now,
                now + 3600,
                "mesh-a.invalid:9000",
            ));
        }
        assert_eq!(subjects.len(), 480);
        let revoked = fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot()
            .users
            .iter()
            .flat_map(|user| &user.devices)
            .filter(|device| !device.authorized && device.is_stale)
            .count();
        assert!(revoked >= 96);
        let (sender, incoming) = flume::unbounded();
        fixture.core.core_sender = sender;
        fixture.core.discovery_relays = test_support::pool();
        let runtime = fixture.core.runtime.handle().clone();
        runtime.block_on(async {
            let mut relays = Vec::new();
            for _ in 0..relay_count { relays.push(test_support::relay(events.clone()).await); }
            let urls = relays.iter().map(|relay| relay.url.clone()).collect::<Vec<_>>();
            let expected_initial = events.iter().filter(|event| discovery.iter().any(|filter|
                filter.match_event(event, nostr::filter::MatchEventOptions::new())))
                .map(|event| event.id.to_hex()).collect::<Vec<_>>();
            let subscriptions = fixture.core.private_discovery_subscriptions(discovery).unwrap();
            assert_eq!(subscriptions.len(), 1);
            let worker = subscriptions[0].0.upgrade().unwrap();
            super::super::protocol::private_reads::apply_private_discovery_subscriptions(subscriptions, &urls).await.unwrap();
            let endpoint = discovery_test_support::endpoint().await;
            fixture.core.install_device_sync_endpoint_for_test(endpoint.clone());
            let providers = PrivateMeshProviders::new(vec![], None, vec![], urls.clone()).with_relay_pool(fixture.core.discovery_relays.clone());
            let mut mesh = PrivateMeshDiscovery::default();
            mesh.update(Arc::new(providers), mesh_filters, endpoint.clone(), fixture.core.core_sender.clone(), &fixture.core.runtime);
            let settling = Instant::now();
            loop {
                dispatch_pending(&mut fixture.core, &incoming);
                if relays.iter().all(|relay| relay.ready_subjects() == subjects)
                    && expected_initial.iter().all(|id| fixture.core.has_seen_event(id))
                    && incoming.is_empty() { break; }
                assert!(settling.elapsed() < Duration::from_secs(60), "all populated authors must become live");
                sleep(Duration::from_millis(20)).await;
            }
            // Finish all initial bodies and any backpressure replay before
            // sampling. Readiness requires a stable request count and empty core queue.
            let mut stable_since = Instant::now();
            let mut requests = relays.iter().map(test_support::Relay::requests).collect::<Vec<_>>();
            while stable_since.elapsed() < Duration::from_secs(1) {
                dispatch_pending(&mut fixture.core, &incoming);
                let current = relays.iter().map(test_support::Relay::requests).collect::<Vec<_>>();
                if current != requests || !incoming.is_empty() { requests = current; stable_since = Instant::now(); }
                assert!(settling.elapsed() < Duration::from_secs(60));
                sleep(Duration::from_millis(20)).await;
            }
            let settle_seconds = settling.elapsed().as_secs_f64();
            for relay in &relays { assert_eq!(relay.connections.load(Ordering::SeqCst), 1); }
            let ticks_per_second = tick_rate();
            let mut samples = Vec::new();
            for _ in 0..2 {
                enqueue_discovery_replay(&mut fixture.core, &replay_events);
                let app_before = app_writes.load(Ordering::SeqCst);
                let protocol_before = protocol_writes.load(Ordering::SeqCst);
                let before = relays.iter().map(|relay| (relay.wire.lock().unwrap().len(), relay.connections.load(Ordering::SeqCst), relay.sent_bytes.load(Ordering::SeqCst))).collect::<Vec<_>>();
                let ticks = cpu_ticks(); let started = Instant::now();
                let processed = sample_with_core(&mut fixture.core, &incoming, Duration::from_secs(125)).await;
                let seconds = started.elapsed().as_secs_f64();
                let cpu = (cpu_ticks() - ticks) as f64 / ticks_per_second * 100.0 / seconds;
                let relay_samples = relays.iter().enumerate().map(|(index, relay)| {
                    let rows = relay.wire.lock().unwrap();
                    let application_wire_bytes = rows[before[index].0..].iter().map(|(_, frame)| frame.to_string().len()).sum::<usize>() + relay.sent_bytes.load(Ordering::SeqCst) - before[index].2;
                    let connections = relay.connections.load(Ordering::SeqCst) - before[index].1;
                    assert_eq!(application_wire_bytes, 0, "idle discovery sent application traffic");
                    assert_eq!(connections, 0, "idle discovery opened a connection");
                    assert_eq!(relay.active.load(Ordering::SeqCst), 1);
                    serde_json::json!({"relay_index": index, "subjects": relay.ready_subjects().len(), "active_connections": 1,
                        "application_wire_bytes": application_wire_bytes, "connections": connections})
                }).collect::<Vec<_>>();
                let app_delta = app_writes.load(Ordering::SeqCst) - app_before;
                let protocol_delta = protocol_writes.load(Ordering::SeqCst) - protocol_before;
                assert_eq!(app_delta, 0); assert_eq!(protocol_delta, 0);
                assert!(processed >= replay_events.len());
                assert!(cpu <= 5.0, "idle shared discovery exceeded one-core budget: {cpu}");
                let sample = serde_json::json!({"seconds": seconds, "cpu_percent_one_core": cpu,
                    "application_wire_bytes": 0, "connections": 0, "app_row_writes": app_delta, "protocol_writes": protocol_delta,
                    "core_messages_processed": processed, "evicted_discovery_replays": replay_events.len(), "relay_samples": relay_samples});
                println!("POPULATED_DISCOVERY_IDLE_SAMPLE={sample}"); samples.push(sample);
            }
            let before = relays.iter().map(|relay| relay.wire.lock().unwrap().len()).collect::<Vec<_>>();
            for relay in &relays { relay.events.lock().unwrap().push(proof.clone()); }
            worker.refresh(Some(proof.pubkey));
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                dispatch_pending(&mut fixture.core, &incoming);
                if fixture.core.app_keys[&proof.pubkey.to_hex()].created_at_secs == proof.created_at.as_secs() { break; }
                assert!(Instant::now() < deadline, "targeted roster recovery did not apply its proof");
                sleep(Duration::from_millis(10)).await;
            }
            // Only the requested author may appear in the explicit recovery REQ.
            for (index, relay) in relays.iter().enumerate() {
                test_support::until(|| relay.wire.lock().unwrap()[before[index]..].iter().any(|(_, frame)| frame[0] == "CLOSE")).await;
                let rows = relay.wire.lock().unwrap();
                let requests = rows[before[index]..].iter().filter(|(_, frame)| frame[0] == "REQ").collect::<Vec<_>>();
                assert!(!requests.is_empty(), "targeted recovery omitted relay {index}");
                let responses = relay.responses.lock().unwrap();
                for (connection, frame) in requests {
                    let id = frame[1].as_str().unwrap();
                    assert!(responses.iter().any(|(peer, subscription, event)| peer == connection && subscription == id && *event == Some(proof.id)), "relay {index} did not send the requested proof");
                    assert!(responses.iter().any(|(peer, subscription, event)| peer == connection && subscription == id && event.is_none()), "relay {index} did not complete recovery");
                    for filter in &frame.as_array().unwrap()[2..] {
                        assert_eq!(filter["authors"], serde_json::json!([proof.pubkey.to_hex()]));
                    }
                }
            }
            let mut readiness = Vec::new();
            for (index, relay) in relays.iter().enumerate() {
                let rows = relay.wire.lock().unwrap();
                let auth = rows.iter().filter(|(_, frame)| frame[0] == "AUTH").collect::<Vec<_>>();
                assert_eq!(auth.len(), 1); assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
                let signer: Event = serde_json::from_value(auth[0].1[1].clone()).unwrap(); signer.verify().unwrap();
                assert!(!forbidden.contains(&signer.pubkey) && !subjects.contains(&signer.pubkey.to_hex()));
                for (_, frame) in rows.iter() {
                    assert_ne!(frame[0], "EVENT");
                    if frame[0] == "REQ" { for filter in &frame.as_array().unwrap()[2..] {
                        assert!(filter.get("#p").is_none());
                        assert!(filter["authors"].as_array().unwrap().iter().all(|author| subjects.contains(author.as_str().unwrap())));
                    }}
                }
                readiness.push(serde_json::json!({"relay_index": index, "subjects": relay.ready_subjects().len(), "connections": 1, "anonymous_authentications": 1}));
            }
            println!("POPULATED_DISCOVERY_IDLE_RESULT={}", serde_json::json!({
                "database_bytes": fixture.database_bytes, "message_rows": fixture.rows, "contacts": CONTACTS,
                "peer_devices": CONTACTS * DEVICES, "own_devices": DEVICES, "groups": 8,
                "discovery_subjects": 384, "mesh_discovery_subjects": MESH_DEVICES, "total_discovery_subjects": subjects.len(),
                "relay_count": relay_count, "anonymous_connections": relay_count, "relay_readiness": readiness,
                "revoked_stale_records": revoked, "targeted_fresh_proof_delivered": true, "privacy_wire_passed": true,
                "quiet_samples": samples, "settle_seconds": settle_seconds, "debug_assertions": cfg!(debug_assertions),
                "fixture_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("synthetic_history_scale.rs"))),
                "support_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("populated_discovery_idle_support.rs"))),
                "test_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("populated_discovery_idle.rs"))),
                "discovery_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("../protocol/public_discovery.rs"))),
                "query_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("../protocol/public_discovery/query.rs"))),
                "relay_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("../protocol/public_discovery/test_support.rs")))
            }));
            fixture.core.private_discovery_subscriptions(Vec::new()).unwrap(); drop(worker);
            discovery_test_support::shutdown(&mut mesh).await;
            for relay in &relays { test_support::until(|| relay.active.load(Ordering::SeqCst) == 0).await; }
            endpoint.shutdown().await.unwrap();
        });
        fixture.core.shutdown();
    }

    #[test]
    fn populated_account_discovery_stays_quiet_and_targeted_demand_progresses() {
        run_case(1, TEST, "populated-discovery-idle");
    }

    #[test]
    #[ignore = "serialized optimized five-relay CPU gate in scripts/test_rust.sh"]
    fn populated_account_five_relay_discovery_stays_quiet_and_targeted_demand_progresses() {
        run_case(5, FIVE_RELAY_TEST, "populated-discovery-five-relay");
    }

    fn run_case(relay_count: usize, test: &str, artifact: &str) {
        if std::env::var(CHILD).as_deref() == Ok("1") {
            child(relay_count);
            return;
        }
        let artifacts = std::env::var_os("IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT")
            .map(std::path::PathBuf::from)
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        if let Some(directory) = &artifacts {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join(format!("{artifact}.json")),
                b"{\"schema\":\"iris-populated-discovery-idle/v2\",\"passed\":false,\"status\":\"starting\"}\n").unwrap();
        }
        // File-backed pipes keep a failing assertion's full output without
        // blocking the child on the test runner's pipe capacity.
        let logs = tempfile::Builder::new()
            .prefix("iris-populated-discovery-idle-")
            .tempdir()
            .unwrap();
        let stdout = logs.path().join("stdout.log");
        let stderr = logs.path().join("stderr.log");
        let started = Instant::now();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", test, "--nocapture", "--test-threads=1"]);
        if relay_count == 5 {
            command.arg("--ignored");
        }
        let mut child = command
            .env(CHILD, "1")
            .env("TOKIO_WORKER_THREADS", "2")
            .env("RUST_BACKTRACE", "0")
            .env("RUST_LOG", "off")
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if started.elapsed() >= Duration::from_secs(540) {
                child.kill().unwrap();
                break true;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let status = child.wait().unwrap();
        let stdout = std::fs::read_to_string(&stdout).unwrap();
        let stderr = std::fs::read_to_string(&stderr).unwrap();
        let log = format!(
            "populated discovery child status={status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        let measurements = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("POPULATED_DISCOVERY_IDLE_RESULT="))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let passed = !timed_out && status.success() && measurements.len() == 1;
        let receipt = serde_json::json!({
            "schema": "iris-populated-discovery-idle/v2", "passed": passed,
            "timed_out": timed_out, "exit_code": status.code(),
            "seconds": started.elapsed().as_secs_f64(),
            "measurements": measurements, "log_bytes": log.len(),
            "log_sha256": format!("{:x}", sha2::Sha256::digest(log.as_bytes())),
        });
        if let Some(directory) = &artifacts {
            std::fs::write(directory.join(format!("{artifact}.log")), &log).unwrap();
            std::fs::write(
                directory.join(format!("{artifact}.json")),
                serde_json::to_vec_pretty(&receipt).unwrap(),
            )
            .unwrap();
        }
        // Bare libtest -q otherwise hides successful CPU evidence. Nextest's
        // capture is covered by the companion artifact files above.
        std::io::stdout().write_all(log.as_bytes()).unwrap();
        std::io::stdout()
            .write_all(format!("POPULATED_DISCOVERY_IDLE_RECEIPT={receipt}\n").as_bytes())
            .unwrap();
        assert!(
            !timed_out,
            "populated discovery child exceeded 540s absolute deadline"
        );
        assert!(
            passed,
            "populated discovery child failed; full output retained above"
        );
    }
}
