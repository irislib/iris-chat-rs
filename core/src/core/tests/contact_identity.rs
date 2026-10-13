#[test]
fn contact_identity_keeps_first_name_until_exact_approval_and_persists_private_history() {
    let mut pair = chat_read_receipt_pair("contact-identity");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    let metadata = |name: &str, time| {
        EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
            .custom_created_at(Timestamp::from_secs(time))
            .sign_with_keys(&peer)
            .unwrap()
    };
    pair.a.apply_profile_metadata_event(&metadata("Alice", 10));
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.apply_profile_metadata_event(&metadata("Bob", 11));
    pair.a.rebuild_state();
    assert_eq!(
        pair.a.state.current_chat.as_ref().unwrap().display_name,
        "Alice"
    );
    assert_eq!(
        pair.a
            .state
            .current_chat
            .as_ref()
            .unwrap()
            .contact_identity
            .as_ref()
            .unwrap()
            .pending_name
            .as_deref(),
        Some("Bob")
    );
    pair.a.apply_profile_metadata_event(&metadata("Carol", 12));
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Bob".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Alice");
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Carol".into(),
    });
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Carol".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Carol");
    assert_eq!(
        pair.a.threads[&id]
            .messages
            .iter()
            .filter(|m| m.body == "Name change approved: Alice → Carol")
            .count(),
        1
    );
    let pending = pair.a.pending_relay_publishes.len();
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    let avatar = pair
        .a
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .social_connection
        .as_ref()
        .unwrap();
    assert!(
        avatar.is_favorite,
        "The private favorite must reach the open chat avatar"
    );
    assert!(
        pair.a
            .state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == id)
            .unwrap()
            .social_connection
            .as_ref()
            .unwrap()
            .is_favorite,
        "The chat-list avatar must update with the same private favorite"
    );
    // Favorites queue authenticated sibling ratchet events, never public follow metadata.
    assert!(pair.a.pending_relay_publishes.len() > pending);
    assert_only_public_invites_pending(&pair.a);
    assert!(
        pair.a.private_contacts.state.as_ref().unwrap().contacts[&id]["favorite"]
            .value
            .as_bool()
            .unwrap()
    );
    let stored = pair.a.load_persisted().unwrap().unwrap();
    let memory = &stored.owner_profiles[&id].contact_memory;
    assert_eq!(memory.first_seen_name.as_deref(), Some("Alice"));
    assert_eq!(memory.accepted_name.as_deref(), Some("Carol"));
    assert_eq!(memory.name_changes.len(), 1);
    assert!(memory.favorite);
    let participant = pair
        .a
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .participants
        .iter()
        .find(|participant| participant.owner_pubkey_hex == id)
        .unwrap();
    assert!(participant.social_connection.as_ref().unwrap().is_favorite);
    let public = build_profile_metadata_json(&stored.owner_profiles[&id]);
    assert!(!public.contains("Alice"));
    assert!(!public.contains("favorite"));
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: false,
    });
    assert!(!pair
        .a
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .social_connection
        .as_ref()
        .is_some_and(|connection| connection.is_favorite));
    assert!(!pair
        .a
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == id)
        .unwrap()
        .social_connection
        .as_ref()
        .is_some_and(|connection| connection.is_favorite));
    save_contact_details(&mut pair.a, &peer, "Work friend", "");
    pair.a.apply_profile_metadata_event(&metadata("Dana", 13));
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Dana".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Work friend");
    assert_eq!(
        pair.a.owner_profiles[&id]
            .contact_memory
            .first_seen_name
            .as_deref(),
        Some("Alice")
    );
}

#[test]
fn contact_identity_read_only_snapshot_keeps_unread_and_pending_name_off_route() {
    let mut pair = chat_read_sync_pair("contact-read-only");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    let profile = |name: &str, time| {
        EventBuilder::new(
            Kind::Metadata,
            serde_json::json!({"name": name}).to_string(),
        )
        .custom_created_at(Timestamp::from_secs(time))
        .sign_with_keys(&peer)
        .unwrap()
    };
    pair.a.apply_profile_metadata_event(&profile("Alice", 10));
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.apply_profile_metadata_event(&profile("Alicia", 11));
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    pair.a.threads.get_mut(&id).unwrap().unread_count = 3;
    pair.a.rebuild_persist_and_emit_state();
    let mut state = pair.a.state.clone();
    state.current_chat = None;
    let bounded = chat_read_state(&state, &id, Some(1));
    let shared = pair.a.app_store.shared();
    let read = chat_snapshot_from_state_and_db(&bounded, Some(&shared), &id, 1).unwrap();
    let memory = read.contact_identity.unwrap();
    assert_eq!(memory.saved_name.as_deref(), Some("Alice"));
    assert_eq!(memory.pending_name.as_deref(), Some("Alicia"));
    assert!(memory.is_favorite);
    assert_eq!(pair.a.threads[&id].unread_count, 3);
    assert!(bounded.current_chat.is_none());
}
#[test]
fn contact_name_approval_offline_negentropy_recovers_without_public_profile() {
    for supported in [false, true] {
        let mut pair = chat_read_sync_pair("name-approval-offline");
        for roster in pair.a.app_keys.values_mut() {
            roster
                .devices
                .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
        }
        pair.b.app_keys = pair.a.app_keys.clone();
        let peer = Keys::generate();
        let target = peer.public_key().to_hex();
        for core in [&mut pair.a, &mut pair.b] {
            core.push_incoming_message_from(
                &target,
                Some("hello".into()),
                "hi".into(),
                20,
                None,
                None,
                Some(target.clone()),
                None,
            );
            core.owner_profiles
                .entry(target.clone())
                .or_default()
                .contact_memory
                .observe_name("Alice");
        }
        // Only the approving device has fetched the new public name.
        let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"Alicia"}"#)
            .sign_with_keys(&peer)
            .unwrap();
        pair.a.apply_profile_metadata_event(&profile);
        pair.a.approve_contact_name(&target, "Alicia");
        assert_eq!(pair.a.owner_display_label(&target), "Alicia");
        let endpoint = Arc::new(
            pair.a
                .runtime
                .block_on(
                    fips_core::FipsEndpoint::builder()
                        .without_system_tun()
                        .bind(),
                )
                .unwrap(),
        );
        let (a_tx, a_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        let (b_tx, b_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        pair.a.install_device_sync_sender_for_test(
            endpoint.clone(),
            a_tx,
            vec![test_fips_peer(&pair.b_device)],
        );
        pair.b.install_device_sync_sender_for_test(
            endpoint.clone(),
            b_tx,
            vec![test_fips_peer(&pair.a_device)],
        );
        let mut request =
            serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1});
        if supported {
            request["nameApprovals"] = 1.into();
        }
        let request = serde_json::to_vec(&request).unwrap();
        for _ in 0..2 {
            pair.a.handle_device_sync_packet(
                &pair.b_device.public_key().to_hex(),
                DEVICE_SYNC_PORT,
                &request,
            );
            pair.b.handle_device_sync_packet(
                &pair.a_device.public_key().to_hex(),
                DEVICE_SYNC_PORT,
                &request,
            );
            let mut trace = Vec::new();
            for _ in 0..2048 {
                let a = drain_history_wire(
                    &mut pair.a,
                    &pair.a_device,
                    &mut pair.b,
                    &pair.b_device,
                    &a_rx,
                    &mut trace,
                );
                let b = drain_history_wire(
                    &mut pair.b,
                    &pair.b_device,
                    &mut pair.a,
                    &pair.a_device,
                    &b_rx,
                    &mut trace,
                );
                if !a && !b {
                    break;
                }
            }
            assert_eq!(
                pair.b.owner_profiles[&target]
                    .contact_memory
                    .accepted_name
                    .as_deref(),
                Some(if supported { "Alicia" } else { "Alice" }),
                "wire: {trace:?}"
            );
            assert_eq!(
                pair.b.owner_profiles[&target]
                    .contact_memory
                    .name_changes
                    .len(),
                usize::from(supported)
            );
            if !supported {
                assert!(!trace
                    .iter()
                    .any(|packet| packet.to_string().contains("privateNameApproval")));
            }
        }
        pair.a.runtime.block_on(endpoint.shutdown()).unwrap();
    }
}

fn name_approval_test_event(core: &AppCore) -> Event {
    let record = core
        .export_sync_record_values_for_test()
        .unwrap()
        .into_iter()
        .find(|record| record["type"] == "privateNameApproval")
        .expect("durable private approval");
    serde_json::from_value(record["event"].clone()).unwrap()
}

fn approve_test_contact_name(core: &mut AppCore, peer: &Keys, name: &str) {
    let target = peer.public_key().to_hex();
    core.push_incoming_message_from(
        &target,
        Some("hello".into()),
        "hi".into(),
        20,
        None,
        None,
        Some(target.clone()),
        None,
    );
    core.owner_profiles
        .entry(target.clone())
        .or_default()
        .contact_memory
        .observe_name("Alice");
    let profile = EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
        .sign_with_keys(peer)
        .unwrap();
    core.apply_profile_metadata_event(&profile);
    core.approve_contact_name(&target, name);
}

#[test]
fn contact_name_approval_concurrent_order_replay_restart_and_authorization() {
    let mut pair = chat_read_sync_pair("name-approval-order");
    pair.b.app_keys = pair.a.app_keys.clone();
    let peer = Keys::generate();
    let target = peer.public_key().to_hex();
    approve_test_contact_name(&mut pair.a, &peer, "Alicia");
    approve_test_contact_name(&mut pair.b, &peer, "Ally");
    let a = name_approval_test_event(&pair.a);
    let b = name_approval_test_event(&pair.b);
    assert!(a.verify().is_ok() && b.verify().is_ok());
    let body = serde_json::json!({"type":"private-name-approval","v":1,"event":a}).to_string();
    let stranger = Keys::generate();
    assert!(pair.b.receive_name_approval_control(
        stranger.public_key(),
        Some(pair.a_device.public_key()),
        &body
    ));
    assert!(pair.b.receive_name_approval_control(
        pair.owner.public_key(),
        Some(stranger.public_key()),
        &body
    ));
    assert_eq!(pair.b.owner_display_label(&target), "Ally");
    let mut tampered = a.clone();
    tampered.content.push(' ');
    assert!(!pair.b.apply_name_approval_event(tampered));
    for _ in 0..2 {
        assert!(pair.b.receive_name_approval_control(
            pair.owner.public_key(),
            Some(pair.a_device.public_key()),
            &body
        ));
        assert!(pair.a.apply_name_approval_event(b.clone()));
    }
    assert_eq!(
        pair.a.owner_profiles[&target].contact_memory,
        pair.b.owner_profiles[&target].contact_memory
    );
    assert_eq!(
        pair.a.owner_profiles[&target]
            .contact_memory
            .name_changes
            .len(),
        2
    );
    // A later explicit decision must beat both concurrent decisions.
    let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"Carol"}"#)
        .custom_created_at(Timestamp::from_secs(unix_now().get() + 1))
        .sign_with_keys(&peer)
        .unwrap();
    pair.a.apply_profile_metadata_event(&profile);
    pair.a.approve_contact_name(&target, "Carol");
    let latest: Event = pair
        .a
        .export_sync_record_values_for_test()
        .unwrap()
        .into_iter()
        .filter(|r| r["type"] == "privateNameApproval")
        .filter_map(|r| serde_json::from_value::<Event>(r["event"].clone()).ok())
        .find(|e| serde_json::from_str::<serde_json::Value>(&e.content).unwrap()["revision"] == 2)
        .unwrap();
    assert!(pair.b.apply_name_approval_event(latest));
    assert!(pair.b.apply_name_approval_event(a));
    assert_eq!(pair.b.owner_display_label(&target), "Carol");
    pair.a.persist_best_effort_inner();
    let data_dir = pair.a_dir.path().to_string_lossy().into_owned();
    drop(pair.a);
    let restored = restart_block_history_core(&pair.owner, &pair.a_device, data_dir);
    assert_eq!(
        restored.owner_profiles[&target]
            .contact_memory
            .accepted_name
            .as_deref(),
        Some("Carol")
    );
    assert_eq!(
        restored.owner_profiles[&target]
            .contact_memory
            .name_changes
            .len(),
        3
    );
    let foreign = Keys::generate();
    let mut foreign_core =
        logged_in_test_core("name-approval-foreign", &foreign, &Keys::generate());
    assert!(!foreign_core.apply_name_approval_event(b));
}

#[test]
fn contact_name_approval_storage_failure_keeps_the_old_name() {
    let mut pair = chat_read_sync_pair("name-approval-save-fail");
    let peer = Keys::generate();
    let target = peer.public_key().to_hex();
    pair.a.push_incoming_message_from(
        &target,
        Some("hello".into()),
        "hi".into(),
        20,
        None,
        None,
        Some(target.clone()),
        None,
    );
    pair.a
        .owner_profiles
        .entry(target.clone())
        .or_default()
        .contact_memory
        .observe_name("Alice");
    let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"Alicia"}"#)
        .sign_with_keys(&peer)
        .unwrap();
    pair.a.apply_profile_metadata_event(&profile);
    pair.a.persist_best_effort_inner();
    pair.a
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    pair.a.approve_contact_name(&target, "Alicia");
    pair.a
        .app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    assert_eq!(pair.a.owner_display_label(&target), "Alice");
    assert!(pair.a.owner_profiles[&target]
        .contact_memory
        .name_changes
        .is_empty());
}

#[test]
fn contact_name_approval_migrates_existing_acceptance_without_overwriting_later_decisions() {
    let mut pair = chat_read_sync_pair("name-approval-migration");
    let peer = Keys::generate();
    let target = peer.public_key().to_hex();
    let memory = &mut pair
        .a
        .owner_profiles
        .entry(target.clone())
        .or_default()
        .contact_memory;
    memory.observe_name("Alice");
    memory.approve_name("Alicia", "Alicia", 10).unwrap();
    pair.a.migrate_name_approvals();
    pair.a.migrate_name_approvals();
    let legacy = name_approval_test_event(&pair.a);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&legacy.content).unwrap()["revision"],
        0
    );
    assert!(pair.b.apply_name_approval_event(legacy.clone()));
    assert_eq!(
        pair.b.owner_profiles[&target]
            .contact_memory
            .accepted_name
            .as_deref(),
        Some("Alicia")
    );
    approve_test_contact_name(&mut pair.b, &peer, "Ally");
    assert!(pair.b.apply_name_approval_event(legacy));
    assert_eq!(
        pair.b.owner_profiles[&target]
            .contact_memory
            .accepted_name
            .as_deref(),
        Some("Ally")
    );
    assert_eq!(
        pair.b.owner_profiles[&target]
            .contact_memory
            .first_seen_name
            .as_deref(),
        Some("Alice")
    );
}
