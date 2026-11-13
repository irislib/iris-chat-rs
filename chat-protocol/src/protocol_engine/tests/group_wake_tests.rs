use super::*;
use nostr::JsonUtil;

#[test]
fn private_group_ready_publications_are_due_after_restart_until_outbox_ack() {
    let mut fixture = remote_send_fixture();
    observe_sibling_invite(&mut fixture.sender, &fixture.late, &fixture.late_device);
    let group_id = fixture
        .sender
        .create_group(
            "durable publication".into(),
            vec![fixture.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap()
        .snapshot
        .unwrap()
        .group_id;
    let sent = fixture
        .sender
        .send_group_payload(
            &group_id,
            b"recover before outbox acknowledgement".to_vec(),
            Some("ready-after-restart".into()),
        )
        .unwrap();
    let sent_data = |effects: &[ProtocolEffect]| {
        effects
            .iter()
            .filter_map(|ProtocolEffect::Publish(p)| {
                (p.carrier == ProtocolCarrier::SharedGroup).then(|| p.event.as_json())
            })
            .collect::<Vec<_>>()
    };
    let data = sent_data(&sent.effects);
    assert_eq!(
        data.len(),
        1,
        "one signed ciphertext serves both recipient devices"
    );
    let group_event = Event::from_json(&data[0]).unwrap();
    assert!(group_event.verify().is_ok());
    assert_eq!(
        group_event
            .tags
            .iter()
            .map(|t| &t.as_slice()[0])
            .collect::<Vec<_>>(),
        ["v", "header"]
    );
    assert!(sent
        .effects
        .iter()
        .all(|ProtocolEffect::Publish(p)| p.is_publishable()));
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
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
        ids(&sent.effects)
    );
    let now = NdrUnixSeconds(unix_now().get());
    assert!(restored.has_pending_retry_work());
    assert!(
        restored.has_due_pending_retry_work(now),
        "durable ciphertext must be offered to the app outbox even with no other due work"
    );
    let sessions = serde_json::to_value(restored.session_manager.snapshot()).unwrap();
    let retry = restored.retry_pending_protocol(now).unwrap();
    assert_eq!(ids(&retry.effects), ids(&sent.effects));
    assert_eq!(
        serde_json::to_value(restored.session_manager.snapshot()).unwrap(),
        sessions
    );
    assert!(restored.has_due_pending_retry_work(now));
    restored.ack_group_publish_effects(&retry.effects).unwrap();
    assert!(!restored.has_pending_retry_work());
    assert!(!restored.has_due_pending_retry_work(now));
}

#[test]
fn group_data_broadcast_uses_one_rollback_safe_checkpoint() {
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
        "the ciphertext and durable publication must share a single checkpoint"
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
            assert!(publish.event.tags.public_keys().next().is_none());
            assert_eq!(
                publish.carrier.pairwise_device(),
                Some(fixture.ready_device.public_key())
            );
        }
    }
    assert_eq!((broadcast, wakes), (1, 0));
    let is_wake = |pending: &ProtocolPendingGroupFanout| match pending.fanout.as_ref() {
        GroupPendingFanout::Remote { payload, .. }
        | GroupPendingFanout::LocalSiblings { payload } => {
            serde_json::from_slice::<serde_json::Value>(payload)
                .is_ok_and(|value| value["kind"] == 21_113)
        }
    };
    assert!(
        !fixture.sender.pending_group_fanouts.iter().any(is_wake),
        "one publication covers offline devices without queued private data copies"
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
        fixture
            .sender
            .pending_group_publish_effects()
            .iter()
            .any(|ProtocolEffect::Publish(p)| p.carrier == ProtocolCarrier::SharedGroup),
        "a DM block does not revoke group membership"
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
