use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct SqlMetrics {
    writes_by_table: Arc<Mutex<BTreeMap<String, usize>>>,
    vm_steps: Arc<AtomicUsize>,
    commits: Arc<AtomicUsize>,
}

impl SqlMetrics {
    fn attach(core: &AppCore) -> Self {
        let writes_by_table = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let vm_steps = Arc::new(AtomicUsize::new(0));
        let commits = Arc::new(AtomicUsize::new(0));
        let sink = writes_by_table.clone();
        let steps_sink = vm_steps.clone();
        let commits_sink = commits.clone();
        let db = core.app_store.shared();
        let conn = db.lock().unwrap();
        conn.update_hook(Some(move |_, _: &str, table: &str, _: i64| {
            *sink.lock().unwrap().entry(table.to_string()).or_default() += 1;
        }));
        conn.progress_handler(
            1,
            Some(move || {
                steps_sink.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        conn.commit_hook(Some(move || {
            commits_sink.fetch_add(1, Ordering::Relaxed);
            false
        }));
        Self {
            writes_by_table,
            vm_steps,
            commits,
        }
    }

    fn reset(&self) {
        self.writes_by_table.lock().unwrap().clear();
        self.vm_steps.store(0, Ordering::Relaxed);
        self.commits.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (usize, usize, BTreeMap<String, usize>) {
        let steps = self.vm_steps.load(Ordering::Relaxed);
        let commits = self.commits.load(Ordering::Relaxed);
        let tables = self.writes_by_table.lock().unwrap().clone();
        (steps, commits, tables)
    }

    fn detach(core: &AppCore) {
        let db = core.app_store.shared();
        let conn = db.lock().unwrap();
        conn.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
        conn.progress_handler(0, None::<fn() -> bool>);
        conn.commit_hook(None::<fn() -> bool>);
    }
}

fn summarize_writes(tables: &BTreeMap<String, usize>) -> serde_json::Value {
    let total: usize = tables.values().sum();
    let fts: usize = tables
        .iter()
        .filter(|(k, _)| k.starts_with("messages_fts"))
        .map(|(_, v)| *v)
        .sum();
    serde_json::json!({
        "total_row_writes": total,
        "threads": tables.get("threads").copied().unwrap_or(0),
        "messages": tables.get("messages").copied().unwrap_or(0),
        "messages_fts": fts,
        "owner_profiles": tables.get("owner_profiles").copied().unwrap_or(0),
        "app_keys": tables.get("app_keys").copied().unwrap_or(0),
        "groups": tables.get("groups").copied().unwrap_or(0),
        "seen_events": tables.get("seen_events").copied().unwrap_or(0),
        "app_meta": tables.get("app_meta").copied().unwrap_or(0),
    })
}

fn wal_bytes(dir: &std::path::Path) -> u64 {
    std::fs::metadata(dir.join("core.sqlite3-wal"))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn checkpoint_wal(core: &AppCore) {
    let db = core.app_store.shared();
    let conn = db.lock().unwrap();
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}

fn median_ms(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn populate_realistic_graph_and_decisions(core: &mut AppCore, chat_ids: &[String]) {
    let owner_hex = core.logged_in.as_ref().unwrap().owner_pubkey.to_hex();
    let mut graph = nostr_social_graph::SocialGraph::from_binary(
        &owner_hex,
        include_bytes!("../../../assets/socialGraph.bin"),
    )
    .unwrap();
    // Follow the first 40 direct contacts, mute 5 others, and create realistic 2nd-hop edges.
    let direct: Vec<String> = chat_ids
        .iter()
        .filter(|id| !is_group_chat_id(id))
        .cloned()
        .collect();
    let tags: Vec<Vec<String>> = direct
        .iter()
        .take(40)
        .map(|peer| vec!["p".into(), peer.clone()])
        .collect();
    graph.handle_event(
        &nostr_social_graph::NostrEvent {
            pubkey: owner_hex.clone(),
            kind: 3,
            created_at: 100,
            id: format!("{:064x}", 100),
            tags,
            content: String::new(),
            sig: String::new(),
        },
        true,
        1.0,
    );
    let mute_tags: Vec<Vec<String>> = direct
        .iter()
        .skip(40)
        .take(5)
        .map(|peer| vec!["p".into(), peer.clone()])
        .collect();
    graph.handle_event(
        &nostr_social_graph::NostrEvent {
            pubkey: owner_hex.clone(),
            kind: 10_000,
            created_at: 101,
            id: format!("{:064x}", 101),
            tags: mute_tags,
            content: String::new(),
            sig: String::new(),
        },
        true,
        1.0,
    );
    // Friends also follow several other contacts to exercise follower scans.
    for (idx, friend) in direct.iter().take(20).enumerate() {
        let peer_tags: Vec<Vec<String>> = direct
            .iter()
            .skip(20)
            .take(40)
            .map(|peer| vec!["p".into(), peer.clone()])
            .collect();
        graph.handle_event(
            &nostr_social_graph::NostrEvent {
                pubkey: friend.clone(),
                kind: 3,
                created_at: 102 + idx as u64,
                id: format!("{:064x}", 102 + idx as u64),
                tags: peer_tags,
                content: String::new(),
                sig: String::new(),
            },
            true,
            1.0,
        );
    }
    core.user_discovery.owner_pubkey_hex = Some(owner_hex);
    core.user_discovery.social_graph = Some(graph.to_binary().unwrap());
    core.refresh_social_graph();

    // Populate 40 stored requestDecision records and 10 pinned + 10 muted chats.
    for chat in direct.iter().take(40) {
        core.sync_request_acceptance(chat);
    }
    core.preferences.pinned_chat_ids = direct.iter().take(10).cloned().collect();
    core.preferences.muted_chat_ids = direct.iter().skip(10).take(10).cloned().collect();
    // Fill seen_event_order to full production capacity (2048).
    for i in 0..MAX_SEEN_EVENT_IDS {
        core.remember_event(format!("bench-seen-{i:056x}"));
    }
    core.persist_best_effort_inner();
}

#[test]
#[ignore = "opt-in hot user flow performance benchmark"]
fn measure_hot_user_flows_benchmark() {
    let mut fixture = synthetic_history_fixture(12, 96, 8);
    let chat_ids = fixture.chat_ids.clone();
    populate_realistic_graph_and_decisions(&mut fixture.core, &chat_ids);
    let active_chat = chat_ids[0].clone();
    fixture.core.open_chat(&active_chat);
    fixture.core.open_chat_finalize(&active_chat);
    fixture.core.persist_best_effort_inner();
    assert_eq!(fixture.core.threads[&active_chat].messages.len(), 80);

    let metrics = SqlMetrics::attach(&fixture.core);

    // 1. rebuild_state + emit_state (20 iterations with slight visible change)
    let mut rebuild_times = Vec::new();
    let mut rebuild_steps = 0usize;
    for i in 0..20 {
        metrics.reset();
        fixture.core.state.toast = Some(format!("tick-{i}"));
        let t0 = Instant::now();
        fixture.core.rebuild_state();
        fixture.core.emit_state();
        rebuild_times.push(t0.elapsed().as_secs_f64() * 1000.0);
        let (steps, _, _) = metrics.snapshot();
        rebuild_steps += steps;
    }

    // 2. SetDraft in open chat (80 loaded messages), 10 edits
    checkpoint_wal(&fixture.core);
    let draft_wal_before = wal_bytes(fixture.directory.path());
    let mut draft_times = Vec::new();
    let mut draft_steps = 0usize;
    let mut draft_commits = 0usize;
    let mut draft_writes = BTreeMap::<String, usize>::new();
    for i in 0..10 {
        metrics.reset();
        let t0 = Instant::now();
        fixture.core.handle_action(AppAction::SetChatDraft {
            chat_id: active_chat.clone(),
            text: format!("typing draft message {i}"),
        });
        draft_times.push(t0.elapsed().as_secs_f64() * 1000.0);
        let (steps, commits, tables) = metrics.snapshot();
        draft_steps += steps;
        draft_commits += commits;
        for (k, v) in tables {
            *draft_writes.entry(k).or_default() += v;
        }
    }
    let draft_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(draft_wal_before);

    // 3. OpenChat + OpenChatFinalize across 10 different chats
    checkpoint_wal(&fixture.core);
    let open_wal_before = wal_bytes(fixture.directory.path());
    let mut open_times = Vec::new();
    let mut finalize_times = Vec::new();
    let mut open_steps = 0usize;
    let mut finalize_steps = 0usize;
    let mut finalize_writes = BTreeMap::<String, usize>::new();
    for chat in chat_ids.iter().skip(1).take(10) {
        metrics.reset();
        let t0 = Instant::now();
        fixture.core.open_chat(chat);
        open_times.push(t0.elapsed().as_secs_f64() * 1000.0);
        let (steps, _, _) = metrics.snapshot();
        open_steps += steps;

        metrics.reset();
        let t1 = Instant::now();
        fixture.core.open_chat_finalize(chat);
        finalize_times.push(t1.elapsed().as_secs_f64() * 1000.0);
        let (steps, _, tables) = metrics.snapshot();
        finalize_steps += steps;
        for (k, v) in tables {
            *finalize_writes.entry(k).or_default() += v;
        }
    }
    let open_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(open_wal_before);

    // 4. SendMessage (10 sends in open chat with 80+ messages)
    checkpoint_wal(&fixture.core);
    let send_wal_before = wal_bytes(fixture.directory.path());
    let mut send_times = Vec::new();
    let mut send_steps = 0usize;
    let mut send_writes = BTreeMap::<String, usize>::new();
    let send_chat = chat_ids[1].clone();
    for i in 0..10 {
        metrics.reset();
        let t0 = Instant::now();
        fixture.core.handle_action(AppAction::SendMessage {
            chat_id: send_chat.clone(),
            text: format!("hello world benchmark send {i}"),
        });
        send_times.push(t0.elapsed().as_secs_f64() * 1000.0);
        let (steps, _, tables) = metrics.snapshot();
        send_steps += steps;
        for (k, v) in tables {
            *send_writes.entry(k).or_default() += v;
        }
    }
    let send_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(send_wal_before);

    // 5. Receive 1 incoming message + persist_best_effort in open chat (90+ loaded messages)
    checkpoint_wal(&fixture.core);
    let recv_wal_before = wal_bytes(fixture.directory.path());
    metrics.reset();
    let recv_t0 = Instant::now();
    fixture.core.push_incoming_message_from(
        &send_chat,
        Some("bench-incoming-1".into()),
        "incoming benchmark message".into(),
        unix_now().get(),
        None,
        None,
        Some(send_chat.clone()),
        Some("bench-outer-1".into()),
    );
    fixture.core.persist_best_effort_inner();
    let recv_ms = recv_t0.elapsed().as_secs_f64() * 1000.0;
    let (recv_steps, _, recv_writes) = metrics.snapshot();
    let recv_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(recv_wal_before);

    // 5b. Mark 1 incoming message seen (delivery update, body unchanged) + persist
    checkpoint_wal(&fixture.core);
    let receipt_wal_before = wal_bytes(fixture.directory.path());
    metrics.reset();
    let receipt_t0 = Instant::now();
    fixture.core.handle_action(AppAction::MarkMessagesSeen {
        chat_id: send_chat.clone(),
        message_ids: vec!["bench-incoming-1".into()],
    });
    let receipt_ms = receipt_t0.elapsed().as_secs_f64() * 1000.0;
    let (receipt_steps, _, receipt_writes) = metrics.snapshot();
    let receipt_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(receipt_wal_before);

    // 6. Single profile update + persist_best_effort (96 cached profiles)
    metrics.reset();
    let prof_t0 = Instant::now();
    if let Some(p) = fixture.core.owner_profiles.get_mut(&chat_ids[2]) {
        p.display_name = Some("Updated Contact Name".into());
        p.updated_at_secs = unix_now().get();
    }
    fixture.core.persist_best_effort_inner();
    let prof_ms = prof_t0.elapsed().as_secs_f64() * 1000.0;
    let (prof_steps, _, prof_writes) = metrics.snapshot();

    // 7. Single app_keys update + persist_best_effort (97 cached app_keys)
    metrics.reset();
    let keys_t0 = Instant::now();
    if let Some(k) = fixture.core.app_keys.get_mut(&chat_ids[2]) {
        k.created_at_secs += 1;
    }
    fixture.core.persist_best_effort_inner();
    let keys_ms = keys_t0.elapsed().as_secs_f64() * 1000.0;
    let (keys_steps, _, keys_writes) = metrics.snapshot();

    // 8. Cold reopen + first unchanged persist_best_effort
    SqlMetrics::detach(&fixture.core);
    let logged_in = fixture.core.logged_in.as_ref().unwrap();
    let owner = logged_in.owner_keys.clone().unwrap();
    let device = logged_in.device_keys.clone();
    fixture.core.shutdown();
    drop(fixture.core);

    let mut reopened = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        fixture.directory.path().to_string_lossy().into_owned(),
    );
    let load_t0 = Instant::now();
    let persisted = reopened.load_persisted().unwrap().unwrap();
    let load_ms = load_t0.elapsed().as_secs_f64() * 1000.0;
    reopened.active_chat_id = persisted.active_chat_id.clone();
    reopened.next_message_id = persisted.next_message_id.max(1);
    reopened.owner_profiles = persisted.owner_profiles.clone();
    reopened.chat_message_ttl_seconds = persisted.chat_message_ttl_seconds.clone();
    crate::core::persistence::apply_persisted_preferences(
        &mut reopened.preferences,
        &persisted.preferences,
    );
    reopened.seen_event_order = persisted.seen_event_ids.iter().cloned().collect();
    reopened.seen_event_ids = reopened.seen_event_order.iter().cloned().collect();
    reopened.app_keys = persisted
        .app_keys
        .into_iter()
        .map(|e| (e.owner_pubkey_hex.clone(), e))
        .collect();
    reopened.groups = persisted
        .groups
        .into_iter()
        .map(|g| (g.group_id.clone(), g))
        .collect();
    reopened.group_pictures = persisted.group_pictures;
    reopened.threads = persisted
        .threads
        .into_iter()
        .map(|t| {
            (
                t.chat_id.clone(),
                ThreadRecord {
                    chat_id: t.chat_id,
                    unread_count: t.unread_count,
                    updated_at_secs: t.updated_at_secs,
                    draft: t.draft,
                    messages: t
                        .messages
                        .iter()
                        .map(chats::chat_message_from_persisted)
                        .collect(),
                },
            )
        })
        .collect();

    checkpoint_wal(&reopened);
    let reopen_wal_before = wal_bytes(fixture.directory.path());
    let reopen_metrics = SqlMetrics::attach(&reopened);
    reopen_metrics.reset();
    let first_save_t0 = Instant::now();
    reopened.persist_best_effort_inner();
    let first_save_ms = first_save_t0.elapsed().as_secs_f64() * 1000.0;
    let (first_save_steps, first_save_commits, first_save_writes) = reopen_metrics.snapshot();
    let reopen_wal_bytes = wal_bytes(fixture.directory.path()).saturating_sub(reopen_wal_before);
    SqlMetrics::detach(&reopened);
    reopened.shutdown();

    let summary = serde_json::json!({
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "contacts": 96,
        "groups": 8,
        "rows": fixture.rows,
        "rebuild_emit": {
            "median_ms": median_ms(rebuild_times),
            "sql_vm_steps_per_call": rebuild_steps / 20,
        },
        "set_draft_80_msgs": {
            "median_ms": median_ms(draft_times),
            "sql_vm_steps_per_call": draft_steps / 10,
            "commits_10_calls": draft_commits,
            "wal_bytes_10_calls": draft_wal_bytes,
            "writes_total_10_calls": summarize_writes(&draft_writes),
        },
        "open_chat": {
            "open_median_ms": median_ms(open_times),
            "open_sql_vm_steps_per_call": open_steps / 10,
            "finalize_median_ms": median_ms(finalize_times),
            "finalize_sql_vm_steps_per_call": finalize_steps / 10,
            "wal_bytes_10_calls": open_wal_bytes,
            "finalize_writes_total_10_calls": summarize_writes(&finalize_writes),
        },
        "send_message": {
            "median_ms": median_ms(send_times),
            "sql_vm_steps_per_call": send_steps / 10,
            "wal_bytes_10_calls": send_wal_bytes,
            "writes_total_10_calls": summarize_writes(&send_writes),
        },
        "receive_message_persist": {
            "ms": recv_ms,
            "sql_vm_steps": recv_steps,
            "wal_bytes": recv_wal_bytes,
            "writes": summarize_writes(&recv_writes),
        },
        "mark_message_seen": {
            "ms": receipt_ms,
            "sql_vm_steps": receipt_steps,
            "wal_bytes": receipt_wal_bytes,
            "writes": summarize_writes(&receipt_writes),
        },
        "single_profile_update_persist": {
            "ms": prof_ms,
            "sql_vm_steps": prof_steps,
            "writes": summarize_writes(&prof_writes),
        },
        "single_app_keys_update_persist": {
            "ms": keys_ms,
            "sql_vm_steps": keys_steps,
            "writes": summarize_writes(&keys_writes),
        },
        "cold_reopen": {
            "load_ms": load_ms,
            "first_unchanged_save_ms": first_save_ms,
            "first_unchanged_save_vm_steps": first_save_steps,
            "first_unchanged_save_commits": first_save_commits,
            "first_unchanged_save_wal_bytes": reopen_wal_bytes,
            "first_unchanged_save_writes": summarize_writes(&first_save_writes),
        },
    });
    println!(
        "HOT_FLOW_PERF_RESULT={}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
}

#[test]
fn hot_user_flows_bound_sqlite_writes_and_reads() {
    let mut fixture = synthetic_history_fixture(1, 12, 2);
    let chat_ids = fixture.chat_ids.clone();
    populate_realistic_graph_and_decisions(&mut fixture.core, &chat_ids);
    let active_chat = chat_ids[0].clone();
    fixture.core.open_chat(&active_chat);
    fixture.core.open_chat_finalize(&active_chat);
    fixture.core.persist_best_effort_inner();
    assert_eq!(fixture.core.threads[&active_chat].messages.len(), 80);

    // Warm the per-account request-decision cache once before measuring steady-state rebuilds.
    fixture.core.rebuild_state();
    fixture.core.emit_state();

    let metrics = SqlMetrics::attach(&fixture.core);

    // 1. Steady-state rebuild_state + emit_state must touch SQLite zero times.
    metrics.reset();
    fixture.core.state.toast = Some("steady".into());
    fixture.core.rebuild_state();
    fixture.core.emit_state();
    let (rebuild_steps, rebuild_commits, rebuild_writes) = metrics.snapshot();
    assert_eq!(
        rebuild_steps, 0,
        "steady-state rebuild_state/emit_state must not query SQLite"
    );
    assert_eq!(rebuild_commits, 0);
    assert!(rebuild_writes.is_empty());

    // 2. Typing a draft in an 80-message chat must only update the 1 threads row.
    metrics.reset();
    fixture.core.handle_action(AppAction::SetChatDraft {
        chat_id: active_chat.clone(),
        text: "draft regression check".into(),
    });
    let (_, _, draft_writes) = metrics.snapshot();
    assert_eq!(draft_writes.get("threads").copied().unwrap_or(0), 1);
    assert_eq!(draft_writes.get("messages").copied().unwrap_or(0), 0);
    assert_eq!(
        draft_writes
            .iter()
            .filter(|(k, _)| k.starts_with("messages_fts"))
            .map(|(_, v)| *v)
            .sum::<usize>(),
        0
    );

    // 3. Opening another chat and finalizing must not rewrite its 80 already-stored messages
    //    or all cached owner_profiles.
    let next_chat = chat_ids[1].clone();
    metrics.reset();
    fixture.core.open_chat(&next_chat);
    fixture.core.open_chat_finalize(&next_chat);
    let (_, _, open_writes) = metrics.snapshot();
    assert_eq!(open_writes.get("messages").copied().unwrap_or(0), 0);
    assert_eq!(
        open_writes
            .iter()
            .filter(|(k, _)| k.starts_with("messages_fts"))
            .map(|(_, v)| *v)
            .sum::<usize>(),
        0
    );
    assert!(
        open_writes.get("owner_profiles").copied().unwrap_or(0) <= 1,
        "opening one chat must not rewrite all owner_profiles: {open_writes:?}"
    );

    // 4. Sending 1 message in an 80-message chat must not rewrite the existing 80 messages.
    metrics.reset();
    fixture.core.handle_action(AppAction::SendMessage {
        chat_id: next_chat.clone(),
        text: "regression send body".into(),
    });
    let (_, _, send_writes) = metrics.snapshot();
    assert!(
        send_writes.get("messages").copied().unwrap_or(0) <= 3,
        "sending 1 message should only touch the new message row: {send_writes:?}"
    );

    // 5. Receiving 1 message and marking it seen must only write that 1 message (and 0 FTS on seen).
    metrics.reset();
    fixture.core.push_incoming_message_from(
        &next_chat,
        Some("reg-incoming-1".into()),
        "incoming regression check".into(),
        unix_now().get(),
        None,
        None,
        Some(next_chat.clone()),
        Some("reg-outer-1".into()),
    );
    fixture.core.persist_best_effort_inner();
    let (_, _, recv_writes) = metrics.snapshot();
    assert!(
        recv_writes.get("messages").copied().unwrap_or(0) <= 2,
        "receiving 1 message in an 80-message chat must only touch the new message: {recv_writes:?}"
    );
    assert!(
        recv_writes
            .iter()
            .filter(|(k, _)| k.starts_with("messages_fts"))
            .map(|(_, v)| *v)
            .sum::<usize>()
            <= 10,
        "receiving 1 message must only index that 1 message in FTS5: {recv_writes:?}"
    );

    metrics.reset();
    fixture.core.handle_action(AppAction::MarkMessagesSeen {
        chat_id: next_chat.clone(),
        message_ids: vec!["reg-incoming-1".into()],
    });
    let (_, _, seen_writes) = metrics.snapshot();
    assert!(
        seen_writes.get("messages").copied().unwrap_or(0) <= 2,
        "marking 1 message seen must not rewrite the whole thread: {seen_writes:?}"
    );
    assert_eq!(
        seen_writes
            .iter()
            .filter(|(k, _)| k.starts_with("messages_fts"))
            .map(|(_, v)| *v)
            .sum::<usize>(),
        0,
        "marking 1 message seen must not rewrite messages_fts: {seen_writes:?}"
    );

    // 6. Updating 1 profile and 1 app_keys entry must write only 1 row each.
    metrics.reset();
    if let Some(profile) = fixture.core.owner_profiles.get_mut(&chat_ids[2]) {
        profile.display_name = Some("Regression Name".into());
        profile.updated_at_secs = unix_now().get();
    }
    fixture.core.persist_best_effort_inner();
    let (_, _, prof_writes) = metrics.snapshot();
    assert_eq!(prof_writes.get("owner_profiles").copied().unwrap_or(0), 1);

    metrics.reset();
    if let Some(keys) = fixture.core.app_keys.get_mut(&chat_ids[2]) {
        keys.created_at_secs += 1;
    }
    fixture.core.persist_best_effort_inner();
    let (_, _, keys_writes) = metrics.snapshot();
    assert_eq!(keys_writes.get("app_keys").copied().unwrap_or(0), 1);

    // 7. Cold reopen followed by an unchanged persist must commit 0 transactions and write 0 rows.
    SqlMetrics::detach(&fixture.core);
    let logged_in = fixture.core.logged_in.as_ref().unwrap();
    let owner = logged_in.owner_keys.clone().unwrap();
    let device = logged_in.device_keys.clone();
    fixture.core.shutdown();
    drop(fixture.core);

    let mut reopened = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        fixture.directory.path().to_string_lossy().into_owned(),
    );
    let persisted = reopened.load_persisted().unwrap().unwrap();
    reopened.active_chat_id = persisted.active_chat_id.clone();
    reopened.next_message_id = persisted.next_message_id.max(1);
    reopened.owner_profiles = persisted.owner_profiles.clone();
    reopened.chat_message_ttl_seconds = persisted.chat_message_ttl_seconds.clone();
    crate::core::persistence::apply_persisted_preferences(
        &mut reopened.preferences,
        &persisted.preferences,
    );
    reopened.seen_event_order = persisted.seen_event_ids.iter().cloned().collect();
    reopened.seen_event_ids = reopened.seen_event_order.iter().cloned().collect();
    reopened.app_keys = persisted
        .app_keys
        .into_iter()
        .map(|e| (e.owner_pubkey_hex.clone(), e))
        .collect();
    reopened.groups = persisted
        .groups
        .into_iter()
        .map(|g| (g.group_id.clone(), g))
        .collect();
    reopened.group_pictures = persisted.group_pictures;
    reopened.threads = persisted
        .threads
        .into_iter()
        .map(|t| {
            (
                t.chat_id.clone(),
                ThreadRecord {
                    chat_id: t.chat_id,
                    unread_count: t.unread_count,
                    updated_at_secs: t.updated_at_secs,
                    draft: t.draft,
                    messages: t
                        .messages
                        .iter()
                        .map(chats::chat_message_from_persisted)
                        .collect(),
                },
            )
        })
        .collect();

    let reopen_metrics = SqlMetrics::attach(&reopened);
    reopen_metrics.reset();
    reopened.persist_best_effort_inner();
    let (_, _, reopen_writes) = reopen_metrics.snapshot();
    for table in [
        "threads",
        "messages",
        "owner_profiles",
        "app_keys",
        "groups",
        "seen_events",
        "app_meta",
    ] {
        assert_eq!(
            reopen_writes.get(table).copied().unwrap_or(0),
            0,
            "unchanged save after cold load must not rewrite {table}: {reopen_writes:?}"
        );
    }
    assert_eq!(
        reopen_writes
            .iter()
            .filter(|(k, _)| k.starts_with("messages_fts"))
            .map(|(_, v)| *v)
            .sum::<usize>(),
        0
    );
    // A second unchanged persist after preferences normalization must commit zero times.
    reopen_metrics.reset();
    reopened.persist_best_effort_inner();
    let (_, second_commits, second_writes) = reopen_metrics.snapshot();
    assert_eq!(second_commits, 0);
    assert!(second_writes.is_empty());
    SqlMetrics::detach(&reopened);
    reopened.shutdown();
}
