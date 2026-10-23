thread_local! {
    static FANOUT_RETRY_DEADLINE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_fanout_retry_deadline_scan() {
    FANOUT_RETRY_DEADLINE_SCANS.with(|count| count.set(count.get() + 1));
}

fn take_fanout_retry_deadline_scans() -> usize {
    FANOUT_RETRY_DEADLINE_SCANS.with(|count| count.replace(0))
}

fn deferred_fanout_prepared(
    owner: NdrOwnerPubkey,
    fanout: GroupPendingFanout,
) -> GroupPreparedPublish {
    GroupPreparedPublish {
        deliveries: Vec::new(),
        invite_responses: Vec::new(),
        sender_key_messages: Vec::new(),
        relay_gaps: vec![nostr_double_ratchet::RelayGap::MissingRoster {
            owner_pubkey: owner,
        }],
        pending_fanouts: vec![fanout],
    }
}

#[test]
fn fanout_deadline_scans_are_bounded_across_32_continuations() {
    let mut f = checkpoint_backlog_fixture(64, 0);
    f.sender.ack_pending_decrypted_deliveries().unwrap();
    // Establishing the fixture's session also queues the greeting for the
    // undiscovered peer device. Preserve that intent, but defer its separate
    // deadline so the public scheduler reaches the fanout due check.
    assert_eq!(f.sender.pending_remote_sends.len(), 1);
    let greeting = &mut f.sender.pending_remote_sends[0];
    assert_eq!(
        greeting.recipient_owner,
        ndr_owner(f.peer_owner.public_key())
    );
    assert_eq!(greeting.created_at_secs, 10);
    assert!(greeting.next_retry_at_secs <= 1_000);
    assert!(greeting
        .eligible_devices
        .contains(&ndr_device(f.late_device.public_key())));
    assert!(!greeting
        .completed_devices
        .contains(&ndr_device(f.late_device.public_key())));
    greeting.next_retry_at_secs = u64::MAX;
    let deferred_remote = serde_json::to_value(&f.sender.pending_remote_sends).unwrap();
    let prototype = f.sender.pending_group_fanouts[0].clone();
    for index in 64..24_990 {
        let mut pending = prototype.clone();
        pending.inner_event_id = Some(format!("deferred-{index}"));
        f.sender.pending_group_fanouts.push(pending);
    }
    let now = NdrUnixSeconds(1_000);
    for pending in &mut f.sender.pending_group_fanouts[..64] {
        pending.next_retry_at_secs = now.get();
    }
    let mut expected = f.sender.pending_group_fanouts.clone();
    for pending in &mut expected[..64] {
        pending.next_retry_at_secs = 1_060;
    }
    let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: f.sender.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    f.sender.storage = storage.clone();
    take_fanout_retry_deadline_scans();
    assert!(f.sender.has_due_pending_retry_work(now));
    assert_eq!(
        take_fanout_retry_deadline_scans(),
        1,
        "a positive query must stop at the first due row"
    );
    GROUP_FANOUT_RETRY_VISITED.with(|count| count.set(0));
    GROUP_FANOUT_RETRY_PREPARE_ATTEMPTS.with(|count| count.set(0));
    for turn in 0..32 {
        assert_eq!(f.sender.has_due_pending_retry_work(now), turn == 0);
        let result = f.sender.retry_pending_group_fanouts(now).unwrap();
        assert!(result.events.is_empty() && result.effects.is_empty());
    }
    let scans = take_fanout_retry_deadline_scans();
    assert_eq!(GROUP_FANOUT_RETRY_VISITED.with(|count| count.get()), 64);
    assert_eq!(
        GROUP_FANOUT_RETRY_PREPARE_ATTEMPTS.with(|count| count.get()),
        64
    );
    assert_eq!(f.sender.pending_group_fanouts, expected);
    assert!(!f.sender.has_due_pending_retry_work(NdrUnixSeconds(1_059)));
    assert!(f.sender.has_due_pending_retry_work(NdrUnixSeconds(1_060)));
    let result = f
        .sender
        .retry_pending_group_fanouts(NdrUnixSeconds(1_060))
        .unwrap();
    assert!(result.events.is_empty() && result.effects.is_empty());
    for pending in &mut expected[..64] {
        pending.next_retry_at_secs = 1_120;
    }
    assert_eq!(f.sender.pending_group_fanouts, expected);
    assert_eq!(
        GROUP_FANOUT_RETRY_PREPARE_ATTEMPTS.with(|count| count.get()),
        128
    );
    assert_eq!(
        serde_json::to_value(f.sender.session_manager.snapshot()).unwrap(),
        sessions
    );
    assert!(f.sender.pending_decrypted_deliveries.is_empty());
    assert_eq!(
        serde_json::to_value(&f.sender.pending_remote_sends).unwrap(),
        deferred_remote
    );
    assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 0);
    eprintln!("fanout_deadline turns=32 total=24990 due=64 deadline_reads={scans}");
    assert!(
        (24_990..=24_992).contains(&scans),
        "32 continuations inspected {scans} fanout deadlines"
    );
}

#[test]
fn fanout_deadline_notices_earlier_enqueues_after_empty_or_deferred_queue() {
    for initial_count in [0, 2] {
        for local in [false, true] {
            let mut engine = test_engine(&Keys::generate(), &Keys::generate());
            engine.pending_group_fanouts = (0..initial_count)
                .map(|index| {
                    pending_local_group_fanout("old", Some(&index.to_string()), b"old", u64::MAX)
                })
                .collect();
            let original = engine.pending_group_fanouts.clone();
            assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(0)));
            let owner = if local {
                engine.local_owner
            } else {
                ndr_owner(Keys::generate().public_key())
            };
            let fanout = if local {
                GroupPendingFanout::LocalSiblings {
                    payload: b"new".to_vec(),
                }
            } else {
                GroupPendingFanout::Remote {
                    recipient_owner: owner,
                    payload: b"new".to_vec(),
                }
            };
            assert!(engine.queue_group_pending_fanouts(
                "new",
                &deferred_fanout_prepared(owner, fanout),
                Some("new".into())
            ));
            assert_eq!(engine.pending_group_fanouts[..initial_count], original);
            let added = &engine.pending_group_fanouts[initial_count];
            assert_eq!(added.remaining_devices, None);
            let deadline = added.next_retry_at_secs;
            assert!(deadline >= added.created_at_secs + PENDING_RETRY_DELAY_SECS);
            assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(deadline - 1)));
            assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(deadline)));
        }
    }
}

#[test]
fn fanout_deadline_same_length_sync_replacement_and_duplicate_repair() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    engine
        .pending_group_fanouts
        .push(pending_local_group_fanout("sync", None, b"old", u64::MAX));
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(0)));
    let prepared = deferred_fanout_prepared(
        engine.local_owner,
        GroupPendingFanout::LocalSiblings {
            payload: b"new".to_vec(),
        },
    );
    assert!(engine.queue_group_pending_fanouts("sync", &prepared, None));
    assert_eq!(engine.pending_group_fanouts.len(), 1);
    assert_eq!(
        engine.pending_group_fanouts[0].fanout.as_ref(),
        &prepared.pending_fanouts[0]
    );
    let deadline = engine.pending_group_fanouts[0].next_retry_at_secs;
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(deadline)));

    let mut f = checkpoint_backlog_fixture(1, 0);
    let original = f.sender.pending_group_fanouts.clone();
    assert!(!f
        .sender
        .has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
    let prepared = GroupPreparedPublish {
        deliveries: Vec::new(),
        invite_responses: Vec::new(),
        sender_key_messages: Vec::new(),
        relay_gaps: vec![nostr_double_ratchet::RelayGap::MissingDeviceInvite {
            owner_pubkey: ndr_owner(f.peer_owner.public_key()),
            device_pubkey: ndr_device(f.late_device.public_key()),
        }],
        pending_fanouts: vec![original[0].fanout.as_ref().clone()],
    };
    assert!(!f
        .sender
        .queue_group_pending_fanouts("backlog-group", &prepared, None));
    assert_eq!(f.sender.pending_group_fanouts, original);
    assert!(!f
        .sender
        .has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
}

#[test]
fn fanout_deadline_owner_wake_changes_only_matching_remote_deadlines() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let peer = ndr_owner(Keys::generate().public_key());
    for recipient in [peer, ndr_owner(Keys::generate().public_key())] {
        let mut pending = pending_local_group_fanout("g", Some("event"), b"control", 1_000);
        pending.fanout = GroupPendingFanout::Remote {
            recipient_owner: recipient,
            payload: b"control".to_vec(),
        }
        .into();
        engine.pending_group_fanouts.push(pending);
    }
    engine
        .pending_group_fanouts
        .push(pending_local_group_fanout(
            "g",
            Some("local"),
            b"local",
            1_000,
        ));
    let mut expected = engine.pending_group_fanouts.clone();
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(999)));
    engine.wake_pending_protocol_for_owner(peer);
    expected[0].next_retry_at_secs = 0;
    assert_eq!(engine.pending_group_fanouts, expected);
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(0)));
}

#[test]
fn fanout_deadline_block_retirement_preserves_control_and_reload_cutoff() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let wake = UnsignedEvent::new(
        owner.public_key(),
        Timestamp::from_secs(10),
        Kind::Custom(21_113),
        [],
        "wake",
    );
    let mut pending = pending_local_group_fanout("direct", Some("wake"), b"", 1_000);
    pending.fanout = GroupPendingFanout::Remote {
        recipient_owner: ndr_owner(peer.public_key()),
        payload: serde_json::to_vec(&wake).unwrap(),
    }
    .into();
    let control = pending_local_group_fanout("group", Some("control"), b"control", 2_000);
    engine.pending_group_fanouts = vec![pending, control.clone()];
    engine.persist().unwrap();
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(999)));
    assert_eq!(
        engine
            .retire_pending_direct_sends(peer.public_key(), None)
            .unwrap(),
        1
    );
    assert_eq!(engine.pending_group_fanouts, vec![control.clone()]);
    assert!(!engine.has_due_pending_retry_work(NdrUnixSeconds(1_000)));
    assert!(engine.has_due_pending_retry_work(NdrUnixSeconds(2_000)));
    let restored = ProtocolEngine::load_or_create_for_local_device(
        engine.storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    assert_eq!(restored.pending_group_fanouts, vec![control]);
    take_fanout_retry_deadline_scans();
    assert!(!restored.has_due_pending_group_fanout_retry(NdrUnixSeconds(1_999)));
    assert_eq!(
        take_fanout_retry_deadline_scans(),
        1,
        "reload must derive the retained cutoff"
    );
    assert!(restored.has_due_pending_group_fanout_retry(NdrUnixSeconds(2_000)));
}

#[test]
fn fanout_deadline_general_and_fanout_rollback_discard_transient_cutoff() {
    for fanout_checkpoint in [false, true] {
        let mut f = checkpoint_backlog_fixture(3, 0);
        f.sender.ack_pending_decrypted_deliveries().unwrap();
        for pending in &mut f.sender.pending_group_fanouts {
            pending.next_retry_at_secs = 1_000;
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
        assert!(!f
            .sender
            .has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
        let finish = |engine: &mut ProtocolEngine| -> anyhow::Result<()> {
            assert!(engine
                .pending_group_fanouts
                .iter()
                .all(|pending| pending.next_retry_at_secs == 1_060));
            assert!(!engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
            engine.persist()
        };
        let result = if fanout_checkpoint {
            f.sender.with_group_fanout_checkpoint(|engine, undo| {
                engine.retry_pending_group_fanouts_inner(NdrUnixSeconds(1_000), undo)?;
                finish(engine)
            })
        } else {
            f.sender.with_state_checkpoint(|engine| {
                engine.retry_pending_group_fanouts(NdrUnixSeconds(1_000))?;
                finish(engine)
            })
        };
        assert!(result.unwrap_err().is::<StorageError>());
        assert_eq!(f.sender.pending_group_fanouts, original);
        assert_eq!(
            serde_json::to_value(f.sender.session_manager.snapshot()).unwrap(),
            sessions
        );
        take_fanout_retry_deadline_scans();
        assert!(f
            .sender
            .has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
        assert_eq!(
            take_fanout_retry_deadline_scans(),
            1,
            "rollback must discard the transient future cutoff"
        );
        let restored = ProtocolEngine::load_or_create_for_local_device(
            storage,
            f.owner.public_key(),
            &f.device,
        )
        .unwrap();
        assert_eq!(restored.pending_group_fanouts, original);
        take_fanout_retry_deadline_scans();
        assert!(!restored.has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
        assert_eq!(take_fanout_retry_deadline_scans(), original.len());
        assert!(restored.has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
        let persisted = restored
            .storage
            .get(PROTOCOL_ENGINE_STATE_KEY)
            .unwrap()
            .unwrap();
        let state: serde_json::Value = serde_json::from_str(&persisted).unwrap();
        assert!(state.get("pending_group_fanout_retry_deadline").is_none());
        assert_eq!(
            state["pending_group_fanouts"],
            serde_json::to_value(&original).unwrap()
        );
    }
}

#[test]
fn fanout_deadline_failed_invite_save_rederives_restored_cutoff() {
    let mut engine = test_engine(&Keys::generate(), &Keys::generate());
    let peer = Keys::generate();
    let peer_engine = test_engine(&peer, &peer);
    let invite = invite_unsigned_event(&peer_engine.local_invite().unwrap())
        .unwrap()
        .sign_with_keys(&peer)
        .unwrap();
    let mut pending = pending_local_group_fanout("g", Some("event"), b"control", 1_000);
    pending.fanout = GroupPendingFanout::Remote {
        recipient_owner: ndr_owner(peer.public_key()),
        payload: b"control".to_vec(),
    }
    .into();
    engine.pending_group_fanouts.push(pending);
    let original = engine.pending_group_fanouts.clone();
    engine.storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    assert!(!engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
    assert!(engine
        .observe_invite_event(&invite)
        .unwrap_err()
        .is::<StorageError>());
    assert_eq!(engine.pending_group_fanouts, original);
    take_fanout_retry_deadline_scans();
    assert!(!engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
    assert_eq!(
        take_fanout_retry_deadline_scans(),
        1,
        "invite rollback must rederive the cutoff"
    );
    assert!(engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
}

#[test]
fn fanout_deadline_standalone_group_restore_routes_rederive_cutoff() {
    for receive in [false, true] {
        let (mut engine, _, _) = queued_group_history(1);
        engine
            .pending_group_fanouts
            .push(pending_local_group_fanout(
                "queued-group",
                Some("event"),
                b"control",
                1_000,
            ));
        let original = engine.pending_group_fanouts.clone();
        let group = engine.group_manager.snapshot();
        let message = if receive {
            let parsed = engine.pending_group_sender_key_messages.pop().unwrap();
            Some(
                engine
                    .group_sender_key_message_from_parsed(&parsed)
                    .unwrap(),
            )
        } else {
            None
        };
        engine.persist().unwrap();
        engine.storage = Arc::new(GroupRetryCountingStorage {
            inner: engine.storage.clone(),
            puts: 0.into(),
            fail_next: true.into(),
        });
        assert!(!engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
        let result = if let Some(message) = message.clone() {
            engine.handle_group_sender_key_message(message)
        } else {
            engine.retry_eligible_group_sender_key_messages()
        };
        assert!(result.unwrap_err().is::<StorageError>());
        assert_eq!(engine.pending_group_fanouts, original);
        assert_eq!(engine.group_manager.snapshot(), group);
        assert!(engine.pending_decrypted_deliveries.is_empty());
        take_fanout_retry_deadline_scans();
        assert!(!engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(999)));
        assert_eq!(
            take_fanout_retry_deadline_scans(),
            1,
            "standalone group rollback must rederive the cutoff"
        );
        assert!(engine.has_due_pending_group_fanout_retry(NdrUnixSeconds(1_000)));
        let retried = if let Some(message) = message {
            engine.handle_group_sender_key_message(message)
        } else {
            engine.retry_eligible_group_sender_key_messages()
        }
        .unwrap();
        assert_eq!(retried.events.len(), 1);
        assert_eq!(engine.pending_decrypted_deliveries.len(), 1);
        assert_eq!(engine.pending_group_fanouts, original);
    }
}
