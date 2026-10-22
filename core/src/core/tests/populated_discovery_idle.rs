// Keep process CPU independent of Cargo's parallel test runner. All identities,
// rosters and history are generated; this never opens an existing account.
#[cfg(target_os = "linux")]
mod populated_discovery_idle {
    use super::super::protocol::anonymous_reads::tls_tests;
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use sha2::Digest;
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    const TEST: &str = "core::tests::populated_discovery_idle::populated_account_discovery_stays_quiet_and_targeted_demand_progresses";
    const CHILD: &str = "IRIS_TEST_POPULATED_DISCOVERY_IDLE_CHILD";
    const CONTACTS: usize = 96;
    const DEVICES: usize = 3;

    #[derive(Default)]
    struct Wire {
        rows: Mutex<Vec<(usize, serde_json::Value)>>,
        rounds: Mutex<BTreeMap<String, usize>>,
        connections: AtomicUsize,
        bytes: AtomicUsize,
        tls_full: AtomicUsize,
        tls_resumed: AtomicUsize,
    }

    async fn connection<S: AsyncRead + AsyncWrite + Unpin>(
        stream: S,
        id: usize,
        wire: Arc<Wire>,
        events: Arc<Mutex<Vec<Event>>>,
    ) {
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        if socket
            .send(Message::Text(
                serde_json::json!(["AUTH", format!("fixture-{id}")]).to_string(),
            ))
            .await
            .is_err()
        {
            return;
        }
        let mut authenticated = false;
        let mut answered = false;
        let mut subjects = HashSet::new();
        while let Some(Ok(incoming)) = socket.next().await {
            let text = match incoming {
                Message::Text(text) => text,
                Message::Ping(payload) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return;
                    }
                    continue;
                }
                Message::Pong(_) => continue,
                _ => return,
            };
            wire.bytes.fetch_add(text.len(), Ordering::SeqCst);
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            wire.rows.lock().unwrap().push((id, value.clone()));
            let frames = match value[0].as_str() {
                Some("AUTH") => {
                    let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                    event.verify().expect("real signed authentication");
                    assert_eq!(event.kind, Kind::Authentication);
                    authenticated = true;
                    vec![serde_json::json!(["OK", event.id.to_hex(), true, ""]).to_string()]
                }
                Some("REQ") if authenticated => {
                    let filters = value.as_array().unwrap()[2..]
                        .iter()
                        .cloned()
                        .map(serde_json::from_value::<Filter>)
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    subjects.extend(
                        filters
                            .iter()
                            .filter_map(|f| f.authors.as_ref())
                            .flatten()
                            .map(PublicKey::to_hex),
                    );
                    let mut frames = events
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|e| {
                            filters
                                .iter()
                                .any(|f| f.match_event(e, nostr::filter::MatchEventOptions::new()))
                        })
                        .map(|e| serde_json::json!(["EVENT", value[1], e]).to_string())
                        .collect::<Vec<_>>();
                    frames.push(serde_json::json!(["EOSE", value[1]]).to_string());
                    frames
                }
                Some("CLOSE") if authenticated && !answered => {
                    // AUTH causes REQ to be sent twice on one connection. A
                    // finished poll is counted only after its EOSE-driven CLOSE.
                    for author in &subjects {
                        *wire
                            .rounds
                            .lock()
                            .unwrap()
                            .entry(author.clone())
                            .or_default() += 1;
                    }
                    answered = true;
                    Vec::new()
                }
                _ => Vec::new(),
            };
            for frame in frames {
                wire.bytes.fetch_add(frame.len(), Ordering::SeqCst);
                if socket.send(Message::Text(frame)).await.is_err() {
                    return;
                }
            }
        }
    }

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

    fn verify_wire(
        wire: &Wire,
        own: &HashSet<PublicKey>,
        subjects: &HashSet<String>,
        known_identities: &HashSet<PublicKey>,
        expected_discovery: &BTreeMap<String, HashSet<String>>,
        expected_receiver: &HashSet<String>,
    ) -> usize {
        let rows = wire.rows.lock().unwrap();
        let mut auth = HashMap::new();
        let mut keys = HashSet::new();
        for (connection, value) in rows.iter().filter(|(_, row)| row[0] == "AUTH") {
            let event: Event = serde_json::from_value(value[1].clone()).unwrap();
            event.verify().unwrap();
            assert!(
                auth.insert(*connection, event.pubkey).is_none(),
                "one AUTH per disposable connection"
            );
            if !own.contains(&event.pubkey) {
                assert!(
                    keys.insert(event.pubkey),
                    "every anonymous connection needs fresh authentication"
                );
                assert!(
                    !known_identities.contains(&event.pubkey),
                    "known owner/device identity used for anonymous authentication"
                );
            }
        }
        let mut contexts = BTreeMap::<usize, HashSet<String>>::new();
        let mut observed_receiver = HashSet::new();
        for (connection, request) in rows.iter().filter(|(_, row)| row[0] == "REQ") {
            let filters = &request.as_array().unwrap()[2..];
            assert!(!filters.is_empty(), "request must contain a planned filter");
            let signer = auth
                .get(connection)
                .expect("every captured connection authenticates");
            if own.contains(signer) {
                for filter in filters {
                    assert!(
                        expected_receiver.contains(&filter.to_string()),
                        "receiver filter differs from its exact own-account plan"
                    );
                    observed_receiver.insert(filter.to_string());
                }
                for subject in subjects {
                    assert!(
                        !request.to_string().contains(subject),
                        "own-account receiver revealed a peer identity"
                    );
                }
            } else {
                let mut authors = HashSet::new();
                let mut actual_filters = HashSet::new();
                for filter in filters {
                    assert!(
                        filter.get("#p").is_none(),
                        "public author lookup added a recipient selector"
                    );
                    let filter_authors = filter["authors"]
                        .as_array()
                        .expect("every anonymous filter has an author");
                    assert_eq!(
                        filter_authors.len(),
                        1,
                        "every anonymous filter has one subject"
                    );
                    let author = filter_authors[0].as_str().unwrap().to_string();
                    let expected = expected_discovery
                        .get(&author)
                        .expect("unplanned anonymous author");
                    assert!(
                        expected.contains(&filter.to_string()),
                        "anonymous filter differs from its exact public plan"
                    );
                    authors.insert(author);
                    actual_filters.insert(filter.to_string());
                }
                assert_eq!(authors.len(), 1, "one anonymous subject per connection");
                let author = authors.iter().next().unwrap();
                assert_eq!(
                    &actual_filters, &expected_discovery[author],
                    "request omitted planned discovery filters"
                );
                for key in own {
                    assert!(
                        !request.to_string().contains(&key.to_hex()),
                        "own identity in anonymous request"
                    );
                }
                contexts.entry(*connection).or_default().extend(authors);
            }
        }
        assert_eq!(
            &observed_receiver, expected_receiver,
            "own receiver did not send its planned filters"
        );
        assert!(contexts.values().all(|authors| authors.len() == 1));
        assert!(
            rows.iter().all(|(_, row)| row[0] != "EVENT"),
            "discovery must never publish"
        );
        keys.len()
    }

    fn child() {
        let (mut fixture, mut events) = restored_fixture();
        let proof_index = events
            .iter()
            .position(|e| {
                e.kind.as_u16() as u32 == APP_KEYS_EVENT_KIND && e.created_at.as_secs() == 6
            })
            .unwrap();
        let proof = events.remove(proof_index);
        let target = proof.pubkey;
        let own_roster = &fixture.core.app_keys[&fixture
            .core
            .logged_in
            .as_ref()
            .unwrap()
            .owner_pubkey
            .to_hex()];
        let mut own = own_roster
            .devices
            .iter()
            .map(|d| PublicKey::from_hex(&d.identity_pubkey_hex).unwrap())
            .collect::<HashSet<_>>();
        own.insert(fixture.core.logged_in.as_ref().unwrap().owner_pubkey);
        assert_eq!(own.len(), 4, "three own devices plus owner");
        let snapshot = fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot();
        let mut known_identities = own.clone();
        known_identities.insert(snapshot.local_owner_pubkey.to_nostr().unwrap());
        known_identities.insert(snapshot.local_device_pubkey.to_nostr().unwrap());
        for user in &snapshot.users {
            known_identities.insert(user.owner_pubkey.to_nostr().unwrap());
            for device in &user.devices {
                known_identities.insert(device.device_pubkey.to_nostr().unwrap());
                if let Some(owner) = device.claimed_owner_pubkey {
                    known_identities.insert(owner.to_nostr().unwrap());
                }
            }
            if let Some(roster) = &user.roster {
                known_identities.extend(
                    roster
                        .devices
                        .iter()
                        .map(|device| device.device_pubkey.to_nostr().unwrap()),
                );
            }
        }
        let filters = fixture.core.recent_protocol_metadata_filters(unix_now());
        let (receiver, discovery) = fixture.core.split_private_discovery_filters(filters);
        let subjects = discovery
            .iter()
            .flat_map(|f| f.authors.iter().flatten())
            .map(PublicKey::to_hex)
            .collect::<HashSet<_>>();
        assert_eq!(
            subjects.len(),
            CONTACTS * (DEVICES + 1),
            "all persisted owners/devices need real discovery interests"
        );
        assert!(subjects.contains(&target.to_hex()));
        assert!(
            snapshot
                .users
                .iter()
                .flat_map(|user| &user.devices)
                .filter(|device| !device.authorized && device.is_stale)
                .map(|device| device.device_pubkey.to_nostr().unwrap())
                .any(|key| known_identities.contains(&key)
                    && !own.contains(&key)
                    && !subjects.contains(&key.to_hex())),
            "authentication deny set must include identities outside active discovery subjects"
        );
        // Freeze the exact serialized plans before either set is moved into
        // production subscription setup; do not normalize away selectors.
        let mut expected_discovery = BTreeMap::<String, HashSet<String>>::new();
        for filter in &discovery {
            let authors = filter.authors.as_ref().unwrap();
            assert_eq!(authors.len(), 1);
            expected_discovery
                .entry(authors.first().unwrap().to_hex())
                .or_default()
                .insert(serde_json::to_value(filter).unwrap().to_string());
        }
        let (sender, incoming) = flume::unbounded();
        fixture.core.core_sender = sender;
        let runtime = fixture.core.runtime.handle().clone();
        let wire = Arc::new(Wire::default());
        let events = Arc::new(Mutex::new(events));
        let ticks_per_second = tick_rate();
        runtime.block_on(async {
            // The existing account receiver stays on its own socket. Private
            // discovery exercises real certificate verification and TLS tickets.
            let receiver_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let receiver_relay = RelayUrl::parse(&format!("ws://{}", receiver_listener.local_addr().unwrap())).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let relay = RelayUrl::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
            let acceptor = tls_tests::acceptor(true, false);
            let server_wire = wire.clone();
            let server_events = events.clone();
            let server = tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (stream, _) = accepted.unwrap();
                            let id = server_wire.connections.fetch_add(1, Ordering::SeqCst);
                            let acceptor = acceptor.clone();
                            let wire = server_wire.clone();
                            let events = server_events.clone();
                            connections.spawn(async move {
                                // A worker can start before its test-only trust
                                // root is installed; normal rejection is retained.
                                let Ok(stream) = acceptor.accept(stream).await else { return; };
                                let tls = stream.get_ref().1;
                                assert_eq!(tls.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
                                assert!(tls.negotiated_key_exchange_group().is_some(), "resumption must retain fresh key exchange");
                                match tls.handshake_kind().unwrap() {
                                    rustls::HandshakeKind::Full => { wire.tls_full.fetch_add(1, Ordering::SeqCst); },
                                    rustls::HandshakeKind::Resumed => { wire.tls_resumed.fetch_add(1, Ordering::SeqCst); },
                                    other => panic!("unexpected TLS handshake {other:?}"),
                                }
                                connection(stream, id, wire, events).await;
                            });
                        },
                        accepted = receiver_listener.accept() => {
                            let (stream, _) = accepted.unwrap();
                            let id = server_wire.connections.fetch_add(1, Ordering::SeqCst);
                            connections.spawn(connection(stream, id, server_wire.clone(), server_events.clone()));
                        },
                        joined = connections.join_next(), if !connections.is_empty() => {
                            joined.unwrap().expect("serialized-wire fixture task failed");
                        }
                    }
                }
            });
            let client = fixture.core.logged_in.as_ref().unwrap().client.clone();
            client.add_relay(receiver_relay).await.unwrap();
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            let receiver = receiver.into_iter().filter(|f| f.authors.as_ref().is_some_and(|authors| authors.iter().all(|key| own.contains(key)))).collect::<Vec<_>>();
            assert!(!receiver.is_empty());
            let expected_receiver = receiver.iter().map(|filter| serde_json::to_value(filter).unwrap().to_string()).collect::<HashSet<_>>();
            for filter in receiver {
                client.subscribe(filter, None).await.unwrap();
            }
            let scopes = fixture.core.private_discovery_subscriptions(discovery).unwrap();
            let workers = scopes.iter().map(|(worker, _)| worker.upgrade().unwrap()).collect::<Vec<_>>();
            super::super::protocol::private_reads::apply_private_discovery_subscriptions(scopes, std::slice::from_ref(&relay)).await.unwrap();
            for worker in workers {
                worker.set_test_tls_roots(&relay, tls_tests::certificates()).unwrap();
            }
            let settling = Instant::now();
            loop {
                assert!(!server.is_finished(), "wire fixture failed");
                let ready = {
                    let rounds = wire.rounds.lock().unwrap();
                    subjects.iter().all(|author| rounds.get(author).copied().unwrap_or(0) >= 3)
                };
                while incoming.try_recv().is_ok() {}
                if ready { break; }
                assert!(settling.elapsed() < Duration::from_secs(120), "every populated scope must reach its quiet interval");
                sleep(Duration::from_millis(100)).await;
            }
            // EOSE and CLOSE must finish before snapshots; no traffic from the
            // just-completed third round may leak into the unchanged sample.
            sleep(Duration::from_secs(1)).await;
            let settle_seconds = settling.elapsed().as_secs_f64();
            let before = wire.rounds.lock().unwrap().clone();
            let readiness_round_min = subjects.iter().map(|author| before[author]).min().unwrap();
            let completed_peer_polls: usize = subjects.iter().map(|author| before[author]).sum();
            let mut samples = Vec::new();
            for _ in 0..2 {
                let bytes = wire.bytes.load(Ordering::SeqCst);
                let connections = wire.connections.load(Ordering::SeqCst);
                let ticks = cpu_ticks();
                let started = Instant::now();
                sleep(Duration::from_secs(35)).await;
                let seconds = started.elapsed().as_secs_f64();
                let cpu = (cpu_ticks() - ticks) as f64 / ticks_per_second * 100.0 / seconds;
                let byte_delta = wire.bytes.load(Ordering::SeqCst) - bytes;
                let connection_delta = wire.connections.load(Ordering::SeqCst) - connections;
                let sample = serde_json::json!({"seconds": seconds, "cpu_percent_one_core": cpu,
                    "application_wire_bytes": byte_delta, "connections": connection_delta});
                println!("POPULATED_DISCOVERY_IDLE_SAMPLE={sample}");
                samples.push(sample);
                assert!(!server.is_finished());
                assert!(cpu <= 5.0, "populated unchanged discovery exceeded 5% of one CPU: {cpu:.3}");
                assert_eq!(byte_delta, 0, "quiet discovery repeated signed metadata verification/network work");
                assert_eq!(connection_delta, 0, "quiet discovery reopened anonymous sockets");
            }
            assert_eq!(*wire.rounds.lock().unwrap(), before);
            // The old 70s measurement ended before the 120s timer fired. Keep
            // that quiet-window assertion, then cover two actual steady polls
            // for every author, including TLS, authentication and verification.
            let mut cycle_samples = Vec::new();
            for _ in 0..2 {
                let baseline = wire.rounds.lock().unwrap().clone();
                let full = wire.tls_full.load(Ordering::SeqCst);
                let resumed = wire.tls_resumed.load(Ordering::SeqCst);
                let bytes = wire.bytes.load(Ordering::SeqCst);
                let connections = wire.connections.load(Ordering::SeqCst);
                let ticks = cpu_ticks();
                let started = Instant::now();
                sleep(Duration::from_secs(125)).await;
                let seconds = started.elapsed().as_secs_f64();
                let cpu = (cpu_ticks() - ticks) as f64 / ticks_per_second * 100.0 / seconds;
                let after = wire.rounds.lock().unwrap().clone();
                for author in &subjects {
                    let delta = after[author] - baseline[author];
                    assert!((1..=2).contains(&delta), "each full cycle must include bounded real polls for every author");
                }
                let full_delta = wire.tls_full.load(Ordering::SeqCst) - full;
                let resumed_delta = wire.tls_resumed.load(Ordering::SeqCst) - resumed;
                let sample = serde_json::json!({"seconds": seconds, "cpu_percent_one_core": cpu,
                    "application_wire_bytes": wire.bytes.load(Ordering::SeqCst) - bytes,
                    "connections": wire.connections.load(Ordering::SeqCst) - connections,
                    "tls_full_handshakes": full_delta, "tls_resumed_handshakes": resumed_delta,
                    "every_author_polled": true});
                println!("POPULATED_DISCOVERY_CYCLE_SAMPLE={sample}");
                cycle_samples.push(sample);
                assert!(!server.is_finished(), "TLS wire fixture failed");
                assert_eq!(full_delta, 0, "quiet scopes repeated full TLS handshakes");
                assert!(resumed_delta >= subjects.len(), "measurement missed the recurring TLS work");
                assert!(cpu <= 5.0, "populated full-cycle discovery exceeded 5% of one CPU: {cpu:.3}");
            }
            let before = wire.rounds.lock().unwrap().clone();
            while incoming.try_recv().is_ok() {}
            events.lock().unwrap().push(proof.clone());
            fixture.core.refresh_private_discovery_for_author(target);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(CoreMsg::Internal(event)) = incoming.try_recv() {
                    if matches!(*event, InternalEvent::RelayEvent(ref found) if found.id == proof.id) { break; }
                }
                assert!(Instant::now() < deadline, "targeted proof demand did not deliver its fresh signed roster");
                sleep(Duration::from_millis(5)).await;
            }
            let target = target.to_hex();
            let mut expected_after = before.clone();
            *expected_after.get_mut(&target).unwrap() += 1;
            loop {
                assert!(!server.is_finished(), "wire fixture failed");
                let after = wire.rounds.lock().unwrap().clone();
                for (author, count) in &after {
                    let expected = expected_after.get(author).expect("targeted demand introduced an unrelated scope");
                    if author == &target {
                        assert!(count <= expected, "targeted demand duplicated its poll");
                    } else {
                        assert_eq!(count, expected, "targeted demand woke an unrelated contact");
                    }
                }
                if after == expected_after { break; }
                assert!(Instant::now() < deadline, "targeted proof poll did not finish its CLOSE acknowledgement");
                sleep(Duration::from_millis(5)).await;
            }
            // This is only an extra-work observation window after the actual
            // acknowledgement, never a substitute for poll completion.
            sleep(Duration::from_millis(100)).await;
            assert_eq!(*wire.rounds.lock().unwrap(), expected_after,
                "targeted demand woke an unrelated contact or was duplicated");
            let auth_count = verify_wire(&wire, &own, &subjects, &known_identities, &expected_discovery, &expected_receiver);
            println!("POPULATED_DISCOVERY_IDLE_RESULT={}", serde_json::json!({
                "database_bytes": fixture.database_bytes, "message_rows": fixture.rows,
                "contacts": CONTACTS, "peer_devices": CONTACTS * DEVICES, "own_devices": 3,
                "groups": 8, "discovery_subjects": subjects.len(), "settle_seconds": settle_seconds,
                "readiness_round_min": readiness_round_min, "completed_peer_polls": completed_peer_polls,
                "quiet_samples": samples, "full_cycle_tls_samples": cycle_samples,
                "targeted_fresh_proof_delivered": true,
                "fresh_anonymous_authentications": auth_count, "privacy_wire_passed": true,
                "revoked_stale_records": fixture.core.protocol_engine.as_ref().unwrap().session_manager_snapshot()
                    .users.iter().flat_map(|user| &user.devices).filter(|device| !device.authorized && device.is_stale).count(),
                "fixture_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("synthetic_history_scale.rs"))),
                "test_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("populated_discovery_idle.rs")))
            }));
            fixture.core.private_discovery_subscriptions(Vec::new()).unwrap();
            client.shutdown().await;
            server.abort();
            let _ = server.await;
        });
        fixture.core.shutdown();
    }

    #[test]
    fn populated_account_discovery_stays_quiet_and_targeted_demand_progresses() {
        if std::env::var(CHILD).as_deref() == Ok("1") {
            child();
            return;
        }
        let artifacts = std::env::var_os("IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT")
            .map(std::path::PathBuf::from)
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        if let Some(directory) = &artifacts {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("populated-discovery-idle.json"),
                b"{\"schema\":\"iris-populated-discovery-idle/v1\",\"passed\":false,\"status\":\"starting\"}\n").unwrap();
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
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
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
            "schema": "iris-populated-discovery-idle/v1", "passed": passed,
            "timed_out": timed_out, "exit_code": status.code(),
            "seconds": started.elapsed().as_secs_f64(),
            "measurements": measurements, "log_bytes": log.len(),
            "log_sha256": format!("{:x}", sha2::Sha256::digest(log.as_bytes())),
        });
        if let Some(directory) = &artifacts {
            std::fs::write(directory.join("populated-discovery-idle.log"), &log).unwrap();
            std::fs::write(
                directory.join("populated-discovery-idle.json"),
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
