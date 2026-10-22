#[test]
fn private_group_wake_and_broadcast_share_one_rollback_safe_checkpoint() {
    let mut fixture = remote_send_fixture();
    let group_id = fixture
        .sender
        .create_group(
            "private checkpoint".into(),
            vec![fixture.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap()
        .snapshot
        .unwrap()
        .group_id;
    let mut deferred = synthetic_pending_inbound(&mut fixture.sender, 4_214, 0, unix_now().get());
    for pending in &mut deferred {
        pending.next_retry_at_secs = u64::MAX;
    }
    fixture.sender.pending_inbound = deferred.clone();
    let sessions = serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap();
    let groups = serde_json::to_value(fixture.sender.group_manager.snapshot()).unwrap();
    let pending = fixture.sender.pending_group_fanouts.clone();
    let storage = observe_mutation_journal(&mut fixture.sender);
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(fixture
        .sender
        .send_group_payload(
            &group_id,
            b"private content".to_vec(),
            Some("group-message-id".into()),
        )
        .is_err());
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap(),
        sessions
    );
    assert_eq!(
        serde_json::to_value(fixture.sender.group_manager.snapshot()).unwrap(),
        groups
    );
    assert_eq!(fixture.sender.pending_group_fanouts, pending);
    assert!(fixture.sender.pending_group_publish_effects().is_empty());
    assert_eq!(storage.writes.lock().unwrap().len(), 1);

    storage.writes.lock().unwrap().clear();
    let sent = fixture
        .sender
        .send_group_payload(
            &group_id,
            b"private content".to_vec(),
            Some("group-message-id".into()),
        )
        .unwrap();
    assert_eq!(
        storage.writes.lock().unwrap().len(),
        1,
        "broadcast and private notification work must share a single checkpoint"
    );
    let authors = fixture
        .sender
        .group_sender_event_pubkeys_for_owner(fixture.owner.public_key());
    let (mut broadcast, mut wakes) = (0, 0);
    for effect in &sent.effects {
        let ProtocolEffect::Publish(publish) = effect;
        if publish.event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND {
            continue;
        }
        assert_eq!(publish.inner_event_id.as_deref(), Some("group-message-id"));
        assert!(sent.event_ids.contains(&publish.event.id.to_hex()));
        if authors.contains(&publish.event.pubkey) {
            broadcast += 1;
        } else {
            wakes += 1;
            assert_ne!(publish.event.pubkey, fixture.device.public_key());
            assert!(publish
                .event
                .tags
                .public_keys()
                .any(|key| *key == fixture.ready_device.public_key()));
        }
    }
    assert_eq!((broadcast, wakes), (0, 1));
    let is_wake = |pending: &ProtocolPendingGroupFanout| match pending.fanout.as_ref() {
        GroupPendingFanout::Remote { payload, .. }
        | GroupPendingFanout::LocalSiblings { payload } => {
            serde_json::from_slice::<serde_json::Value>(payload)
                .is_ok_and(|value| value["kind"] == 21_113)
        }
    };
    assert!(
        fixture
            .sender
            .pending_group_fanouts
            .iter()
            .any(|pending| is_wake(pending)
                && pending.remaining_devices.as_ref().is_some_and(
                    |devices| devices.contains(&ndr_device(fixture.late_device.public_key()))
                )),
        "the unavailable device's private wake remains durable"
    );
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert_eq!(
        restored.pending_group_fanouts,
        fixture.sender.pending_group_fanouts
    );
    let ids = |effects: &[ProtocolEffect]| {
        effects
            .iter()
            .map(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                publish.event.id.to_hex()
            })
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        ids(&restored.pending_group_publish_effects()),
        ids(&sent.effects),
        "a crash before the app outbox commits must preserve the entire signed batch"
    );
    let before_retry = serde_json::to_value(restored.session_manager.snapshot()).unwrap();
    let retry = restored
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
        .unwrap();
    assert_eq!(ids(&retry.effects), ids(&sent.effects));
    assert_eq!(
        serde_json::to_value(restored.session_manager.snapshot()).unwrap(),
        before_retry,
        "retry offers original ciphertext without advancing sessions"
    );
    let ack_storage = observe_mutation_journal(&mut restored);
    ack_storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(restored.ack_group_publish_effects(&sent.effects).is_err());
    assert_eq!(
        ids(&restored.pending_group_publish_effects()),
        ids(&sent.effects)
    );
    restored.ack_group_publish_effects(&sent.effects).unwrap();
    assert!(restored.pending_group_publish_effects().is_empty());
    // Install the late device's invite without consuming the pending fanout.
    let pending = std::mem::take(&mut restored.pending_group_fanouts);
    assert!(observe_sibling_invite(&mut restored, &fixture.late, &fixture.late_device).is_empty());
    restored.pending_group_fanouts = pending;
    for pending in &mut restored.pending_group_fanouts {
        pending.next_retry_at_secs = 0;
    }
    restored.persist().unwrap();
    let fanouts_before = restored.pending_group_fanouts.clone();
    let sessions_before = serde_json::to_value(restored.session_manager.snapshot()).unwrap();
    let groups_before = serde_json::to_value(restored.group_manager.snapshot()).unwrap();
    let deliveries_before = restored.pending_decrypted_deliveries.clone();
    // A ready wake uses an immediate atomic save even inside an already-dirty
    // app batch. Failing it must restore the session and original target order.
    restored.enter_batch();
    restored.batch_persist_dirty.set(true);
    ack_storage.fail_next.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(restored.retry_pending_group_fanouts(NdrUnixSeconds(unix_now().get())).is_err());
    assert_eq!(restored.pending_group_fanouts, fanouts_before);
    assert!(restored.pending_inbound == deferred, "failed fanout save changed deferred packets");
    assert_eq!(restored.pending_decrypted_deliveries, deliveries_before);
    assert_eq!(serde_json::to_value(restored.session_manager.snapshot()).unwrap(), sessions_before);
    assert_eq!(serde_json::to_value(restored.group_manager.snapshot()).unwrap(), groups_before);
    assert!(restored.pending_group_publish_effects().is_empty());
    assert!(restored.batch_persist_dirty.get());
    assert_eq!(restored.batch_depth.get(), 1);
    restored.exit_batch().unwrap();
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        ack_storage, fixture.owner.public_key(), &fixture.device,
    ).unwrap();
    assert_eq!(restored.pending_group_fanouts, fanouts_before);
    assert!(restored.pending_inbound == deferred, "fanout restart changed deferred packets");
    let resumed = restored.retry_pending_protocol(NdrUnixSeconds(unix_now().get())).unwrap();
    let effects = resumed
        .group_result
        .effects
        .into_iter()
        .chain(resumed.effects)
        .collect::<Vec<_>>();
    for effect in &effects {
        let ProtocolEffect::Publish(publish) = effect;
        let wire = serde_json::to_string(&publish.event).unwrap();
        for identity in [&fixture.owner, &fixture.device, &fixture.peer_owner, &fixture.ready_device] {
            assert!(!wire.contains(&identity.public_key().to_hex()),
                "late retry must not expose either account or the sending device");
        }
        assert_ne!(publish.event.pubkey, fixture.late_device.public_key());
        publish.event.verify().unwrap();
        assert!(
            !publish
                .event
                .tags
                .public_keys()
                .any(|key| *key == fixture.ready_device.public_key()),
            "restart must not resend the wake to an already prepared device"
        );
    }
    let received = decrypt_own_sync_effects(&mut fixture.late, effects);
    assert_eq!(
        received
            .iter()
            .filter(
                |message| serde_json::from_str::<serde_json::Value>(&message.content)
                    .is_ok_and(|value| value["kind"] == 21_113)
            )
            .count(),
        1,
        "the newly reachable device receives its queued wake after restart"
    );
}

#[test]
fn private_group_wake_ready_queue_is_bounded_and_respects_block_and_leave() {
    let mut fixture = remote_send_fixture();
    let group = fixture
        .sender
        .create_group(
            "queued".into(),
            vec![fixture.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap()
        .snapshot
        .unwrap();
    fixture
        .sender
        .send_group_payload(&group.group_id, b"message".to_vec(), Some("message".into()))
        .unwrap();
    let ready = fixture.sender.ready_group_publications.clone();
    let entry = ready.values().next().unwrap().clone();
    fixture.sender.ready_group_publications = (0..4096)
        .map(|index| (format!("{index:064x}"), entry.clone()))
        .collect();
    let sessions = serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap();
    assert!(fixture
        .sender
        .send_group_payload(&group.group_id, b"full".to_vec(), Some("full".into()))
        .is_err());
    assert_eq!(fixture.sender.ready_group_publications.len(), 4096);
    assert_eq!(
        serde_json::to_value(fixture.sender.session_manager.snapshot()).unwrap(),
        sessions,
        "queue capacity failure must roll back ratchet advancement"
    );
    fixture.sender.ready_group_publications = ready.clone();

    fixture
        .sender
        .hold_pending_direct_sends(fixture.peer_owner.public_key(), true);
    fixture
        .sender
        .retire_pending_direct_sends(fixture.peer_owner.public_key(), None)
        .unwrap();
    assert!(fixture
        .sender
        .ready_group_publications
        .values()
        .all(|pending| pending.wake_recipient.is_none()));
    assert!(
        fixture.sender.pending_group_fanouts.iter().all(|pending| {
            let GroupPendingFanout::Remote { payload, .. } = pending.fanout.as_ref() else {
                return true;
            };
            !serde_json::from_slice::<UnsignedEvent>(payload)
                .is_ok_and(|event| event.kind.as_u16() == 21_113)
        }),
        "blocking also retires unavailable-device wake intents"
    );
    assert!(
        fixture.sender.pending_group_publish_effects().is_empty(),
        "blocking the only recipient must not leave a shared-channel fallback"
    );

    fixture
        .sender
        .hold_pending_direct_sends(fixture.peer_owner.public_key(), false);
    fixture.sender.ready_group_publications = ready;
    assert!(!fixture.sender.pending_group_publish_effects().is_empty());

    let mut removed = fixture.sender.group_snapshot(&group.group_id).unwrap();
    removed
        .members
        .retain(|member| *member != ndr_owner(fixture.owner.public_key()));
    removed.admins = vec![ndr_owner(fixture.peer_owner.public_key())];
    removed.revision += 1;
    removed.updated_at = NdrUnixSeconds(removed.updated_at.get().saturating_add(1));
    assert!(fixture.sender.install_device_sync_group(removed).unwrap());
    assert!(
        fixture.sender.pending_group_publish_effects().is_empty(),
        "left-group ciphertext cannot be offered even before cleanup persists"
    );
    fixture
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
        .unwrap();
    assert!(fixture.sender.ready_group_publications.is_empty());
}
