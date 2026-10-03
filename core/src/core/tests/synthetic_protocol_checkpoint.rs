fn synthetic_protocol_backlog(core: &mut AppCore, target_bytes: usize) -> (usize, usize) {
    let logged_in = core.logged_in.as_ref().unwrap();
    let owner = logged_in.owner_pubkey;
    let owner_keys = logged_in.owner_keys.clone().unwrap();
    let device = logged_in.device_keys.clone();
    let storage = Arc::new(storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut checkpoint: serde_json::Value = serde_json::from_str(
        &storage
            .get(TEST_PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let senders = (0..13).map(|_| Keys::generate()).collect::<Vec<_>>();
    let mut chains = (0..13)
        .map(|index| nostr_double_ratchet::SenderKeyState::new(1, [index + 1; 32], 0))
        .collect::<Vec<_>>();
    let mut pending = Vec::new();
    let mut serialized_bytes = 2;
    while serialized_bytes < target_bytes {
        let index = pending.len();
        let stream = index % senders.len();
        let plaintext = format!(
            "Generated pending message {index}. {}",
            "A fox carries tea. ".repeat(78)
        );
        let (message_number, ciphertext) = chains[stream]
            .encrypt_to_bytes(plaintext.as_bytes())
            .unwrap();
        let outer = nostr_double_ratchet::wire::group_sender_key_message_event(
            &nostr_double_ratchet::GroupSenderKeyMessageEnvelope {
                group_id: format!("synthetic-unmapped-{stream}"),
                sender_event_pubkey: ndr_device_pubkey(senders[stream].public_key()),
                signer_secret_key: senders[stream].secret_key().to_secret_bytes(),
                key_id: 1,
                message_number,
                encrypted_header: None,
                created_at: nostr_double_ratchet::UnixSeconds(1_700_000_000 + index as u64),
                ciphertext,
            },
        )
        .unwrap();
        // Restore the same durable candidate shape as a device which has not
        // discovered these sender mappings yet; every envelope is generated
        // and signed here, and no private account state is copied.
        let parsed =
            nostr_double_ratchet::wire::parse_group_sender_key_message_event_unchecked(&outer)
                .unwrap();
        serialized_bytes += serde_json::to_vec(&parsed).unwrap().len() + 1;
        pending.push(parsed);
    }
    let count = pending.len();
    checkpoint["pending_group_sender_key_messages"] = serde_json::to_value(pending).unwrap();
    let json = serde_json::to_string(&checkpoint).unwrap();
    let checkpoint_bytes = json.len();
    storage.put(TEST_PROTOCOL_ENGINE_STATE_KEY, json).unwrap();
    let mut engine =
        ProtocolEngine::load_or_create_for_local_device(storage, owner, &device).unwrap();
    engine
        .authenticate_local_owner_for_sending(&owner_keys)
        .unwrap();
    assert_eq!(
        engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        count
    );
    core.protocol_engine = Some(engine);
    (count, checkpoint_bytes)
}

fn synthetic_protocol_wait_idle(app: &crate::FfiApp) {
    let start = Instant::now();
    loop {
        if app.foreground_rx.is_empty()
            && app.background_rx.is_empty()
            && !app
                .queue_metrics
                .batch_active
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "synthetic worker must finish its durable checkpoint"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn synthetic_protocol_checkpoint_measure(pending_mib: usize) -> serde_json::Value {
    let mut fixture = synthetic_history_fixture(2, 4, 2);
    fixture.core.preferences.send_typing_indicators = true;
    let generation = Instant::now();
    let (pending_count, checkpoint_bytes) =
        synthetic_protocol_backlog(&mut fixture.core, pending_mib * 1024 * 1024);
    let generation_ms = generation.elapsed().as_secs_f64() * 1000.0;
    let logged_in = fixture.core.logged_in.as_ref().unwrap();
    let owner = logged_in.owner_pubkey;
    let device = logged_in.device_keys.clone();
    let shared_db = fixture.core.app_store.shared();
    let chat_id = fixture.chat_ids[0].clone();
    let app = synthetic_history_app(fixture.core, fixture.updates);
    app.dispatch(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    });
    synthetic_history_wait(&app, |state| {
        state
            .current_chat
            .as_ref()
            .is_some_and(|chat| chat.chat_id == chat_id)
    });
    synthetic_protocol_wait_idle(&app);
    let mut timings = Vec::new();
    let mut sent_ids = Vec::new();
    for index in 0..4 {
        while app.update_rx.try_recv().is_ok() {}
        let body = format!("Synthetic checkpoint send {index}");
        let typing_started = Instant::now();
        let processed = app
            .queue_metrics
            .foreground_processed
            .load(std::sync::atomic::Ordering::Acquire);
        app.dispatch(AppAction::SendTyping {
            chat_id: chat_id.clone(),
        });
        let admitted_separately = index >= 2;
        let mut typing_still_active = false;
        if admitted_separately {
            // The worker drains its foreground batch before marking it active.
            // Waiting for admission puts Send in a later batch, including when
            // the preceding typing ratchet is still being checkpointed.
            loop {
                typing_still_active = app
                    .queue_metrics
                    .batch_active
                    .load(std::sync::atomic::Ordering::Acquire);
                if app.foreground_rx.is_empty()
                    && (typing_still_active
                        || app
                            .queue_metrics
                            .foreground_processed
                            .load(std::sync::atomic::Ordering::Acquire)
                            > processed)
                {
                    break;
                }
                assert!(typing_started.elapsed() < Duration::from_secs(15));
                std::thread::yield_now();
            }
        }
        let start = Instant::now();
        app.dispatch(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text: body.clone(),
        });
        let state = synthetic_history_wait(&app, |state| {
            state.current_chat.as_ref().is_some_and(|chat| {
                chat.messages
                    .iter()
                    .any(|message| message.body == body && message.is_outgoing)
            })
        });
        timings.push(serde_json::json!({
            "mode": if admitted_separately { "send_after_typing_batch_admission" } else { "back_to_back" },
            "typing_active_at_send": typing_still_active,
            "typing_admission_ms": start.duration_since(typing_started).as_secs_f64() * 1000.0,
            "dispatch_send_to_state_ms": start.elapsed().as_secs_f64() * 1000.0,
        }));
        sent_ids.push(
            state
                .current_chat
                .unwrap()
                .messages
                .into_iter()
                .find(|message| message.body == body)
                .unwrap()
                .id,
        );
        synthetic_protocol_wait_idle(&app);
    }
    drop(app);
    let storage = Arc::new(storage::SqliteStorageAdapter::new(
        shared_db.clone(),
        owner.to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, owner, &device).unwrap();
    assert_eq!(
        restored
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        pending_count,
        "typing and sends must retain every unresolved ciphertext after restart"
    );
    let conn = shared_db.lock().unwrap();
    for id in sent_ids {
        assert!(conn.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND id=?2 AND is_outgoing=1 AND delivery!='failed')",
            rusqlite::params![chat_id, id], |row| row.get::<_, bool>(0)).unwrap());
    }
    serde_json::json!({"synthetic_only":true, "pending_ciphertexts":pending_count,
        "initial_protocol_checkpoint_bytes":checkpoint_bytes, "generation_ms":generation_ms,
        "profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "timings":timings,
        "unresolved_ciphertexts_preserved_after_restart":true, "all_sends_durable":true})
}

#[test]
fn synthetic_protocol_checkpoint_preserves_backlog_and_offline_sends() {
    let result = synthetic_protocol_checkpoint_measure(1);
    println!("SYNTHETIC_PROTOCOL_CHECKPOINT_RESULT={result}");
}

#[test]
#[ignore = "generates a synthetic 70 MiB pending protocol checkpoint; use a release profile"]
fn synthetic_protocol_checkpoint_latency_benchmark() {
    let result = synthetic_protocol_checkpoint_measure(70);
    println!("SYNTHETIC_PROTOCOL_CHECKPOINT_RESULT={result}");
}
