#[test]
fn replayed_authorization_does_not_copy_unrelated_pending_backlog() {
    let mut fixture = remote_send_fixture();
    let events = [1, 0].map(|created| {
        signed_app_keys(
            &fixture.peer_owner,
            &[
                fixture.ready_device.public_key(),
                fixture.late_device.public_key(),
            ],
            created,
        )
    });
    let small = events
        .iter()
        .map(|event| {
            let (result, bytes) = remote_roster_tests::allocated_bytes(|| {
                fixture.sender.ingest_app_keys_event(event)
            });
            assert!(result.unwrap().is_empty());
            bytes
        })
        .collect::<Vec<_>>();
    fixture.sender.pending_group_fanouts = (0..4096)
        .map(|index| {
            pending_local_group_fanout(
                "unrelated",
                Some("event"),
                format!("payload-{index}").as_bytes(),
                5,
            )
        })
        .collect();
    let before = fixture.sender.pending_group_fanouts.clone();
    for (event, small) in events.iter().zip(small) {
        let (result, large) =
            remote_roster_tests::allocated_bytes(|| fixture.sender.ingest_app_keys_event(event));
        eprintln!(
            "authorization_replay small_allocated_bytes={small} backlog_allocated_bytes={large}"
        );
        assert!(result.unwrap().is_empty());
        assert_eq!(fixture.sender.pending_group_fanouts, before);
        assert!(
            large <= small + 128 * 1024,
            "replayed authorization copied unrelated backlog: small={small}, large={large}"
        );
    }
}

#[test]
fn replayed_peer_authorization_does_not_copy_unrelated_signed_heads() {
    assert_peer_authorization_replay_does_not_copy_signed_heads(false);
}

#[test]
fn replayed_empty_peer_authorization_does_not_copy_unrelated_signed_heads() {
    assert_peer_authorization_replay_does_not_copy_signed_heads(true);
}

fn assert_peer_authorization_replay_does_not_copy_signed_heads(empty: bool) {
    let mut fixture = remote_send_fixture();
    let devices = if empty {
        Vec::new()
    } else {
        vec![
            fixture.ready_device.public_key(),
            fixture.late_device.public_key(),
        ]
    };
    let current = signed_app_keys(&fixture.peer_owner, &devices, 10);
    let older = signed_app_keys(&fixture.peer_owner, &[fixture.late_device.public_key()], 9);
    fixture.sender.ingest_app_keys_event(&current).unwrap();
    let events = [current, older];
    let small = events.each_ref().map(|event| {
        let (result, bytes) = remote_roster_tests::allocated_bytes(|| {
            fixture.sender.ingest_app_keys_event_with_outcome(event)
        });
        let result = result.unwrap();
        assert!(!result.changed);
        assert!(result.retry_batch.is_empty());
        bytes
    });

    // Populate the exact signed index through its existing admission API.
    // Equal-time conflicting rosters retain two candidates per unrelated owner,
    // exercising the Event/Tag/String copies seen in the idle profile.
    for _ in 0..64 {
        let owner = Keys::generate();
        let devices = (0..16)
            .map(|_| Keys::generate().public_key())
            .collect::<Vec<_>>();
        for roster in devices.chunks(8) {
            let event = signed_app_keys(&owner, roster, 10);
            assert!(fixture
                .sender
                .session_manager
                .observe_peer_app_keys_event(event, NdrUnixSeconds(10))
                .unwrap());
        }
    }
    fixture.sender.persist().unwrap();
    let before = fixture.sender.session_manager.snapshot();
    assert!(before.verified_peer_app_keys_events.len() >= 129);
    let evidence_before = fixture.sender.invite_owner_app_keys_evidence.clone();
    let provenance_before = fixture.sender.verified_app_keys_owners.clone();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: fixture.sender.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    fixture.sender.storage = storage.clone();
    let mut amplified = Vec::new();
    for repeat in 0..3 {
        for (event, small) in events.iter().zip(small) {
            let (result, large) = remote_roster_tests::allocated_bytes(|| {
                fixture.sender.ingest_app_keys_event_with_outcome(event)
            });
            let result = result.unwrap();
            assert!(!result.changed);
            assert!(result.retry_batch.is_empty());
            eprintln!(
                "signed_heads_replay empty={empty} created={} repeat={repeat} small_allocated_bytes={small} populated_allocated_bytes={large}",
                event.created_at.as_secs()
            );
            if large > small + 128 * 1024 {
                amplified.push((event.created_at.as_secs(), repeat, small, large));
            }
        }
    }
    assert_eq!(fixture.sender.session_manager.snapshot(), before);
    assert_eq!(
        fixture.sender.invite_owner_app_keys_evidence,
        evidence_before
    );
    assert_eq!(fixture.sender.verified_app_keys_owners, provenance_before);
    assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(
        amplified.is_empty(),
        "unchanged authorization copied unrelated signed heads: {amplified:?}"
    );
}

#[test]
fn replayed_local_authorization_does_not_persist_or_retry() {
    let mut fixture = remote_send_fixture();
    let event = signed_app_keys(
        &fixture.owner,
        &[fixture.device.public_key()],
        unix_now().get() + 1,
    );
    fixture.sender.ingest_app_keys_event(&event).unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: fixture.sender.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    fixture.sender.storage = storage.clone();
    let before = fixture.sender.session_manager.snapshot();
    assert!(fixture
        .sender
        .ingest_app_keys_event(&event)
        .unwrap()
        .is_empty());
    assert!(fixture
        .sender
        .ingest_app_keys_snapshot(
            event.pubkey,
            AppKeys::from_event(&event).unwrap(),
            event.created_at.as_secs()
        )
        .unwrap()
        .is_empty());
    assert_eq!(fixture.sender.session_manager.snapshot(), before);
    assert_eq!(
        storage.puts.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "identical roster must not cause a checkpoint"
    );
}

#[test]
fn authorization_admission_failure_restores_exact_previous_generation() {
    for local in [false, true] {
        for missing_evidence in [false, true] {
            let mut fixture = checkpoint_backlog_fixture(32, 8);
            let signer = if local {
                &fixture.owner
            } else {
                &fixture.peer_owner
            };
            let owner = ndr_owner(signer.public_key());
            if missing_evidence {
                fixture.sender.invite_owner_app_keys_evidence.remove(&owner);
                fixture.sender.verified_app_keys_owners.remove(&owner);
                if local {
                    fixture.sender.local_app_keys_observed = false;
                }
            }
            fixture.sender.persist().unwrap();
            let before: serde_json::Value = serde_json::from_str(
                &fixture
                    .store
                    .get(PROTOCOL_ENGINE_STATE_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let storage = Arc::new(GroupRetryCountingStorage {
                inner: fixture.store.clone(),
                puts: 0.into(),
                fail_next: true.into(),
            });
            fixture.sender.storage = storage;
            let event = signed_app_keys(signer, &[], unix_now().get() + 1);
            assert!(fixture.sender.ingest_app_keys_event(&event).is_err());
            fixture.sender.persist().unwrap();
            let after: serde_json::Value = serde_json::from_str(
                &fixture
                    .store
                    .get(PROTOCOL_ENGINE_STATE_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                after, before,
                "admission rollback local={local} missing_evidence={missing_evidence}"
            );
        }
    }
}

#[test]
fn unchanged_peer_admission_repairs_metadata_with_durable_rollback_and_retry() {
    for (missing_evidence, missing_provenance) in [(true, false), (false, true), (true, true)] {
        let mut fixture = remote_send_fixture();
        let owner = ndr_owner(fixture.peer_owner.public_key());
        let Some(ProtocolAppKeysEvidence::Verified(event)) =
            fixture.sender.invite_owner_app_keys_evidence.get(&owner)
        else {
            panic!("fixture requires exact signed peer evidence");
        };
        let event = event.as_ref().clone();
        let manager_before = fixture.sender.session_manager.snapshot();
        assert!(!fixture
            .sender
            .session_manager
            .peer_app_keys_event_would_change(&event, NdrUnixSeconds(unix_now().get()))
            .unwrap());
        if missing_evidence {
            fixture.sender.invite_owner_app_keys_evidence.remove(&owner);
        }
        if missing_provenance {
            fixture.sender.verified_app_keys_owners.remove(&owner);
        }
        fixture.sender.persist().unwrap();
        let durable_before = fixture.store.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
        let evidence_before = fixture.sender.invite_owner_app_keys_evidence.clone();
        let provenance_before = fixture.sender.verified_app_keys_owners.clone();
        let storage = Arc::new(GroupRetryCountingStorage {
            inner: fixture.store.clone(),
            puts: 0.into(),
            fail_next: true.into(),
        });
        fixture.sender.storage = storage.clone();

        assert!(fixture
            .sender
            .ingest_app_keys_event_with_outcome(&event)
            .is_err());
        assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(fixture.sender.session_manager.snapshot(), manager_before);
        assert_eq!(
            fixture.sender.invite_owner_app_keys_evidence,
            evidence_before
        );
        assert_eq!(fixture.sender.verified_app_keys_owners, provenance_before);
        assert_eq!(
            fixture.store.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(),
            durable_before
        );

        let repaired = fixture
            .sender
            .ingest_app_keys_event_with_outcome(&event)
            .unwrap();
        assert!(
            repaired.changed,
            "missing_evidence={missing_evidence} missing_provenance={missing_provenance}"
        );
        assert!(repaired.retry_batch.is_empty());
        assert_eq!(fixture.sender.session_manager.snapshot(), manager_before);
        assert!(
            matches!(fixture.sender.invite_owner_app_keys_evidence.get(&owner),
            Some(ProtocolAppKeysEvidence::Verified(retained)) if retained.as_ref() == &event)
        );
        assert!(fixture.sender.verified_app_keys_owners.contains(&owner));
        let writes = storage.puts.load(std::sync::atomic::Ordering::Relaxed);
        assert!(writes >= 2);
        let replay = fixture
            .sender
            .ingest_app_keys_event_with_outcome(&event)
            .unwrap();
        assert!(!replay.changed && replay.retry_batch.is_empty());
        assert_eq!(
            storage.puts.load(std::sync::atomic::Ordering::Relaxed),
            writes
        );

        let mut reopened = ProtocolEngine::load_or_create_for_local_device(
            fixture.store.clone(),
            fixture.owner.public_key(),
            &fixture.device,
        )
        .unwrap();
        assert!(reopened.verified_app_keys_owners.contains(&owner));
        let replay = reopened.ingest_app_keys_event_with_outcome(&event).unwrap();
        assert!(!replay.changed && replay.retry_batch.is_empty());
    }
}

#[test]
fn equal_timestamp_conflicting_authorization_remains_ambiguous() {
    let mut fixture = remote_send_fixture();
    let created = unix_now().get();
    let first = signed_app_keys(
        &fixture.peer_owner,
        &[fixture.ready_device.public_key()],
        created,
    );
    let conflicting = signed_app_keys(
        &fixture.peer_owner,
        &[fixture.late_device.public_key()],
        created,
    );
    fixture.sender.ingest_app_keys_event(&first).unwrap();
    fixture.sender.ingest_app_keys_event(&conflicting).unwrap();
    assert!(
        matches!(fixture.sender.invite_owner_app_keys_evidence.get(&ndr_owner(fixture.peer_owner.public_key())), Some(ProtocolAppKeysEvidence::Ambiguous { created_at_secs }) if *created_at_secs == created)
    );
    fixture.sender.ingest_app_keys_event(&first).unwrap();
    assert!(matches!(
        fixture
            .sender
            .invite_owner_app_keys_evidence
            .get(&ndr_owner(fixture.peer_owner.public_key())),
        Some(ProtocolAppKeysEvidence::Ambiguous { .. })
    ));
}

#[test]
fn checkpoint_cold_layout_recovers_core_without_matching_quoted_or_reordered_fields() {
    let prefix = format!("{{\"version\":{},\"pending_group_sender_key_messages\":[],\"pending_group_fanouts\":[],\"pending_inbound\":[]", PROTOCOL_ENGINE_STATE_VERSION);
    let sessions = serde_json::json!({"users": (0..40).map(|i|
        serde_json::json!({"i":i,"state":"x".repeat(2300)})).collect::<Vec<_>>()});
    let core = format!("{prefix},\"session_manager\":{sessions}}}");
    let (core, layout) = layout_protocol_checkpoint(core, &ProtocolCheckpointLayout::default());
    let quoted = serde_json::to_string("\"pending_inbound\":[],\"session_manager\":{} 🦊").unwrap();
    let json = format!("{{\"version\":{},\"pending_group_sender_key_messages\":[{{\"text\":{quoted}}}],\"pending_group_fanouts\":[],\"pending_inbound\":[]{}   ", PROTOCOL_ENGINE_STATE_VERSION, &core[prefix.len()..]);
    let restored = ProtocolCheckpointLayout::from_json(&json);
    let restored_core = restored.core.unwrap();
    assert_eq!(restored_core.record_ends, layout.record_ends);
    assert_eq!(restored_core.bytes, core.trim_end().len());
    // Old/reordered JSON is still readable, but its unrelated whitespace must
    // not be mistaken for this serializer's separately laid-out core.
    let reordered = format!("{{\"session_manager\":{sessions},\"version\":1,\"pending_group_sender_key_messages\":[],\"pending_group_fanouts\":[],\"pending_inbound\":[]}}");
    assert!(ProtocolCheckpointLayout::from_json(&reordered)
        .core
        .is_none());
    let nested =
        serde_json::json!({"nested": serde_json::from_str::<serde_json::Value>(&json).unwrap()})
            .to_string();
    assert!(ProtocolCheckpointLayout::from_json(&nested).core.is_none());
}

#[test]
fn checkpoint_cold_reopen_retains_core_boundaries_after_real_expiry() {
    let (mut fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let peer = fixture.peer_owner.public_key();
    fixture.sender.enter_batch();
    for index in 0..33 {
        let text = format!(
            "cold-start-{index}:{}",
            "x".repeat(if index == 0 { 8_000 } else { 1_200 })
        );
        let sent = fixture
            .sender
            .send_direct_text(
                peer,
                &peer.to_hex(),
                &text,
                (index == 0).then_some(200),
                UnixSeconds(100),
            )
            .unwrap();
        assert_eq!(
            decrypt_own_sync_effects(&mut fixture.ready, sent.effects).len(),
            1
        );
    }
    fixture.sender.exit_batch().unwrap();
    let before = fixture.sender.pending_remote_sends.len();
    assert!(fixture
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(200))
        .unwrap()
        .is_empty());
    assert_eq!(fixture.sender.pending_remote_sends.len(), before - 1);
    // Real expiry leaves the following smaller records in the saved larger
    // slots. A cold load must retain those boundaries instead of repacking.
    assert!(
        fixture
            .sender
            .checkpoint_layout
            .borrow()
            .core
            .as_ref()
            .unwrap()
            .bytes
            > 128 * 1024
    );
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let expected: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let owner = fixture.owner.public_key();
    let device = fixture.device.clone();
    drop(saved);
    drop(fixture);
    drop(storage);
    drop(connection);

    let mut writes = Vec::new();
    for _ in 0..2 {
        let conn = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
        conn.execute_batch("PRAGMA wal_autocheckpoint=0;
            CREATE TEMP TABLE startup_payload_writes(immutable INTEGER NOT NULL, bytes INTEGER NOT NULL);
            CREATE TEMP TRIGGER startup_payload_insert AFTER INSERT ON main.ndr_kv_checkpoint_chunks BEGIN
                INSERT INTO startup_payload_writes VALUES(substr(NEW.payload_key,1,1)=x'01',length(NEW.data));
            END;
            CREATE TEMP TRIGGER startup_payload_update AFTER UPDATE ON main.ndr_kv_checkpoint_chunks BEGIN
                INSERT INTO startup_payload_writes VALUES(substr(NEW.payload_key,1,1)=x'01',length(NEW.data));
            END;").unwrap();
        let connection = Arc::new(Mutex::new(conn));
        let storage = Arc::new(SqliteStorageAdapter::new(
            connection.clone(),
            owner.to_hex(),
            device.public_key().to_hex(),
        ));
        let before = checkpoint_growth_written_bytes(&connection);
        let restored =
            ProtocolEngine::load_or_create_for_local_device(storage.clone(), owner, &device)
                .unwrap();
        let written = checkpoint_growth_written_bytes(&connection) - before;
        let (immutable, inline): (u64,u64) = connection.lock().unwrap().query_row(
            "SELECT coalesce(sum(CASE WHEN immutable THEN bytes ELSE 0 END),0),coalesce(sum(CASE WHEN NOT immutable THEN bytes ELSE 0 END),0) FROM startup_payload_writes",[],|row|Ok((row.get(0)?,row.get(1)?))
        ).unwrap();
        assert_eq!(
            immutable, 0,
            "cold caches must not reinsert immutable bodies"
        );
        let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
        assert_eq!(saved.len(), restored.checkpoint_layout.borrow().bytes);
        assert!(serde_json::from_str::<serde_json::Value>(&saved).unwrap() == expected,
            "startup must preserve the complete queued state, ratchets, journal and authorization evidence");
        eprintln!("checkpoint_cold_reopen physical_write_bytes={written} immutable_insert_update_bytes={immutable} inline_insert_update_bytes={inline}");
        writes.push(written);
        drop(restored);
        drop(storage);
        drop(connection);
    }
    // The materialized JSON does not distinguish core trailing whitespace
    // from the final reserve. A first restore may normalize its last chunk.
    assert!(
        writes[0] <= 128 * 1024,
        "cold restore repacked the mutable core: {}",
        writes[0]
    );
    assert_eq!(
        writes[1], 0,
        "a second unchanged cold restore must not write data"
    );
}

#[test]
fn checkpoint_cold_reopen_migrates_legacy_once_without_losing_state() {
    let (fixture, directory, connection, storage) = checkpoint_growth_fixture();
    let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
    let expected: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let owner = fixture.owner.public_key();
    let device = fixture.device.clone();
    // The ordinary adapter path represents an older whole-value checkpoint;
    // no logical-record metadata survives this replacement.
    storage.put(PROTOCOL_ENGINE_STATE_KEY, saved).unwrap();
    let count: usize = connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM ndr_kv_checkpoint", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    drop(fixture);
    drop(storage);
    drop(connection);
    let mut writes = Vec::new();
    for _ in 0..2 {
        let connection = Arc::new(Mutex::new(
            rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap(),
        ));
        let storage = Arc::new(SqliteStorageAdapter::new(
            connection.clone(),
            owner.to_hex(),
            device.public_key().to_hex(),
        ));
        let before = checkpoint_growth_written_bytes(&connection);
        let restored =
            ProtocolEngine::load_or_create_for_local_device(storage.clone(), owner, &device)
                .unwrap();
        writes.push(checkpoint_growth_written_bytes(&connection) - before);
        let saved = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap().unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&saved).unwrap() == expected);
        let count: usize = connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM ndr_kv_checkpoint", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        drop(restored);
        drop(storage);
        drop(connection);
    }
    assert!(
        writes[0] > 0,
        "first migration must actually store the payloads"
    );
    assert_eq!(
        writes[1], 0,
        "completed migration must not recur on cold reopen"
    );
}
