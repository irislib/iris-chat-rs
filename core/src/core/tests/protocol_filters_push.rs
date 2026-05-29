#[test]
fn queued_direct_send_starts_targeted_owner_protocol_fetch() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-direct-targeted-fetch", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;
    observe_current_device_appkeys_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &owner,
        &device,
    );

    core.send_direct_message(
        &peer.public_key().to_hex(),
        "queued until targeted app keys arrive",
        UnixSeconds(1_777_000_000),
        None,
    );

    let target = format!("owner:{}", peer.public_key().to_hex());
    assert!(
        core.debug_log.iter().any(|entry| {
            entry.category == "appcore.protocol.queued" && entry.detail.contains(&target)
        }) && core.debug_log.iter().any(|entry| {
            entry.category == "protocol.engine_fetch.fetch" && entry.detail.contains("filters=1")
        }),
        "queued direct owner work should start a narrow AppKeys fetch for {target}"
    );
}

#[test]
fn retry_batch_coalesces_duplicate_queued_protocol_fetches() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-retry-coalesce-fetches", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    let target = format!("owner:{}", peer.public_key().to_hex());
    let filters = vec![Filter::new()
        .author(peer.public_key())
        .kind(Kind::Custom(APP_KEYS_EVENT_KIND as u16))];
    let result = ProtocolRetryResult {
        message_id: "message-1".to_string(),
        chat_id: peer.public_key().to_hex(),
        effects: vec![ProtocolEffect::FetchProtocolState {
            filters,
            reason: "retry",
        }],
        queued_targets: vec![target.clone()],
        ..ProtocolRetryResult::default()
    };

    core.process_protocol_engine_retry_batch(
        "test_retry_dedupe",
        ProtocolRetryBatch {
            direct_results: vec![result.clone(), result],
            ..ProtocolRetryBatch::default()
        },
    );

    let retry_log = core
        .debug_log
        .iter()
        .find(|entry| entry.category == "appcore.protocol.retry")
        .expect("retry log");
    assert!(
        retry_log.detail.contains("queued_targets=1"),
        "retry log should count unique queued protocol targets: {}",
        retry_log.detail
    );
    let queued_log = core
        .debug_log
        .iter()
        .find(|entry| entry.category == "appcore.protocol.queued")
        .expect("queued log");
    assert_eq!(
        queued_log.detail.matches(&target).count(),
        1,
        "queued log should list each target once: {}",
        queued_log.detail
    );
    assert_eq!(
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "protocol.engine_fetch.fetch")
            .count(),
        1,
        "duplicate retry effects should schedule one targeted fetch"
    );
}

#[test]
fn queued_protocol_filters_are_narrow_for_missing_owner_roster() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let result = engine
        .send_direct_text(
            peer.public_key(),
            &peer.public_key().to_hex(),
            "queued until appkeys",
            None,
            UnixSeconds(1_777_159_500),
        )
        .expect("direct send");
    let filters = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::FetchProtocolState { filters, .. } => Some(filters.clone()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(filters.len(), 1);
    assert!(has_filter_with_kind_author(
        &filters,
        APP_KEYS_EVENT_KIND,
        peer.public_key()
    ));
    assert!(
        !has_bootstrap_message_filter(&filters),
        "queued owner discovery must not depend on an unscoped message bootstrap filter"
    );
}

#[test]
fn queued_self_send_fetches_owner_appkeys_for_concrete_sibling_target() {
    let owner = Keys::generate();
    let phone_device = Keys::generate();
    let desktop_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &phone_device);
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![
                DeviceEntry::new(phone_device.public_key(), 1),
                DeviceEntry::new(desktop_device.public_key(), 1),
            ]),
            1,
        )
        .expect("local AppKeys");

    let result = engine
        .send_direct_text(
            owner.public_key(),
            &owner.public_key().to_hex(),
            "queued until sibling invite",
            None,
            UnixSeconds(1_777_159_500),
        )
        .expect("self direct send");
    let filters = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::FetchProtocolState { filters, .. } => Some(filters.clone()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();

    assert!(has_filter_with_kind_author(
        &filters,
        APP_KEYS_EVENT_KIND,
        owner.public_key()
    ));
    assert!(has_filter_with_kind_author_tag(
        &filters,
        INVITE_EVENT_KIND,
        desktop_device.public_key(),
        "#l",
        NDR_INVITES_L_TAG,
    ));
}

#[test]
fn queued_group_create_schedules_fast_protocol_retry_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-fast-retry", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.create_group("Queued group", &[peer.public_key().to_hex()]);

    let debug = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert!(
        debug.pending_group_fanout_count > 0,
        "missing group member protocol state should leave a durable group fanout"
    );
    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("queued group work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "queued group work should schedule a fast retry tick, not wait for the normal liveness interval"
    );
}

#[test]
fn current_queued_protocol_targets_includes_group_fanout_targets() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-targets", &owner, &device);

    core.create_group("Queued group", &[peer.public_key().to_hex()]);

    let targets = core.current_queued_protocol_targets();
    assert!(
        targets.contains(&peer.public_key().to_hex()),
        "queued protocol targets should include lightweight group fanout targets"
    );
}

#[test]
fn queued_group_retry_without_protocol_progress_reschedules_fast_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-retry-reschedule", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.create_group("Queued group", &[peer.public_key().to_hex()]);
    core.protocol_subscription_runtime.liveness_due_at = None;

    let retry_at = unix_now().get().saturating_add(10_000);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .retry_pending_protocol(NdrUnixSeconds(retry_at))
        .expect("retry pending protocol");
    assert!(
        !batch.group_result.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::PublishSigned(_)
                | ProtocolEffect::PublishSignedForInnerEvent { .. }
                | ProtocolEffect::PublishStagedFirstContact { .. }
        )),
        "missing member protocol state should not produce group publishes yet"
    );
    assert!(
        !batch.group_result.queued_targets.is_empty(),
        "the retry batch must report the still-queued group target"
    );

    core.process_protocol_engine_retry_batch("test_group_retry", batch);

    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("still-queued group work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "still-queued group work should keep a fast retry tick alive"
    );
}

#[test]
fn appcore_protocol_engine_partial_fanout_publishes_ready_device_and_queues_missing_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_phone = Keys::generate();
    let peer_laptop = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let peer_app_keys = AppKeys::new(vec![
        DeviceEntry::new(peer_phone.public_key(), 1),
        DeviceEntry::new(peer_laptop.public_key(), 1),
    ]);
    engine
        .ingest_app_keys_snapshot(peer_owner.public_key(), peer_app_keys, 1)
        .expect("peer appkeys");

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(2), &mut rng);
    let phone_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_phone.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("phone invite");
    let phone_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&phone_invite)
        .expect("invite event")
        .sign_with_keys(&peer_phone)
        .expect("signed invite");
    engine
        .observe_invite_event(&phone_invite_event)
        .expect("observe phone invite");

    let result = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "hello",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    assert_eq!(result.event_ids.len(), 1);
    assert!(
        result
            .queued_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "missing peer laptop should remain queued"
    );
    let staged = result
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => {
                Some((bootstrap, payload))
            }
            _ => None,
        })
        .expect("first contact should stage bootstrap before payload");
    assert!(
        staged
            .0
            .iter()
            .any(|publish| publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND),
        "bootstrap phase should contain the invite response"
    );
    assert_eq!(
        staged.0[0].inner_event_id.as_deref(),
        Some(result.message_id.as_str()),
        "bootstrap publish must be tied to the app message so payload can wait on it"
    );
    assert_eq!(
        staged.0[0].target_owner_pubkey_hex.as_deref(),
        Some(peer_owner.public_key().to_hex().as_str())
    );
    assert_eq!(
        staged.1.len(),
        1,
        "payload phase should contain the ready phone delivery"
    );
    assert_eq!(
        staged.1[0].target_owner_pubkey_hex.as_deref(),
        Some(peer_owner.public_key().to_hex().as_str())
    );

    let mut ctx = ProtocolContext::new(NdrUnixSeconds(120), &mut rng);
    let laptop_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_laptop.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("laptop invite");
    let laptop_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&laptop_invite)
        .expect("invite event")
        .sign_with_keys(&peer_laptop)
        .expect("signed invite");
    let batch = engine
        .observe_invite_event(&laptop_invite_event)
        .expect("observe laptop invite");

    assert_eq!(batch.direct_results.len(), 1);
    let retry = &batch.direct_results[0];
    assert_eq!(retry.message_id, result.message_id);
    assert_eq!(retry.event_ids.len(), 1);
    assert!(
        !retry
            .queued_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "all remote devices should be prepared after the missing invite arrives"
    );
    assert!(
        !engine
            .debug_snapshot()
            .pending_outbound_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "remote peer fanout should be fully drained"
    );
}

#[test]
fn appcore_ownerless_invite_uses_known_roster_owner_for_first_contact() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(2), &mut rng);
    let ownerless_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        None,
        None,
    )
    .expect("ownerless invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&ownerless_invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");

    engine
        .observe_invite_event(&invite_event)
        .expect("observe ownerless invite");

    let result = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "hello",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    assert!(
        !result
            .queued_targets
            .contains(&peer_device.public_key().to_hex())
            && !result
                .queued_targets
                .contains(&format!("owner:{}", peer_owner.public_key().to_hex())),
        "known ownerless invite should not leave the peer device queued: {:?}",
        result.queued_targets
    );
    assert!(
        result.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::PublishStagedFirstContact { bootstrap, .. }
                if bootstrap.iter().any(|publish| publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
        )),
        "first contact should publish an invite response for ownerless peer invites"
    );
}

#[test]
fn appcore_message_author_tracking_includes_current_next_and_skipped_sender_keys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let current_sender = Keys::generate();
    let next_sender = Keys::generate();
    let skipped_sender = Keys::generate();
    let our_current = Keys::generate();
    let our_next = Keys::generate();
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_device = NdrDevicePubkey::from_bytes(device.public_key().to_bytes());
    let peer_owner_pubkey = NdrOwnerPubkey::from_bytes(peer_owner.public_key().to_bytes());
    let peer_device_pubkey = NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes());
    let current_sender_pubkey = NdrDevicePubkey::from_bytes(current_sender.public_key().to_bytes());
    let next_sender_pubkey = NdrDevicePubkey::from_bytes(next_sender.public_key().to_bytes());
    let skipped_sender_pubkey = NdrDevicePubkey::from_bytes(skipped_sender.public_key().to_bytes());
    let mut skipped_keys = BTreeMap::new();
    skipped_keys.insert(
        skipped_sender_pubkey,
        nostr_double_ratchet::SkippedKeysEntry::default(),
    );
    let session_state = SessionState {
        root_key: [1; 32],
        their_current_nostr_public_key: Some(current_sender_pubkey),
        their_next_nostr_public_key: Some(next_sender_pubkey),
        our_previous_nostr_key: None,
        our_current_nostr_key: Some(serializable_key_pair_for_test(&our_current)),
        our_next_nostr_key: serializable_key_pair_for_test(&our_next),
        receiving_chain_key: Some([2; 32]),
        sending_chain_key: Some([3; 32]),
        sending_chain_message_number: 0,
        receiving_chain_message_number: 0,
        previous_sending_chain_message_count: 0,
        skipped_keys,
    };
    let seed_session_manager = SessionManagerSnapshot {
        local_owner_pubkey: local_owner,
        local_device_pubkey: local_device,
        local_invite: None,
        users: vec![nostr_double_ratchet::UserRecordSnapshot {
            owner_pubkey: peer_owner_pubkey,
            roster: Some(DeviceRoster::new(
                NdrUnixSeconds(1),
                vec![AuthorizedDevice::new(peer_device_pubkey, NdrUnixSeconds(1))],
            )),
            devices: vec![nostr_double_ratchet::DeviceRecordSnapshot {
                device_pubkey: peer_device_pubkey,
                authorized: true,
                is_stale: false,
                stale_since: None,
                claimed_owner_pubkey: Some(peer_owner_pubkey),
                public_invite: None,
                invite_response_generated: true,
                active_session: Some(session_state),
                inactive_sessions: Vec::new(),
                last_activity: Some(NdrUnixSeconds(1)),
                created_at: NdrUnixSeconds(1),
            }],
        }],
    };
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    seed_protocol_storage_for_test(
        storage.as_ref(),
        seed_session_manager,
        NostrGroupManager::new(local_owner).snapshot(),
    )
    .expect("seed protocol state");
    let engine = ProtocolEngine::load_or_create_for_local_device(
        storage,
        owner.public_key(),
        &device,
    )
    .expect("protocol engine");

    let authors = engine.message_author_pubkeys_for_owner(peer_owner.public_key());
    assert!(
        authors.contains(&current_sender.public_key()),
        "current sender author must stay subscribed"
    );
    assert!(
        authors.contains(&next_sender.public_key()),
        "next sender author must be subscribed so the next ratchet event is not missed"
    );
    assert!(
        authors.contains(&skipped_sender.public_key()),
        "skipped sender author must be backfilled for out-of-order relay delivery"
    );
    assert_eq!(
        engine.known_message_author_cache_build_count_for_test(),
        0
    );
    assert!(engine.is_known_message_author(current_sender.public_key()));
    assert_eq!(
        engine.known_message_author_cache_build_count_for_test(),
        1
    );
    assert!(engine.is_known_message_author(next_sender.public_key()));
    assert!(!engine.is_known_message_author(Keys::generate().public_key()));
    assert_eq!(
        engine.known_message_author_cache_build_count_for_test(),
        1,
        "known author membership should reuse the cached author set"
    );
}

#[test]
fn local_sibling_direct_send_uses_author_known_before_publish() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut primary = test_protocol_engine(&owner, &primary_device);
    let mut linked = test_protocol_engine(&owner, &linked_device);

    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    primary
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
        .expect("primary local appkeys");
    linked
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("linked local appkeys");

    let linked_invite = linked.local_invite().expect("linked invite");
    let linked_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&linked_invite)
        .expect("linked invite event")
        .sign_with_keys(&linked_device)
        .expect("signed linked invite");
    primary
        .observe_invite_event(&linked_invite_event)
        .expect("primary observes linked invite");

    let (session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &response_event,
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");

    let known_authors_before = linked.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        !known_authors_before.is_empty(),
        "linked device must know at least one primary sender author after link setup"
    );

    let result = primary
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "sender copy should be immediately discoverable",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    let local_sibling_events = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                event,
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref()
                == Some(owner.public_key().to_hex().as_str())
                && target_device_id.as_deref()
                    == Some(linked_device.public_key().to_hex().as_str()) =>
            {
                Some(event)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        local_sibling_events.len(),
        1,
        "direct send should prepare one sender-copy event for the linked device"
    );
    assert!(
        known_authors_before.contains(&local_sibling_events[0].pubkey),
        "sender-copy event author {} must already be in the linked device's message subscriptions; known={:?}",
        local_sibling_events[0].pubkey.to_hex(),
        known_authors_before
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>()
    );
    assert!(
        !result.effects.iter().any(|effect| {
            matches!(
                effect,
                ProtocolEffect::PublishSigned(event)
                    if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND
            )
        }),
        "ordinary direct sender-copy fanout must not refresh the linked-device bootstrap session"
    );
}

#[test]
fn remote_group_metadata_syncs_to_local_sibling() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let admin_owner = Keys::generate();
    let admin_device = Keys::generate();
    let mut primary = test_protocol_engine(&owner, &primary_device);
    let mut linked = test_protocol_engine(&owner, &linked_device);

    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    primary
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
        .expect("primary local appkeys");
    linked
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("linked local appkeys");

    let linked_invite = linked.local_invite().expect("linked invite");
    let (primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            primary_session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet_nostr::invite_response_event(&response)
            .expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");
    let mut primary_invite = primary
        .local_invite()
        .expect("primary invite for linked sibling");
    primary_invite.owner_public_key = Some(owner.public_key());
    primary_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let primary_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&primary_invite)
        .expect("primary invite unsigned")
        .sign_with_keys(&primary_device)
        .expect("primary invite event");
    linked
        .observe_invite_event(&primary_invite_event)
        .expect("linked observes primary invite");

    let admin_app_keys = AppKeys::new(vec![DeviceEntry::new(admin_device.public_key(), 1)]);
    primary
        .ingest_app_keys_snapshot(admin_owner.public_key(), admin_app_keys, 1)
        .expect("primary admin appkeys");

    let group_id = "remote-group-local-sibling-sync".to_string();
    let snapshot = test_group_snapshot(
        &group_id,
        "Remote Group",
        admin_owner.public_key(),
        vec![admin_owner.public_key(), owner.public_key()],
        vec![admin_owner.public_key()],
        1,
    );
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(admin_device.public_key()),
            created_at: NdrUnixSeconds(11),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("metadata payload");

    let outcome = primary
        .process_group_pairwise_payload(
            &metadata_payload,
            admin_owner.public_key(),
            Some(admin_device.public_key()),
        )
        .expect("primary processes remote group metadata");
    assert!(
        outcome.events.iter().any(|event| {
            matches!(event, GroupIncomingEvent::MetadataUpdated(group) if group.group_id == group_id)
        }),
        "primary should apply remote group metadata before syncing siblings"
    );

    let target_owner_hex = owner.public_key().to_hex();
    let target_device_hex = linked_device.public_key().to_hex();
    let mut bootstrap_events = Vec::new();
    let mut sibling_payload_events = Vec::new();
    for effect in &outcome.effects {
        match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                event,
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                && target_device_id.as_deref() == Some(target_device_hex.as_str()) =>
            {
                sibling_payload_events.push(event.clone());
            }
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => {
                bootstrap_events.extend(bootstrap.iter().map(|publish| publish.event.clone()));
                sibling_payload_events.extend(
                    payload
                        .iter()
                        .filter(|publish| {
                            publish.target_owner_pubkey_hex.as_deref()
                                == Some(target_owner_hex.as_str())
                                && publish.target_device_id.as_deref()
                                    == Some(target_device_hex.as_str())
                        })
                        .map(|publish| publish.event.clone()),
                );
            }
            _ => {}
        }
    }
    assert!(
        !sibling_payload_events.is_empty(),
        "remote group metadata should be republished to linked local devices"
    );

    for event in &bootstrap_events {
        linked
            .observe_invite_response_event(event)
            .expect("linked processes sibling bootstrap");
    }
    let mut linked_group_events = Vec::new();
    for event in &sibling_payload_events {
        let decrypted = linked
            .process_direct_message_event(event)
            .expect("linked processes sibling group sync")
            .expect("linked decrypts sibling group sync");
        let outcome = linked
            .process_group_pairwise_payload(
                decrypted.content.as_bytes(),
                decrypted.sender,
                decrypted.sender_device,
            )
            .expect("linked applies sibling group payload");
        linked_group_events.extend(outcome.events);
    }
    assert!(
        linked_group_events.iter().any(|event| {
            matches!(event, GroupIncomingEvent::MetadataUpdated(group) if group.group_id == group_id)
        }),
        "linked device should learn the remote-created group from its primary sibling"
    );
}

#[test]
fn local_sibling_group_send_bootstrap_makes_staged_payload_author_fetchable() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let admin_owner = Keys::generate();
    let admin_device = Keys::generate();
    let mut primary = test_protocol_engine(&owner, &primary_device);
    let mut linked = test_protocol_engine(&owner, &linked_device);

    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    primary
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
        .expect("primary local appkeys");
    linked
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("linked local appkeys");

    let linked_invite = linked.local_invite().expect("linked invite");
    let (primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            primary_session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet_nostr::invite_response_event(&response)
            .expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");
    let mut primary_invite = primary
        .local_invite()
        .expect("primary invite for linked sibling");
    primary_invite.owner_public_key = Some(owner.public_key());
    primary_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let primary_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&primary_invite)
        .expect("primary invite unsigned")
        .sign_with_keys(&primary_device)
        .expect("primary invite event");
    linked
        .observe_invite_event(&primary_invite_event)
        .expect("linked observes primary invite");

    let admin_app_keys = AppKeys::new(vec![DeviceEntry::new(admin_device.public_key(), 1)]);
    primary
        .ingest_app_keys_snapshot(admin_owner.public_key(), admin_app_keys.clone(), 1)
        .expect("primary admin appkeys");
    linked
        .ingest_app_keys_snapshot(admin_owner.public_key(), admin_app_keys, 1)
        .expect("linked admin appkeys");

    let group_id = "linked-sibling-group".to_string();
    let mut snapshot = test_group_snapshot(
        &group_id,
        "Linked Sibling Group",
        admin_owner.public_key(),
        vec![admin_owner.public_key(), owner.public_key()],
        vec![admin_owner.public_key()],
        1,
    );
    snapshot.protocol = nostr_double_ratchet::GroupProtocol::PairwiseFanoutV1;
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(admin_device.public_key()),
            created_at: NdrUnixSeconds(11),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("metadata payload");
    primary
        .process_group_pairwise_payload(
            &metadata_payload,
            admin_owner.public_key(),
            Some(admin_device.public_key()),
        )
        .expect("primary processes group metadata");
    let linked_metadata_result = linked
        .process_group_pairwise_payload(
            &metadata_payload,
            admin_owner.public_key(),
            Some(admin_device.public_key()),
        )
        .expect("linked processes group metadata");

    let target_owner_hex = owner.public_key().to_hex();
    let target_device_hex = primary_device.public_key().to_hex();
    let metadata_bootstrap_events = linked_metadata_result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload }
                if payload.iter().any(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                        && publish.target_device_id.as_deref() == Some(target_device_hex.as_str())
                }) =>
            {
                bootstrap
                    .iter()
                    .map(|publish| publish.event.clone())
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    for event in &metadata_bootstrap_events {
        primary
            .observe_invite_response_event(event)
            .expect("primary processes linked metadata bootstrap response");
    }

    let known_primary_authors = primary.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        !known_primary_authors.is_empty(),
        "primary must know linked-device message authors after sibling setup"
    );

    let result = linked
        .send_group_payload(
            &group_id,
            b"linked sibling group body".to_vec(),
            Some("linked-group-inner".to_string()),
        )
        .expect("linked group send");
    let local_sibling_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                event,
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                && target_device_id.as_deref() == Some(target_device_hex.as_str()) =>
            {
                vec![event.clone()]
            }
            ProtocolEffect::PublishStagedFirstContact { payload, .. } => payload
                .iter()
                .filter(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                        && publish.target_device_id.as_deref() == Some(target_device_hex.as_str())
                })
                .map(|publish| publish.event.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    let local_sibling_bootstrap_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload }
                if payload.iter().any(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                        && publish.target_device_id.as_deref() == Some(target_device_hex.as_str())
                }) =>
            {
                bootstrap
                    .iter()
                    .map(|publish| publish.event.clone())
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();

    assert!(
        !local_sibling_events.is_empty(),
        "group send should prepare a local sibling copy for the primary device; queued={:?} pending_group_fanouts={} pending_targets={:?}",
        result.queued_targets,
        linked.debug_snapshot().pending_group_fanout_count,
        linked.debug_snapshot().pending_group_fanout_targets
    );
    assert!(
        !metadata_bootstrap_events.is_empty() || !local_sibling_bootstrap_events.is_empty(),
        "first-contact local sibling group copy should include invite-response bootstrap"
    );
    for event in &local_sibling_bootstrap_events {
        primary
            .observe_invite_response_event(event)
            .expect("primary processes linked bootstrap response");
    }
    let known_primary_authors_after_bootstrap =
        primary.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        local_sibling_events
            .iter()
            .all(|event| known_primary_authors_after_bootstrap.contains(&event.pubkey)),
        "local sibling group event authors must be known after first-contact bootstrap; before={:?} after={:?} event_authors={:?}",
        known_primary_authors
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>(),
        known_primary_authors_after_bootstrap
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>(),
        local_sibling_events
            .iter()
            .map(|event| event.pubkey.to_hex())
            .collect::<Vec<_>>()
    );

    let mut received_messages = Vec::new();
    for event in &local_sibling_events {
        let decrypted = primary
            .process_direct_message_event(event)
            .expect("primary processes linked group copy")
            .expect("primary decrypts linked group copy");
        let outcome = primary
            .process_group_pairwise_payload(
                decrypted.content.as_bytes(),
                decrypted.sender,
                decrypted.sender_device,
            )
            .expect("primary processes group payload from linked copy");
        received_messages.extend(outcome.events.into_iter().filter_map(|event| match event {
            GroupIncomingEvent::Message(message) => Some(message),
            _ => None,
        }));
    }
    assert!(
        received_messages.iter().any(|message| {
            message.group_id == group_id
                && message.sender_owner == ndr_owner_pubkey(owner.public_key())
                && message.sender_device == Some(ndr_device_pubkey(linked_device.public_key()))
                && message.body == b"linked sibling group body".to_vec()
        }),
        "primary should apply linked-device group copy as an owner-authored message"
    );
}

#[test]
fn local_sibling_publish_ack_does_not_mark_peer_recipient_sent() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("local-sibling-ack-direct-delivery", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "direct-first".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "first".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );

    let local_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "local sibling")
        .sign_with_keys(&device)
        .expect("local sibling event");
    let local_event_id = local_event.id.to_string();
    core.pending_relay_publishes.insert(
        local_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: local_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&local_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(owner.public_key().to_hex()),
            target_device_id: Some(sibling.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: local_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );
    core.handle_relay_publish_finished(
        local_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "local sibling ack".to_string(),
    );

    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after local ack");
    assert_eq!(message.delivery, DeliveryState::Pending);
    assert_eq!(message.recipient_deliveries.len(), 1);
    assert_eq!(
        message.recipient_deliveries[0].owner_pubkey_hex,
        peer.public_key().to_hex()
    );
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Pending
    );

    let lingering_local_event =
        EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "local still pending")
            .sign_with_keys(&device)
            .expect("lingering local event");
    let lingering_local_event_id = lingering_local_event.id.to_string();
    core.pending_relay_publishes.insert(
        lingering_local_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: lingering_local_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&lingering_local_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(owner.public_key().to_hex()),
            target_device_id: Some(sibling.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: lingering_local_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let peer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "peer")
        .sign_with_keys(&device)
        .expect("peer event");
    let peer_event_id = peer_event.id.to_string();
    core.pending_relay_publishes.insert(
        peer_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: peer_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&peer_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: Some(peer.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: peer_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );
    core.handle_relay_publish_finished(
        peer_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "peer ack".to_string(),
    );

    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after peer ack");
    assert!(
        core.pending_relay_publishes
            .contains_key(&lingering_local_event_id),
        "local sibling pending relay work should not decide peer recipient delivery"
    );
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Sent
    );
}

#[test]
fn first_contact_payload_waits_for_bootstrap_publish_success() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("first-contact-bootstrap-gates-payload", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "direct-first-contact".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "first".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );

    let bootstrap_event = EventBuilder::new(
        Kind::from(INVITE_RESPONSE_KIND as u16),
        "first contact bootstrap",
    )
    .sign_with_keys(&device)
    .expect("bootstrap event");
    let bootstrap_event_id = bootstrap_event.id.to_string();
    core.pending_relay_publishes.insert(
        bootstrap_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: bootstrap_event_id.clone(),
            label: APPCORE_PROTOCOL_BOOTSTRAP_LABEL.to_string(),
            event_json: serde_json::to_string(&bootstrap_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: None,
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: bootstrap_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let payload_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "payload")
        .sign_with_keys(&device)
        .expect("payload event");
    let payload_event_id = payload_event.id.to_string();
    core.pending_relay_publishes.insert(
        payload_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: payload_event_id.clone(),
            label: APPCORE_PROTOCOL_FIRST_CONTACT_LABEL.to_string(),
            event_json: serde_json::to_string(&payload_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: Some(peer.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: payload_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let payload_pending = core
        .pending_relay_publishes
        .get(&payload_event_id)
        .expect("payload pending");
    assert!(
        core.should_delay_first_contact_payload_publish(payload_pending),
        "payload must not publish while its invite-response bootstrap is still pending"
    );

    core.handle_relay_publish_finished(
        bootstrap_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "bootstrap ack".to_string(),
    );
    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after bootstrap ack");
    assert_eq!(
        message.delivery,
        DeliveryState::Pending,
        "bootstrap relay ack must not mark the peer message sent"
    );
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Pending
    );
    let payload_pending = core
        .pending_relay_publishes
        .get(&payload_event_id)
        .expect("payload still pending after bootstrap");
    assert!(
        !core.should_delay_first_contact_payload_publish(payload_pending),
        "payload may publish after bootstrap succeeds"
    );

    core.handle_relay_publish_finished(
        payload_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "payload ack".to_string(),
    );
    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after payload ack");
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Sent
    );
}

#[test]
fn appcore_hot_path_has_no_runtime_references() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![manifest.join("src/core.rs")];
    collect_core_rs_files(&manifest.join("src/core"), &mut files);

    let forbidden = [
        "NdrRuntime",
        "ndr_runtime",
        "setup_user",
        "process_runtime_effects",
        "RuntimeEffect",
    ];
    let mut hits = Vec::new();
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) == Some("tests.rs")
            || path.components().any(|component| {
                component.as_os_str().to_str() == Some("tests")
            })
        {
            continue;
        }
        let content = std::fs::read_to_string(&path).expect("read core source");
        for needle in forbidden {
            if content.contains(needle) {
                hits.push(format!("{} contains {needle}", path.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "AppCore hot-path runtime references remain:\n{}",
        hits.join("\n")
    );
}

fn collect_core_rs_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read core dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_core_rs_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

#[test]
fn ndr_runtime_invite_session_round_trips_text() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        None,
        None,
    );
    bob.init().expect("bob init");
    accept_invite_and_deliver(&bob, &bob_keys, &invite, alice_keys.public_key(), &alice);
    complete_first_contact(&bob, &bob_keys, alice_keys.public_key(), &alice);

    alice
        .send_text(bob_keys.public_key(), "hello bob".to_string(), None)
        .expect("alice sends");
    deliver_published_events(&alice, &alice_keys, &bob);
    assert!(drain_text_messages(&bob)
        .iter()
        .any(|message| message == "hello bob"));

    bob.send_text(alice_keys.public_key(), "hello alice".to_string(), None)
        .expect("bob sends");
    deliver_published_events(&bob, &bob_keys, &alice);
    assert!(drain_text_messages(&alice)
        .iter()
        .any(|message| message == "hello alice"));
}

#[test]
fn local_identity_artifacts_offer_profile_metadata_to_nearby() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.owner_profiles.insert(
        owner.public_key().to_hex(),
        OwnerProfileRecord {
            nickname: None,
            name: None,
            display_name: None,
            picture: Some("htree://profile-picture".to_string()),
            about: Some("Building with friends.\nhttps://iris.to".to_string()),
            updated_at_secs: 1,
            ..OwnerProfileRecord::default()
        },
    );

    core.publish_local_identity_artifacts();

    let nearby_events = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind, event_json, ..
            } => Some((kind, event_json)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let nearby_kinds = nearby_events
        .iter()
        .map(|(kind, _)| *kind)
        .collect::<Vec<_>>();
    assert!(
        nearby_kinds.contains(&0),
        "profile metadata should be included in nearby inventory; got {nearby_kinds:?}"
    );
    let profile_event = nearby_events
        .iter()
        .find_map(|(kind, event_json)| (*kind == 0).then_some(event_json))
        .expect("profile event");
    let event_value = serde_json::from_str::<serde_json::Value>(profile_event).expect("event json");
    let content = event_value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .expect("profile content");
    let metadata = serde_json::from_str::<serde_json::Value>(content).expect("metadata json");
    assert_eq!(
        metadata.get("about").and_then(serde_json::Value::as_str),
        Some("Building with friends.\nhttps://iris.to")
    );
}

#[test]
fn editing_profile_preserves_extra_metadata_fields_and_tags() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let owner_hex = owner.public_key().to_hex();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    // Simulate having received a kind:0 from another client that carried
    // fields and tags we don't model directly. A naive save would blank
    // these out on republish.
    let remote_metadata = serde_json::json!({
        "name": "Old Name",
        "display_name": "Old Display",
        "picture": "https://example.com/old.png",
        "about": "Old about",
        "nip05": "alice@example.com",
        "lud16": "alice@walletofsatoshi.com",
        "website": "https://alice.example",
    });
    let remote_event = EventBuilder::new(Kind::Metadata, remote_metadata.to_string())
        .tag(nostr::Tag::parse(["alt", "Custom alt tag"]).expect("alt tag"))
        .tag(nostr::Tag::parse(["i", "github:alice", "proof123"]).expect("i tag"))
        .sign_with_keys(&owner)
        .expect("sign remote metadata");
    assert!(core.apply_profile_metadata_event(&remote_event));
    let _ = update_rx.try_iter().collect::<Vec<_>>();

    core.handle_action(AppAction::UpdateProfileMetadata {
        name: "New Name".to_string(),
        picture_url: Some("https://example.com/new.png".to_string()),
        about: Some("New about".to_string()),
    });

    let profile_event_json = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind: 0,
                event_json,
                ..
            } => Some(event_json),
            _ => None,
        })
        .last()
        .expect("profile event published");
    let profile_event: Event =
        serde_json::from_str(&profile_event_json).expect("parse profile event");
    assert_eq!(profile_event.pubkey.to_hex(), owner_hex);

    let content: serde_json::Value =
        serde_json::from_str(&profile_event.content).expect("metadata content");
    assert_eq!(
        content.get("name").and_then(|v| v.as_str()),
        Some("New Name")
    );
    assert_eq!(
        content.get("picture").and_then(|v| v.as_str()),
        Some("https://example.com/new.png")
    );
    assert_eq!(
        content.get("about").and_then(|v| v.as_str()),
        Some("New about")
    );
    assert_eq!(
        content.get("nip05").and_then(|v| v.as_str()),
        Some("alice@example.com"),
        "nip05 from prior event must survive a profile edit"
    );
    assert_eq!(
        content.get("lud16").and_then(|v| v.as_str()),
        Some("alice@walletofsatoshi.com"),
        "lud16 from prior event must survive a profile edit"
    );
    assert_eq!(
        content.get("website").and_then(|v| v.as_str()),
        Some("https://alice.example"),
        "website from prior event must survive a profile edit"
    );

    let tags: Vec<Vec<String>> = profile_event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    assert!(
        tags.iter().any(|tag| tag.as_slice()
            == ["alt".to_string(), "Custom alt tag".to_string()].as_slice()),
        "expected `alt` tag preserved, got tags={tags:?}"
    );
    assert!(
        tags.iter().any(|tag| tag.as_slice()
            == [
                "i".to_string(),
                "github:alice".to_string(),
                "proof123".to_string()
            ]
            .as_slice()),
        "expected `i` identity tag preserved, got tags={tags:?}"
    );
}

#[test]
fn create_account_without_name_still_offers_profile_metadata_to_nearby() {
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.create_account("");

    let account = core.state.account.as_ref().expect("account");
    let profile = core
        .owner_profiles
        .get(&account.public_key_hex)
        .expect("local profile record");
    assert_eq!(
        profile.display_name.as_deref(),
        Some(account.display_name.as_str())
    );

    let profile_event_json = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind: 0,
                event_json,
                ..
            } => Some(event_json),
            _ => None,
        })
        .last()
        .expect("profile event");
    let profile_event: Event = serde_json::from_str(&profile_event_json).expect("profile event");
    assert_eq!(profile_event.pubkey.to_hex(), account.public_key_hex);
}

#[test]
fn delete_profile_metadata_publishes_blank_profile_and_clears_local_record() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let owner_hex = owner.public_key().to_hex();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.owner_profiles.insert(
        owner_hex.clone(),
        OwnerProfileRecord {
            nickname: None,
            name: Some("Alice".to_string()),
            display_name: Some("Alice".to_string()),
            picture: Some("https://example.com/alice.jpg".to_string()),
            about: None,
            updated_at_secs: 1,
            ..OwnerProfileRecord::default()
        },
    );

    core.handle_action(AppAction::DeleteProfileMetadata);

    assert!(!core.owner_profiles.contains_key(&owner_hex));
    let profile_event_json = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind: 0,
                event_json,
                ..
            } => Some(event_json),
            _ => None,
        })
        .last()
        .expect("profile deletion event");
    let profile_event: Event =
        serde_json::from_str(&profile_event_json).expect("profile deletion event");
    assert_eq!(profile_event.pubkey.to_hex(), owner_hex);
    assert_eq!(profile_event.content, "{}");
}

#[test]
fn nearby_presence_event_binds_owner_to_ble_nonce_pair() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });

    let event_json = core.build_nearby_presence_event_json(
        "peer-a",
        "nonce-a",
        "nonce-b",
        "f".repeat(64).as_str(),
    );
    let event: Event = serde_json::from_str(&event_json).expect("presence event");
    assert_eq!(event.kind.as_u16(), NEARBY_PRESENCE_KIND);
    event.verify().expect("valid signature");
    assert_eq!(event.pubkey, owner.public_key());

    let content: serde_json::Value = serde_json::from_str(&event.content).expect("content");
    assert_eq!(content["protocol"], "iris-nearby-v1");
    assert_eq!(content["peer_id"], "peer-a");
    assert_eq!(content["my_nonce"], "nonce-a");
    assert_eq!(content["their_nonce"], "nonce-b");
    assert_eq!(content["profile_event_id"], "f".repeat(64));

    let verified =
        crate::verify_nearby_presence_event_json(&event_json, "peer-a", "nonce-b", "nonce-a");
    let verified: serde_json::Value = serde_json::from_str(&verified).expect("verified presence");
    assert_eq!(verified["owner_pubkey_hex"], owner.public_key().to_hex());
    assert_eq!(verified["profile_event_id"], "f".repeat(64));

    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "peer-a",
        "wrong-receiver-nonce",
        "nonce-a",
    )
    .is_empty());
    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "wrong-peer",
        "nonce-b",
        "nonce-a",
    )
    .is_empty());
}

#[test]
fn mobile_push_decrypt_preview_does_not_mutate_persisted_ratchet_state() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_engine =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_storage.clone());
    let message = "closed-app preview stays read-only";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    let state_key = "appcore/protocol-engine-state-v1";
    let before = bob_storage
        .get(state_key)
        .expect("read stored appcore state before notification")
        .expect("stored appcore state before notification");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&message_event).expect("outer event json"),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();
    let apns_payload = serde_json::json!({
        "event": message_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, message);
    let apns_resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        apns_payload,
    );
    assert!(apns_resolution.should_show);
    assert_eq!(apns_resolution.body, message);

    let after = bob_storage
        .get(state_key)
        .expect("read stored appcore state after notification")
        .expect("stored appcore state after notification");
    assert_eq!(
        before, after,
        "notification preview must not advance persisted protocol state"
    );

    let bob_restarted_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("restarted db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_restarted =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_restarted_storage);
    let decrypted = bob_restarted
        .process_direct_message_event(&message_event)
        .expect("foreground protocol decrypt")
        .expect("foreground decrypted message");
    let runtime_rumor = parse_runtime_rumor(&decrypted.content).expect("runtime rumor");
    assert_eq!(runtime_rumor.content, message);
}

#[test]
fn mobile_push_decrypts_compacted_apns_event_payload() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_engine = test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_storage);
    let message = "compacted apns preview";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    for (key, event_payload) in [
        ("event", compact_event_payload_for_apns_test(&message_event)),
        (
            "outer_event",
            compact_event_payload_for_apns_test(&message_event),
        ),
        (
            "outer_event_json",
            serde_json::Value::String(
                serde_json::to_string(&message_event).expect("outer event json"),
            ),
        ),
        (
            "nostr_event_json",
            serde_json::Value::String(
                serde_json::to_string(&message_event).expect("nostr event json"),
            ),
        ),
    ] {
        let mut payload = serde_json::json!({
            "aps": {
                "alert": {
                    "title": "Iris Chat",
                    "body": "New message",
                },
                "mutable-content": 1,
            },
            "title": "New message",
            "body": "New message",
        });
        payload[key] = event_payload;

        let resolution = decrypt_mobile_push_notification(
            data_dir.to_string_lossy().to_string(),
            bob_keys.public_key().to_hex(),
            bob_keys
                .secret_key()
                .to_bech32()
                .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
            payload.to_string(),
        );

        assert!(
            resolution.should_show,
            "{key} payload should decrypt to a visible message"
        );
        assert_eq!(resolution.body, message, "{key} payload body");
    }
}

#[test]
fn mobile_push_payload_ingest_feeds_full_event_into_runtime() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let bob_storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut bob_engine =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, Arc::clone(&bob_storage));
    let message = "push-only event";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    let payload = serde_json::json!({
        "event": message_event.clone(),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload.clone(),
    });
    let chat_id = alice_keys.public_key().to_hex();
    assert!(
        !core.threads.contains_key(&chat_id),
        "push event waits for session restore"
    );

    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    install_test_protocol_engine(&mut core, &bob_keys, &bob_keys, bob_storage, None, None);

    core.drain_pending_mobile_push_events();
    let thread = core.threads.get(&chat_id).expect("sender thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, message);
    let event_id = message_event.id.to_string();
    assert_eq!(
        thread.messages[0].source_event_id.as_deref(),
        Some(event_id.as_str())
    );

    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload,
    });
    let thread = core.threads.get(&chat_id).expect("sender thread after dup");
    assert_eq!(thread.messages.len(), 1, "duplicate push event is ignored");
}

#[test]
fn mobile_push_decrypt_suppresses_typing_rumors() {
    // Even with valid keys and a working ratchet, the notification
    // extension should suppress non-chat-message rumors. Typing/seen/
    // reaction wrappers are noise as standalone notifications — the
    // chat list updates when the user opens the app.
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_storage),
        None,
    );
    bob.init().expect("bob init");
    accept_invite_and_deliver(&bob, &bob_keys, &invite, alice_keys.public_key(), &alice);
    complete_first_contact(&bob, &bob_keys, alice_keys.public_key(), &alice);

    alice
        .send_typing(bob_keys.public_key(), None)
        .expect("alice sends typing");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let typing_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("typing event for Bob");
    let payload = serde_json::json!({
        "event": typing_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );

    assert!(
        !resolution.should_show,
        "typing rumors must not surface as standalone notifications"
    );
}

#[test]
fn mobile_push_fallback_suppresses_opaque_encrypted_events() {
    let encrypted_outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&Keys::generate())
        .expect("outer event");
    let payload = serde_json::json!({
        "event": encrypted_outer_event,
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert!(resolution.title.is_empty());
    assert!(resolution.body.is_empty());
}

#[test]
fn mobile_push_fallback_suppresses_opaque_encrypted_alias_events_with_string_kind() {
    let encrypted_outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&Keys::generate())
        .expect("outer event");
    let mut event_json =
        serde_json::to_value(&encrypted_outer_event).expect("outer event json value");
    event_json["kind"] = serde_json::Value::String(MESSAGE_EVENT_KIND.to_string());
    let payload = serde_json::json!({
        "outer_event_json": event_json.to_string(),
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert!(resolution.title.is_empty());
    assert!(resolution.body.is_empty());
}

#[test]
fn mobile_push_preview_resolves_from_sqlite_when_decrypt_fails() {
    // Simulates the "main app already consumed this event, ratchet has
    // moved on, push extension can't decrypt anymore" race: the
    // foreground app stored a message keyed by the outer event id; the
    // extension can't decrypt the same event, so it falls back to the
    // body/sender already on disk.
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    // Pretend Alice's profile is already known.
    let alice = Keys::generate();
    core.owner_profiles.insert(
        alice.public_key().to_hex(),
        OwnerProfileRecord {
            nickname: None,
            name: Some("alice".to_string()),
            display_name: Some("Alice from work".to_string()),
            picture: None,
            about: None,
            updated_at_secs: 1,
            ..OwnerProfileRecord::default()
        },
    );
    let outer_event_id = "a".repeat(64);
    let chat_id = alice.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: 200,
            messages: vec![ChatMessageSnapshot {
                id: "rumor-1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: alice.public_key().to_hex(),
                author_owner_pubkey_hex: Some(alice.public_key().to_hex()),
                author_picture_url: None,
                body: "lunch?".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 199,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: Some(outer_event_id.clone()),
            }],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();

    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&alice)
        .expect("outer event");
    let payload = serde_json::json!({
        "event": serde_json::json!({
            "id": outer_event_id,
            "kind": MESSAGE_EVENT_KIND,
            "content": "",
            "tags": [],
            "pubkey": alice.public_key().to_hex(),
            "created_at": 200,
            "sig": outer_event.sig.to_string(),
        }),
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    // Use bogus device keys so NDR decrypt fails immediately and we
    // exercise the SQLite-backed preview path.
    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        owner.public_key().to_hex(),
        "nsec1invalid".to_string(),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, "lunch?");
    assert_eq!(resolution.title, "Alice from work");
}

#[test]
fn appcore_persists_pending_group_sender_key_outer_when_no_group_message_emits() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_event = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .expect("primary session");
    let outer = unknown_group_sender_key_outer_event(&sender_event);
    let event_id = outer.id.to_string();

    core.handle_relay_event(outer);

    assert!(
        core.seen_event_ids.contains(&event_id),
        "unknown group sender-key outer should be consumed by group runtime instead of falling through to pairwise decrypt"
    );
    assert_eq!(
        stored_pending_group_sender_key_message_count(&core, &owner, &device),
        1,
        "group runtime pending outer must be durably stored even when no app-visible group message is emitted yet"
    );
}

#[test]
fn appcore_defers_decrypted_delivery_ack_until_app_state_is_persisted() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, &bob_keys, &bob_keys, storage, None, None);

    let message = "ack after app persist";
    let message_event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &alice_keys,
        message,
        200,
    );

    core.enter_batch();
    core.handle_relay_event(message_event);

    assert!(
        core.threads
            .get(&alice_keys.public_key().to_hex())
            .is_some_and(|thread| thread
                .messages
                .iter()
                .any(|message| message.body == "ack after app persist")),
        "decrypted message should be applied in memory immediately"
    );
    assert_eq!(
        stored_message_count(&core),
        1,
        "notification-preview durability may write the message row immediately, but full app-state persistence is still batch-deferred"
    );
    // The engine's protocol-state persist is batch-deferred (catch-up bursts
    // would otherwise stack N serialize+write rounds on the SQLite connection
    // mutex and freeze UI reads on iOS). The contract this test guards is the
    // *in-memory* invariant: the runtime decrypted delivery must remain
    // pending until app state is durably saved, so the ack can't race ahead
    // of the message-row durability.
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .pending_decrypted_deliveries_len_for_test(),
        1,
        "runtime decrypted delivery must remain pending until app state is durably saved"
    );

    core.exit_batch();

    assert_eq!(stored_message_count(&core), 1);
    assert_eq!(
        stored_pending_decrypted_delivery_count(&core, &bob_keys, &bob_keys),
        0,
        "persisting app state should ack and clear the runtime decrypted delivery"
    );
}

#[test]
fn mobile_push_fallback_suppresses_decrypted_non_message_kinds() {
    for kind in [
        0_u64,
        TYPING_KIND as u64,
        RECEIPT_KIND as u64,
        40_u64,
        CHAT_SETTINGS_KIND as u64,
        12345,
    ] {
        let inner_event = serde_json::json!({
            "kind": kind,
            "content": "not a chat message",
            "created_at": 1_777_159_483u64,
            "tags": [],
            "pubkey": "0".repeat(64),
            "id": format!("{kind:064x}"),
        });
        let payload = serde_json::json!({
            "inner_event_json": inner_event.to_string(),
            "title": "Iris Chat",
            "body": "New message",
        })
        .to_string();

        let resolution = resolve_mobile_push_notification(payload);

        assert!(
            !resolution.should_show,
            "inner kind {kind} should not produce a visible mobile push"
        );
    }
}

#[test]
fn mobile_push_fallback_allows_chat_message_kind() {
    let payload = serde_json::json!({
        "inner_kind": CHAT_MESSAGE_KIND.to_string(),
        "title": "Alice",
        "body": "hello",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Alice");
    assert_eq!(resolution.body, "hello");
}

#[test]
fn mobile_push_fallback_suppresses_unverified_invite_acceptance() {
    let keys = Keys::generate();
    let p_tag_pubkey = "a".repeat(64);
    let event = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "ciphertext")
        .tag(nostr::Tag::parse(["p", p_tag_pubkey.as_str()]).expect("p tag"))
        .sign_with_keys(&keys)
        .expect("invite response event");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&event).expect("event json"),
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert_eq!(resolution.title, "");
    assert_eq!(resolution.body, "");
}

#[test]
fn mobile_push_decrypt_renders_matching_pending_invite_response_with_chat_id() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-invite-response-match", &owner, &device);
    core.handle_action(AppAction::CreatePublicInvite);
    let invite = core
        .private_chat_invites
        .values()
        .next()
        .expect("private invite")
        .clone();
    let (_session, response) = invite
        .accept_with_owner(
            peer.public_key(),
            peer.secret_key().to_secret_bytes(),
            Some(peer.public_key().to_hex()),
            Some(peer.public_key()),
        )
        .expect("accept invite");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&response_event).expect("event json"),
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        core.data_dir.to_string_lossy().to_string(),
        owner.public_key().to_hex(),
        device
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| device.secret_key().to_secret_hex()),
        payload,
    );

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Invite accepted");
    assert_eq!(resolution.body, "Someone joined your chat");
    let resolved: serde_json::Value =
        serde_json::from_str(&resolution.payload_json).expect("payload json");
    let peer_chat_id = peer.public_key().to_hex();
    assert_eq!(
        resolved.get("chat_id").and_then(|value| value.as_str()),
        Some(peer_chat_id.as_str())
    );
    assert_eq!(
        resolved
            .get("inner_kind")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<u64>().ok()),
        Some(INVITE_RESPONSE_KIND as u64)
    );
}

#[test]
fn mobile_push_decrypt_suppresses_unmatched_invite_response() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let core = logged_in_test_core("mobile-push-invite-response-miss", &owner, &device);
    let missing_invite =
        Invite::create_new(device.public_key(), Some(device.public_key().to_hex()), Some(1))
            .expect("missing invite");
    let (_session, response) = missing_invite
        .accept_with_owner(
            peer.public_key(),
            peer.secret_key().to_secret_bytes(),
            Some(peer.public_key().to_hex()),
            Some(peer.public_key()),
        )
        .expect("accept invite");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&response_event).expect("event json"),
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        core.data_dir.to_string_lossy().to_string(),
        owner.public_key().to_hex(),
        device
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| device.secret_key().to_secret_hex()),
        payload,
    );

    assert!(!resolution.should_show);
    assert_eq!(resolution.title, "");
    assert_eq!(resolution.body, "");
}

#[test]
fn mobile_push_subscription_body_includes_invite_response_filter() {
    let owner = Keys::generate();
    let author = Keys::generate().public_key().to_hex();
    let invite_response_pubkey = Keys::generate().public_key().to_hex();
    let request = build_mobile_push_create_subscription_request(
        owner
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| owner.secret_key().to_secret_hex()),
        "ios".to_string(),
        "apns-token".to_string(),
        Some("to.iris.chat".to_string()),
        vec![author.clone()],
        vec![invite_response_pubkey.clone()],
        true,
        None,
    )
    .expect("subscription request");
    let body: serde_json::Value =
        serde_json::from_str(request.body_json.as_deref().expect("body json")).expect("json");

    assert_eq!(body["filter"]["authors"][0].as_str(), Some(author.as_str()));
    assert_eq!(
        body["filters"][1]["kinds"][0],
        serde_json::json!(INVITE_RESPONSE_KIND)
    );
    assert_eq!(
        body["filters"][1]["#p"][0].as_str(),
        Some(invite_response_pubkey.as_str())
    );
}

#[test]
fn app_invite_consumers_use_protocol_owned_local_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (core, login_invite, protocol_invite) =
        core_with_divergent_login_and_protocol_invites(&owner, &device);

    let login_pubkey = login_invite.inviter_ephemeral_public_key.to_hex();
    let protocol_pubkey = protocol_invite.inviter_ephemeral_public_key.to_hex();
    assert_ne!(login_pubkey, protocol_pubkey);

    let public_invite = core
        .build_public_invite_snapshot()
        .and_then(|snapshot| super::invites::parse_public_invite_input(&snapshot.url).ok())
        .expect("public invite fallback");
    assert_eq!(
        public_invite.inviter_ephemeral_public_key,
        protocol_invite.inviter_ephemeral_public_key
    );

    let push = core.build_mobile_push_sync_snapshot();
    assert_eq!(push.invite_response_pubkeys, vec![protocol_pubkey.clone()]);

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        filters.iter().any(|filter| {
            serde_json::to_value(filter)
                .ok()
                .and_then(|value| value.get("#p").cloned())
                .and_then(|pubkeys| pubkeys.as_array().cloned())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|pubkey| pubkey.as_str() == Some(protocol_pubkey.as_str()))
                })
        }),
        "protocol backfill filters should use protocol-owned local invite pubkey"
    );
}

#[test]
fn local_identity_publish_uses_protocol_owned_local_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let (mut core, login_invite, protocol_invite) =
        core_with_divergent_login_and_protocol_invites_with_updates(&owner, &device, update_tx);

    let login_pubkey = login_invite.inviter_ephemeral_public_key.to_hex();
    let protocol_pubkey = protocol_invite.inviter_ephemeral_public_key.to_hex();
    assert_ne!(login_pubkey, protocol_pubkey);

    core.publish_local_identity_artifacts();

    let published_invite = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind, event_json, ..
            } if kind == INVITE_EVENT_KIND => serde_json::from_str::<Event>(&event_json).ok(),
            _ => None,
        })
        .filter_map(|event| parse_invite_event(&event).ok())
        .next()
        .expect("published invite event");

    assert_eq!(
        published_invite.inviter_ephemeral_public_key.to_hex(),
        protocol_pubkey
    );
    assert_ne!(
        published_invite.inviter_ephemeral_public_key.to_hex(),
        login_pubkey
    );
}

#[test]
fn mobile_push_snapshot_tracks_local_invite_when_enabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("mobile-push-invite-response", &owner, &device);

    let snapshot = core.build_mobile_push_sync_snapshot();

    let invite_pubkey = core
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("local invite")
        .inviter_ephemeral_public_key
        .to_hex();
    assert_eq!(snapshot.invite_response_pubkeys, vec![invite_pubkey]);
}

#[test]
fn mobile_push_snapshot_tracks_private_invite_when_enabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-private-invite-response", &owner, &device);
    core.handle_action(AppAction::CreatePublicInvite);

    let snapshot = core.build_mobile_push_sync_snapshot();

    let local_invite_pubkey = core
        .protocol_engine
        .as_ref()
        .and_then(ProtocolEngine::local_invite)
        .expect("local invite")
        .inviter_ephemeral_public_key
        .to_string();
    let private_invite_pubkey = core
        .private_chat_invites
        .values()
        .next()
        .expect("private invite")
        .inviter_ephemeral_public_key
        .to_string();

    assert!(snapshot
        .invite_response_pubkeys
        .contains(&local_invite_pubkey));
    assert!(snapshot
        .invite_response_pubkeys
        .contains(&private_invite_pubkey));
}

#[test]
fn mobile_push_snapshot_tracks_group_sender_key_authors() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-group-sender-key", &owner, &device);

    core.create_group("Push group", &[]);

    let group_authors = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .known_group_sender_event_pubkeys()
        .into_iter()
        .map(|pubkey| pubkey.to_hex())
        .collect::<Vec<_>>();
    assert!(
        !group_authors.is_empty(),
        "group creation should create a sender-key event author"
    );

    let snapshot = core.build_mobile_push_sync_snapshot();

    for author in group_authors {
        assert!(
            snapshot.message_author_pubkeys.contains(&author),
            "push subscriptions must include group sender-key author {author}; snapshot={:?}",
            snapshot.message_author_pubkeys
        );
    }
}

#[test]
fn mobile_push_snapshot_omits_local_invite_when_disabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-invite-response-disabled", &owner, &device);
    core.set_invite_acceptance_notifications_enabled(false);

    let snapshot = core.build_mobile_push_sync_snapshot();

    assert!(snapshot.invite_response_pubkeys.is_empty());
}

fn core_with_divergent_login_and_protocol_invites(
    owner: &Keys,
    device: &Keys,
) -> (AppCore, Invite, Invite) {
    let (update_tx, _update_rx) = flume::unbounded();
    core_with_divergent_login_and_protocol_invites_with_updates(owner, device, update_tx)
}

fn core_with_divergent_login_and_protocol_invites_with_updates(
    owner: &Keys,
    device: &Keys,
    update_tx: Sender<AppUpdate>,
) -> (AppCore, Invite, Invite) {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();

    let device_id = device.public_key().to_hex();
    let mut login_invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None)
            .expect("login invite");
    login_invite.owner_public_key = Some(owner.public_key());
    login_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));

    let mut protocol_invite = Invite::create_new(device.public_key(), Some(device_id), None)
        .expect("protocol invite");
    protocol_invite.owner_public_key = Some(owner.public_key());
    protocol_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));

    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });

    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let local_owner = ndr_owner_pubkey(owner.public_key());
    let mut seed_session_manager =
        SessionManager::new(local_owner, device.secret_key().to_secret_bytes()).snapshot();
    seed_session_manager.local_invite = Some(protocol_invite.clone());
    let seed_group_manager = NostrGroupManager::new(local_owner).snapshot();
    seed_protocol_storage_for_test(
        storage.as_ref(),
        seed_session_manager,
        seed_group_manager,
    )
    .expect("seed protocol state");
    core.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(
            storage,
            owner.public_key(),
            device,
        )
        .expect("protocol engine"),
    );

    (core, login_invite, protocol_invite)
}

#[test]
fn typing_indicators_default_to_enabled() {
    assert!(PersistedPreferences::default().send_typing_indicators);
    assert!(AppState::empty().preferences.send_typing_indicators);
    let decoded: PersistedPreferences = serde_json::from_str("{}").expect("decode preferences");
    assert!(decoded.send_typing_indicators);
}

#[test]
fn startup_at_login_defaults_to_enabled() {
    assert!(PersistedPreferences::default().startup_at_login_enabled);
    assert!(AppState::empty().preferences.startup_at_login_enabled);
}

#[test]
fn nearby_bluetooth_defaults_to_disabled() {
    assert!(!PersistedPreferences::default().nearby_bluetooth_enabled);
    assert!(!AppState::empty().preferences.nearby_bluetooth_enabled);
}

#[test]
fn nearby_lan_defaults_to_disabled() {
    assert!(!PersistedPreferences::default().nearby_lan_enabled);
    assert!(!AppState::empty().preferences.nearby_lan_enabled);
}

#[test]
fn nearby_master_toggle_preserves_transport_preferences() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("nearby-master-preserves-transports", &owner, &device);

    core.handle_action(AppAction::SetNearbyBluetoothEnabled { enabled: true });
    core.handle_action(AppAction::SetNearbyLanEnabled { enabled: true });
    core.handle_action(AppAction::SetNearbyEnabled { enabled: false });

    assert!(!core.state.preferences.nearby_enabled);
    assert!(core.state.preferences.nearby_bluetooth_enabled);
    assert!(core.state.preferences.nearby_lan_enabled);

    core.handle_action(AppAction::SetNearbyEnabled { enabled: true });

    assert!(core.state.preferences.nearby_enabled);
    assert!(core.state.preferences.nearby_bluetooth_enabled);
    assert!(core.state.preferences.nearby_lan_enabled);
}

#[test]
fn unknown_direct_messages_default_to_allowed() {
    assert!(PersistedPreferences::default().accept_unknown_direct_messages);
    assert!(AppState::empty().preferences.accept_unknown_direct_messages);
}
