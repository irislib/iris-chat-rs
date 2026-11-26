use super::*;

fn assert_wire_event_private(event: &Event, forbidden_identities: &[&Keys]) {
    assert_eq!(event.kind.as_u16() as u32, MESSAGE_EVENT_KIND);
    assert!(event.verify().is_ok(), "event signature must verify");
    assert!(
        event.tags.public_keys().next().is_none(),
        "kind 1060 message events must never tag a recipient"
    );
    let wire = serde_json::to_string(event).unwrap();
    for identity in forbidden_identities {
        assert!(
            !wire.contains(&identity.public_key().to_hex()),
            "serialized wire event leaked an identity key"
        );
    }
}

#[test]
fn local_sibling_group_sync_uses_one_session_and_preserves_wire_privacy() {
    let mut f = remote_send_fixture();
    observe_sibling_invite(&mut f.ready, &f.late, &f.late_device);

    let created = f
        .sender
        .create_group(
            "single-session-sync".into(),
            vec![f.peer_owner.public_key()],
            UnixSeconds(12),
        )
        .unwrap();
    let controls = decrypt_own_sync_effects(&mut f.ready, created.effects);
    let mut checked_commands = 0;
    for msg in controls {
        let payload = msg.content.as_bytes();
        let Some(command) = JsonGroupPayloadCodecV1
            .decode_pairwise_command(payload)
            .unwrap()
        else {
            continue;
        };
        if !matches!(
            command,
            GroupPairwiseCommand::MetadataSnapshot { .. }
                | GroupPairwiseCommand::SenderKeyDistribution { .. }
        ) {
            continue;
        }
        // Before forwarding the second control command, duplicate the active
        // sibling session into inactive_sessions so two send-capable sessions exist.
        if checked_commands == 1 {
            let mut sessions = f.ready.session_manager.snapshot();
            let local_user = sessions
                .users
                .iter_mut()
                .find(|user| user.owner_pubkey == f.ready.local_owner)
                .unwrap();
            let sibling_record = local_user
                .devices
                .iter_mut()
                .find(|device| device.device_pubkey == ndr_device(f.late_device.public_key()))
                .unwrap();
            let active = sibling_record.active_session.clone().unwrap();
            sibling_record.inactive_sessions.push(active);
            f.ready.session_manager = SessionManager::from_snapshot(
                sessions,
                f.ready_device.secret_key().to_secret_bytes(),
            )
            .unwrap();
        }
        let outcome = f
            .ready
            .process_group_pairwise_payload(payload, msg.sender, msg.sender_device)
            .unwrap();
        let message_events = outcome
            .effects
            .iter()
            .filter(|ProtocolEffect::Publish(publish)| {
                publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
            })
            .collect::<Vec<_>>();
        assert_eq!(
            message_events.len(),
            1,
            "sibling forward must emit one kind 1060 envelope per linked device, not fan out across inactive sessions"
        );
        if checked_commands == 1 {
            assert_eq!(
                outcome.effects.len(),
                1,
                "established sibling forward must not emit extra handshake or duplicate session envelopes"
            );
        }
        for ProtocolEffect::Publish(publish) in message_events {
            assert_wire_event_private(
                &publish.event,
                &[
                    &f.owner,
                    &f.device,
                    &f.peer_owner,
                    &f.ready_device,
                    &f.late_device,
                ],
            );
        }
        let delivered = decrypt_own_sync_effects(&mut f.late, outcome.effects);
        assert_eq!(delivered.len(), 1);
        checked_commands += 1;
    }
    assert_eq!(checked_commands, 2);
}

#[test]
fn repair_response_targets_requesting_device_progressively_and_resets_on_rotation() {
    let mut f = remote_send_fixture();
    // Establish sessions for both already-authorized peer devices (ready + late).
    observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);

    let created = f
        .sender
        .create_group(
            "progressive-repair".into(),
            vec![f.peer_owner.public_key()],
            UnixSeconds(100),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    for msg in decrypt_own_sync_effects(&mut f.ready, created.effects.clone()) {
        f.ready
            .process_group_pairwise_payload(msg.content.as_bytes(), msg.sender, msg.sender_device)
            .unwrap();
    }
    for msg in decrypt_own_sync_effects(&mut f.late, created.effects) {
        f.late
            .process_group_pairwise_payload(msg.content.as_bytes(), msg.sender, msg.sender_device)
            .unwrap();
    }

    // Add a second rotated sender key epoch (key_id 200 at t=200 after key 1 at t=100).
    let mut snap = f.sender.group_manager.snapshot();
    let record = snap
        .sender_keys
        .iter_mut()
        .find(|r| r.group_id == group_id && r.sender_owner == f.sender.local_owner)
        .unwrap();
    let author = record.sender_event_pubkey;
    let first_key_id = record.latest_key_id.unwrap();
    let second_key_id = first_key_id.wrapping_add(1);
    let second_dist = nostr_double_ratchet::SenderKeyDistribution {
        group_id: group_id.clone(),
        key_id: second_key_id,
        sender_event_pubkey: author,
        chain_key: [42; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(200),
    };
    record
        .states
        .push(nostr_double_ratchet::SenderKeyState::new(
            second_key_id,
            [42; 32],
            0,
        ));
    record.latest_key_id = Some(second_key_id);
    record.distribution_history.push(second_dist.clone());
    record
        .repair_snapshots
        .push(nostr_double_ratchet::GroupSenderKeyRepairSnapshot {
            key_id: second_key_id,
            distribution: second_dist,
            recipients: vec![f.sender.local_owner, ndr_owner(f.peer_owner.public_key())],
        });
    f.sender.group_manager = GroupEventManager::from_snapshot(snap).unwrap();

    let request = SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: author,
        key_id: None,
        message_number: None,
        required_revision: None,
        created_at: NdrUnixSeconds(210),
    };

    // Round 0: targets ONLY ready_device (not late_device), sends ONLY latest key (+ context metadata).
    let r0 = f
        .sender
        .sender_key_repair_response_effects_for_device(
            ndr_owner(f.peer_owner.public_key()),
            Some(ndr_device(f.ready_device.public_key())),
            &request,
            NdrUnixSeconds(210),
        )
        .unwrap();
    assert!(!r0.is_empty());
    for ProtocolEffect::Publish(publish) in &r0 {
        assert_wire_event_private(
            &publish.event,
            &[
                &f.owner,
                &f.device,
                &f.peer_owner,
                &f.ready_device,
                &f.late_device,
            ],
        );
        assert_eq!(
            publish.carrier.pairwise_device(),
            Some(f.ready_device.public_key()),
            "repair response must target only the requesting device"
        );
    }
    let ready_r0 = decrypt_own_sync_effects(&mut f.ready, r0.clone());
    let late_r0 = decrypt_own_sync_effects(&mut f.late, r0);
    assert!(late_r0.is_empty(), "sibling device must not be spammed");
    let r0_keys = ready_r0
        .iter()
        .filter_map(|m| {
            match JsonGroupPayloadCodecV1
                .decode_pairwise_command(m.content.as_bytes())
                .unwrap()
            {
                Some(GroupPairwiseCommand::SenderKeyDistribution { distribution }) => {
                    Some(distribution.key_id)
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        r0_keys,
        vec![second_key_id],
        "first blind repair response should send only the latest eligible key"
    );

    // Round 1 (after 10s cooldown): sends full eligible history oldest-first.
    let r1 = f
        .sender
        .sender_key_repair_response_effects_for_device(
            ndr_owner(f.peer_owner.public_key()),
            Some(ndr_device(f.ready_device.public_key())),
            &request,
            NdrUnixSeconds(221),
        )
        .unwrap();
    let ready_r1 = decrypt_own_sync_effects(&mut f.ready, r1);
    let r1_keys = ready_r1
        .iter()
        .filter_map(|m| {
            match JsonGroupPayloadCodecV1
                .decode_pairwise_command(m.content.as_bytes())
                .unwrap()
            {
                Some(GroupPairwiseCommand::SenderKeyDistribution { distribution }) => {
                    Some(distribution.key_id)
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        r1_keys,
        vec![first_key_id, second_key_id],
        "second blind repair response should include all eligible keys in chronological order"
    );

    // Rotate to a third key while within the 30s cooldown: new material must bypass old throttle.
    let third_key_id = second_key_id.wrapping_add(1);
    let mut snap = f.sender.group_manager.snapshot();
    let record = snap
        .sender_keys
        .iter_mut()
        .find(|r| r.group_id == group_id && r.sender_owner == f.sender.local_owner)
        .unwrap();
    let third_dist = nostr_double_ratchet::SenderKeyDistribution {
        group_id: group_id.clone(),
        key_id: third_key_id,
        sender_event_pubkey: author,
        chain_key: [99; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(225),
    };
    record
        .states
        .push(nostr_double_ratchet::SenderKeyState::new(
            third_key_id,
            [99; 32],
            0,
        ));
    record.latest_key_id = Some(third_key_id);
    record.distribution_history.push(third_dist.clone());
    record
        .repair_snapshots
        .push(nostr_double_ratchet::GroupSenderKeyRepairSnapshot {
            key_id: third_key_id,
            distribution: third_dist,
            recipients: vec![f.sender.local_owner, ndr_owner(f.peer_owner.public_key())],
        });
    f.sender.group_manager = GroupEventManager::from_snapshot(snap).unwrap();

    let r_rotated = f
        .sender
        .sender_key_repair_response_effects_for_device(
            ndr_owner(f.peer_owner.public_key()),
            Some(ndr_device(f.ready_device.public_key())),
            &request,
            NdrUnixSeconds(226),
        )
        .unwrap();
    assert!(
        !r_rotated.is_empty(),
        "a new sender-key epoch must reset response throttling immediately"
    );

    // Non-admin must never emit a remote MetadataSnapshot on revision repair.
    let rev_request = SenderKeyRepairRequest {
        group_id: group_id.clone(),
        sender_event_pubkey: author,
        key_id: None,
        message_number: None,
        required_revision: Some(1),
        created_at: NdrUnixSeconds(230),
    };
    let non_admin_reply = f
        .ready
        .sender_key_repair_response_effects_for_device(
            f.sender.local_owner,
            Some(f.sender.local_device),
            &rev_request,
            NdrUnixSeconds(230),
        )
        .unwrap();
    assert!(
        non_admin_reply.is_empty(),
        "non-admin member must not emit remote MetadataSnapshot on revision repair"
    );

    // Removed member must receive zero effects for post-removal repair requests.
    let mut snap = f.sender.group_manager.snapshot();
    snap.groups[0]
        .members
        .retain(|m| *m != ndr_owner(f.peer_owner.public_key()));
    snap.groups[0].revision += 1;
    f.sender.group_manager = GroupEventManager::from_snapshot(snap).unwrap();
    let post_removal = f
        .sender
        .sender_key_repair_response_effects_for_device(
            ndr_owner(f.peer_owner.public_key()),
            Some(ndr_device(f.ready_device.public_key())),
            &request,
            NdrUnixSeconds(500),
        )
        .unwrap();
    assert!(
        post_removal.is_empty(),
        "removed member must never receive repair responses"
    );
}

#[test]
fn out_of_order_newer_distribution_preserves_older_pending_ciphertexts() {
    let (mut engine, sender, sender_device) = queued_group_history(1);
    let author = engine.group_manager.snapshot().sender_keys[0].sender_event_pubkey;
    let old_ciphertext = engine.pending_group_sender_key_messages[0].clone();

    // Start without key 1 installed, simulating a reconnect that receives key 2 first.
    let mut snap = engine.group_manager.snapshot();
    snap.sender_keys[0].states.clear();
    snap.sender_keys[0].latest_key_id = None;
    engine.group_manager = GroupEventManager::from_snapshot(snap).unwrap();

    let newer_dist = nostr_double_ratchet::SenderKeyDistribution {
        group_id: "queued-group".into(),
        key_id: 2,
        sender_event_pubkey: author,
        chain_key: [77; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(500),
    };
    let newer_payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(500),
            },
            &GroupPairwiseCommand::SenderKeyDistribution {
                distribution: newer_dist,
            },
        )
        .unwrap();
    engine
        .process_group_pairwise_payload(
            &newer_payload,
            sender.public_key(),
            Some(sender_device.public_key()),
        )
        .unwrap();
    drain_group_retry(&mut engine, NdrUnixSeconds(505));
    assert_eq!(
        engine.pending_group_sender_key_messages.len(),
        1,
        "receiving a newer key first must not discard older pending ciphertexts"
    );
    assert_eq!(engine.pending_group_sender_key_messages[0], old_ciphertext);

    // Delivering the older key afterwards must unlock and deliver the queued ciphertext.
    let older_dist = nostr_double_ratchet::SenderKeyDistribution {
        group_id: "queued-group".into(),
        key_id: 1,
        sender_event_pubkey: author,
        chain_key: [9; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(10),
    };
    let older_payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(10),
            },
            &GroupPairwiseCommand::SenderKeyDistribution {
                distribution: older_dist,
            },
        )
        .unwrap();
    let delivered = engine
        .process_group_pairwise_payload(
            &older_payload,
            sender.public_key(),
            Some(sender_device.public_key()),
        )
        .unwrap();
    let mut messages = delivered
        .events
        .into_iter()
        .filter(|e| matches!(e, GroupIncomingEvent::Message(_)))
        .collect::<Vec<_>>();
    messages.extend(
        drain_group_retry(&mut engine, NdrUnixSeconds(506))
            .into_iter()
            .filter(|e| matches!(e, GroupIncomingEvent::Message(_))),
    );
    assert_eq!(messages.len(), 1);
    assert!(engine.pending_group_sender_key_messages.is_empty());
}

#[test]
fn multi_key_frontier_probe_and_selective_retry_wakeups_avoid_redundant_blind_scans() {
    let (mut engine, _, sender_device) = queued_group_history(1);
    let author = engine.group_manager.snapshot().sender_keys[0].sender_event_pubkey;

    // Install a second rotated key (key_id=2 > key_id=1) as latest_key_id.
    let rotated = nostr_double_ratchet::SenderKeyState::new(2, [55; 32], 0);
    let mut snap = engine.group_manager.snapshot();
    snap.sender_keys[0].states.push(rotated.clone());
    snap.sender_keys[0].latest_key_id = Some(2);
    engine.group_manager = GroupEventManager::from_snapshot(snap).unwrap();

    // Encrypt a live message with rotated key 2 and verify frontier probe hits immediately.
    let plaintext = JsonGroupPayloadCodecV1
        .encode_sender_key_plaintext(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(30),
            },
            &nostr_double_ratchet::GroupSenderKeyPlaintext {
                group_id: "queued-group".into(),
                revision: 1,
                body: b"live-rotated".to_vec(),
            },
        )
        .unwrap();
    let mut encryptor = rotated;
    let (_, live_ct) = encryptor.encrypt_to_bytes(&plaintext).unwrap();
    engine.pending_group_sender_key_messages[0].ciphertext = live_ct;
    let delivered = drain_group_retry(&mut engine, NdrUnixSeconds(35));
    assert_eq!(delivered.len(), 1);
    assert!(
        engine.group_retry_pacing.borrow().key_trials <= 2,
        "frontier probe must hit latest_key_id immediately instead of scanning 10,001 steps of key 1"
    );

    // Queue an un-decryptable ciphertext and exhaust its initial blind scan once.
    let stuck_ct = nostr_double_ratchet::SenderKeyState::new(999, [88; 32], 0)
        .encrypt_to_bytes(b"un-decryptable")
        .unwrap()
        .1;
    engine.pending_group_sender_key_messages.push(
        nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
            sender_event_pubkey: author,
            key_id: 999,
            message_number: 0,
            encrypted_header: Some("hidden".into()),
            created_at: NdrUnixSeconds(40),
            ciphertext: stuck_ct,
        },
    );
    drain_group_retry(&mut engine, NdrUnixSeconds(45));
    let attempts_after_initial = engine.group_sender_key_retry.borrow().total_attempts;

    // 1) A +1 live ratchet advance, 2) an unrelated member edit, and 3) a revision bump
    // must NOT re-queue a failed PendingDistribution ciphertext.
    let (_, next_live_ct) = encryptor.encrypt_to_bytes(&plaintext).unwrap();
    let live_msg = GroupSenderKeyMessage {
        group_id: "queued-group".into(),
        sender_event_pubkey: author,
        key_id: 2,
        message_number: 1,
        encrypted_header: Some("hidden".into()),
        created_at: NdrUnixSeconds(46),
        ciphertext: next_live_ct,
    };
    let live_res = engine.handle_group_sender_key_message(live_msg).unwrap();
    assert_eq!(live_res.events.len(), 1);
    let mut snap = engine.group_manager.snapshot();
    snap.groups[0]
        .members
        .push(ndr_owner(Keys::generate().public_key()));
    snap.groups[0].revision += 1;
    engine.group_manager = GroupEventManager::from_snapshot(snap).unwrap();
    drain_group_retry(&mut engine, NdrUnixSeconds(50));
    assert_eq!(
        engine.group_sender_key_retry.borrow().total_attempts,
        attempts_after_initial,
        "unrelated member edits, revision bumps, and +1 live ratchets must not rescan stuck PendingDistribution history"
    );
}

#[test]
fn unchanged_repair_feedback_backs_off_steeply_and_wakes_on_newer_ciphertext() {
    let (mut engine, sender, sender_device) = queued_group_history(1);
    let author = engine.group_manager.snapshot().sender_keys[0].sender_event_pubkey;
    let dist = nostr_double_ratchet::SenderKeyDistribution {
        group_id: "queued-group".into(),
        key_id: 1,
        sender_event_pubkey: author,
        chain_key: [9; 32],
        iteration: 0,
        created_at: NdrUnixSeconds(10),
    };
    let dist_payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(sender_device.public_key()),
                created_at: NdrUnixSeconds(10),
            },
            &GroupPairwiseCommand::SenderKeyDistribution { distribution: dist },
        )
        .unwrap();
    // Install distribution first so subsequent replays are unchanged.
    engine
        .process_group_pairwise_payload(
            &dist_payload,
            sender.public_key(),
            Some(sender_device.public_key()),
        )
        .unwrap();

    // Queue an un-decryptable ciphertext and trigger 3 unchanged writer responses.
    let stuck_ct = nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 0)
        .encrypt_to_bytes(b"during-removal")
        .unwrap()
        .1;
    engine.pending_group_sender_key_messages.push(
        nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent {
            sender_event_pubkey: author,
            key_id: 999,
            message_number: 0,
            encrypted_header: Some("hidden".into()),
            created_at: NdrUnixSeconds(50),
            ciphertext: stuck_ct,
        },
    );
    let base_now = unix_now().get();
    let request = SenderKeyRepairRequest {
        group_id: "queued-group".into(),
        sender_event_pubkey: author,
        key_id: None,
        message_number: None,
        required_revision: None,
        created_at: NdrUnixSeconds(base_now),
    };
    for (round, now_secs) in [base_now, base_now + 15, base_now + 50]
        .into_iter()
        .enumerate()
    {
        engine
            .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(now_secs))
            .unwrap();
        engine
            .process_group_pairwise_payload(
                &dist_payload,
                sender.public_key(),
                Some(sender_device.public_key()),
            )
            .unwrap();
        assert_eq!(
            engine.pending_group_sender_key_repairs[0].unchanged_response_rounds,
            (round + 1) as u32
        );
    }
    let repair = &engine.pending_group_sender_key_repairs[0];
    assert_eq!(
        ProtocolEngine::pending_group_sender_key_repair_due_at_secs(repair),
        base_now + 50 + 300,
        "three unchanged writer responses must back off repair retry to 5 minutes"
    );
    assert_eq!(
        engine.pending_group_sender_key_messages.len(),
        1,
        "backoff must retain pending ciphertexts rather than deleting them"
    );

    // A genuinely newer un-decryptable ciphertext must wake the parked repair immediately.
    let newer_msg = GroupSenderKeyMessage {
        group_id: "queued-group".into(),
        sender_event_pubkey: author,
        key_id: 999,
        message_number: 1,
        encrypted_header: Some("hidden".into()),
        created_at: NdrUnixSeconds(base_now + 120),
        ciphertext: nostr_double_ratchet::SenderKeyState::new(999, [99; 32], 1)
            .encrypt_to_bytes(b"post-rotation")
            .unwrap()
            .1,
    };
    // Run the blind scan to exhaustion on newer_msg so it reaches PendingDistribution.
    engine.group_sender_key_retry.borrow_mut().reset_budget();
    for _ in 0..64 {
        engine.group_sender_key_retry.borrow_mut().reset_budget();
        let res = engine
            .handle_group_sender_key_message(newer_msg.clone())
            .unwrap();
        if engine.pending_group_sender_key_repairs[0].unchanged_response_rounds == 0 {
            assert!(res.pending);
            break;
        }
    }
    let woken = &engine.pending_group_sender_key_repairs[0];
    assert_eq!(woken.unchanged_response_rounds, 0);
    assert_eq!(woken.request_count, 1);
}

#[test]
fn frontier_probe_leaves_budget_for_resumable_history_with_many_epochs() {
    let (mut engine, _, _) = queued_group_history(1);
    let mut snapshot = engine.group_manager.snapshot();
    for id in 2_u32..=257 {
        let mut chain_key = [55; 32];
        chain_key[..4].copy_from_slice(&id.to_le_bytes());
        snapshot.sender_keys[0]
            .states
            .push(nostr_double_ratchet::SenderKeyState::new(id, chain_key, 0));
    }
    snapshot.sender_keys[0].latest_key_id = Some(257);
    engine.group_manager = GroupEventManager::from_snapshot(snapshot).unwrap();
    let result = engine.retry_pending_protocol(NdrUnixSeconds(30)).unwrap();
    assert_eq!(
        result.group_result.events.len(),
        1,
        "Optional frontier probes must not starve the ordinary history search"
    );
    assert!(engine.pending_group_sender_key_messages.is_empty());
    assert!(engine.group_retry_pacing.borrow().key_trials <= 256);
}
