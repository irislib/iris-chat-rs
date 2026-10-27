fn legacy_group_ingress_fixture() -> (ProtocolEngine, String) {
    let (mut engine, request) = repair_retry_fixture();
    engine
        .sender_key_repair_request_effects(request, NdrUnixSeconds(100))
        .unwrap();
    for (index, body) in ["user message", "metadata", "private wake", "distribution"]
        .into_iter()
        .enumerate()
    {
        engine
            .pending_group_fanouts
            .push(pending_local_group_fanout(
                "queued-group",
                Some(&format!("outgoing-{index}")),
                body.as_bytes(),
                100,
            ));
    }
    for (event_id, content) in [
        ("ordinary-direct-delivery", "retain ordinary DM journal"),
        (
            "group-delivery:legacy",
            "preserve unproven group journal bytes",
        ),
    ] {
        engine
            .pending_decrypted_deliveries
            .push(ProtocolPendingDecryptedDelivery {
                discarded: false,
                sender: engine.owner_pubkey,
                sender_device: Some(public_device(engine.local_device).unwrap()),
                conversation_owner: None,
                content: content.into(),
                event_id: Some(event_id.into()),
                created_at_secs: 100,
            });
    }
    engine.persist_now().unwrap();
    let current = engine
        .storage
        .get(PROTOCOL_ENGINE_STATE_KEY)
        .unwrap()
        .unwrap();
    let marker = ",\"group_ingress_provenance_version\":1";
    assert_eq!(current.matches(marker).count(), 1);
    let raw = current.replace(marker, "");
    engine
        .storage
        .put(PROTOCOL_ENGINE_STATE_KEY, raw.clone())
        .unwrap();
    (engine, raw)
}

fn reload_group_ingress(
    engine: &ProtocolEngine,
    storage: Arc<dyn StorageAdapter>,
) -> anyhow::Result<ProtocolEngine> {
    let device = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    ProtocolEngine::load_or_create_for_local_device(storage, engine.owner_pubkey, &device)
}

#[test]
fn legacy_group_ingress_is_quarantined_once_without_retiring_outgoing_work() {
    let (engine, raw) = legacy_group_ingress_fixture();
    let before: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut restored = reload_group_ingress(&engine, engine.storage.clone()).unwrap();
    assert_eq!(
        engine
            .storage
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap()
            .as_deref(),
        Some(raw.as_str())
    );
    assert!(restored.pending_group_sender_key_messages.is_empty());
    assert!(restored.pending_group_sender_key_repairs.is_empty());
    assert_eq!(restored.pending_decrypted_deliveries.len(), 1);
    assert_eq!(
        restored.pending_decrypted_deliveries[0].content,
        "retain ordinary DM journal"
    );
    assert!(restored
        .pending_group_decrypted_delivery("group-delivery:legacy")
        .is_err());
    let after: serde_json::Value = serde_json::from_str(
        &restored
            .storage
            .get(PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    for field in [
        "session_manager",
        "group_manager",
        "pending_group_fanouts",
        "ready_group_publications",
        "pending_remote_sends",
        "pending_local_sibling_sends",
        "pending_group_pairwise_payloads",
        "group_roster_fact_histories",
        "processed_group_sender_key_messages",
    ] {
        assert_eq!(after[field], before[field], "migration changed {field}");
    }
    assert_eq!(after["group_ingress_provenance_version"], 1);
    assert!(restored
        .retry_pending_group_sender_key_repairs(NdrUnixSeconds(u64::MAX))
        .unwrap()
        .is_empty());
    let again = reload_group_ingress(&restored, restored.storage.clone()).unwrap();
    assert_eq!(again.pending_group_fanouts, restored.pending_group_fanouts);
    assert_eq!(
        again.pending_decrypted_deliveries,
        restored.pending_decrypted_deliveries
    );
    assert_eq!(
        again
            .storage
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap()
            .as_deref(),
        Some(raw.as_str())
    );
}

struct GroupIngressFailStorage {
    inner: Arc<dyn StorageAdapter>,
    fail_key: &'static str,
    write_before_failure: bool,
}

impl StorageAdapter for GroupIngressFailStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }
    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        if key == self.fail_key {
            if self.write_before_failure {
                self.inner.put(key, value)?;
            }
            return Err(StorageError::new("injected ingress migration failure"));
        }
        self.inner.put(key, value)
    }
    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
}

#[test]
fn legacy_group_ingress_migration_write_failures_preserve_the_original_and_retry() {
    for (fail_key, write_before_failure) in [
        (LEGACY_GROUP_INGRESS_QUARANTINE_KEY, false),
        (PROTOCOL_ENGINE_STATE_KEY, false),
        (LEGACY_GROUP_INGRESS_QUARANTINE_KEY, true),
    ] {
        let (engine, raw) = legacy_group_ingress_fixture();
        let failed = Arc::new(GroupIngressFailStorage {
            inner: engine.storage.clone(),
            fail_key,
            write_before_failure,
        });
        assert!(reload_group_ingress(&engine, failed.clone()).is_err());
        if write_before_failure {
            // A sync failure leaves matching bytes behind. Reopening must
            // retry preservation and fail again, not advance the checkpoint.
            assert!(reload_group_ingress(&engine, failed).is_err());
        }
        assert_eq!(
            engine
                .storage
                .get(PROTOCOL_ENGINE_STATE_KEY)
                .unwrap()
                .as_deref(),
            Some(raw.as_str())
        );
        let quarantine = engine
            .storage
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap();
        if fail_key == LEGACY_GROUP_INGRESS_QUARANTINE_KEY && !write_before_failure {
            assert!(quarantine.is_none());
        } else {
            assert_eq!(quarantine.as_deref(), Some(raw.as_str()));
        }
        let restored = reload_group_ingress(&engine, engine.storage.clone()).unwrap();
        assert!(restored.pending_group_sender_key_messages.is_empty());
        assert!(restored.pending_group_sender_key_repairs.is_empty());
        assert_eq!(restored.pending_group_fanouts, engine.pending_group_fanouts);
        assert_eq!(
            restored
                .storage
                .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
                .unwrap()
                .as_deref(),
            Some(raw.as_str())
        );
    }
}

#[test]
fn legacy_group_ingress_quarantine_conflict_and_future_version_fail_closed() {
    for future in [false, true] {
        let (engine, mut raw) = legacy_group_ingress_fixture();
        if future {
            let mut state: serde_json::Value = serde_json::from_str(&raw).unwrap();
            state["group_ingress_provenance_version"] = serde_json::json!(2);
            raw = state.to_string();
            engine
                .storage
                .put(PROTOCOL_ENGINE_STATE_KEY, raw.clone())
                .unwrap();
        } else {
            engine
                .storage
                .put(
                    LEGACY_GROUP_INGRESS_QUARANTINE_KEY,
                    "previous preserved state".into(),
                )
                .unwrap();
        }
        assert!(reload_group_ingress(&engine, engine.storage.clone()).is_err());
        assert_eq!(
            engine
                .storage
                .get(PROTOCOL_ENGINE_STATE_KEY)
                .unwrap()
                .as_deref(),
            Some(raw.as_str())
        );
        assert_eq!(
            engine
                .storage
                .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
                .unwrap()
                .as_deref(),
            if future {
                None
            } else {
                Some("previous preserved state")
            }
        );
    }
}

#[test]
fn legacy_group_ingress_segmented_storage_and_outer_rollback_preserve_exact_bytes() {
    for checkpoint in [false, true] {
        let (engine, raw) = legacy_group_ingress_fixture();
        let connection = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        connection.lock().unwrap().execute_batch(
            "CREATE TABLE ndr_kv(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key));"
        ).unwrap();
        let storage = Arc::new(SqliteStorageAdapter::new(
            connection.clone(),
            engine.owner_pubkey.to_hex(),
            engine.local_device.to_hex(),
        ));
        let raw = format!("{raw}{}", " ".repeat(128 * 1024));
        if checkpoint {
            let mut saved = StorageCheckpoint::new();
            saved.push_inline("legacy", raw.clone(), 0);
            storage
                .put_checkpoint(PROTOCOL_ENGINE_STATE_KEY, &saved)
                .unwrap();
        } else {
            storage.put(PROTOCOL_ENGINE_STATE_KEY, raw.clone()).unwrap();
        }
        connection
            .lock()
            .unwrap()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        let restored = reload_group_ingress(&engine, storage.clone()).unwrap();
        assert!(restored.pending_group_sender_key_messages.is_empty());
        assert_eq!(
            storage
                .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
                .unwrap()
                .as_deref(),
            Some(raw.as_str())
        );
        connection
            .lock()
            .unwrap()
            .execute_batch("ROLLBACK")
            .unwrap();
        assert!(storage
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap()
            .is_none());
        assert_eq!(
            storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().as_deref(),
            Some(raw.as_str())
        );
        let reopened = reload_group_ingress(&engine, storage.clone()).unwrap();
        assert_eq!(reopened.pending_group_fanouts, engine.pending_group_fanouts);
        assert!(reopened.pending_group_sender_key_repairs.is_empty());
        assert_eq!(
            storage
                .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
                .unwrap()
                .as_deref(),
            Some(raw.as_str())
        );
    }
}

#[test]
fn legacy_group_ingress_archive_precedes_debounced_state_checkpoint() {
    let (engine, raw) = legacy_group_ingress_fixture();
    let directory = tempfile::tempdir().unwrap();
    let files = FileStorageAdapter::new(directory.path().to_owned()).unwrap();
    files.put(PROTOCOL_ENGINE_STATE_KEY, raw.clone()).unwrap();
    let buffered =
        Arc::new(DebouncedFileStorage::new(directory.path().to_owned(), 60_000).unwrap());
    let restored = reload_group_ingress(&engine, buffered.clone()).unwrap();
    assert!(restored.pending_group_sender_key_messages.is_empty());
    // Simulate stopping before the buffered checkpoint: the old state and its
    // exact quarantine are already on disk, and the next startup can repeat.
    assert_eq!(
        files.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().as_deref(),
        Some(raw.as_str())
    );
    assert_eq!(
        files
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap()
            .as_deref(),
        Some(raw.as_str())
    );
    drop(restored);
    drop(buffered);
    let buffered =
        Arc::new(DebouncedFileStorage::new(directory.path().to_owned(), 60_000).unwrap());
    let restored = reload_group_ingress(&engine, buffered.clone()).unwrap();
    buffered.flush().unwrap();
    assert_eq!(
        files
            .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
            .unwrap()
            .as_deref(),
        Some(raw.as_str())
    );
    let reopened = reload_group_ingress(&restored, Arc::new(files)).unwrap();
    assert!(reopened.pending_group_sender_key_messages.is_empty());
    assert_eq!(reopened.pending_group_fanouts, engine.pending_group_fanouts);
}

#[test]
fn authenticated_local_group_queue_and_delivery_journal_survive_restart() {
    let (mut engine, _, _) = queued_group_history(1);
    let author = Keys::generate();
    let mut group = engine.group_manager.snapshot();
    group.sender_keys[0].sender_event_pubkey = ndr_device(author.public_key());
    engine.group_manager = GroupEventManager::from_snapshot(group).unwrap();
    let current = engine.pending_group_sender_key_messages.remove(0);
    let nested =
        group_sender_key_message_event(&nostr_double_ratchet::GroupSenderKeyMessageEnvelope {
            group_id: "queued-group".into(),
            sender_event_pubkey: ndr_device(author.public_key()),
            signer_secret_key: author.secret_key().to_secret_bytes(),
            key_id: current.key_id,
            message_number: current.message_number,
            encrypted_header: current.encrypted_header,
            created_at: current.created_at,
            ciphertext: current.ciphertext,
        })
        .unwrap();
    // The trusted local API receives the nested ciphertext after pairwise
    // authentication. Force its normal budget yield to exercise persisted work.
    engine
        .group_sender_key_retry
        .borrow_mut()
        .key_trials_remaining = 0;
    assert!(engine.process_group_outer_event(&nested).unwrap().pending);
    assert_eq!(engine.pending_group_sender_key_messages.len(), 1);
    let mut restored = reload_group_ingress(&engine, engine.storage.clone()).unwrap();
    assert_eq!(restored.pending_group_sender_key_messages.len(), 1);
    assert!(restored
        .storage
        .get(LEGACY_GROUP_INGRESS_QUARANTINE_KEY)
        .unwrap()
        .is_none());
    let result = restored.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(result.group_result.events.len(), 1);
    assert!(restored.pending_group_sender_key_messages.is_empty());
    let delivery = restored
        .pending_decrypted_deliveries
        .iter()
        .find(|delivery| is_synthetic_group_delivery(delivery))
        .unwrap();
    let id = delivery.event_id.clone().unwrap();
    let reopened = reload_group_ingress(&restored, restored.storage.clone()).unwrap();
    assert!(reopened
        .pending_group_decrypted_delivery(&id)
        .unwrap()
        .is_some());
}
