// The device retained this queue size while writing several MiB each minute.
// Generate all keys, payloads and SQLite state; never open an exported account.
mod populated_retry_idle {
    use super::*;
    use std::sync::Mutex;

    pub(super) fn restored_backlog(core: &mut AppCore) {
        let local = core.logged_in.as_ref().unwrap();
        let owner = local.owner_keys.clone().unwrap();
        let device = local.device_keys.clone();
        let storage = Arc::new(storage::SqliteStorageAdapter::new(
            core.shared_db(),
            owner.public_key().to_hex(),
            device.public_key().to_hex(),
        ));
        let mut checkpoint: serde_json::Value = serde_json::from_str(
            &storage
                .get(TEST_PROTOCOL_ENGINE_STATE_KEY)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let prototypes = checkpoint["pending_group_fanouts"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            !prototypes.is_empty(),
            "real group creation must leave unavailable-device fanouts"
        );
        let now = unix_now().get();
        checkpoint["pending_group_fanouts"] = serde_json::Value::Array(
            (0..28_648)
                .map(|index| {
                    let mut entry = prototypes[index % prototypes.len()].clone();
                    entry["inner_event_id"] =
                        serde_json::json!(format!("synthetic-pending-{index}"));
                    entry["created_at_secs"] = serde_json::json!(now - 600);
                    entry["next_retry_at_secs"] = serde_json::json!(now - 1);
                    entry
                })
                .collect(),
        );
        // A valid wire envelope from a different generated receiver cannot
        // decrypt here, and remains pending until its session is discovered.
        let unrelated_owner = Keys::generate();
        let unrelated_device = Keys::generate();
        let mut unrelated = ProtocolEngine::load_or_create_for_local_device(
            Arc::new(InMemoryStorage::new()),
            unrelated_owner.public_key(),
            &unrelated_device,
        )
        .unwrap();
        let (prototype, author) = appcore_direct_message_event_with_author_keys_for_test(
            &mut unrelated,
            &Keys::generate(),
            "Synthetic unavailable session",
            now - 600,
        );
        checkpoint["pending_inbound"] = serde_json::Value::Array(
            (0..268)
                .map(|index| {
                    let event = EventBuilder::new(prototype.kind, prototype.content.clone())
                        .tags(prototype.tags.iter().cloned())
                        .custom_created_at(Timestamp::from_secs(now - 600 - index))
                        .sign_with_keys(&author)
                        .unwrap();
                    serde_json::json!({"event_id": event.id.to_hex(), "event": event,
                "created_at_secs": now - 600, "next_retry_at_secs": now - 1})
                })
                .collect(),
        );
        storage
            .put(TEST_PROTOCOL_ENGINE_STATE_KEY, checkpoint.to_string())
            .unwrap();
        let mut engine =
            ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
                .unwrap();
        engine.authenticate_local_owner_for_sending(&owner).unwrap();
        let state = engine.debug_snapshot();
        assert_eq!(state.pending_group_fanout_count, 28_648);
        assert_eq!(state.pending_inbound_count, 268);
        println!(
            "restored synthetic backlog: authors={} fanouts={} inbound={}",
            state.known_message_author_count,
            state.pending_group_fanout_count,
            state.pending_inbound_count
        );
        core.protocol_engine = Some(engine);
    }

    #[test]
    fn unchanged_populated_retry_backlog_does_not_rewrite_durable_state() {
        let mut fixture = synthetic_history_fixture_with_devices(1, 54, 2, 3).0;
        restored_backlog(&mut fixture.core);
        let writes = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let recorded = writes.clone();
        fixture.core.shared_db().lock().unwrap().update_hook(Some(
            move |_, _: &str, table: &str, _: i64| {
                *recorded
                    .lock()
                    .unwrap()
                    .entry(table.to_owned())
                    .or_default() += 1;
            },
        ));
        let engine = fixture.core.protocol_engine.as_mut().unwrap();
        let now = unix_now().get() + 10;
        // Complete initial due work, including the bounded fanout continuations.
        for _ in 0..500 {
            if !engine.has_due_pending_retry_work(NdrUnixSeconds(now)) {
                break;
            }
            engine.retry_pending_protocol(NdrUnixSeconds(now)).unwrap();
        }
        assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(now)));
        let before = serde_json::to_value(engine.session_manager_snapshot()).unwrap();
        writes.lock().unwrap().clear();
        let started = Instant::now();
        for minute in 1..=2 {
            let due = NdrUnixSeconds(now + minute * 60);
            for _ in 0..500 {
                if !engine.has_due_pending_retry_work(due) {
                    break;
                }
                let batch = engine.retry_pending_protocol(due).unwrap();
                assert!(
                    batch.is_empty(),
                    "unavailable devices must not create new delivery work"
                );
            }
            assert!(!engine.has_due_pending_retry_work(due));
        }
        assert_eq!(
            serde_json::to_value(engine.session_manager_snapshot()).unwrap(),
            before
        );
        let state = engine.debug_snapshot();
        assert_eq!(state.pending_group_fanout_count, 28_648);
        assert_eq!(state.pending_inbound_count, 268);
        println!(
            "two synthetic retry minutes: elapsed_ms={} sqlite_mutations={:?}",
            started.elapsed().as_millis(),
            writes.lock().unwrap()
        );
        assert!(
            writes.lock().unwrap().is_empty(),
            "unchanged retry clocks must not rewrite durable state"
        );
    }
}
