#[test]
fn refreshed_sender_key_repair_requests_share_response_backoff() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let requester = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let group = engine
        .create_group(
            "Repair burst".to_string(),
            vec![requester.public_key()],
            UnixSeconds(100),
        )
        .unwrap()
        .snapshot
        .unwrap();
    engine.pending_group_fanouts.clear();
    let mut request = SenderKeyRepairRequest {
        group_id: group.group_id,
        sender_event_pubkey: ndr_device(Keys::generate().public_key()),
        key_id: None,
        message_number: None,
        required_revision: Some(group.revision),
        created_at: NdrUnixSeconds(101),
    };
    let requester_owner = ndr_owner(requester.public_key());
    engine
        .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(102))
        .unwrap();
    let pending = engine.pending_group_fanouts.clone();
    let answered = engine.answered_group_sender_key_repairs.clone();
    for now in 103..112 {
        request.created_at = NdrUnixSeconds(now);
        let effects = engine
            .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(now))
            .unwrap();
        assert!(effects.is_empty());
        assert_eq!(engine.pending_group_fanouts, pending);
        assert_eq!(
            engine.answered_group_sender_key_repairs, answered,
            "refreshing a request timestamp must not bypass the response cooldown"
        );
    }
    request.created_at = NdrUnixSeconds(112);
    engine
        .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(112))
        .unwrap();
    assert_eq!(engine.answered_group_sender_key_repairs.len(), 1);
    let answered = &engine.answered_group_sender_key_repairs[0];
    assert_eq!(answered.last_responded_at_secs, 112);
    assert_eq!(answered.response_count, 2);
    assert_eq!(answered.next_response_at_secs, 142);

    // A different missing key/revision is independent work, not the same retry.
    request.required_revision = Some(group.revision + 1);
    assert!(!engine.group_sender_key_repair_response_throttled(
        requester_owner,
        &request,
        NdrUnixSeconds(113)
    ));
}

#[test]
fn refreshed_repair_requests_do_not_duplicate_an_undeliverable_intent() {
    let (mut engine, _, _) = queued_group_history(1);
    let stream = engine.group_manager.snapshot().sender_keys[0].sender_event_pubkey;
    let mut request = SenderKeyRepairRequest {
        group_id: "queued-group".into(),
        sender_event_pubkey: stream,
        key_id: Some(1),
        message_number: Some(0),
        required_revision: Some(1),
        created_at: NdrUnixSeconds(100),
    };
    engine
        .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(100))
        .unwrap();
    assert_eq!(engine.pending_group_fanouts.len(), 1);
    let original = engine.pending_group_fanouts[0].clone();
    request.created_at = NdrUnixSeconds(161);
    engine
        .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(161))
        .unwrap();
    assert_eq!(engine.pending_group_sender_key_repairs.len(), 1);
    assert_eq!(engine.pending_group_fanouts[0], original);
    assert_eq!(
        engine.pending_group_fanouts.len(),
        1,
        "refreshing timestamps must not accumulate another unsent copy of the same repair intent"
    );
}

#[test]
fn repair_coalescing_preserves_every_recovery_selector_and_existing_entry() {
    use nostr_double_ratchet::{GroupPayloadEncodeContext, RelayGap};
    let (mut engine, _, _) = queued_group_history(1);
    let request = SenderKeyRepairRequest {
        group_id: "queued-group".into(),
        sender_event_pubkey: engine.group_manager.snapshot().sender_keys[0].sender_event_pubkey,
        key_id: Some(1),
        message_number: Some(0),
        required_revision: Some(1),
        created_at: NdrUnixSeconds(100),
    };
    engine
        .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(100))
        .unwrap();
    let original = engine.pending_group_fanouts[0].clone();
    let GroupPendingFanout::Remote {
        recipient_owner, ..
    } = original.fanout.as_ref()
    else {
        panic!("remote repair");
    };
    let recipient = *recipient_owner;
    let devices = original.remaining_devices.clone().unwrap();
    let sender = engine.local_device;
    let enqueue = |engine: &mut ProtocolEngine,
                   request: &SenderKeyRepairRequest,
                   sender,
                   recipient,
                   devices: &BTreeSet<NdrDevicePubkey>,
                   outer_group: &str| {
        let payload = JsonGroupPayloadCodecV1
            .encode_pairwise_command(
                GroupPayloadEncodeContext {
                    local_device_pubkey: sender,
                    created_at: NdrUnixSeconds(500),
                },
                &GroupPairwiseCommand::SenderKeyRepairRequest {
                    request: request.clone(),
                },
            )
            .unwrap();
        let mut prepared = GroupPreparedPublish::empty();
        prepared.pending_fanouts.push(GroupPendingFanout::Remote {
            recipient_owner: recipient,
            payload,
        });
        prepared.relay_gaps = devices
            .iter()
            .map(|device| RelayGap::MissingDeviceInvite {
                owner_pubkey: recipient,
                device_pubkey: *device,
            })
            .collect();
        engine.queue_group_pending_fanouts(outer_group, &prepared, None);
    };
    // Existing historical duplicates are preserved, including their original clocks.
    engine.pending_group_fanouts.push(original.clone());
    let before = engine.pending_group_fanouts.clone();
    let mut refreshed = request.clone();
    refreshed.created_at = NdrUnixSeconds(500);
    enqueue(
        &mut engine,
        &refreshed,
        sender,
        recipient,
        &devices,
        &request.group_id,
    );
    assert_eq!(engine.pending_group_fanouts, before);
    let mut variants = Vec::new();
    let mut different = refreshed.clone();
    different.key_id = Some(2);
    variants.push(different);
    let mut different = refreshed.clone();
    different.key_id = None;
    variants.push(different);
    let mut different = refreshed.clone();
    different.message_number = Some(1);
    variants.push(different);
    let mut different = refreshed.clone();
    different.required_revision = Some(2);
    variants.push(different);
    let mut different = refreshed.clone();
    different.sender_event_pubkey = ndr_device(Keys::generate().public_key());
    variants.push(different);
    let mut different = refreshed.clone();
    different.group_id = "other-group".into();
    variants.push(different);
    for different in variants {
        let count = engine.pending_group_fanouts.len();
        enqueue(
            &mut engine,
            &different,
            sender,
            recipient,
            &devices,
            &request.group_id,
        );
        assert_eq!(engine.pending_group_fanouts.len(), count + 1);
    }
    for (sender, recipient, devices, group) in [
        (
            ndr_device(Keys::generate().public_key()),
            recipient,
            devices.clone(),
            request.group_id.as_str(),
        ),
        (
            sender,
            ndr_owner(Keys::generate().public_key()),
            devices.clone(),
            request.group_id.as_str(),
        ),
        (
            sender,
            recipient,
            BTreeSet::from([ndr_device(Keys::generate().public_key())]),
            request.group_id.as_str(),
        ),
        (
            sender,
            recipient,
            BTreeSet::new(),
            request.group_id.as_str(),
        ),
        (sender, recipient, devices.clone(), "other-outer-group"),
    ] {
        let count = engine.pending_group_fanouts.len();
        enqueue(&mut engine, &refreshed, sender, recipient, &devices, group);
        assert_eq!(engine.pending_group_fanouts.len(), count + 1);
    }
    assert_eq!(&engine.pending_group_fanouts[..2], before.as_slice());
    // An unknown target set and a known target set are distinct.
    let mut unknown = original.clone();
    unknown.remaining_devices = None;
    assert!(!original.same_pending_repair(&unknown));
    let mut message = original.clone();
    message.inner_event_id = Some("message".into());
    assert!(!original.same_pending_repair(&message));
    // Identity cache shares the immutable payload lifetime and is rebuilt on reload.
    assert!(original.fanout.repair_identity().is_some());
    let cloned = original.clone();
    assert!(Arc::ptr_eq(
        &original.fanout.repair_identity,
        &cloned.fanout.repair_identity
    ));
    let restored: ProtocolPendingGroupFanout =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    assert!(original.same_pending_repair(&restored));
    let weak = Arc::downgrade(&cloned.fanout.repair_identity);
    drop(cloned);
    drop(unknown);
    drop(message);
    drop(original);
    drop(before);
    drop(engine);
    assert!(weak.upgrade().is_none());
}

include!("repair_retry_persistence_tests.rs");

#[test]
fn repair_coalescing_keeps_ready_ratchet_delivery_across_reload() {
    let mut f = remote_send_fixture();
    let group = f
        .sender
        .create_group(
            "Repair delivery".into(),
            vec![f.peer_owner.public_key()],
            UnixSeconds(90),
        )
        .unwrap();
    let group_id = group.snapshot.unwrap().group_id;
    for message in decrypt_own_sync_effects(&mut f.ready, group.effects) {
        f.ready
            .process_group_pairwise_payload(
                message.content.as_bytes(),
                message.sender,
                message.sender_device,
            )
            .unwrap();
    }
    f.sender.pending_group_fanouts.clear();
    let mut request = SenderKeyRepairRequest {
        group_id,
        sender_event_pubkey: ndr_device(Keys::generate().public_key()),
        key_id: Some(7),
        message_number: Some(12),
        required_revision: Some(1),
        created_at: NdrUnixSeconds(100),
    };
    let mut original = None;
    for now in [100, 161, 240] {
        if now == 240 {
            f.sender.persist().unwrap();
            f.sender = ProtocolEngine::load_or_create_for_local_device(
                f.store.clone(),
                f.owner.public_key(),
                &f.device,
            )
            .unwrap();
        }
        request.created_at = NdrUnixSeconds(now);
        let effects = f
            .sender
            .sender_key_repair_request_effects(request.clone(), NdrUnixSeconds(now))
            .unwrap();
        assert_eq!(f.sender.pending_group_fanouts.len(), 1);
        if let Some(original) = &original {
            assert_eq!(&f.sender.pending_group_fanouts[0], original);
        } else {
            original = Some(f.sender.pending_group_fanouts[0].clone());
        }
        assert!(
            !effects.is_empty(),
            "ready recipient must still get the current retry"
        );
        for ProtocolEffect::Publish(publish) in &effects {
            assert_eq!(publish.event.kind.as_u16() as u32, MESSAGE_EVENT_KIND);
            assert!(publish.event.verify().is_ok());
            let wire = serde_json::to_string(&publish.event).unwrap();
            for identity in [&f.owner, &f.device, &f.peer_owner, &f.late_device] {
                assert!(!wire.contains(&identity.public_key().to_hex()));
            }
            assert_ne!(publish.event.pubkey, f.ready_device.public_key());
            assert_eq!(
                publish
                    .event
                    .tags
                    .public_keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![f.ready_device.public_key()]
            );
        }
        let messages = decrypt_own_sync_effects(&mut f.ready, effects);
        assert_eq!(messages.len(), 1);
        let command = JsonGroupPayloadCodecV1
            .decode_pairwise_command(messages[0].content.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(
            command,
            GroupPairwiseCommand::SenderKeyRepairRequest {
                request: request.clone()
            }
        );
    }
}

#[test]
fn repair_coalescing_rejects_other_kinds_invalid_payloads_and_unknown_changes() {
    let owner = ndr_owner(Keys::generate().public_key());
    let device = ndr_device(Keys::generate().public_key());
    let request = SenderKeyRepairRequest {
        group_id: "scope".into(),
        sender_event_pubkey: device,
        key_id: Some(1),
        message_number: Some(0),
        required_revision: Some(1),
        created_at: NdrUnixSeconds(100),
    };
    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: device,
                created_at: NdrUnixSeconds(100),
            },
            &GroupPairwiseCommand::SenderKeyRepairRequest { request },
        )
        .unwrap();
    let wrap = |payload| {
        CheckpointGroupFanout::from(GroupPendingFanout::Remote {
            recipient_owner: owner,
            payload,
        })
    };
    let baseline = wrap(payload.clone());
    assert!(baseline.repair_identity().is_some());
    let mut event: UnsignedEvent = serde_json::from_slice(&payload).unwrap();
    let mut content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
    content
        .as_object_mut()
        .unwrap()
        .insert("futureRecoverySelector".into(), serde_json::json!(1));
    event.content = content.to_string();
    event.id = None;
    let extended = wrap(serde_json::to_vec(&event).unwrap());
    assert!(extended.repair_identity().is_some());
    assert_ne!(baseline.repair_identity(), extended.repair_identity());
    event.kind = Kind::from(21113);
    event.id = None;
    assert!(wrap(serde_json::to_vec(&event).unwrap())
        .repair_identity()
        .is_none());
    event.kind = Kind::from(10446);
    event.id = None;
    assert!(wrap(serde_json::to_vec(&event).unwrap())
        .repair_identity()
        .is_none());
    event.kind = Kind::from(10447);
    event.content = "invalid repair".into();
    event.id = None;
    assert!(wrap(serde_json::to_vec(&event).unwrap())
        .repair_identity()
        .is_none());
    assert!(
        CheckpointGroupFanout::from(GroupPendingFanout::LocalSiblings { payload })
            .repair_identity()
            .is_none()
    );
}
