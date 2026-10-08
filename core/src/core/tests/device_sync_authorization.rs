#[test]
fn device_sync_authorization_changes_even_when_routing_peers_stay_the_same() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let contact = Keys::generate();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("sync-authorization", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let owner_hex = owner.public_key().to_hex();
    // A previously linked device may also remain a routable contact device.
    let mut foreign_roster = core.app_keys[&owner_hex].clone();
    foreign_roster.owner_pubkey_hex = contact.public_key().to_hex();
    core.app_keys.insert(contact.public_key().to_hex(), foreign_roster);
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!("IPv4 fixture");
    };
    drop(socket);
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(core.device_sync_has_sibling_tcp_for_test());
    core.app_keys.get_mut(&owner_hex).unwrap().devices.retain(|device| {
        device.identity_pubkey_hex != sibling.public_key().to_hex()
    });
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(!core.device_sync_has_sibling_tcp_for_test(),
        "revocation must retire the private service even when the device remains a routing contact");
    assert!(core.device_sync_endpoint_for_test().is_some(), "shared routing remains available");
    core.stop_device_sync();
}

#[test]
fn device_sync_locally_revoked_device_has_no_private_sync_peers() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("sync-local-revocation", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    assert!(core.device_sync_peer_is_authorized(&sibling.public_key().to_hex()));
    core.app_keys.get_mut(&owner.public_key().to_hex()).unwrap().devices.retain(|device| {
        device.identity_pubkey_hex != local.public_key().to_hex()
    });
    assert!(!core.device_sync_peer_is_authorized(&sibling.public_key().to_hex()));
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = socket.local_addr().unwrap() else {
        unreachable!("IPv4 fixture");
    };
    drop(socket);
    core.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    assert!(!core.device_sync_has_sibling_tcp_for_test(),
        "a revoked local device must not start a private history service");
    core.stop_device_sync();
}

#[test]
fn device_sync_stale_runtime_cannot_export_metadata_to_revoked_sibling() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _, _dir) = logged_in_test_core_with_updates("sync-stale-runtime", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let endpoint = Arc::new(core.runtime.block_on(
        fips_core::FipsEndpoint::builder().without_system_tun().bind()).unwrap());
    let (sender, records) = DeviceSyncTcpSender::test_channel(64, 64 * 1024);
    core.install_device_sync_sender_for_test(endpoint.clone(), sender, vec![test_fips_peer(&sibling)]);
    core.broadcast_device_sync_snapshot();
    assert!(records.try_recv().is_ok(), "the authorized positive control must export metadata");
    while records.try_recv().is_ok() {}
    core.app_keys.get_mut(&owner.public_key().to_hex()).unwrap().devices.retain(|device| {
        device.identity_pubkey_hex != sibling.public_key().to_hex()
    });
    // Exercise the window before runtime reconciliation replaces its cached allowlist.
    core.broadcast_device_sync_snapshot();
    assert!(records.is_empty(), "current account authorization must gate every metadata export");
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
    let (mut core, _, _dir) = logged_in_test_core_with_updates("sync-private-protocol", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let owner_hex = owner.public_key().to_hex();
    let sibling_hex = sibling.public_key().to_hex();
    let mut foreign_roster = core.app_keys[&owner_hex].clone();
    foreign_roster.owner_pubkey_hex = foreign_owner.public_key().to_hex();
    foreign_roster.devices[1].identity_pubkey_hex = foreign_device.public_key().to_hex();
    // The other account may even claim our local key. Its roster is never authority for ours.
    core.app_keys.insert(foreign_owner.public_key().to_hex(), foreign_roster);
    let original_roster = core.app_keys[&owner_hex].clone();
    let endpoint = Arc::new(core.runtime.block_on(
        fips_core::FipsEndpoint::builder().without_system_tun().bind()).unwrap());
    let (sender, records) = DeviceSyncTcpSender::test_channel(64, 64 * 1024);
    core.install_device_sync_sender_for_test(endpoint.clone(), sender, vec![test_fips_peer(&sibling)]);
    let mut engine = nostr_pubsub_reconcile::Session::new(
        [], nostr_pubsub_reconcile::Filter { since: 0, until: 0 }, Default::default()).unwrap();
    let frame = engine.initiate().unwrap().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let session = "ab".repeat(16);
    let request = serde_json::json!({"type":"request","v":1,"rosterAt":100,"recordReconcile":1});
    let open = serde_json::json!({"type":"historyOpen","v":1,"scope":"state","session":session,"since":0,"until":0,"frame":frame});
    for packet in [&request, &open] {
        core.handle_device_sync_packet(&sibling_hex, DEVICE_SYNC_PORT, &serde_json::to_vec(packet).unwrap());
    }
    assert_eq!(core.device_history_session_count_for_test(), 1,
        "an authorized sibling must open real negentropy reconciliation");
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
        request.clone(), open.clone(), forged_roster, snapshot.clone(),
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
    for source in [foreign_owner.public_key().to_hex(), foreign_device.public_key().to_hex(),
        unknown.public_key().to_hex(), owner_hex.clone(), local.public_key().to_hex(),
        local.public_key().to_hex().to_uppercase()] {
        assert!(!core.device_sync_peer_is_authorized(&source));
        for packet in &attacks {
            core.handle_device_sync_packet(&source, DEVICE_SYNC_PORT, &serde_json::to_vec(packet).unwrap());
        }
        assert!(records.is_empty(), "unapproved source must never receive a private response");
        assert!(!core.threads.contains_key(&chat), "unapproved source must never import a private record");
        assert_eq!(core.app_keys[&owner_hex], original_roster, "an unapproved source cannot authorize itself");
        assert_eq!(core.device_history_session_count_for_test(), 1, "another account cannot hijack or cancel a sibling's session");
    }
    core.handle_device_sync_packet(&sibling_hex, DEVICE_SYNC_PORT, &serde_json::to_vec(&snapshot).unwrap());
    assert!(has_device_sync_message(&core, &chat, "private-sync-message"),
        "the same private snapshot must import from the authorized sibling");
    while records.try_recv().is_ok() {}
    // Revoke the local device while the in-memory negentropy transcript still exists.
    core.app_keys.get_mut(&owner_hex).unwrap().devices.retain(|device| {
        device.identity_pubkey_hex != local.public_key().to_hex()
    });
    for packet in &attacks {
        core.handle_device_sync_packet(&sibling_hex, DEVICE_SYNC_PORT, &serde_json::to_vec(packet).unwrap());
    }
    assert!(records.is_empty(), "local revocation closes both directions of private sync");
    assert_eq!(core.device_history_session_count_for_test(), 0);
    core.runtime.block_on(endpoint.shutdown()).unwrap();
}
