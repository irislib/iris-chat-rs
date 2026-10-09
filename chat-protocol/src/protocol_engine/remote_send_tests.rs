struct RemoteSendFixture {
    store: Arc<InMemoryStorage>,
    owner: Keys,
    device: Keys,
    peer_owner: Keys,
    ready_device: Keys,
    late_device: Keys,
    sender: ProtocolEngine,
    ready: ProtocolEngine,
    late: ProtocolEngine,
}

fn remote_send_fixture() -> RemoteSendFixture {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let ready_device = Keys::generate();
    let late_device = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut ready = test_engine(&peer_owner, &ready_device);
    let mut late = test_engine(&peer_owner, &late_device);
    let own = signed_app_keys(&owner, &[device.public_key()], 1);
    let peer = signed_app_keys(
        &peer_owner,
        &[ready_device.public_key(), late_device.public_key()],
        1,
    );
    for engine in [&mut sender, &mut ready, &mut late] {
        engine.ingest_app_keys_event(&own).unwrap();
        engine.ingest_app_keys_event(&peer).unwrap();
    }
    observe_sibling_invite(&mut sender, &ready, &ready_device);
    RemoteSendFixture {
        store,
        owner,
        device,
        peer_owner,
        ready_device,
        late_device,
        sender,
        ready,
        late,
    }
}

#[test]
fn remote_direct_readiness_needs_one_reachable_authorized_device() {
    let f = remote_send_fixture();
    assert_eq!(
        f.sender.direct_send_readiness(f.peer_owner.public_key()),
        DirectSendReadiness::Ready,
        "one undiscovered device must not prevent delivery to the contact's ready device"
    );
}

#[test]
fn remote_direct_partial_delivery_recovers_after_restart_without_resending_ready_device() {
    for peer_only in [false, true] {
        let mut f = remote_send_fixture();
        let peer = f.peer_owner.public_key();
        let rumor = own_seen_rumor(&f.owner);
        let sent = if peer_only {
            f.sender.send_direct_unsigned_event_to_peer_only(
                peer,
                &peer.to_hex(),
                rumor,
                UnixSeconds(10),
            )
        } else {
            f.sender
                .send_direct_unsigned_event(peer, &peer.to_hex(), rumor, UnixSeconds(10))
        }
        .expect("ready devices receive immediately while missing devices remain queued");
        assert_eq!(
            decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
            1
        );
        assert!(f.sender.has_pending_retry_work());
        assert!(f
            .sender
            .retry_pending_protocol(NdrUnixSeconds(20))
            .unwrap()
            .effects
            .is_empty());
        f.sender = ProtocolEngine::load_or_create_for_local_device(
            f.store,
            f.owner.public_key(),
            &f.device,
        )
        .unwrap();
        let retry = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
        assert_eq!(retry.effects.iter().filter(|effect| matches!(effect, ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)).count(), 1,
            "a completed recipient must not be encrypted and sent again");
        assert_eq!(
            decrypt_own_sync_effects(&mut f.late, retry.effects).len(),
            1
        );
        assert!(!f.sender.has_pending_retry_work());
    }
}

#[test]
fn remote_direct_pending_history_excludes_new_devices_and_prunes_revoked_devices() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "only originally authorized devices",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let added_device = Keys::generate();
    let added = test_engine(&f.peer_owner, &added_device);
    let expanded = signed_app_keys(
        &f.peer_owner,
        &[
            f.ready_device.public_key(),
            f.late_device.public_key(),
            added_device.public_key(),
        ],
        30,
    );
    assert!(f
        .sender
        .ingest_app_keys_event(&expanded)
        .unwrap()
        .effects
        .is_empty());
    assert!(
        observe_sibling_invite(&mut f.sender, &added, &added_device)
            .effects
            .is_empty(),
        "new device must not receive old pending content"
    );
    let revoked = signed_app_keys(
        &f.peer_owner,
        &[f.ready_device.public_key(), added_device.public_key()],
        40,
    );
    assert!(f
        .sender
        .ingest_app_keys_event(&revoked)
        .unwrap()
        .effects
        .is_empty());
    assert!(
        !f.sender.has_pending_retry_work(),
        "revoked pending target must be discarded"
    );
    let restored = signed_app_keys(
        &f.peer_owner,
        &[
            f.ready_device.public_key(),
            f.late_device.public_key(),
            added_device.public_key(),
        ],
        50,
    );
    f.sender.ingest_app_keys_event(&restored).unwrap();
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty(),
        "reauthorizing a device must not resurrect discarded history"
    );
}

#[test]
fn remote_direct_stop_typing_reaches_ready_device_and_expires_pending_retry() {
    let mut f = remote_send_fixture();
    let now = unix_now().get();
    let peer = f.peer_owner.public_key();
    let stop = pairwise_codec::typing_event(
        f.owner.public_key(),
        pairwise_codec::EncodeOptions::new(now, now.saturating_mul(1000)).with_expiration(1),
    ).unwrap();
    let sent = f.sender.send_direct_unsigned_event(peer, &peer.to_hex(), stop, UnixSeconds(now)).unwrap();
    assert!(sent.effects.iter().any(|effect| matches!(effect, ProtocolEffect::Publish(_))));
    assert!(f.sender.has_pending_retry_work());
    assert!(f.sender.retry_pending_protocol(NdrUnixSeconds(now + 11)).unwrap().effects.is_empty());
    assert!(!f.sender.has_pending_retry_work());
}

#[test]
fn remote_direct_pending_payload_is_discarded_when_expired() {
    let mut f = remote_send_fixture();
    let now = unix_now().get();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "temporary",
            Some(now + 30),
            UnixSeconds(now),
        )
        .unwrap();
    assert!(f.sender.has_pending_retry_work());
    assert!(f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(now + 31))
        .unwrap()
        .effects
        .is_empty());
    assert!(!f.sender.has_pending_retry_work());
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty()
    );
}

#[test]
fn remote_direct_pending_payload_is_discarded_when_local_device_is_revoked() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "before removal",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let remaining = Keys::generate();
    let revoke = signed_app_keys(&f.owner, &[remaining.public_key()], 30);
    assert!(f
        .sender
        .ingest_app_keys_event(&revoke)
        .unwrap()
        .effects
        .is_empty());
    assert!(!f.sender.has_pending_retry_work());
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty()
    );
}

#[test]
fn group_members_cannot_promote_themselves() {
    let mut f = remote_send_fixture();
    let created = f
        .sender
        .create_group(
            "Admin authorization".into(),
            vec![f.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    for message in decrypt_own_sync_effects(&mut f.ready, created.effects) {
        f.ready
            .process_group_pairwise_payload(
                message.content.as_bytes(),
                message.sender,
                message.sender_device,
            )
            .unwrap();
    }
    assert_eq!(
        f.ready.group_manager.group(&group_id).unwrap().admins,
        vec![ndr_owner(f.owner.public_key())]
    );
    assert!(f
        .ready
        .set_group_admin(&group_id, f.peer_owner.public_key(), true)
        .is_err());

    f.sender
        .add_group_members(&group_id, vec![Keys::generate().public_key()])
        .unwrap();
    let original = f.sender.group_manager.group(&group_id).unwrap();
    assert_eq!(
        original.admins,
        vec![ndr_owner(f.owner.public_key())],
        "Adding members must not make them admins"
    );
    let mut forged = original.clone();
    forged.admins.push(ndr_owner(f.peer_owner.public_key()));
    forged.revision += 1;
    forged.updated_at = NdrUnixSeconds(forged.updated_at.get() + 1);
    let fact = group_roster_fact_event_for_test(&f.peer_owner, &forged);
    assert!(
        f.sender
            .ingest_group_roster_fact_event(&fact)
            .unwrap()
            .is_none(),
        "A member cannot authorize their own promotion by signing a new membership list"
    );

    let payload = JsonGroupPayloadCodecV1
        .encode_pairwise_command(
            nostr_double_ratchet::GroupPayloadEncodeContext {
                local_device_pubkey: ndr_device(f.ready_device.public_key()),
                created_at: forged.updated_at,
            },
            &GroupPairwiseCommand::MetadataSnapshot { snapshot: forged },
        )
        .unwrap();
    // Check both an honest sender identity and a claim to be the creator,
    // while retaining the member's authenticated device identity.
    for claimed_sender in [f.peer_owner.public_key(), f.owner.public_key()] {
        let result = f
            .sender
            .process_group_pairwise_payload(
                &payload,
                claimed_sender,
                Some(f.ready_device.public_key()),
            )
            .unwrap();
        assert!(result.events.is_empty());
        assert_eq!(f.sender.group_manager.group(&group_id).unwrap(), original);
    }
    f.sender =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    f.sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    assert_eq!(
        f.sender.group_manager.group(&group_id).unwrap(),
        original,
        "Restarting or retrying rejected changes must not grant admin access"
    );
}

#[test]
fn group_fanout_retries_only_missing_devices_after_restart() {
    let mut f = remote_send_fixture();
    let created = f
        .sender
        .create_group(
            "partial group".into(),
            vec![f.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    let delivered = decrypt_own_sync_effects(&mut f.ready, created.effects);
    assert!(
        !delivered.is_empty(),
        "ready recipient gets group metadata immediately"
    );
    let retry = f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    assert!(
        retry.group_result.effects.is_empty(),
        "unavailable sibling must not repeatedly encrypt and send to completed devices"
    );
    f.sender =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    let retry = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    let recovered = decrypt_own_sync_effects(&mut f.late, retry.group_result.effects);
    assert_eq!(
        recovered.len(),
        delivered.len(),
        "late device receives exactly the original controls"
    );
    for message in recovered {
        f.late
            .process_group_pairwise_payload(
                message.content.as_bytes(),
                message.sender,
                message.sender_device,
            )
            .unwrap();
    }
    assert!(f.late.group_manager.group(&group_id).is_some());
    assert_eq!(f.sender.debug_snapshot().pending_group_fanout_count, 0);
}

#[test]
fn group_fanout_drops_revoked_targets_and_never_adds_new_devices_to_pending_history() {
    let mut f = remote_send_fixture();
    f.sender
        .create_group(
            "pending history".into(),
            vec![f.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap();
    let added_key = Keys::generate();
    let added = test_engine(&f.peer_owner, &added_key);
    f.sender
        .ingest_app_keys_event(&signed_app_keys(
            &f.peer_owner,
            &[
                f.ready_device.public_key(),
                f.late_device.public_key(),
                added_key.public_key(),
            ],
            2,
        ))
        .unwrap();
    let retry = observe_sibling_invite(&mut f.sender, &added, &added_key);
    assert!(
        retry.group_result.effects.is_empty(),
        "new devices do not inherit pending history"
    );
    let retry = f
        .sender
        .ingest_app_keys_event(&signed_app_keys(
            &f.peer_owner,
            &[f.ready_device.public_key(), added_key.public_key()],
            3,
        ))
        .unwrap();
    assert!(retry.group_result.effects.is_empty());
    assert_eq!(
        f.sender.debug_snapshot().pending_group_fanout_count,
        0,
        "revoked targets are pruned durably"
    );
    f.sender =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    assert_eq!(f.sender.debug_snapshot().pending_group_fanout_count, 0);
}

#[test]
fn group_fanout_save_failure_keeps_unsent_targets_and_ratchets_retryable() {
    struct ReadOnlyStorage(Arc<InMemoryStorage>);
    impl StorageAdapter for ReadOnlyStorage {
        fn get(&self, key: &str) -> StorageResult<Option<String>> {
            self.0.get(key)
        }
        fn put(&self, _: &str, _: String) -> StorageResult<()> {
            Err(StorageError::new("injected disk write failure"))
        }
        fn del(&self, key: &str) -> StorageResult<()> {
            self.0.del(key)
        }
        fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
            self.0.list(prefix)
        }
    }
    let mut f = remote_send_fixture();
    f.sender
        .create_group(
            "durable fanout".into(),
            vec![f.peer_owner.public_key()],
            unix_now(),
        )
        .unwrap();
    // Install the late device's invitation without executing the queued sends.
    let pending = std::mem::take(&mut f.sender.pending_group_fanouts);
    observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    f.sender.pending_group_fanouts = pending.clone();
    let before = serde_json::to_value(f.sender.session_manager_snapshot()).unwrap();
    f.sender.storage = Arc::new(ReadOnlyStorage(f.store.clone()));
    assert!(f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .is_err());
    assert!(
        f.sender.pending_group_fanouts == pending,
        "failed persistence must not acknowledge unpublished controls"
    );
    assert!(
        serde_json::to_value(f.sender.session_manager_snapshot()).unwrap() == before,
        "failed publication preparation must not advance ratchets"
    );
    f.sender.storage = f.store;
    let retry = f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    assert!(!decrypt_own_sync_effects(&mut f.late, retry.group_result.effects).is_empty());
    assert_eq!(f.sender.debug_snapshot().pending_group_fanout_count, 0);
}

#[test]
fn direct_foreign_recipient_does_not_enter_retry_queues() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let sent = f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "for the ready device",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let event = sent
        .effects
        .iter()
        .map(|ProtocolEffect::Publish(publish)| &publish.event)
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap();
    assert_eq!(
        parse_message_event(event).unwrap().recipient,
        Some(ndr_device(f.ready_device.public_key()))
    );
    assert!(f
        .late
        .process_direct_message_event(event)
        .unwrap()
        .is_none());
    assert!(
        f.late.pending_inbound.is_empty(),
        "a sibling's ciphertext is not a missing local session"
    );
    assert!(
        f.late.pending_group_sender_key_messages.is_empty(),
        "a targeted direct message is not a group candidate"
    );
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
        1
    );
}

#[test]
fn direct_foreign_recipient_is_pruned_from_persisted_retry_queues() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let sent = f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "legacy queued foreign delivery",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let event = sent
        .effects
        .iter()
        .map(|ProtocolEffect::Publish(publish)| &publish.event)
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap()
        .clone();
    // Reproduce an older client's durable queue, then upgrade/restart.
    let envelope = parse_message_event(&event).unwrap();
    f.late
        .queue_header_group_sender_key_candidate(&event)
        .unwrap();
    f.late
        .queue_pending_inbound_direct_event(event, 10, Some(&envelope), None)
        .unwrap();
    assert!(!f.late.pending_inbound.is_empty());
    assert!(!f.late.pending_group_sender_key_messages.is_empty());
    let storage = f.late.storage.clone();
    f.late = ProtocolEngine::load_or_create_for_local_device(storage.clone(), peer, &f.late_device)
        .unwrap();
    let retry = f
        .late
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    assert!(retry.is_empty());
    assert!(
        f.late.pending_inbound.is_empty(),
        "foreign ciphertext must not retry forever after upgrade"
    );
    assert!(f.late.pending_group_sender_key_messages.is_empty());
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, peer, &f.late_device).unwrap();
    assert!(restored.pending_inbound.is_empty());
    assert!(restored.pending_group_sender_key_messages.is_empty());
}

#[test]
fn direct_recipient_routing_preserves_legacy_owner_and_multi_recipient_messages() {
    for variant in ["legacy", "device", "owner", "multi-device", "multi-owner"] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let mut receiver = test_engine(&owner, &device);
        receiver
            .ingest_app_keys_event(&signed_app_keys(
                &peer_owner,
                &[peer_device.public_key()],
                10,
            ))
            .unwrap();
        let (mut session, response) = receiver
            .local_invite()
            .unwrap()
            .accept_with_owner(
                peer_device.public_key(),
                peer_device.secret_key().to_secret_bytes(),
                Some(peer_device.public_key().to_hex()),
                Some(peer_owner.public_key()),
            )
            .unwrap();
        receiver
            .observe_invite_response_event(&invite_response_event(&response).unwrap())
            .unwrap();
        let plan = session
            .plan_send(variant.as_bytes(), NdrUnixSeconds(20))
            .unwrap();
        let mut envelope = session.apply_send(plan).envelope;
        envelope.recipient = match variant {
            "legacy" => None,
            "device" => Some(ndr_device(device.public_key())),
            "owner" => Some(ndr_device(owner.public_key())),
            _ => Some(ndr_device(Keys::generate().public_key())),
        };
        let mut event = nostr_double_ratchet::message_event(&envelope).unwrap();
        if variant.starts_with("multi-") {
            let local = if variant == "multi-device" {
                device.public_key()
            } else {
                owner.public_key()
            };
            event = nostr::EventBuilder::new(event.kind, event.content.clone())
                .tags(event.tags.iter().cloned())
                .tag(nostr::Tag::public_key(local))
                .custom_created_at(event.created_at)
                .sign_with_keys(&Keys::new(
                    nostr::SecretKey::from_slice(&envelope.signer_secret_key).unwrap(),
                ))
                .unwrap();
        }
        let received = receiver
            .process_direct_message_event(&event)
            .unwrap()
            .expect(variant);
        assert_eq!(received.content, variant);
        assert_eq!(received.sender, peer_owner.public_key());
    }
}

#[test]
fn removed_group_member_cannot_retry_messages_after_restart_but_controls_survive() {
    fn apply_controls(receiver: &mut ProtocolEngine, effects: Vec<ProtocolEffect>) {
        for message in decrypt_own_sync_effects(receiver, effects) {
            receiver
                .process_group_pairwise_payload(
                    message.content.as_bytes(),
                    message.sender,
                    message.sender_device,
                )
                .unwrap();
        }
    }
    fn pending_command(pending: &ProtocolPendingGroupFanout) -> GroupPairwiseCommand {
        let payload = match &pending.fanout {
            GroupPendingFanout::Remote { payload, .. }
            | GroupPendingFanout::LocalSiblings { payload } => payload,
        };
        JsonGroupPayloadCodecV1
            .decode_pairwise_command(payload)
            .unwrap()
            .unwrap()
    }

    let mut f = remote_send_fixture();
    let third_member = Keys::generate().public_key();
    let created = f
        .sender
        .create_group(
            "removal while offline".into(),
            vec![f.peer_owner.public_key(), third_member],
            unix_now(),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    apply_controls(&mut f.ready, created.effects);
    let promoted = f
        .sender
        .set_group_admin(&group_id, f.peer_owner.public_key(), true)
        .unwrap();
    apply_controls(&mut f.ready, promoted.effects);
    let sent = f
        .sender
        .send_group_payload(
            &group_id,
            b"unsent text".to_vec(),
            Some("queued-message".into()),
        )
        .unwrap();
    assert!(!sent.effects.is_empty());
    assert!(
        f.sender.pending_group_fanouts.iter().any(|pending| {
            pending.inner_event_id.is_none()
                && matches!(
                    pending_command(pending),
                    GroupPairwiseCommand::SenderKeyDistribution { .. }
                )
        }),
        "group creation queued the handoff needed to decrypt the later message"
    );

    // An earlier removal must still reach the absent peer device even after the
    // other administrator removes us. Both operations use real control payloads.
    let removal = f
        .sender
        .remove_group_member(&group_id, third_member)
        .unwrap();
    apply_controls(&mut f.ready, removal.effects);
    let removal = f
        .ready
        .remove_group_member(&group_id, f.owner.public_key())
        .unwrap();
    apply_controls(&mut f.sender, removal.effects);
    assert!(!f
        .sender
        .group_manager
        .group(&group_id)
        .unwrap()
        .members
        .contains(&ndr_owner(f.owner.public_key())));
    f.sender = ProtocolEngine::load_or_create_for_local_device(
        f.store.clone(),
        f.owner.public_key(),
        &f.device,
    )
    .unwrap();
    let retry = f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(unix_now().get() + 60))
        .unwrap();
    assert!(retry.group_result.effects.is_empty());
    assert!(
        f.sender.pending_group_fanouts.iter().all(|pending| {
            matches!(
                pending_command(pending),
                GroupPairwiseCommand::MetadataSnapshot { .. }
            )
        }),
        "removed members must not retry queued sender-key handoffs"
    );
    assert!(
        f.sender.pending_group_fanouts.iter().any(|pending| {
            matches!(pending_command(pending), GroupPairwiseCommand::MetadataSnapshot { snapshot }
            if !snapshot.members.contains(&ndr_owner(third_member)))
        }),
        "the unavailable device still needs the earlier membership removal"
    );
    assert!(f
        .sender
        .send_group_payload(
            &group_id,
            b"blocked new text".to_vec(),
            Some("new-message".into())
        )
        .is_err());

    f.sender =
        ProtocolEngine::load_or_create_for_local_device(f.store, f.owner.public_key(), &f.device)
            .unwrap();
    assert!(
        f.sender.pending_group_fanouts.iter().all(|pending| {
            matches!(
                pending_command(pending),
                GroupPairwiseCommand::MetadataSnapshot { .. }
            )
        }),
        "cancellation is durable"
    );
    let retry = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
    let recovered = decrypt_own_sync_effects(&mut f.late, retry.group_result.effects);
    assert!(!recovered.is_empty());
    assert!(recovered.iter().all(|message| {
        matches!(
            JsonGroupPayloadCodecV1
                .decode_pairwise_command(message.content.as_bytes())
                .unwrap(),
            Some(GroupPairwiseCommand::MetadataSnapshot { .. })
        )
    }));
    assert!(recovered.iter().any(|message| {
        matches!(JsonGroupPayloadCodecV1.decode_pairwise_command(message.content.as_bytes()).unwrap(),
            Some(GroupPairwiseCommand::MetadataSnapshot { snapshot })
                if !snapshot.members.contains(&ndr_owner(third_member)))
    }), "late devices receive the membership removal, without old sender keys");
}

#[test]
fn sibling_group_removal_requires_new_revision_to_restore_membership() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let admin = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut engine =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let joined = group_snapshot_for_test(
        "removed-sibling-snapshot",
        "Friends",
        1,
        &admin,
        &[admin.public_key(), owner.public_key()],
    );
    engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &joined))
        .unwrap();
    let mut removed = joined.clone();
    removed.revision = 2;
    removed.updated_at = NdrUnixSeconds(20);
    removed
        .members
        .retain(|member| *member != ndr_owner(owner.public_key()));
    assert!(engine.install_device_sync_group(removed.clone()).unwrap());
    engine = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();

    let mut conflicting = joined.clone();
    conflicting.revision = removed.revision;
    conflicting.updated_at = removed.updated_at;
    let result = engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &conflicting))
        .unwrap()
        .unwrap();
    assert!(
        result.snapshot.is_none(),
        "equal-version signed replay must not undo a sibling removal"
    );
    assert!(!engine
        .install_device_sync_group(conflicting.clone())
        .unwrap());
    conflicting.updated_at = NdrUnixSeconds(21);
    assert!(
        !engine
            .install_device_sync_group(conflicting.clone())
            .unwrap(),
        "a newer clock without a newer membership revision is not a re-add"
    );
    assert_eq!(engine.group_manager.group(&joined.group_id), Some(removed));

    conflicting.revision += 1;
    conflicting.members.sort();
    let result = engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&admin, &conflicting))
        .unwrap()
        .unwrap();
    assert_eq!(result.snapshot, Some(conflicting.clone()));
    assert_eq!(
        engine.group_manager.group(&joined.group_id),
        Some(conflicting)
    );
}

#[test]
fn sibling_group_protocol_cannot_contradict_signed_roster_after_restart() {
    let mut keys = [1, 2].map(|seed| Keys::new(nostr::SecretKey::from_slice(&[seed; 32]).unwrap()));
    keys.sort_by_key(Keys::public_key);
    for (admin, owner) in [(&keys[0], &keys[1]), (&keys[1], &keys[0])] {
        assert_sibling_group_protocol_matches_signed_roster_after_restart(admin, owner);
    }
}

fn assert_sibling_group_protocol_matches_signed_roster_after_restart(admin: &Keys, owner: &Keys) {
    let device = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut engine =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut original = group_snapshot_for_test(
        "signed-group-protocol",
        "Friends",
        1,
        admin,
        &[admin.public_key(), owner.public_key()],
    );
    engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(admin, &original))
        .unwrap();
    // Signed roster ingestion canonicalizes members regardless of their input order.
    original.members.sort();
    engine =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();

    for revision in [original.revision, original.revision + 1] {
        let mut downgrade = original.clone();
        downgrade.protocol = GroupProtocol::pairwise_fanout_v1();
        downgrade.revision = revision;
        downgrade.updated_at = NdrUnixSeconds(100 + revision);
        assert!(
            !engine.install_device_sync_group(downgrade).unwrap(),
            "a sibling clock cannot override the signed group protocol"
        );
        assert_eq!(
            engine.group_manager.group(&original.group_id),
            Some(original.clone())
        );
    }

    let mut newer = original.clone();
    newer.name = "Renamed friends".into();
    newer.revision += 1;
    newer.updated_at = NdrUnixSeconds(200);
    assert!(engine.install_device_sync_group(newer.clone()).unwrap());
    newer
        .members
        .retain(|member| *member != ndr_owner(owner.public_key()));
    newer.revision += 1;
    assert!(engine.install_device_sync_group(newer.clone()).unwrap());
    engine = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    assert_eq!(engine.group_manager.group(&original.group_id), Some(newer));
}

#[test]
fn sibling_group_protocol_can_recover_to_signed_protocol_and_follow_signed_advancement() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut engine =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let original = group_snapshot_for_test(
        "recover-group-protocol",
        "Friends",
        1,
        &owner,
        &[owner.public_key()],
    );
    engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&owner, &original))
        .unwrap();

    // Reproduce state left behind by a previously accepted malformed sibling echo.
    let mut poisoned = original.clone();
    poisoned.protocol = GroupProtocol::pairwise_fanout_v1();
    assert!(engine.install_group_roster_snapshot(poisoned).unwrap());
    engine.persist().unwrap();
    engine =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    assert!(engine.install_device_sync_group(original.clone()).unwrap());
    assert_eq!(
        engine.group_manager.group(&original.group_id),
        Some(original.clone())
    );

    let mut advanced = original.clone();
    advanced.protocol = GroupProtocol::pairwise_fanout_v1();
    advanced.revision += 1;
    advanced.updated_at = NdrUnixSeconds(200);
    let installed = engine
        .ingest_group_roster_fact_event(&group_roster_fact_event_for_test(&owner, &advanced))
        .unwrap()
        .unwrap();
    assert_eq!(installed.snapshot, Some(advanced.clone()));
    engine = ProtocolEngine::load_or_create_for_local_device(store, owner.public_key(), &device)
        .unwrap();
    let mut unsigned_change = advanced.clone();
    unsigned_change.protocol = GroupProtocol::sender_key_v1();
    unsigned_change.revision += 1;
    assert!(!engine.install_device_sync_group(unsigned_change).unwrap());
    assert_eq!(
        engine.group_manager.group(&original.group_id),
        Some(advanced)
    );
}
