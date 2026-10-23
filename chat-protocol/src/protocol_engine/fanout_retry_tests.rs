#[test]
fn group_fanout_retry_allocations_do_not_scale_with_deferred_fanouts() {
    fn allocated_with_tail(total: usize) -> usize {
        let mut f = checkpoint_backlog_fixture(64, 0);
        f.sender.ack_pending_decrypted_deliveries().unwrap();
        let prototype = f.sender.pending_group_fanouts[0].clone();
        for index in 64..total {
            let mut pending = prototype.clone();
            pending.inner_event_id = Some(format!("deferred-{index}"));
            f.sender.pending_group_fanouts.push(pending);
        }
        let now = 1_000;
        for pending in &mut f.sender.pending_group_fanouts[..64] {
            pending.next_retry_at_secs = now;
        }
        let mut expected = f.sender.pending_group_fanouts.clone();
        for pending in &mut expected[..64] {
            pending.next_retry_at_secs = now + 8 * 60;
        }
        let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
        let storage = Arc::new(GroupRetryCountingStorage {
            inner: f.sender.storage.clone(),
            puts: 0.into(),
            fail_next: false.into(),
        });
        f.sender.storage = storage.clone();
        let ((), allocated) = remote_roster_tests::allocated_bytes(|| {
            for turn in 0..8 {
                let result = f
                    .sender
                    .retry_pending_group_fanouts(NdrUnixSeconds(now + turn * 60))
                    .unwrap();
                assert!(result.events.is_empty() && result.effects.is_empty());
            }
        });
        assert!(
            f.sender.pending_group_fanouts == expected,
            "backoff retries must preserve every target and the full queue order"
        );
        assert!(
            serde_json::to_value(f.sender.session_manager.snapshot()).unwrap() == sessions,
            "unavailable targets must not advance sessions"
        );
        assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 0);
        allocated
    }
    let small = allocated_with_tail(64);
    let large = allocated_with_tail(24_990);
    eprintln!("fanout_queue_retry turns=8 due=64 total=24990 small_allocated_bytes={small} large_allocated_bytes={large}");
    assert!(
        large <= small + 64 * 1024,
        "fanout retries copied deferred queue metadata: {small} -> {large} bytes"
    );
}

#[test]
fn group_fanout_retry_mixed_updates_restore_order_after_failed_save() {
    let mut f = checkpoint_backlog_fixture(5, 0);
    f.sender.ack_pending_decrypted_deliveries().unwrap();
    let now = 1_000;
    for (index, pending) in f.sender.pending_group_fanouts.iter_mut().enumerate() {
        pending.inner_event_id = Some(format!("ordered-{index}"));
        if index != 2 {
            pending.next_retry_at_secs = now;
        }
        if index == 1 || index == 3 {
            pending.remaining_devices = Some(BTreeSet::new());
        }
    }
    f.sender.persist().unwrap();
    let original = f.sender.pending_group_fanouts.clone();
    let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    f.sender.storage = storage.clone();
    let error = f
        .sender
        .retry_pending_group_fanouts(NdrUnixSeconds(now))
        .unwrap_err();
    assert!(error.downcast_ref::<StorageError>().is_some());
    assert!(
        f.sender.pending_group_fanouts == original,
        "failed save must restore replacements, removals and deferred positions"
    );
    assert!(serde_json::to_value(f.sender.session_manager.snapshot()).unwrap() == sessions);
    assert!(f.sender.ready_group_publications.is_empty());
    assert!(!f.sender.batch_persist_dirty.get());
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    assert!(
        restored.pending_group_fanouts == original,
        "failed save must leave the restart queue unchanged"
    );
    let result = restored
        .retry_pending_group_fanouts(NdrUnixSeconds(now))
        .unwrap();
    assert!(result.events.is_empty() && result.effects.is_empty());
    let expected = original
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut pending)| {
            if index == 1 || index == 3 {
                None
            } else {
                if index != 2 {
                    pending.next_retry_at_secs = now + 60;
                }
                Some(pending)
            }
        })
        .collect::<Vec<_>>();
    assert!(
        restored.pending_group_fanouts == expected,
        "successful retry must preserve retained order and exact deadlines"
    );
    let restarted =
        ProtocolEngine::load_or_create_for_local_device(storage, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(
        restarted.pending_group_fanouts == expected,
        "successful queue changes must survive restart"
    );
}

#[test]
fn group_fanout_retry_retires_ineligible_deferred_entries_beyond_due_limit() {
    let mut f = checkpoint_backlog_fixture(68, 0);
    f.sender.ack_pending_decrypted_deliveries().unwrap();
    let now = 1_000;
    for (index, pending) in f.sender.pending_group_fanouts.iter_mut().enumerate() {
        pending.inner_event_id = Some(format!("retirement-{index}"));
        if index < 64 {
            pending.next_retry_at_secs = now;
        }
    }
    f.sender
        .hold_pending_direct_sends(f.peer_owner.public_key(), true);
    let wake = UnsignedEvent::new(
        f.owner.public_key(),
        Timestamp::from_secs(now),
        Kind::Custom(21_113),
        [],
        "private wake",
    );
    f.sender.pending_group_fanouts[64].fanout = GroupPendingFanout::Remote {
        recipient_owner: ndr_owner(f.peer_owner.public_key()),
        payload: serde_json::to_vec(&wake).unwrap(),
    }
    .into();
    let inactive = group_snapshot_for_test(
        "removed-group",
        "Removed",
        2,
        &f.peer_owner,
        &[f.peer_owner.public_key()],
    );
    f.sender.group_manager = GroupEventManager::from_snapshot(GroupManagerSnapshot {
        local_owner_pubkey: f.sender.local_owner,
        groups: vec![inactive.clone()],
        sender_keys: vec![],
    })
    .unwrap();
    f.sender.pending_group_fanouts[65].group_id = inactive.group_id.clone();
    f.sender.pending_group_fanouts[66].group_id = inactive.group_id.clone();
    f.sender.pending_group_fanouts[66].fanout = GroupPendingFanout::Remote {
        recipient_owner: ndr_owner(f.peer_owner.public_key()),
        payload: JsonGroupPayloadCodecV1
            .encode_pairwise_command(
                nostr_double_ratchet::GroupPayloadEncodeContext {
                    local_device_pubkey: ndr_device(f.device.public_key()),
                    created_at: NdrUnixSeconds(now),
                },
                &GroupPairwiseCommand::MetadataSnapshot { snapshot: inactive },
            )
            .unwrap(),
    }
    .into();
    let mut expected = f.sender.pending_group_fanouts.clone();
    expected.remove(65);
    expected.remove(64);
    for pending in &mut expected[..64] {
        pending.next_retry_at_secs = now + 60;
    }
    let result = f
        .sender
        .retry_pending_group_fanouts(NdrUnixSeconds(now))
        .unwrap();
    assert!(result.events.is_empty() && result.effects.is_empty());
    assert!(f.sender.pending_group_fanouts == expected,
        "held wakes and revoked controls must retire even beyond the due cap; membership metadata and other deferred entries must retain order");
    assert_eq!(f.sender.pending_group_fanouts.len(), 66);
}
