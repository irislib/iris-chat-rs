// Foreground catch-up and retained discovery share the ordinary supervisor.
// Accounts, durable history and every relay below are generated locally.
mod foreground_discovery {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use nostr::EventId;
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::Message;

    const TEST: &str = "core::tests::foreground_discovery::foreground_catch_up_preserves_discovery_scopes_and_finishes_with_a_stalled_relay";
    const CHILD: &str = "IRIS_TEST_FOREGROUND_DISCOVERY_CHILD";
    type Wire = Arc<Mutex<Vec<(usize, usize, Value)>>>;

    async fn serve(
        stream: TcpStream,
        relay: usize,
        connection: usize,
        wire: Wire,
        events: Arc<Vec<Event>>,
        late_profile: EventId,
        late_roster: EventId,
    ) {
        if relay == 4 {
            // Accept TCP but stall the handshake: exercise the real network
            // timeout, with no external host or shortened production budget.
            let _stream = stream;
            std::future::pending::<()>().await;
            return;
        }
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let challenge = format!("foreground-{relay}-{connection}");
        if socket
            .send(Message::Text(json!(["AUTH", challenge]).to_string()))
            .await
            .is_err()
        {
            return;
        }
        let mut authenticated = false;
        while let Some(Ok(frame)) = socket.next().await {
            let text = match frame {
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
            let value: Value = serde_json::from_str(&text).unwrap();
            wire.lock()
                .unwrap()
                .push((relay, connection, value.clone()));
            let mut replies = Vec::new();
            match value[0].as_str() {
                Some("AUTH") => {
                    let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                    event.verify().unwrap();
                    assert_eq!(event.kind, Kind::Authentication);
                    assert!(event
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["challenge", challenge.as_str()]));
                    authenticated = true;
                    replies.push(json!(["OK", event.id, true, ""]));
                }
                Some("REQ") if authenticated => {
                    let filters = value.as_array().unwrap()[2..]
                        .iter()
                        .cloned()
                        .map(serde_json::from_value::<Filter>)
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    if relay == 3 {
                        tokio::time::sleep(Duration::from_millis(750)).await;
                    }
                    for event in events.iter() {
                        if relay != 3 && (event.id == late_profile || event.id == late_roster) {
                            continue;
                        }
                        if filters.iter().any(|filter| {
                            filter.match_event(event, nostr::filter::MatchEventOptions::new())
                        }) {
                            replies.push(json!(["EVENT", value[1], event]));
                        }
                    }
                    replies.push(json!(["EOSE", value[1]]));
                }
                Some("REQ") => replies.push(json!(["CLOSED", value[1], "auth-required: fixture"])),
                Some("EVENT") => {
                    let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                    event.verify().unwrap();
                    replies.push(json!(["OK", event.id, true, ""]));
                }
                _ => {}
            }
            for reply in replies {
                if socket.send(Message::Text(reply.to_string())).await.is_err() {
                    return;
                }
            }
        }
    }

    fn assert_wire(
        wire: &Wire,
        own: &HashSet<String>,
        identities: &HashSet<String>,
        stable: &BTreeMap<String, BTreeSet<String>>,
    ) -> (BTreeSet<(usize, String)>, bool, bool) {
        let rows = wire.lock().unwrap();
        let mut authentication = BTreeMap::new();
        let mut anonymous_keys = HashSet::new();
        for (_, connection, value) in rows.iter().filter(|(_, _, value)| value[0] == "AUTH") {
            let event: Event = serde_json::from_value(value[1].clone()).unwrap();
            event.verify().unwrap();
            let signer = event.pubkey.to_hex();
            if let Some(previous) = authentication.insert(*connection, signer.clone()) {
                assert_eq!(previous, signer);
            } else if !own.contains(&signer) {
                assert!(
                    !identities.contains(&signer),
                    "peer identity authenticated a read"
                );
                assert!(
                    anonymous_keys.insert(signer),
                    "anonymous authentication reused across connections"
                );
            }
        }
        let mut contexts = BTreeMap::<usize, BTreeSet<String>>::new();
        let mut observed = BTreeSet::new();
        let mut private_history = false;
        let mut recipient_catch_up = false;
        for (relay, connection, request) in rows.iter().filter(|(_, _, value)| value[0] == "REQ") {
            // Inspect every serialized request, including rejected/unfinished
            // authentication. Signer checks apply once AUTH is observed.
            let signer = authentication.get(connection);
            for value in &request.as_array().unwrap()[2..] {
                let filter: Filter = serde_json::from_value(value.clone()).unwrap();
                let authors = filter
                    .authors
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .map(PublicKey::to_hex)
                    .collect::<Vec<_>>();
                let public = authors
                    .iter()
                    .filter(|author| stable.contains_key(*author))
                    .collect::<Vec<_>>();
                if !public.is_empty() {
                    assert!(
                        signer.is_none_or(|signer| !own.contains(signer)),
                        "account authentication exposed peer discovery"
                    );
                    assert_eq!(
                        authors.len(),
                        1,
                        "public discovery must isolate each author"
                    );
                    assert!(
                        value.get("#p").is_none(),
                        "public discovery exposed a recipient"
                    );
                    assert!(
                        own.iter().all(|key| !request.to_string().contains(key)),
                        "public discovery exposed our identity"
                    );
                    let author = public[0];
                    // Profiles may be uncovered cold lookups. Covered public
                    // metadata must retain its exact stable selectors instead
                    // of also issuing broad cold since/limit queries.
                    let profile_only = value["kinds"] == json!([0])
                        && value.get("since").is_none()
                        && value.get("limit").is_none();
                    // Foreground also reconciles the isolated FIPS fixture;
                    // its advert lookup is not protocol metadata catch-up.
                    let mesh_only = value["kinds"] == json!([37195])
                        && value.get("since").is_none()
                        && value.get("limit").is_none();
                    assert!(profile_only || mesh_only || stable[author].contains(&value.to_string()),
                        "covered public metadata duplicated through broad cold catch-up: relay={relay}, filter={value}");
                    contexts
                        .entry(*connection)
                        .or_default()
                        .insert(author.clone());
                    if stable[author].contains(&value.to_string()) {
                        observed.insert((*relay, author.clone()));
                    }
                } else if signer.is_some_and(|signer| !own.contains(signer)) {
                    let kinds = filter.kinds.as_ref().unwrap();
                    private_history |= kinds.contains(&Kind::from(MESSAGE_EVENT_KIND as u16));
                    recipient_catch_up |= kinds.contains(&Kind::from(INVITE_RESPONSE_KIND as u16))
                        && value.get("#p").is_some()
                        && value.get("since").is_some();
                }
            }
        }
        for (connection, authors) in contexts {
            assert_eq!(
                authors.len(),
                1,
                "a read context linked different public authors"
            );
            assert!(
                rows.iter()
                    .all(|(_, id, value)| *id != connection || value[0] != "EVENT"),
                "a discovery connection also published"
            );
        }
        (observed, private_history, recipient_catch_up)
    }

    struct RunningApp(Arc<crate::FfiApp>);
    impl Drop for RunningApp {
        fn drop(&mut self) {
            self.0.shutdown_and_wait();
        }
    }

    #[test]
    fn foreground_catch_up_preserves_discovery_scopes_and_finishes_with_a_stalled_relay() {
        if std::env::var(CHILD).as_deref() == Ok("1") {
            foreground_case();
            return;
        }
        // Cold and retained read admission are process-global. Other tests
        // must not occupy this fixture's slots while its real deadlines run.
        // Files preserve complete failure output without filling a pipe.
        let logs = tempfile::tempdir().unwrap();
        let stdout = logs.path().join("stdout.log");
        let stderr = logs.path().join("stderr.log");
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                if !matches!(self.0.try_wait(), Ok(Some(_))) {
                    let _ = self.0.kill();
                }
                let _ = self.0.wait();
            }
        }
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
                .env(CHILD, "1")
                .stdout(std::fs::File::create(&stdout).unwrap())
                .stderr(std::fs::File::create(&stderr).unwrap())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        let timed_out = loop {
            if child.0.try_wait().unwrap().is_some() {
                break false;
            }
            // Separate setup/teardown allowance; the foreground assertion
            // below retains its original 45-second deadline.
            if started.elapsed() >= Duration::from_secs(90) {
                child.0.kill().unwrap();
                break true;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let status = child.0.wait().unwrap();
        eprintln!(
            "foreground discovery child status={status} timed_out={timed_out}\nstdout:\n{}\nstderr:\n{}",
            std::fs::read_to_string(stdout).unwrap(),
            std::fs::read_to_string(stderr).unwrap(),
        );
        assert!(
            !timed_out,
            "foreground fixture exceeded its parent deadline"
        );
        assert!(status.success(), "foreground discovery child failed");
    }

    fn phase(started: Instant, foreground: Option<Instant>, name: &str) {
        println!(
            "FOREGROUND_DISCOVERY_PHASE={}",
            json!({"phase": name, "elapsed_ms": started.elapsed().as_millis(),
                "foreground_ms": foreground.map(|time| time.elapsed().as_millis())})
        );
    }

    fn foreground_case() {
        let started = Instant::now();
        let (mut fixture, events) = synthetic_history_fixture_with_devices(1, 8, 0, 3);
        phase(started, None, "fixture_created");
        let late_roster = events
            .iter()
            .find(|event| {
                event.kind.as_u16() as u32 == APP_KEYS_EVENT_KIND && event.created_at.as_secs() == 6
            })
            .unwrap();
        let late_profile = events
            .iter()
            .find(|event| event.kind == Kind::Metadata && event.pubkey == late_roster.pubkey)
            .unwrap();
        let target = late_profile.pubkey.to_hex();
        let expected_name: Value = serde_json::from_str(&late_profile.content).unwrap();
        let late_profile_id = late_profile.id;
        let late_roster_id = late_roster.id;
        fixture.core.owner_profiles.remove(&target);
        let local = fixture.core.logged_in.as_ref().unwrap();
        let mut own = HashSet::from([
            local.owner_pubkey.to_hex(),
            local.device_keys.public_key().to_hex(),
        ]);
        own.extend(
            fixture.core.app_keys[&local.owner_pubkey.to_hex()]
                .devices
                .iter()
                .map(|device| device.identity_pubkey_hex.clone()),
        );
        let snapshot = fixture
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot();
        let mut identities = own.clone();
        for user in &snapshot.users {
            identities.insert(user.owner_pubkey.to_nostr().unwrap().to_hex());
            identities.extend(
                user.devices
                    .iter()
                    .map(|device| device.device_pubkey.to_nostr().unwrap().to_hex()),
            );
        }
        let plan = fixture.core.compute_protocol_subscription_plan().unwrap();
        assert!(
            !plan.message_authors.is_empty(),
            "fixture needs real session history selectors"
        );
        assert!(plan.invite_response_recipient.is_some());
        let (_, discovery) = fixture
            .core
            .split_private_discovery_filters(build_protocol_subscription_filters(&plan));
        let mut stable = BTreeMap::<String, BTreeSet<String>>::new();
        for filter in discovery {
            for author in filter.authors.as_ref().unwrap() {
                stable
                    .entry(author.to_hex())
                    .or_default()
                    .insert(serde_json::to_value(&filter).unwrap().to_string());
            }
        }
        assert_eq!(stable.len(), 8 * 4);
        assert!(
            fixture
                .core
                .protocol_subscription_runtime
                .discovery_clients
                .is_empty(),
            "first foreground must precede retained worker configuration"
        );

        let relay_runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        // Keep sibling-device reconciliation in the real foreground path,
        // using its established isolation hook. This suppresses default
        // WebRTC and websocket seeds and reserves a task-local rendezvous.
        let rendezvous = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(address) = rendezvous.local_addr().unwrap() else {
            unreachable!()
        };
        fixture.core.test_fips_rendezvous_addr = Some(address);
        let wire: Wire = Arc::default();
        let events = Arc::new(events);
        let connections = Arc::new(AtomicUsize::new(0));
        let relays = relay_runtime.block_on(async {
            let mut relays = Vec::new();
            for relay in 0..5 {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                relays.push(
                    RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap(),
                );
                let wire = wire.clone();
                let events = events.clone();
                let connections = connections.clone();
                tokio::spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        let id = connections.fetch_add(1, Ordering::SeqCst);
                        tokio::spawn(serve(
                            stream,
                            relay,
                            id,
                            wire.clone(),
                            events.clone(),
                            late_profile_id,
                            late_roster_id,
                        ));
                    }
                });
            }
            relays
        });
        fixture.core.preferences.nostr_relay_urls =
            relays.iter().map(ToString::to_string).collect();
        fixture.core.logged_in.as_mut().unwrap().relay_urls = relays;
        let app = RunningApp(synthetic_history_app(fixture.core, fixture.updates));
        let foreground = Instant::now();
        phase(started, Some(foreground), "foreground_dispatched");
        app.0.dispatch(AppAction::AppForegrounded);
        let deadline = foreground + Duration::from_secs(45);
        let mut saw_syncing = false;
        let mut phases = [false; 5];
        loop {
            let (observed, history, recipients) = assert_wire(&wire, &own, &identities, &stable);
            saw_syncing |= app.0.state().busy.syncing_network;
            let all_healthy = (0..4).all(|relay| {
                stable
                    .keys()
                    .all(|author| observed.contains(&(relay, author.clone())))
            });
            let profile_delivered = app.0.user_profile(target.clone()).name.name
                == expected_name["name"].as_str().unwrap();
            for (index, (complete, name)) in [
                (all_healthy, "healthy_discovery_observed"),
                (history, "private_history_observed"),
                (recipients, "recipient_catch_up_observed"),
                (profile_delivered, "delayed_profile_delivered"),
                (
                    saw_syncing && !app.0.state().busy.syncing_network,
                    "sync_completed",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                if complete && !phases[index] {
                    phase(started, Some(foreground), name);
                    phases[index] = true;
                }
            }
            if all_healthy
                && history
                && recipients
                && profile_delivered
                && saw_syncing
                && !app.0.state().busy.syncing_network
            {
                break;
            }
            assert!(Instant::now() < deadline,
                "foreground did not complete: healthy={all_healthy}, history={history}, recipients={recipients}, profile={profile_delivered}, observed_syncing={saw_syncing}");
            std::thread::sleep(Duration::from_millis(25));
        }
        let support: Value = serde_json::from_str(&app.0.export_support_bundle_json()).unwrap();
        assert_eq!(
            support["protocol_subscription"]["protocol_fetch_in_flight"], false,
            "ordinary SyncComplete must clear foreground catch-up"
        );
        assert!(support["event_counts"]["app_keys_events"].as_u64().unwrap() > 0);
        assert_eq!(app.0.recovery.restart_count.load(Ordering::Relaxed), 0);
        assert_wire(&wire, &own, &identities, &stable);
        phase(started, Some(foreground), "foreground_assertions_complete");
        app.0.shutdown_and_wait();
        drop(app);
        drop(relay_runtime);
        phase(started, Some(foreground), "runtime_closed");
        let database = rusqlite::Connection::open_with_flags(
            fixture.directory.path().join("core.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let persisted_roster: u64 = database
            .query_row(
                "SELECT created_at_secs FROM app_keys WHERE owner_pubkey_hex = ?1",
                [&target],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            persisted_roster, 6,
            "the delayed signed roster must reach durable state"
        );
        drop(database);
        drop(fixture.directory);
        phase(started, Some(foreground), "durable_roster_verified");
    }
}
