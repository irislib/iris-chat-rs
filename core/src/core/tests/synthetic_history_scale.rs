// Opt-in real SQLite/FFI benchmark, using generated identities and messages only:
// IRIS_SYNTHETIC_HISTORY_MIB=100 cargo test --manifest-path core/Cargo.toml \
//   --lib synthetic_history_scale_benchmark -- --ignored --nocapture --test-threads=1
// Also accepts 1024 and 3072 MiB. Run with a release profile for product timings.
// Generation is outside every measured interval; no existing account is opened.

struct SyntheticHistory {
    core: AppCore,
    updates: flume::Receiver<AppUpdate>,
    directory: tempfile::TempDir,
    chat_ids: Vec<String>,
    rows: u64,
    database_bytes: u64,
}

fn synthetic_history_free_space(path: &std::path::Path, required: u64) {
    // The opt-in scale benchmark runs on Unix hosts. Small CI fixtures also run
    // on other platforms and have a fixed, low allocation bound.
    #[cfg(unix)]
    {
        let output = std::process::Command::new("df")
            .args(["-Pk"])
            .arg(path)
            .output()
            .expect("inspect fixture filesystem space");
        assert!(output.status.success());
        let free_kib: u64 = std::str::from_utf8(&output.stdout)
            .unwrap()
            .lines()
            .last()
            .unwrap()
            .split_whitespace()
            .nth(3)
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            free_kib * 1024 >= required,
            "insufficient space for synthetic fixture and 6 GiB reserve"
        );
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        assert!(
            required <= 6 * 1024_u64.pow(3) + 72 * 1024 * 1024,
            "large synthetic fixture requires a Unix host with disk-space guard"
        );
    }
}

fn synthetic_history_fixture(mib: u64, contacts: usize, groups: usize) -> SyntheticHistory {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, directory) =
        logged_in_test_core_with_updates("synthetic-history", &owner, &device);
    let target = mib * 1024 * 1024;
    let reserve = 6 * 1024_u64.pow(3);
    synthetic_history_free_space(directory.path(), reserve + target + 64 * 1024 * 1024);
    core.app_store.bind_account(owner.public_key()).unwrap();
    core.preferences.nearby_enabled = false;
    core.preferences.nearby_bluetooth_enabled = false;
    core.preferences.nearby_lan_enabled = false;
    core.preferences.nostr_relay_urls.clear();
    core.preferences.mobile_push_server_url.clear();
    core.preferences.send_read_receipts = false;
    let peers = (0..contacts).map(|_| Keys::generate()).collect::<Vec<_>>();
    let mut engine = core.protocol_engine.take().unwrap();
    engine.enter_batch();
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let mut chat_ids = Vec::new();
    for (index, peer) in peers.iter().enumerate() {
        let peer_device = Keys::generate();
        observe_peer_device_invite_for_test(&mut engine, peer, &peer_device, 2);
        let chat_id = peer.public_key().to_hex();
        // Establish a real send-capable session, without publishing its effects.
        engine
            .send_direct_text(
                peer.public_key(),
                &chat_id,
                "Synthetic setup",
                None,
                UnixSeconds(3),
            )
            .unwrap();
        core.app_keys.insert(
            chat_id.clone(),
            account_app_keys::known_app_keys_from_ndr(
                peer.public_key(),
                &AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 2)]),
                2,
            ),
        );
        core.owner_profiles.insert(
            chat_id.clone(),
            OwnerProfileRecord {
                name: Some(format!("Synthetic contact {index}")),
                updated_at_secs: unix_now().get(),
                ..Default::default()
            },
        );
        core.preferences
            .accepted_owner_pubkeys
            .push(chat_id.clone());
        chat_ids.push(chat_id);
    }
    for index in 0..groups {
        let snapshot = engine
            .create_group(
                format!("Synthetic group {index}"),
                peers.iter().take(4).map(Keys::public_key).collect(),
                UnixSeconds(4),
            )
            .unwrap()
            .snapshot
            .unwrap();
        let id = group_chat_id(&snapshot.group_id);
        core.groups.insert(snapshot.group_id.clone(), snapshot);
        chat_ids.push(id);
    }
    engine.exit_batch().unwrap();
    core.protocol_engine = Some(engine);
    for id in &chat_ids {
        core.ensure_thread_record(id, 1_700_000_000);
    }
    core.persist_best_effort();
    let database = core.shared_db();
    let mut rows = 0_u64;
    let database_bytes;
    {
        let mut conn = database.lock().unwrap();
        let page_size: u64 = conn
            .pragma_query_value(None, "page_size", |row| row.get(0))
            .unwrap();
        loop {
            synthetic_history_free_space(directory.path(), reserve + 32 * 1024 * 1024);
            let tx = conn.transaction().unwrap();
            {
                let mut insert = tx.prepare_cached(
                    "INSERT INTO messages(chat_id,id,kind,author,author_owner_pubkey_hex,body,is_outgoing,created_at_secs,delivery)
                     VALUES (?1,?2,'user','Synthetic author',?3,?4,?5,?6,'seen')"
                ).unwrap();
                for _ in 0..1024 {
                    let index = rows as usize % chat_ids.len();
                    let outgoing = rows.is_multiple_of(3);
                    let author = if outgoing {
                        owner.public_key()
                    } else {
                        let peer_index = if index < contacts {
                            index
                        } else {
                            (index - contacts) % peers.len().min(4)
                        };
                        peers[peer_index].public_key()
                    };
                    let repetitions = match rows % 64 {
                        0 => 460, // occasional 16 KiB message, below the app's limit
                        1..=8 => 60,
                        _ => 4 + rows as usize % 12,
                    };
                    let body = format!(
                        "Synthetic message {rows} in conversation {index}. {}",
                        "A quiet fox brings tea to the moon. ".repeat(repetitions)
                    );
                    insert
                        .execute(rusqlite::params![
                            chat_ids[index],
                            rows.to_string(),
                            author.to_hex(),
                            body,
                            outgoing,
                            1_700_000_000 + rows
                        ])
                        .unwrap();
                    rows += 1;
                }
            }
            tx.commit().unwrap();
            let pages: u64 = conn
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .unwrap();
            if pages * page_size >= target && rows >= chat_ids.len() as u64 * 200 {
                break;
            }
        }
        conn.execute("UPDATE threads SET updated_at_secs=(SELECT MAX(created_at_secs) FROM messages WHERE messages.chat_id=threads.chat_id)", []).unwrap();
        conn.execute(
            "UPDATE app_meta SET value=?1 WHERE key='next_message_id'",
            [rows.to_string()],
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let stored: u64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, rows);
        let indexed: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages_fts WHERE messages_fts MATCH 'quiet')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(indexed, "the production full-text index must be populated");
        database_bytes = std::fs::metadata(directory.path().join("core.sqlite3"))
            .unwrap()
            .len();
    }
    core.next_message_id = rows;
    // Production restoration retains bounded recent pages, not the whole DB.
    let restored = core.load_persisted().unwrap().unwrap();
    for thread in restored.threads {
        core.threads.insert(
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
        );
    }
    core.rebuild_state();
    core.emit_state();
    SyntheticHistory {
        core,
        updates,
        directory,
        chat_ids,
        rows,
        database_bytes,
    }
}

fn synthetic_history_app(
    mut core: AppCore,
    updates: flume::Receiver<AppUpdate>,
) -> Arc<crate::FfiApp> {
    // Use the ordinary supervisor/priority queues while retaining a completely
    // offline test account. No second writer or mock page reader is introduced.
    let mut app = crate::ffi_app_failure(String::new());
    let slot = Arc::get_mut(&mut app).unwrap();
    slot.update_rx = updates;
    slot.shared_state = core.shared_state.clone();
    core.core_sender = slot.background_tx.clone();
    core.priority_sender = slot.foreground_tx.clone();
    crate::set_shared_db(&slot.shared_db, Some(core.shared_db()));
    let supervisor = crate::CoreSupervisor {
        data_dir: core.data_dir.to_string_lossy().into_owned(),
        update_tx: core.update_tx.clone(),
        core_sender: slot.background_tx.clone(),
        priority_sender: slot.foreground_tx.clone(),
        foreground_rx: slot.foreground_rx.clone(),
        background_rx: slot.background_rx.clone(),
        shared_state: slot.shared_state.clone(),
        shared_db: slot.shared_db.clone(),
        queue_metrics: slot.queue_metrics.clone(),
        recovery: slot.recovery.clone(),
    };
    *slot.core_worker.lock().unwrap() =
        Some(crate::spawn_core_supervisor(core, supervisor).unwrap());
    app
}

fn synthetic_history_wait(app: &crate::FfiApp, predicate: impl Fn(&AppState) -> bool) -> AppState {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let update = app
            .update_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("synthetic foreground action must complete");
        if let AppUpdate::FullState(state) = update {
            if predicate(&state) {
                return state;
            }
        }
    }
}

fn synthetic_history_distribution(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    serde_json::json!({"samples": values.len(), "median_ms": values[values.len()/2],
        "p95_ms": values[((values.len()*95).div_ceil(100)-1).min(values.len()-1)], "max_ms": values[values.len()-1]})
}

fn synthetic_history_measure(mib: u64, contacts: usize, groups: usize) -> serde_json::Value {
    let generation_started = Instant::now();
    let SyntheticHistory {
        core,
        updates,
        directory,
        chat_ids,
        rows,
        database_bytes,
    } = synthetic_history_fixture(mib, contacts, groups);
    let generation_ms = generation_started.elapsed().as_secs_f64() * 1000.0;
    let app = synthetic_history_app(core, updates);
    let mut first = Vec::new();
    let mut older = Vec::new();
    let mut around = Vec::new();
    let mut open = Vec::new();
    let mut send = Vec::new();
    let mut sent_ids = Vec::new();
    for (index, id) in chat_ids.iter().enumerate() {
        let start = Instant::now();
        let page = app.chat_snapshot(id.clone(), 80).unwrap();
        first.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(page.messages.len(), 80);
        assert!(page
            .messages
            .windows(2)
            .all(|pair| pair[0].created_at_secs <= pair[1].created_at_secs));
        let before = page.messages[0].id.clone();
        let start = Instant::now();
        let older_page = app
            .chat_snapshot_before(id.clone(), before.clone(), 80)
            .unwrap();
        older.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(older_page.messages.len(), 80);
        assert!(
            older_page.messages.last().unwrap().created_at_secs < page.messages[0].created_at_secs
        );
        assert!(older_page
            .messages
            .iter()
            .all(|message| message.body.starts_with("Synthetic message ")));
        let start = Instant::now();
        let middle_id =
            ((rows / chat_ids.len() as u64 / 2) * chat_ids.len() as u64 + index as u64).to_string();
        let surrounding = app
            .chat_snapshot_around_message(id.clone(), middle_id.clone(), 20, 20)
            .unwrap();
        around.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(surrounding.messages.len() <= 41);
        assert!(surrounding
            .messages
            .iter()
            .any(|message| message.id == middle_id));
        while app.update_rx.try_recv().is_ok() {}
        let start = Instant::now();
        app.dispatch(AppAction::OpenChat {
            chat_id: id.clone(),
        });
        synthetic_history_wait(&app, |state| {
            state
                .current_chat
                .as_ref()
                .is_some_and(|chat| chat.chat_id == *id && !chat.messages.is_empty())
        });
        open.push(start.elapsed().as_secs_f64() * 1000.0);
        let body = format!("Synthetic timed send {index}");
        let start = Instant::now();
        app.dispatch(AppAction::SendMessage {
            chat_id: id.clone(),
            text: body.clone(),
        });
        let sent = synthetic_history_wait(&app, |state| {
            state.current_chat.as_ref().is_some_and(|chat| {
                chat.messages.iter().any(|message| {
                    message.is_outgoing
                        && message.body == body
                        && !matches!(message.delivery, DeliveryState::Failed)
                })
            })
        });
        send.push(start.elapsed().as_secs_f64() * 1000.0);
        let sent_id = sent
            .current_chat
            .unwrap()
            .messages
            .into_iter()
            .find(|message| message.body == body && message.is_outgoing)
            .unwrap()
            .id;
        sent_ids.push((id.clone(), sent_id));
    }
    assert_eq!(
        app.recovery
            .restart_count
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    // First-state delivery is intentionally optimistic. Let deferred protocol
    // work finish before testing durability and shutting down the real worker.
    let connection = rusqlite::Connection::open_with_flags(
        directory.path().join("core.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_millis(100)).unwrap();
    let durable_started = Instant::now();
    {
        let mut exists = connection.prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND id=?2 AND is_outgoing=1 AND delivery!='failed')",
        ).unwrap();
        loop {
            let durable_sends = sent_ids
                .iter()
                .filter(|(chat_id, message_id)| {
                    exists
                        .query_row([chat_id, message_id], |row| row.get::<_, bool>(0))
                        .unwrap()
                })
                .count();
            if durable_sends == sent_ids.len() {
                break;
            }
            assert!(
                durable_started.elapsed() < Duration::from_secs(10),
                "every measured send must become durable offline: {durable_sends}/{}",
                sent_ids.len()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let durable_ms = durable_started.elapsed().as_secs_f64() * 1000.0;
    drop(connection);
    let result = serde_json::json!({"synthetic_only": true, "requested_mib": mib,
        "sqlite_bytes": database_bytes, "message_rows": rows, "contacts": contacts, "groups": groups,
        "generation_ms": generation_ms, "pending_ciphertexts": 0,
        "durable_after_final_state_ms": durable_ms,
        "body_distribution": "unique row prefix; repeated synthetic prose; roughly 0.2/0.6/2/16 KiB",
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "ffi_first_page": synthetic_history_distribution(first), "ffi_older_page": synthetic_history_distribution(older),
        "ffi_around_message": synthetic_history_distribution(around), "dispatch_open_to_state": synthetic_history_distribution(open),
        "dispatch_send_to_state": synthetic_history_distribution(send)});
    drop(app);
    drop(directory);
    result
}

#[test]
fn synthetic_history_small_fixture_exercises_real_database_and_ffi() {
    let result = synthetic_history_measure(2, 4, 2);
    assert!(result["sqlite_bytes"].as_u64().unwrap() >= 2 * 1024 * 1024);
    println!("SYNTHETIC_HISTORY_RESULT={result}");
}

#[test]
#[ignore = "generates an opt-in 100 MiB/1 GiB/3 GiB database; requires 6 GiB reserve"]
fn synthetic_history_scale_benchmark() {
    let mib: u64 = std::env::var("IRIS_SYNTHETIC_HISTORY_MIB")
        .expect("choose 100, 1024, or 3072 MiB")
        .parse()
        .unwrap();
    assert!([100, 1024, 3072].contains(&mib));
    let result = synthetic_history_measure(mib, 69, 13);
    println!("SYNTHETIC_HISTORY_RESULT={result}");
}
