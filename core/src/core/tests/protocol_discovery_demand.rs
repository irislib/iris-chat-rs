fn cache_discovery_roster(core: &mut AppCore, owner: &Keys, devices: &[PublicKey], at: u64) {
    let roster = AppKeys::new(
        devices
            .iter()
            .map(|key| DeviceEntry::new(*key, at))
            .collect(),
    );
    let event = roster
        .get_event_at(owner.public_key(), at)
        .sign_with_keys(owner)
        .unwrap();
    core.protocol_engine
        .as_mut()
        .unwrap()
        .ingest_app_keys_event(&event)
        .unwrap();
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(owner.public_key(), &roster, at),
    );
}

#[test]
fn discovery_demand_excludes_cache_only_peers_and_activates_isolated_wire_on_open() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("discovery-demand-wire", &owner, &device);
    let (sender, _receiver) = flume::unbounded();
    core.core_sender = sender;
    cache_discovery_roster(&mut core, &peer, &[peer_device.public_key()], 1);
    let cached = (0..32)
        .map(|_| (Keys::generate(), Keys::generate()))
        .collect::<Vec<_>>();
    for (cached_owner, cached_device) in &cached {
        cache_discovery_roster(&mut core, cached_owner, &[cached_device.public_key()], 1);
    }
    core.ensure_thread_record(&peer.public_key().to_hex(), 1);
    let saved = serde_json::to_value(&core.app_keys).unwrap();
    let (relay, wire) = core.runtime.block_on(publish_helpers::authenticated_relay(
        device.public_key(),
        false,
    ));
    let mut expected = vec![peer.public_key(), peer_device.public_key()];
    for opened in [false, true] {
        if opened {
            core.active_chat_id = Some(cached[0].0.public_key().to_hex());
            expected.extend([cached[0].0.public_key(), cached[0].1.public_key()]);
        }
        let plan = core.compute_protocol_subscription_plan().unwrap();
        let (receive, discovery) =
            core.split_private_discovery_filters(build_protocol_subscription_filters(&plan));
        let receive = serde_json::to_string(&receive).unwrap();
        assert!(expected.iter().all(|key| !receive.contains(&key.to_hex())));
        let scopes = core.private_discovery_subscriptions(discovery).unwrap();
        assert_eq!(
            scopes.len(),
            expected.len(),
            "cache-only metadata must not create workers"
        );
        core.runtime
            .block_on(
                protocol::private_reads::apply_private_discovery_subscriptions(
                    scopes,
                    std::slice::from_ref(&relay),
                ),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let rows = wire.lock().unwrap();
            let ready = rows.iter().filter(|(_, value)| value[0] == "AUTH").count()
                >= expected.len()
                && expected.iter().all(|key| {
                    rows.iter().any(|(_, value)| {
                        value[0] == "REQ" && value.to_string().contains(&key.to_hex())
                    })
                });
            drop(rows);
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "demanded author did not reach the relay"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let rows = wire.lock().unwrap().clone();
        let mut subjects = std::collections::HashMap::<usize, HashSet<String>>::new();
        for (connection, value) in &rows {
            let json = value.to_string();
            assert!(!json.contains(&owner.public_key().to_hex()));
            assert!(!json.contains(&device.public_key().to_hex()));
            for (cached_owner, cached_device) in cached.iter().skip(usize::from(opened)) {
                assert!(!json.contains(&cached_owner.public_key().to_hex()));
                assert!(!json.contains(&cached_device.public_key().to_hex()));
            }
            if value[0] == "REQ" {
                let authors = subjects.entry(*connection).or_default();
                for filter in value.as_array().unwrap().iter().skip(2) {
                    let requested = filter["authors"].as_array().unwrap();
                    assert_eq!(requested.len(), 1);
                    authors.insert(requested[0].as_str().unwrap().to_owned());
                }
            }
        }
        assert!(subjects.values().all(|authors| authors.len() == 1));
        let mut signers = HashSet::new();
        for connection in subjects.keys() {
            let (_, auth) = rows
                .iter()
                .find(|(id, value)| id == connection && value[0] == "AUTH")
                .unwrap();
            let signer = auth[1]["pubkey"].as_str().unwrap();
            assert!(expected.iter().all(|key| signer != key.to_hex()));
            assert!(
                signers.insert(signer),
                "independent authors shared an authentication identity"
            );
        }
    }
    core.active_chat_id = None;
    let plan = core.compute_protocol_subscription_plan().unwrap();
    let (_, discovery) =
        core.split_private_discovery_filters(build_protocol_subscription_filters(&plan));
    let removed =
        core.protocol_subscription_runtime.discovery_clients[&cached[0].0.public_key()].clone();
    assert_eq!(
        core.private_discovery_subscriptions(discovery)
            .unwrap()
            .len(),
        2
    );
    assert!(!core
        .protocol_subscription_runtime
        .discovery_clients
        .contains_key(&cached[0].0.public_key()));
    // A held descriptor cannot resurrect the removed scope through catch-up.
    assert!(core
        .runtime
        .block_on(
            protocol::private_reads::apply_private_discovery_subscriptions(
                vec![(
                    Arc::downgrade(&removed),
                    vec![Filter::new()
                        .author(cached[0].0.public_key())
                        .kind(Kind::Metadata)]
                )],
                &[relay],
            )
        )
        .is_err());
    assert_eq!(serde_json::to_value(&core.app_keys).unwrap(), saved);
}

#[test]
fn discovery_demand_restores_orphan_active_and_inactive_ratchets_without_cache_only_users() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("discovery-demand-restart", &owner, &device);
    let peers = (0..3)
        .map(|_| (Keys::generate(), Keys::generate()))
        .collect::<Vec<_>>();
    for (index, (peer, peer_device)) in peers.iter().enumerate() {
        cache_discovery_roster(&mut core, peer, &[peer_device.public_key()], 1);
        if index < 2 {
            core.protocol_engine
                .as_mut()
                .unwrap()
                .import_session_state(
                    peer.public_key(),
                    Some(peer_device.public_key().to_hex()),
                    established_peer_session_state_for_test(peer_device, &device),
                    unix_now(),
                )
                .unwrap();
        }
    }
    let mut snapshot = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .session_manager_snapshot();
    let inactive_owner = ndr_owner_pubkey(peers[1].0.public_key());
    let inactive = &mut snapshot
        .users
        .iter_mut()
        .find(|user| user.owner_pubkey == inactive_owner)
        .unwrap()
        .devices[0];
    inactive
        .inactive_sessions
        .push(inactive.active_session.take().unwrap());
    let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    seed_protocol_storage_for_test(
        storage.as_ref(),
        snapshot,
        GroupEventManager::new(ndr_owner_pubkey(owner.public_key())).snapshot(),
    )
    .unwrap();
    let mut previous = None;
    for _ in 0..2 {
        core.protocol_engine = Some(
            ProtocolEngine::load_or_create_for_local_device(
                storage.clone(),
                owner.public_key(),
                &device,
            )
            .unwrap(),
        );
        assert!(core.threads.is_empty());
        assert!(core.recent_handshake_peers.is_empty());
        let plan = core.compute_protocol_subscription_plan().unwrap();
        for (index, (peer, peer_device)) in peers.iter().enumerate() {
            assert_eq!(
                plan.roster_authors.contains(&peer.public_key().to_hex()),
                index < 2
            );
            assert_eq!(
                plan.invite_authors
                    .contains(&peer_device.public_key().to_hex()),
                index < 2
            );
        }
        let restored = core
            .protocol_engine
            .as_ref()
            .unwrap()
            .session_manager_snapshot();
        let retained = &restored
            .users
            .iter()
            .find(|user| user.owner_pubkey == inactive_owner)
            .unwrap()
            .devices[0];
        assert!(retained.active_session.is_none());
        assert_eq!(retained.inactive_sessions.len(), 1);
        if let Some(previous) = &previous {
            assert_eq!(&plan, previous);
        }
        previous = Some(plan);
    }
    assert_eq!(core.app_keys.len(), 3);
}

#[test]
fn discovery_demand_preserves_pending_owner_claim_across_restart_until_verified_delivery() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("discovery-demand-pending", &owner, &device);
    let storage = Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, &owner, &device, storage.clone(), None, None);
    let engine = core.protocol_engine.as_mut().unwrap();
    let invite = engine.local_invite().unwrap();
    let (mut peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer.public_key()),
        )
        .unwrap();
    engine
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    let sent = peer_session
        .plan_send(b"pending owner demand", NdrUnixSeconds(11))
        .unwrap();
    let sent = peer_session.apply_send(sent);
    let event = nostr_double_ratchet::message_event(&sent.envelope).unwrap();
    assert!(engine
        .process_direct_message_event(&event)
        .unwrap()
        .is_none());
    for _ in 0..2 {
        assert!(core.threads.is_empty());
        assert!(!core.app_keys.contains_key(&peer.public_key().to_hex()));
        assert_eq!(
            core.protocol_engine
                .as_ref()
                .unwrap()
                .debug_snapshot()
                .pending_inbound_count,
            1
        );
        assert!(core
            .compute_protocol_subscription_plan()
            .unwrap()
            .roster_authors
            .contains(&peer.public_key().to_hex()));
        core.protocol_engine = Some(
            ProtocolEngine::load_or_create_for_local_device(
                storage.clone(),
                owner.public_key(),
                &device,
            )
            .unwrap(),
        );
    }
    let proof = signed_app_keys_authorization_event(&peer, peer_device.public_key(), 12);
    let batch = core
        .protocol_engine
        .as_mut()
        .unwrap()
        .ingest_app_keys_event(&proof)
        .unwrap();
    assert_eq!(batch.direct_messages.len(), 1);
    assert_eq!(batch.direct_messages[0].sender, peer.public_key());
    assert_eq!(batch.direct_messages[0].content, "pending owner demand");
}

#[test]
fn discovery_demand_keeps_group_recent_and_private_invite_reasons_without_threads() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peers = (0..4)
        .map(|_| (Keys::generate(), Keys::generate()))
        .collect::<Vec<_>>();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("discovery-demand-reasons", &owner, &device);
    let group = test_group_snapshot(
        "discovery-group",
        "Group",
        owner.public_key(),
        vec![owner.public_key(), peers[0].0.public_key()],
        vec![owner.public_key()],
        1,
    );
    core.groups.insert(group.group_id.clone(), group);
    core.remember_recent_handshake_peer(
        peers[1].0.public_key().to_hex(),
        peers[1].1.public_key().to_hex(),
        unix_now().get(),
    );
    core.handle_action(AppAction::AcceptInvite {
        invite_input: forged_private_invite_url(&peers[2].1, peers[2].0.public_key()),
    });
    assert!(core.pending_outgoing_invite_acceptance.is_some());
    let invite = create_private_invite_for_test(&mut core);
    core.handle_relay_event(crafted_private_invite_response(
        &invite,
        &peers[3].1,
        Some(peers[3].1.public_key().to_hex()),
        Some(peers[3].0.public_key()),
    ));
    assert_eq!(core.pending_private_invite_responses.len(), 1);
    assert!(core.pending_outgoing_invite_acceptance.is_some());
    let plan = core.compute_protocol_subscription_plan().unwrap();
    for (peer, _) in &peers {
        let key = peer.public_key().to_hex();
        assert!(!core.threads.contains_key(&key));
        assert!(!core.app_keys.contains_key(&key));
        assert!(plan.roster_authors.contains(&key));
    }
}

#[test]
fn discovery_demand_keeps_revocation_and_replacement_device_invites_for_conversations() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let old_device = Keys::generate();
    let replacement = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("discovery-demand-revocation", &owner, &device);
    cache_discovery_roster(&mut core, &peer, &[old_device.public_key()], 1);
    core.ensure_thread_record(&peer.public_key().to_hex(), 1);
    core.protocol_engine
        .as_mut()
        .unwrap()
        .import_session_state(
            peer.public_key(),
            Some(old_device.public_key().to_hex()),
            established_peer_session_state_for_test(&old_device, &device),
            UnixSeconds(2),
        )
        .unwrap();
    assert!(core
        .compute_protocol_subscription_plan()
        .unwrap()
        .invite_authors
        .contains(&old_device.public_key().to_hex()));
    let proof = signed_app_keys_authorization_event(&peer, replacement.public_key(), 3);
    assert!(core.apply_app_keys_event(&proof).unwrap());
    let plan = core.compute_protocol_subscription_plan().unwrap();
    assert!(plan.roster_authors.contains(&peer.public_key().to_hex()));
    assert!(plan
        .invite_authors
        .contains(&replacement.public_key().to_hex()));
    assert!(!plan
        .invite_authors
        .contains(&old_device.public_key().to_hex()));
    assert!(core.message_recipient_bootstrap_needed());
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .active_roster_session_count_for_owner(peer.public_key()),
        0
    );
    let (_, discovery) =
        core.split_private_discovery_filters(build_protocol_subscription_filters(&plan));
    let filters = serde_json::to_value(discovery).unwrap();
    assert!(filters
        .as_array()
        .unwrap()
        .iter()
        .any(
            |filter| filter["authors"] == serde_json::json!([replacement.public_key().to_hex()])
                && filter["kinds"] == serde_json::json!([INVITE_EVENT_KIND])
        ));
    assert!(!filters
        .to_string()
        .contains(&old_device.public_key().to_hex()));
}
