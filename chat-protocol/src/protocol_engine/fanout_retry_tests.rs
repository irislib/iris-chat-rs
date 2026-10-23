#[test]
fn group_fanout_retry_processes_a_bounded_due_batch() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    for index in 0..70 {
        engine
            .pending_group_fanouts
            .push(pending_local_group_fanout(
                "group-a",
                Some(&format!("message-{index}")),
                &[index as u8],
                1,
            ));
    }

    engine
        .retry_pending_group_fanouts(NdrUnixSeconds(10))
        .expect("retry pending fanouts");

    let due_remaining = engine
        .pending_group_fanouts
        .iter()
        .filter(|pending| pending.next_retry_at_secs <= 10)
        .count();
    assert_eq!(due_remaining, 6);
}

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
        GROUP_FANOUT_RETRY_VISITED.with(|visited| visited.set(0));
        let ((), allocated) = remote_roster_tests::allocated_bytes(|| {
            for turn in 0..8 {
                let result = f
                    .sender
                    .retry_pending_group_fanouts(NdrUnixSeconds(now + turn * 60))
                    .unwrap();
                assert!(result.events.is_empty() && result.effects.is_empty());
            }
        });
        assert_eq!(
            GROUP_FANOUT_RETRY_VISITED.with(|visited| visited.get()),
            8 * 64,
            "without privacy retirements, each turn must stop at the due cap"
        );
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
    for (held, inactive) in [(true, false), (false, true), (true, true)] {
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
            .hold_pending_direct_sends(f.peer_owner.public_key(), held);
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
        let mut removed_group = group_snapshot_for_test(
            "removed-group",
            "Removed",
            2,
            &f.peer_owner,
            &[f.peer_owner.public_key()],
        );
        if !inactive {
            removed_group.members.push(f.sender.local_owner);
        }
        f.sender.group_manager = GroupEventManager::from_snapshot(GroupManagerSnapshot {
            local_owner_pubkey: f.sender.local_owner,
            groups: vec![removed_group.clone()],
            sender_keys: vec![],
        })
        .unwrap();
        f.sender.pending_group_fanouts[65].group_id = removed_group.group_id.clone();
        f.sender.pending_group_fanouts[66].group_id = removed_group.group_id.clone();
        f.sender.pending_group_fanouts[66].fanout = GroupPendingFanout::Remote {
            recipient_owner: ndr_owner(f.peer_owner.public_key()),
            payload: JsonGroupPayloadCodecV1
                .encode_pairwise_command(
                    nostr_double_ratchet::GroupPayloadEncodeContext {
                        local_device_pubkey: ndr_device(f.device.public_key()),
                        created_at: NdrUnixSeconds(now),
                    },
                    &GroupPairwiseCommand::MetadataSnapshot {
                        snapshot: removed_group,
                    },
                )
                .unwrap(),
        }
        .into();
        let mut expected = f.sender.pending_group_fanouts.clone();
        if inactive {
            expected.remove(65);
        }
        if held {
            expected.remove(64);
        }
        for pending in &mut expected[..64] {
            pending.next_retry_at_secs = now + 60;
        }
        GROUP_FANOUT_RETRY_VISITED.with(|visited| visited.set(0));
        let result = f
            .sender
            .retry_pending_group_fanouts(NdrUnixSeconds(now))
            .unwrap();
        assert!(result.events.is_empty() && result.effects.is_empty());
        assert!(f.sender.pending_group_fanouts == expected,
        "held wakes and revoked controls must retire even beyond the due cap; membership metadata and other deferred entries must retain order");
        assert_eq!(
            f.sender.pending_group_fanouts.len(),
            68 - usize::from(held) - usize::from(inactive)
        );
        assert_eq!(
            GROUP_FANOUT_RETRY_VISITED.with(|visited| visited.get()),
            68,
            "held-only and inactive-only cases must each scan the full tail"
        );
    }
}

fn publication_gap_fixture(local_sibling: bool, padding: usize) -> (RemoteSendFixture, Keys) {
    let mut f = checkpoint_backlog_fixture(4, 0);
    f.sender.ack_pending_decrypted_deliveries().unwrap();
    let target = if local_sibling {
        let target = Keys::generate();
        let pending = std::mem::take(&mut f.sender.pending_group_fanouts);
        assert!(f
            .sender
            .ingest_app_keys_event(&signed_app_keys(
                &f.owner,
                &[f.device.public_key(), target.public_key()],
                20,
            ))
            .unwrap()
            .effects
            .is_empty());
        f.sender.pending_group_fanouts = pending;
        target
    } else {
        f.late_device.clone()
    };
    for (index, pending) in f.sender.pending_group_fanouts.iter_mut().enumerate() {
        let GroupPendingFanout::Remote { payload, .. } = pending.fanout.as_ref() else {
            panic!("backlog fixture must start with remote controls");
        };
        let mut rumor: UnsignedEvent = serde_json::from_slice(payload).unwrap();
        // Whitespace grows a valid repair command without changing its meaning.
        rumor.content.push_str(&" ".repeat(padding));
        rumor.id = None;
        rumor.ensure_id();
        let payload = serde_json::to_vec(&rumor).unwrap();
        pending.fanout = if local_sibling {
            GroupPendingFanout::LocalSiblings {
                payload: local_sibling_payload_with_original_sender(
                    f.peer_owner.public_key(),
                    Some(f.ready_device.public_key()),
                    &payload,
                )
                .unwrap(),
            }
        } else {
            GroupPendingFanout::Remote {
                recipient_owner: ndr_owner(f.peer_owner.public_key()),
                payload,
            }
        }
        .into();
        pending.remaining_devices = Some(BTreeSet::from([ndr_device(target.public_key())]));
        pending.inner_event_id = Some(format!("publication-gap-{index}"));
        pending.next_retry_at_secs = if index == 1 { 1_120 } else { 1_000 };
    }
    (f, target)
}

fn reset_fanout_publication_work() {
    GROUP_FANOUT_RETRY_PREPARE_ATTEMPTS.with(|count| count.set(0));
    GROUP_FANOUT_RETRY_PUBLICATION_CONVERSIONS.with(|count| count.set(0));
    GROUP_FANOUT_RETRY_PAYLOAD_CLASSIFICATIONS.with(|count| count.set(0));
}

fn fanout_publication_work() -> (usize, usize, usize) {
    (
        GROUP_FANOUT_RETRY_PREPARE_ATTEMPTS.with(std::cell::Cell::get),
        GROUP_FANOUT_RETRY_PUBLICATION_CONVERSIONS.with(std::cell::Cell::get),
        GROUP_FANOUT_RETRY_PAYLOAD_CLASSIFICATIONS.with(std::cell::Cell::get),
    )
}

#[test]
fn group_fanout_retry_gap_only_skips_publication_work_and_preserves_durable_clocks() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut observations = Vec::new();
    for local_sibling in [false, true] {
        for padding in [0, 64 * 1024] {
            let (mut f, _) = publication_gap_fixture(local_sibling, padding);
            f.sender.pending_remote_sends = missing_roster_remote_retries(1, 1_000);
            let mut expiry = pairwise_codec::typing_event(
                f.sender.owner_pubkey,
                pairwise_codec::EncodeOptions::new(1_000, 1_000_000).with_expiration(1_060),
            )
            .unwrap();
            expiry.ensure_id();
            let pending = &mut f.sender.pending_remote_sends[0];
            pending.payload = serde_json::to_vec(&expiry).unwrap();
            pending.inner_event_id = expiry.id.map(|id| id.to_hex());
            pending.expires_at_secs = Some(1_060);
            pending.next_retry_at_secs = 1_060;
            f.sender.persist().unwrap();
            let before = f.sender.storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
            let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
            let groups = serde_json::to_value(f.sender.group_manager.snapshot()).unwrap();
            let journal = f.sender.pending_decrypted_deliveries.clone();
            let mut expected = f.sender.pending_group_fanouts.clone();
            for pending in &mut expected {
                if pending.next_retry_at_secs == 1_000 {
                    pending.next_retry_at_secs = 1_060;
                }
            }
            let storage = Arc::new(GroupRetryCountingStorage {
                inner: f.sender.storage.clone(),
                puts: 0.into(),
                fail_next: false.into(),
            });
            f.sender.storage = storage.clone();
            reset_fanout_publication_work();
            let (result, allocated) = remote_roster_tests::allocated_bytes(|| {
                f.sender.retry_pending_group_fanouts(NdrUnixSeconds(1_000))
            });
            let result = result.unwrap();
            let work = fanout_publication_work();
            assert!(result.events.is_empty() && result.effects.is_empty());
            assert_eq!(
                work.0, 3,
                "every due unavailable recipient must be attempted"
            );
            assert_eq!(f.sender.pending_group_fanouts, expected);
            assert_eq!(
                serde_json::to_value(f.sender.session_manager.snapshot()).unwrap(),
                sessions
            );
            assert_eq!(
                serde_json::to_value(f.sender.group_manager.snapshot()).unwrap(),
                groups
            );
            assert_eq!(f.sender.pending_decrypted_deliveries, journal);
            assert!(f.sender.ready_group_publications.is_empty());
            assert_eq!(storage.puts.load(Relaxed), 0);
            assert_eq!(storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(), before);

            // A genuine expiring control must still flush the in-memory retry
            // clocks. Reopening checks exact payloads, target sets and ordering.
            assert!(f
                .sender
                .retry_pending_remote_sends(NdrUnixSeconds(1_060))
                .unwrap()
                .is_empty());
            assert!(f.sender.pending_remote_sends.is_empty());
            assert!(storage.puts.load(Relaxed) > 0);
            let restored = ProtocolEngine::load_or_create_for_local_device(
                storage,
                f.owner.public_key(),
                &f.device,
            )
            .unwrap();
            assert_eq!(restored.pending_group_fanouts, expected);
            assert!(restored.pending_remote_sends.is_empty());
            assert_eq!(
                serde_json::to_value(restored.session_manager.snapshot()).unwrap(),
                sessions
            );
            assert_eq!(
                serde_json::to_value(restored.group_manager.snapshot()).unwrap(),
                groups
            );
            assert_eq!(restored.pending_decrypted_deliveries, journal);
            eprintln!("fanout_publication_work local_sibling={local_sibling} padding_bytes={padding} allocated_bytes={allocated} attempts={} conversions={} classifications={}", work.0, work.1, work.2);
            observations.push(work);
        }
    }
    assert_eq!(observations, vec![(3, 0, 0); 4],
        "relay gaps require real delivery attempts, but no publication conversion or payload classification");
}

#[test]
fn group_fanout_retry_empty_output_preserves_ready_queue_capacity_behavior() {
    let mut observations = Vec::new();
    for (capacity, wake) in [(4_096, true), (4_097, true), (4_097, false)] {
        let (mut f, _) = publication_gap_fixture(false, 0);
        f.sender.pending_group_fanouts.truncate(1);
        if wake {
            let wake = UnsignedEvent::new(
                f.owner.public_key(),
                Timestamp::from_secs(20),
                Kind::Custom(21_113),
                [],
                "wake",
            );
            f.sender.pending_group_fanouts[0].fanout = GroupPendingFanout::Remote {
                recipient_owner: ndr_owner(f.peer_owner.public_key()),
                payload: serde_json::to_vec(&wake).unwrap(),
            }
            .into();
        }
        let sent = f
            .sender
            .send_direct_text(
                f.peer_owner.public_key(),
                "group:backlog-group",
                "queued ciphertext",
                None,
                UnixSeconds(30),
            )
            .unwrap();
        let ProtocolEffect::Publish(publish) = sent.effects.into_iter().next().unwrap();
        let entry = ProtocolReadyGroupPublish {
            publish,
            wake_recipient: Some(ndr_owner(f.peer_owner.public_key())),
        };
        // Deliberately exercise an oversized state as well as the exact limit;
        // no load-time invariant is assumed by the retry path.
        f.sender.ready_group_publications = (0..capacity)
            .map(|index| (format!("{index:064x}"), entry.clone()))
            .collect();
        let ready = serde_json::to_value(&f.sender.ready_group_publications).unwrap();
        let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
        let original = f.sender.pending_group_fanouts.clone();
        let storage = Arc::new(GroupRetryCountingStorage {
            inner: f.sender.storage.clone(),
            puts: 0.into(),
            fail_next: false.into(),
        });
        f.sender.storage = storage.clone();
        reset_fanout_publication_work();
        let result = f.sender.retry_pending_group_fanouts(NdrUnixSeconds(1_000));
        if capacity > 4_096 && wake {
            assert_eq!(
                result.unwrap_err().to_string(),
                "Outgoing group queue is full"
            );
            assert_eq!(f.sender.pending_group_fanouts, original);
        } else {
            let result = result.unwrap();
            assert!(result.events.is_empty() && result.effects.is_empty());
            let mut expected = original;
            expected[0].next_retry_at_secs = 1_060;
            assert_eq!(f.sender.pending_group_fanouts, expected);
        }
        assert_eq!(
            serde_json::to_value(&f.sender.ready_group_publications).unwrap(),
            ready
        );
        assert_eq!(
            serde_json::to_value(f.sender.session_manager.snapshot()).unwrap(),
            sessions
        );
        assert_eq!(storage.puts.load(std::sync::atomic::Ordering::Relaxed), 0);
        observations.push(fanout_publication_work());
    }
    assert_eq!(
        observations,
        vec![(1, 0, 0), (1, 1, 1), (1, 1, 1)],
        "only an already oversized empty-output queue needs the original wake classification"
    );
}

#[test]
fn group_fanout_retry_wrapped_local_wake_retains_real_publications() {
    let (mut f, sibling_keys) = publication_gap_fixture(true, 0);
    let mut sibling = test_engine(&f.owner, &sibling_keys);
    sibling
        .ingest_app_keys_event(&signed_app_keys(
            &f.owner,
            &[f.device.public_key(), sibling_keys.public_key()],
            20,
        ))
        .unwrap();
    sibling
        .ingest_app_keys_event(&signed_app_keys(
            &f.peer_owner,
            &[f.ready_device.public_key(), f.late_device.public_key()],
            1,
        ))
        .unwrap();
    let mut pending = std::mem::take(&mut f.sender.pending_group_fanouts);
    assert!(observe_sibling_invite(&mut f.sender, &sibling, &sibling_keys).is_empty());
    pending.truncate(1);
    let wake = UnsignedEvent::new(
        f.owner.public_key(),
        Timestamp::from_secs(20),
        Kind::Custom(21_113),
        [],
        "wrapped wake",
    );
    let payload = serde_json::to_vec(&wake).unwrap();
    pending[0].fanout = GroupPendingFanout::LocalSiblings {
        payload: local_sibling_payload_with_original_sender(
            f.peer_owner.public_key(),
            Some(f.ready_device.public_key()),
            &payload,
        )
        .unwrap(),
    }
    .into();
    f.sender.pending_group_fanouts = pending;
    f.sender.persist().unwrap();
    reset_fanout_publication_work();
    let result = f
        .sender
        .retry_pending_group_fanouts(NdrUnixSeconds(1_000))
        .unwrap();
    assert_eq!(fanout_publication_work(), (1, 1, 1));
    assert!(!result.effects.is_empty());
    assert!(f.sender.pending_group_fanouts.is_empty());
    assert_eq!(
        f.sender.ready_group_publications.len(),
        result.effects.len()
    );
    assert!(f
        .sender
        .ready_group_publications
        .values()
        .all(|ready| ready.wake_recipient.is_none()));
    for ProtocolEffect::Publish(publish) in &result.effects {
        publish.event.verify().unwrap();
        let wire = serde_json::to_string(&publish.event).unwrap();
        assert!(!wire.contains(&f.peer_owner.public_key().to_hex()));
        assert!(!wire.contains(&f.ready_device.public_key().to_hex()));
    }
    let ready = serde_json::to_value(&f.sender.ready_group_publications).unwrap();
    let sessions = serde_json::to_value(f.sender.session_manager.snapshot()).unwrap();
    let restored =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(restored.pending_group_fanouts.is_empty());
    assert_eq!(
        serde_json::to_value(&restored.ready_group_publications).unwrap(),
        ready
    );
    assert_eq!(
        serde_json::to_value(restored.session_manager.snapshot()).unwrap(),
        sessions
    );
    let messages = decrypt_own_sync_effects(&mut sibling, result.effects);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].conversation_owner,
        Some(f.peer_owner.public_key())
    );
    assert_eq!(messages[0].sender_device, Some(f.ready_device.public_key()));
    assert_eq!(messages[0].content.as_bytes(), payload);
}
