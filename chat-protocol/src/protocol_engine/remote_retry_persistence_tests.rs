fn missing_roster_remote_retries(count: usize, now: u64) -> Vec<ProtocolPendingRemoteSend> {
    let owner = ndr_owner(Keys::generate().public_key());
    let completed = ndr_device(Keys::generate().public_key());
    let waiting = ndr_device(Keys::generate().public_key());
    (0..count)
        .map(|index| ProtocolPendingRemoteSend {
            recipient_owner: owner,
            eligible_devices: BTreeSet::from([completed, waiting]),
            completed_devices: BTreeSet::from([completed]),
            chat_id: owner.to_hex(),
            payload: format!("missing-roster-{index}").into_bytes(),
            inner_event_id: None,
            message_id: format!("missing-{index}"),
            expires_at_secs: None,
            created_at_secs: now.saturating_sub(1_000),
            next_retry_at_secs: now,
        })
        .collect()
}

#[test]
fn remote_retry_missing_roster_backoff_does_not_persist() {
    for (outer_batch, already_dirty) in [(false, false), (true, false), (true, true)] {
        let mut f = remote_send_fixture();
        let now = 1_000;
        f.sender.pending_remote_sends = missing_roster_remote_retries(44, now);
        assert!(f
            .sender
            .session_manager
            .roster(f.sender.pending_remote_sends[0].recipient_owner)
            .is_none());
        f.sender.persist().unwrap();
        let original = f.sender.pending_remote_sends.clone();
        let before = f.sender.storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
        let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
        let storage = Arc::new(GroupRetryCountingStorage {
            inner: f.sender.storage.clone(),
            puts: 0.into(),
            fail_next: false.into(),
        });
        f.sender.storage = storage.clone();
        if outer_batch {
            f.sender.enter_batch();
            if already_dirty {
                f.sender.subscription_generation += 1;
                f.sender.persist().unwrap();
            }
        }
        for turn in 0..3 {
            assert!(f
                .sender
                .retry_pending_remote_sends(NdrUnixSeconds(now + turn * 60))
                .unwrap()
                .is_empty());
            assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert_eq!(f.sender.batch_persist_dirty.get(), already_dirty);
        }
        let mut expected = original.clone();
        for pending in &mut expected {
            pending.next_retry_at_secs = now + 3 * 60;
        }
        assert!(
            serde_json::to_value(&f.sender.pending_remote_sends).unwrap()
                == serde_json::to_value(&expected).unwrap(),
            "missing-roster retries may change only the retry clocks"
        );
        assert!(serde_json::to_value(f.sender.session_manager.snapshot()).unwrap() == sessions);
        assert!(
            storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap() == before,
            "backoff-only retries must not rewrite the durable checkpoint"
        );
        if outer_batch {
            f.sender.exit_batch().unwrap();
        }
        assert_eq!(
            storage.puts.load(std::sync::atomic::Ordering::Relaxed),
            usize::from(already_dirty)
        );
        let mut restored = ProtocolEngine::load_or_create_for_local_device(
            storage.clone(),
            f.owner.public_key(),
            &f.device,
        )
        .unwrap();
        let durable = if already_dirty { &expected } else { &original };
        assert!(
            serde_json::to_value(&restored.pending_remote_sends).unwrap()
                == serde_json::to_value(durable).unwrap(),
            "a restart must preserve the last durable targets and completion state"
        );
        if !already_dirty {
            let writes = storage.puts.load(std::sync::atomic::Ordering::Relaxed);
            assert!(restored
                .retry_pending_remote_sends(NdrUnixSeconds(now + 120))
                .unwrap()
                .is_empty());
            assert_eq!(
                storage.puts.load(std::sync::atomic::Ordering::Relaxed),
                writes
            );
            assert!(
                serde_json::to_value(&restored.pending_remote_sends).unwrap()
                    == serde_json::to_value(&expected).unwrap()
            );
        }
    }
}

#[test]
fn remote_retry_missing_roster_expiry_and_revocation_remain_durable() {
    for revoked in [false, true] {
        let mut f = remote_send_fixture();
        if revoked {
            let other = Keys::generate();
            f.sender
                .ingest_app_keys_event(&signed_app_keys(&f.owner, &[other.public_key()], 30))
                .unwrap();
            assert_eq!(f.sender.signed_local_device_authorization(), Some(false));
        }
        let now = 1_000;
        f.sender.pending_remote_sends = missing_roster_remote_retries(1, now);
        if !revoked {
            f.sender.pending_remote_sends[0].expires_at_secs = Some(now);
        }
        f.sender.persist().unwrap();
        let original = serde_json::to_value(&f.sender.pending_remote_sends).unwrap();
        let storage = Arc::new(GroupRetryCountingStorage {
            inner: f.sender.storage.clone(),
            puts: 0.into(),
            fail_next: true.into(),
        });
        f.sender.storage = storage.clone();
        let error = f
            .sender
            .retry_pending_remote_sends(NdrUnixSeconds(now))
            .unwrap_err();
        assert!(error.downcast_ref::<StorageError>().is_some());
        assert!(
            serde_json::to_value(&f.sender.pending_remote_sends).unwrap() == original,
            "failed retirement must restore the exact pending send"
        );
        assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(f
            .sender
            .retry_pending_remote_sends(NdrUnixSeconds(now))
            .unwrap()
            .is_empty());
        assert!(f.sender.pending_remote_sends.is_empty());
        assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 2);
        let restored = ProtocolEngine::load_or_create_for_local_device(
            storage,
            f.owner.public_key(),
            &f.device,
        )
        .unwrap();
        assert!(
            restored.pending_remote_sends.is_empty(),
            "expiry and local revocation must remain retired after restart"
        );
    }
}

#[test]
fn remote_retry_mixed_progress_and_backoff_restore_after_failed_save() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let sent = f
        .sender
        .send_direct_text(peer, &peer.to_hex(), "late delivery", None, UnixSeconds(10))
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
        1
    );
    let mut pending = std::mem::take(&mut f.sender.pending_remote_sends);
    assert_eq!(pending.len(), 1);
    observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    let now = 1_000;
    pending[0].next_retry_at_secs = now;
    f.sender.pending_remote_sends = missing_roster_remote_retries(1, now);
    f.sender.pending_remote_sends.extend(pending);
    f.sender.persist().unwrap();
    let original = serde_json::to_value(&f.sender.pending_remote_sends).unwrap();
    let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let mut expected = f.sender.pending_remote_sends[..1].to_vec();
    expected[0].next_retry_at_secs = now + 60;
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    f.sender.storage = storage.clone();
    let error = f
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(now))
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert!(
        serde_json::to_value(&f.sender.pending_remote_sends).unwrap() == original,
        "failed mixed retry must restore clocks, targets and completion sets"
    );
    assert!(
        serde_json::to_value(f.sender.session_manager.snapshot()).unwrap() == sessions,
        "failed mixed retry must restore the sending ratchet"
    );
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    assert!(serde_json::to_value(&restored.pending_remote_sends).unwrap() == original);
    let writes = storage.puts.load(std::sync::atomic::Ordering::Relaxed);
    let effects = restored
        .retry_pending_remote_sends(NdrUnixSeconds(now))
        .unwrap();
    assert_eq!(
        storage.puts.load(std::sync::atomic::Ordering::Relaxed),
        writes + 1
    );
    let mut deliveries = 0;
    for effect in &effects {
        let ProtocolEffect::Publish(publish) = effect;
        publish.event.verify().unwrap();
        if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND {
            deliveries += 1;
            let wire = serde_json::to_string(&publish.event).unwrap();
            for identity in [&f.owner, &f.device, &f.peer_owner, &f.ready_device] {
                assert!(
                    !wire.contains(&identity.public_key().to_hex()),
                    "retry wire must not expose either owner or the sending identity"
                );
            }
        }
    }
    assert_eq!(deliveries, 1, "completed devices must not be sent again");
    assert_eq!(decrypt_own_sync_effects(&mut f.late, effects).len(), 1);
    assert!(
        serde_json::to_value(&restored.pending_remote_sends).unwrap()
            == serde_json::to_value(&expected).unwrap()
    );
    let restarted =
        ProtocolEngine::load_or_create_for_local_device(storage, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(
        serde_json::to_value(&restarted.pending_remote_sends).unwrap()
            == serde_json::to_value(&expected).unwrap(),
        "successful delivery progress must survive restart"
    );
}

#[test]
fn remote_retry_known_roster_without_effects_still_persists() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(peer, &peer.to_hex(), "waiting", None, UnixSeconds(10))
        .unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    f.sender.storage = storage.clone();
    assert!(f
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(1_000))
        .unwrap()
        .is_empty());
    assert_eq!(
        storage.puts.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "empty effects cannot certify that known-roster preparation was mutation-free"
    );
    f.sender.hold_pending_direct_sends(peer, true);
    let held = serde_json::to_value(&f.sender.pending_remote_sends).unwrap();
    assert!(f
        .sender
        .retry_pending_remote_sends(NdrUnixSeconds(1_060))
        .unwrap()
        .is_empty());
    assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(
        serde_json::to_value(&f.sender.pending_remote_sends).unwrap() == held,
        "held sends must retain their exact pending state"
    );
}
