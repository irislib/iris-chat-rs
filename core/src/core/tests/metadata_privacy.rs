#[test]
fn metadata_privacy_blocks_identity_signers_even_with_a_version_marker() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("metadata-identity-guard", &owner, &device);
    let recipient = Keys::generate().public_key();
    let invite = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .local_invite()
        .unwrap();
    let invite_signer = Keys::new(
        nostr::SecretKey::from_slice(&invite.inviter_ephemeral_private_key.unwrap()).unwrap(),
    );
    let before = core.pending_relay_publishes.len();
    for signer in [&owner, &device, &invite_signer] {
        for kind in [MESSAGE_EVENT_KIND, INVITE_RESPONSE_KIND] {
            let event = EventBuilder::new(Kind::from(kind as u16), "encrypted")
                .tags([
                    nostr::Tag::public_key(recipient),
                    nostr::Tag::parse(["v", "2"]).unwrap(),
                ])
                .sign_with_keys(signer)
                .unwrap();
            assert!(!core.publish_runtime_event(event.clone(), "privacy-test", None));
            core.emit_nearby_published_event(&event);
        }
    }
    assert_eq!(core.pending_relay_publishes.len(), before);
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::NearbyPublishedEvent { .. })));
}

#[test]
fn metadata_privacy_quarantines_saved_old_ciphertext_and_preserves_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("metadata-restore-guard", &owner, &device);
    let chat = Keys::generate().public_key().to_hex();
    core.push_outgoing_message_with_id(
        "inner".into(),
        &chat,
        "keep this history".into(),
        unix_now().get(),
        None,
        DeliveryState::Queued,
    );
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "old ciphertext")
        .tag(nostr::Tag::public_key(PublicKey::from_hex(&chat).unwrap()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let pending = PendingRelayPublish {
        owner_pubkey_hex: owner.public_key().to_hex(),
        event_id: event.id.to_hex(),
        label: APPCORE_PROTOCOL_LABEL.into(),
        event_json: event.as_json(),
        inner_event_id: Some("inner".into()),
        chat_id: Some(chat.clone()),
        created_at_secs: event.created_at.as_secs(),
        attempt_count: 1,
        last_error: None,
    };
    core.app_store
        .upsert_pending_relay_publish(&pending)
        .unwrap();
    core.persist_best_effort();
    core.start_session(owner.public_key(), Some(owner.clone()), device, true, true)
        .unwrap();
    core.run_session_startup_follow_up();
    assert!(!core
        .pending_relay_publishes
        .contains_key(&event.id.to_hex()));
    let retained = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    let old = retained
        .iter()
        .find(|row| row.event_id == event.id.to_hex())
        .unwrap();
    assert_eq!(old.event_json, pending.event_json);
    assert!(old
        .last_error
        .as_deref()
        .unwrap()
        .contains("will not be resent"));
    let message = core.threads[&chat]
        .messages
        .iter()
        .find(|message| message.id == "inner")
        .unwrap();
    assert_eq!(message.body, "keep this history");
    assert_eq!(message.delivery, DeliveryState::Failed);
}

#[test]
fn metadata_privacy_quarantines_saved_public_group_rosters_and_shared_channels() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let transport = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("metadata-group-restart", &owner, &device);
    let events = [
        EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "shared ciphertext")
            .tag(nostr::Tag::parse(["v", "2"]).unwrap())
            .sign_with_keys(&transport)
            .unwrap(),
        EventBuilder::new(Kind::from(GROUP_ROSTER_FACT_KIND as u16), "")
            .tags([
                nostr::Tag::parse(["type", "group_roster"]).unwrap(),
                nostr::Tag::parse(["d", "private-group"]).unwrap(),
                nostr::Tag::public_key(device.public_key()),
            ])
            .sign_with_keys(&owner)
            .unwrap(),
    ];
    for event in &events {
        assert!(!core.publish_runtime_event(event.clone(), "group-privacy-test", None));
        core.emit_nearby_published_event(event);
        core.app_store
            .upsert_pending_relay_publish(&PendingRelayPublish {
                owner_pubkey_hex: owner.public_key().to_hex(),
                event_id: event.id.to_hex(),
                label: APPCORE_PROTOCOL_LABEL.into(),
                event_json: event.as_json(),
                inner_event_id: None,
                chat_id: Some("group:private-group".into()),
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 7,
                last_error: None,
            })
            .unwrap();
    }
    core.persist_best_effort();
    core.start_session(owner.public_key(), Some(owner.clone()), device, true, true)
        .unwrap();
    core.run_session_startup_follow_up();
    core.retry_pending_relay_publishes("group-privacy-restart");
    let retained = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    for event in &events {
        assert!(!core
            .pending_relay_publishes
            .contains_key(&event.id.to_hex()));
        let saved = retained
            .iter()
            .find(|row| row.event_id == event.id.to_hex())
            .unwrap();
        assert_eq!(saved.event_json, event.as_json());
        assert_eq!(saved.attempt_count, 7);
        assert!(saved
            .last_error
            .as_deref()
            .unwrap()
            .contains("will not be resent"));
    }
    assert!(!updates.try_iter().any(|update| {
        matches!(update, AppUpdate::NearbyPublishedEvent { event_json, .. }
            if events.iter().any(|event| event_json == event.as_json()))
    }));
}

#[test]
fn metadata_privacy_isolates_peer_discovery_from_identity_auth_and_own_receive_filters() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate().public_key();
    let peer_device = Keys::generate().public_key();
    let other_owner = Keys::generate().public_key();
    let other_device = Keys::generate().public_key();
    let peers = [peer_owner, peer_device, other_owner, other_device];
    let message_author = Keys::generate().public_key();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("metadata-connection-isolation", &owner, &device);
    let (relay, wire, client) = core.runtime.block_on(async {
        let (relay, wire) = publish_helpers::authenticated_relay(device.public_key(), false).await;
        let client = publish_helpers::identity_receiver(&device, &relay, &wire).await;
        (relay, wire, client)
    });
    core.logged_in.as_mut().unwrap().client = client;
    core.logged_in.as_mut().unwrap().relay_urls = vec![relay.clone()];
    core.preferences.nostr_relay_urls = vec![relay.to_string()];
    core.relay_status_by_url
        .insert(relay.to_string(), RelayStatus::Connected);
    let own_invite = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .local_invite()
        .unwrap()
        .inviter_ephemeral_public_key
        .to_string();
    let mut plan = protocol_plan_for_test(vec![message_author], Vec::new());
    plan.roster_authors = vec![
        owner.public_key().to_hex(),
        peer_owner.to_hex(),
        other_owner.to_hex(),
    ];
    plan.invite_authors = vec![
        device.public_key().to_hex(),
        peer_device.to_hex(),
        other_device.to_hex(),
    ];
    plan.message_recipients = vec![device.public_key().to_hex()];
    plan.invite_response_recipient = Some(own_invite.clone());
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    let (receiver, discovery) =
        core.split_private_discovery_filters(build_protocol_subscription_filters(&plan));
    let receiver_json = serde_json::to_string(&receiver).unwrap();
    let discovery_json = serde_json::to_string(&discovery).unwrap();
    for peer in peers {
        assert!(!receiver_json.contains(&peer.to_hex()));
    }
    for own in [
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
        own_invite.clone(),
    ] {
        assert!(!discovery_json.contains(&own));
    }
    assert!(discovery
        .iter()
        .all(|filter| filter.authors.as_ref().unwrap().len() == 1));
    core.reconcile_protocol_subscriptions("metadata-wire-test", false);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rows = wire.lock().unwrap().clone();
        let all_requested = peers.iter().all(|peer| {
            rows.iter()
                .any(|(_, event)| event[0] == "REQ" && event.to_string().contains(&peer.to_hex()))
        });
        if all_requested
            && rows.iter().filter(|(_, event)| event[0] == "AUTH").count() > peers.len()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "peer discovery did not subscribe"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let rows = wire.lock().unwrap().clone();
    let own_connections = rows
        .iter()
        .filter(|(_, event)| {
            event[0] == "AUTH" && event[1]["pubkey"] == device.public_key().to_hex()
        })
        .map(|(number, _)| *number)
        .collect::<HashSet<_>>();
    assert!(!own_connections.is_empty());
    for (number, event) in &rows {
        let json = event.to_string();
        if own_connections.contains(number) {
            for peer in peers {
                assert!(!json.contains(&peer.to_hex()));
            }
        } else {
            assert!(!json.contains(&owner.public_key().to_hex()));
            assert!(!json.contains(&device.public_key().to_hex()));
            assert!(!json.contains(&own_invite));
            assert!(!json.contains(&message_author.to_hex()));
        }
    }
    let live_connections = rows
        .iter()
        .map(|(number, _)| *number)
        .collect::<HashSet<_>>();
    // A cold-start lookup also receives a fresh anonymous query context, even
    // when its caller is the account-authenticated receive client.
    let receiver_client = core.logged_in.as_ref().unwrap().client.clone();
    core.runtime.block_on(async {
        let events = protocol::fetch_events_for_filters(
            &receiver_client,
            vec![Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors([peer_owner, other_owner])],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert!(events.is_empty());
    });
    let rows = wire.lock().unwrap();
    let mut scopes: std::collections::HashMap<usize, HashSet<PublicKey>> =
        std::collections::HashMap::new();
    let mut cold_authors = HashSet::new();
    for (number, event) in rows.iter().filter(|(_, event)| event[0] == "REQ") {
        for peer in peers {
            if event.to_string().contains(&peer.to_hex()) {
                scopes.entry(*number).or_default().insert(peer);
                if !live_connections.contains(number) {
                    cold_authors.insert(peer);
                }
            }
        }
    }
    assert_eq!(cold_authors, HashSet::from([peer_owner, other_owner]));
    assert!(
        scopes.values().all(|authors| authors.len() == 1),
        "one connection must never reveal multiple private contact lookups: {scopes:?}"
    );
    for (number, event) in rows.iter().filter(|(_, event)| {
        event[0] == "REQ"
            && peers
                .iter()
                .any(|peer| event.to_string().contains(&peer.to_hex()))
    }) {
        assert!(!own_connections.contains(number));
        assert!(!event.to_string().contains(&device.public_key().to_hex()));
    }
}
