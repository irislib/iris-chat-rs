#[test]
fn device_sync_authorization_changes_even_when_routing_peers_stay_the_same() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let contact = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("sync-authorization", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let owner_hex = owner.public_key().to_hex();
    // A previously linked device may also remain a routable contact device.
    let mut foreign_roster = core.app_keys[&owner_hex].clone();
    foreign_roster.owner_pubkey_hex = contact.public_key().to_hex();
    core.app_keys
        .insert(contact.public_key().to_hex(), foreign_roster);
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!("IPv4 fixture");
    };
    drop(socket);
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(core.device_sync_has_sibling_tcp_for_test());
    core.app_keys
        .get_mut(&owner_hex)
        .unwrap()
        .devices
        .retain(|device| device.identity_pubkey_hex != sibling.public_key().to_hex());
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(
        !core.device_sync_has_sibling_tcp_for_test(),
        "revocation must retire the private service even when the device remains a routing contact"
    );
    assert!(
        core.device_sync_endpoint_for_test().is_some(),
        "shared routing remains available"
    );
    core.stop_device_sync();
}

#[test]
fn device_sync_locally_revoked_device_has_no_private_sync_peers() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("sync-local-revocation", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    assert!(core.device_sync_peer_is_authorized(&sibling.public_key().to_hex()));
    core.app_keys
        .get_mut(&owner.public_key().to_hex())
        .unwrap()
        .devices
        .retain(|device| device.identity_pubkey_hex != local.public_key().to_hex());
    assert!(!core.device_sync_peer_is_authorized(&sibling.public_key().to_hex()));
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!("IPv4 fixture");
    };
    drop(socket);
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(
        !core.device_sync_has_sibling_tcp_for_test(),
        "a revoked local device must not start a private history service"
    );
    core.stop_device_sync();
}

#[test]
fn device_sync_stale_runtime_cannot_export_metadata_to_revoked_sibling() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("sync-stale-runtime", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let endpoint = Arc::new(
        core.runtime
            .block_on(
                fips_core::FipsEndpoint::builder()
                    .without_system_tun()
                    .bind(),
            )
            .unwrap(),
    );
    let (sender, records) = DeviceSyncTcpSender::test_channel(64, 64 * 1024);
    core.install_device_sync_sender_for_test(
        endpoint.clone(),
        sender,
        vec![test_fips_peer(&sibling)],
    );
    core.broadcast_device_sync_snapshot();
    assert!(
        records.try_recv().is_ok(),
        "the authorized positive control must export metadata"
    );
    while records.try_recv().is_ok() {}
    core.app_keys
        .get_mut(&owner.public_key().to_hex())
        .unwrap()
        .devices
        .retain(|device| device.identity_pubkey_hex != sibling.public_key().to_hex());
    // Exercise the window before runtime reconciliation replaces its cached allowlist.
    core.broadcast_device_sync_snapshot();
    assert!(
        records.is_empty(),
        "current account authorization must gate every metadata export"
    );
    core.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn device_sync_private_negentropy_rejects_other_accounts_and_revoked_devices() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let foreign_owner = Keys::generate();
    let foreign_device = Keys::generate();
    let unknown = Keys::generate();
    let chat = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("sync-private-protocol", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling.public_key().to_hex();
    let mut foreign_roster = core.app_keys[&owner_hex].clone();
    foreign_roster.owner_pubkey_hex = foreign_owner.public_key().to_hex();
    foreign_roster.devices[1].identity_pubkey_hex = foreign_device.public_key().to_hex();
    // The other account may even claim our local key. Its roster is never authority for ours.
    core.app_keys
        .insert(foreign_owner.public_key().to_hex(), foreign_roster);
    let original_roster = core.app_keys[&owner_hex].clone();
    let endpoint = Arc::new(
        core.runtime
            .block_on(
                fips_core::FipsEndpoint::builder()
                    .without_system_tun()
                    .bind(),
            )
            .unwrap(),
    );
    let (sender, records) = DeviceSyncTcpSender::test_channel(64, 64 * 1024);
    core.install_device_sync_sender_for_test(
        endpoint.clone(),
        sender,
        vec![test_fips_peer(&sibling)],
    );
    let mut engine = nostr_pubsub_reconcile::Session::new(
        [],
        nostr_pubsub_reconcile::Filter { since: 0, until: 0 },
        Default::default(),
    )
    .unwrap();
    let frame = engine
        .initiate()
        .unwrap()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let session = "ab".repeat(16);
    let request = serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1});
    let open = serde_json::json!({"type":"historyOpen","v":1,"scope":"state","session":session,"since":0,"until":0,"frame":frame});
    for packet in [&request, &open] {
        core.handle_device_sync_packet(
            &sibling_hex,
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(packet).unwrap(),
        );
    }
    assert_eq!(
        core.device_history_session_count_for_test(),
        1,
        "an authorized sibling must open real negentropy reconciliation"
    );
    assert!(records.try_recv().is_ok());
    while records.try_recv().is_ok() {}
    let snapshot = serde_json::json!({
        "type":"snapshot","v":1,"rosterAt":100,"chats":[{"id":chat,"updatedAt":200}],
        "appKeys":[],"groups":[],"messages":[{"chatId":chat,"id":"private-sync-message", "body":"cHJpdmF0ZQ==","author":chat,"createdAt":200}]
    });
    let mut forged_roster = snapshot.clone();
    forged_roster["appKeys"] = serde_json::json!([{
        "ownerPubkey":owner_hex,"createdAt":300,"devices":[
            {"identityPubkey":local.public_key().to_hex(),"createdAt":1},
            {"identityPubkey":foreign_device.public_key().to_hex(),"createdAt":100}]
    }]);
    let attacks = vec![
        request.clone(),
        open.clone(),
        forged_roster,
        snapshot.clone(),
        serde_json::json!({"type":"resyncRequired","v":1}),
        serde_json::json!({"type":"pageEnd","v":1,"rosterAt":100,"next":null,"recordReconcile":1}),
        serde_json::json!({"type":"historyFrame","v":1,"session":session,"frame":frame}),
        serde_json::json!({"type":"historyNeed","v":1,"session":session,"ids":["00".repeat(32)]}),
        serde_json::json!({"type":"historyRecords","v":1,"session":session,"records":[{"type":"message","message":snapshot["messages"][0]}],"requested":["00".repeat(32)]}),
        serde_json::json!({"type":"historyOverflow","v":1,"session":session}),
        serde_json::json!({"type":"historyDone","v":1,"session":session}),
        serde_json::json!({"type":"historyPolicy","v":1,"linkAt":100,"since":0,"linkId":"cd".repeat(32)}),
        serde_json::json!({"type":"historyComplete","v":1,"linkAt":100,"linkId":"cd".repeat(32)}),
    ];
    for source in [
        foreign_owner.public_key().to_hex(),
        foreign_device.public_key().to_hex(),
        unknown.public_key().to_hex(),
        owner_hex.clone(),
        local.public_key().to_hex(),
        local.public_key().to_hex().to_uppercase(),
    ] {
        assert!(!core.device_sync_peer_is_authorized(&source));
        for packet in &attacks {
            core.handle_device_sync_packet(
                &source,
                DEVICE_SYNC_PORT,
                &serde_json::to_vec(packet).unwrap(),
            );
        }
        assert!(
            records.is_empty(),
            "unapproved source must never receive a private response"
        );
        assert!(
            !core.threads.contains_key(&chat),
            "unapproved source must never import a private record"
        );
        assert_eq!(
            core.app_keys[&owner_hex], original_roster,
            "an unapproved source cannot authorize itself"
        );
        assert_eq!(
            core.device_history_session_count_for_test(),
            1,
            "another account cannot hijack or cancel a sibling's session"
        );
    }
    core.handle_device_sync_packet(
        &sibling_hex,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&snapshot).unwrap(),
    );
    assert!(
        has_device_sync_message(&core, &chat, "private-sync-message"),
        "the same private snapshot must import from the authorized sibling"
    );
    while records.try_recv().is_ok() {}
    // Revoke the local device while the in-memory negentropy transcript still exists.
    core.app_keys
        .get_mut(&owner_hex)
        .unwrap()
        .devices
        .retain(|device| device.identity_pubkey_hex != local.public_key().to_hex());
    for packet in &attacks {
        core.handle_device_sync_packet(
            &sibling_hex,
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(packet).unwrap(),
        );
    }
    assert!(
        records.is_empty(),
        "local revocation closes both directions of private sync"
    );
    assert_eq!(core.device_history_session_count_for_test(), 0);
    core.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn restored_authorization_projection_defers_transport_until_complete_roster() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let contact_a = Keys::generate();
    let device_a = Keys::generate();
    let contact_b = Keys::generate();
    let device_b = Keys::generate();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("restore-authorization-runtime", &owner, &local);
    core.preferences.nostr_relay_urls.clear();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!("IPv4 fixture");
    };
    drop(socket);
    core.test_fips_rendezvous_addr = Some(rendezvous);
    let created_at = unix_now().get() + 1;
    let rosters = [
        (&owner, vec![local.public_key(), sibling.public_key()]),
        (&contact_a, vec![device_a.public_key()]),
        (&contact_b, vec![device_b.public_key()]),
    ];
    for (signer, devices) in &rosters {
        let event = AppKeys::new(
            devices
                .iter()
                .copied()
                .map(|device| DeviceEntry::new(device, created_at))
                .collect(),
        )
        .get_event_at(signer.public_key(), created_at)
        .sign_with_keys(signer)
        .unwrap();
        core.capture_authorization_event(&event).unwrap();
    }

    // Real durable replay must restore authority before opening a lifetime LMDB lock.
    core.state.busy.restoring_session = true;
    core.restore_device_sync_record_projection().unwrap();
    for (signer, devices) in &rosters {
        let restored = &core.app_keys[&signer.public_key().to_hex()];
        for device in devices {
            assert!(restored
                .devices
                .iter()
                .any(|entry| entry.identity_pubkey_hex == device.to_hex()));
            assert!(core
                .protocol_engine
                .as_ref()
                .unwrap()
                .has_device_roster_entry_for_owner(signer.public_key(), *device));
        }
    }
    assert!(core.device_sync_peer_is_authorized(&sibling.public_key().to_hex()));
    assert!(!core.device_sync_peer_is_authorized(&device_a.public_key().to_hex()));
    assert!(
        core.device_sync.is_none(),
        "replay must not start FIPS per head"
    );
    assert!(core.attachment_service.is_none());
    assert!(
        !dir.path().join("attachments").exists(),
        "intermediate authorization must not open the attachment LMDB environment"
    );

    // Exercise the real session tail, whose final reconciliation must still run
    // while the outer restore action owns the restoring-session flag.
    core.start_session_inner(
        owner.public_key(),
        Some(owner.clone()),
        local,
        true,
        true,
        false,
    )
    .unwrap();
    assert!(core.state.busy.restoring_session);
    assert!(core.device_sync_has_sibling_tcp_for_test());
    let runtime = core.device_sync.as_ref().unwrap();
    let expected_peers = [
        sibling.public_key(),
        device_a.public_key(),
        device_b.public_key(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    assert_eq!(
        runtime
            .peer_advert_authors
            .iter()
            .copied()
            .collect::<HashSet<_>>(),
        expected_peers
    );
    let endpoint = runtime.endpoint.clone();
    let store_id = core.attachment_service.as_ref().unwrap().store.id;
    core.state.busy.restoring_session = false;
    core.reconcile_device_sync();
    assert!(Arc::ptr_eq(
        &endpoint,
        &core.device_sync.as_ref().unwrap().endpoint
    ));
    assert_eq!(core.attachment_service.as_ref().unwrap().store.id, store_id);
    core.stop_device_sync();
    core.stop_attachment_service();
}

fn restored_authorization_fixture(
    label: &str,
) -> (
    AppCore,
    flume::Receiver<AppUpdate>,
    tempfile::TempDir,
    Keys,
    Keys,
    Vec<Event>,
) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, dir) = logged_in_test_core_with_updates(label, &owner, &device);
    core.preferences.nostr_relay_urls.clear();
    core.state.busy.restoring_session = true;
    core.persist_app_state().unwrap();
    let events = (0..3)
        .map(|_| {
            let peer = Keys::generate();
            AppKeys::new(vec![DeviceEntry::new(Keys::generate().public_key(), 100)])
                .get_event_at(peer.public_key(), 100)
                .sign_with_keys(&peer)
                .unwrap()
        })
        .collect::<Vec<_>>();
    for event in &events {
        core.capture_authorization_event(event).unwrap();
    }
    (core, updates, dir, owner, device, events)
}

#[test]
fn restored_authorization_projection_commits_one_complete_app_snapshot() {
    let (mut core, updates, _dir, _, _, events) =
        restored_authorization_fixture("restore-save-coalescing");
    let shared = core.app_store.shared();
    shared
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TEMP TABLE restored_app_key_writes(owner TEXT);
         CREATE TEMP TRIGGER count_restored_app_key_write AFTER INSERT ON app_keys BEGIN
           INSERT INTO restored_app_key_writes VALUES(NEW.owner_pubkey_hex); END;",
        )
        .unwrap();
    core.restore_device_sync_record_projection().unwrap();
    let written: i64 = shared
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM restored_app_key_writes", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        written as usize,
        core.app_keys.len(),
        "one complete save, not one per replayed head"
    );
    let saved = core.app_store.load_state().unwrap().unwrap();
    for event in &events {
        assert!(saved
            .app_keys
            .iter()
            .any(|known| known.owner_pubkey_hex == event.pubkey.to_hex()));
        assert!(core
            .protocol_engine
            .as_ref()
            .unwrap()
            .has_device_roster_entry_for_owner(
                event.pubkey,
                AppKeys::from_event(event).unwrap().get_all_devices()[0].identity_pubkey
            ));
    }
    assert!(
        !updates
            .try_iter()
            .any(|update| matches!(update, AppUpdate::FullState(_))),
        "the caller publishes the completed restoration, not intermediate authorization heads"
    );
    assert_eq!(core.batch_depth, 0);
    assert!(shared.lock().unwrap().is_autocommit());
}

#[test]
fn restored_authorization_projection_save_failure_preserves_delivery_ack_and_replay() {
    let (mut core, updates, _dir, owner, device, events) =
        restored_authorization_fixture("restore-save-failure");
    let sender = Keys::generate();
    let event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().unwrap(),
        &sender,
        "restore must commit before ack",
        200,
    );
    core.enter_batch();
    core.handle_relay_event(event.clone());
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        1
    );
    drain_app_updates(&updates);
    let shared = core.app_store.shared();
    shared
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TEMP TRIGGER reject_restored_app_keys BEFORE INSERT ON app_keys BEGIN
           SELECT RAISE(ABORT,'injected restored app save failure'); END;",
        )
        .unwrap();
    assert!(core.restore_device_sync_record_projection().is_err());
    assert_eq!(
        core.batch_depth, 1,
        "restore must retain its caller's batch"
    );
    assert!(core
        .pending_decrypted_delivery_acks
        .contains(&event.id.to_hex()));
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        1
    );
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::FullState(_))));
    let saved = core.app_store.load_state().unwrap().unwrap();
    assert!(!saved
        .app_keys
        .iter()
        .any(|known| known.owner_pubkey_hex == events[0].pubkey.to_hex()));
    shared
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_restored_app_keys")
        .unwrap();
    core.restore_device_sync_record_projection().unwrap();
    assert!(core.pending_decrypted_delivery_acks.is_empty());
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .pending_decrypted_deliveries_len_for_test(),
        0
    );
    core.exit_batch();
    assert_eq!(
        stored_pending_decrypted_delivery_count(&core, &owner, &device),
        0
    );
}

#[test]
fn restored_authorization_projection_page_failure_cannot_report_completion() {
    let (mut core, updates, _dir, _, _, _) = restored_authorization_fixture("restore-page-failure");
    let prefix = core.sync_record_prefix().unwrap();
    let shared = core.app_store.shared();
    shared
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_meta(key,value) VALUES(?1,x'01')",
            [format!("{prefix}invalid")],
        )
        .unwrap();
    core.pending_decrypted_delivery_acks
        .insert("not-yet-committed".into());
    let error = core.restore_device_sync_record_projection().unwrap_err();
    assert!(format!("{error:#}").contains("restore device-sync record page"));
    assert!(core
        .pending_decrypted_delivery_acks
        .contains("not-yet-committed"));
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::FullState(_))));
    assert_eq!(core.batch_depth, 0);
}

#[test]
fn restored_authorization_projection_save_failure_does_not_complete_restore_action() {
    let (mut core, updates, _dir, owner, device, _) =
        restored_authorization_fixture("restore-action-save-failure");
    let (sender, _receiver) = flume::unbounded();
    core.core_sender = sender;
    core.device_approval_relay_urls.clear();
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TEMP TRIGGER reject_restored_action BEFORE INSERT ON app_keys BEGIN
           SELECT RAISE(ABORT,'injected restore action save failure'); END;",
        )
        .unwrap();
    drain_app_updates(&updates);
    core.restore_account_bundle(
        Some(owner.secret_key().to_secret_hex()),
        &owner.public_key().to_hex(),
        &device.secret_key().to_secret_hex(),
    );
    assert!(core
        .state
        .toast
        .as_deref()
        .is_some_and(|error| error.contains("persist restored device-sync projection")));
    assert!(
        core.logged_in.is_none(),
        "failed app durability cannot complete account restoration"
    );
    for update in updates.try_iter() {
        match update {
            AppUpdate::FullState(state) => {
                assert!(state.busy.restoring_session || state.account.is_none())
            }
            AppUpdate::PersistAccountBundle { .. } => {
                panic!("failed restore must not export a successful account bundle")
            }
            _ => {}
        }
    }
}
