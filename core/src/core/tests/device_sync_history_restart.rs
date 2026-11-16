use super::*;

#[test]
fn foreground_repairs_missed_group_reactions_on_an_existing_device_connection() {
    foreground_repairs_group_reactions(false);
}

#[test]
fn foreground_restarts_interrupted_group_reaction_reconciliation_without_waiting_for_ttl() {
    foreground_repairs_group_reactions(true);
}

fn foreground_repairs_group_reactions(interrupted: bool) {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let reactor = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let reactor_hex = reactor.public_key().to_hex();
    let chat = "group:foreground-reactions";
    let (mut left, _, _left_dir) = logged_in_test_core_with_updates("foreground-left", &owner, &a);
    let (mut right, _, _right_dir) =
        logged_in_test_core_with_updates("foreground-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    let group = serde_json::to_vec(&serde_json::json!({
        "v":1,"type":"snapshot","rosterAt":100,"groups":[{
            "id":"foreground-reactions","name":"Reactions","createdBy":owner_hex,
            "members":[owner_hex,reactor_hex],"admins":[owner_hex],
            "revision":1,"createdAt":100,"updatedAt":100,"accepted":true
        }],"messages":[]
    }))
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &group);
    right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT, &group);
    for core in [&mut left, &mut right] {
        core.push_incoming_message_from(
            chat,
            Some("target".into()),
            "A message with a missed reaction".into(),
            200,
            None,
            None,
            Some(owner_hex.clone()),
            None,
        );
        core.persist_best_effort_inner();
    }
    // Retain a real runtime through AppForegrounded, but control delivery so
    // neither relay replay nor a TCP reconnect can accidentally repair the gap.
    let std::net::SocketAddr::V4(rendezvous) = reserve_tcp_addr() else {
        unreachable!()
    };
    right.reconcile_device_sync_at_rendezvous_for_test(rendezvous);
    let endpoint = right.device_sync_endpoint_for_test().unwrap_or_else(|| {
        panic!("sync runtime did not start: {:?}", right.debug_log);
    });
    let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.replace_device_sync_sender_for_test(right_tx);
    let request = serde_json::to_vec(&serde_json::json!({
        "type":"request", "v":1, "rosterAt":100, "recordReconcile":1
    }))
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    for _ in 0..64 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert_eq!(right.device_history_session_count_for_test(), 0);
    assert_eq!(left.device_history_session_count_for_test(), 0);
    assert!(left.capture_device_sync_control(
        chat,
        "missed-reaction",
        &reactor_hex,
        201,
        REACTION_KIND,
        "👍",
        &[nostr::Tag::parse(["e", "target"]).unwrap()],
    ));
    if interrupted {
        drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        assert!(right.device_history_session_count_for_test() > 0);
    }
    // Lose the broadcast (or the in-flight inventory response) while inactive.
    while left_rx.try_recv().is_ok() {}
    assert!(right.threads[chat].messages[0].reactors.is_empty());
    right.handle_action(AppAction::AppForegrounded);
    assert!(
        Arc::ptr_eq(&endpoint, &right.device_sync_endpoint_for_test().unwrap()),
        "foreground catch-up must reuse the existing connection"
    );
    trace.clear();
    for _ in 0..64 {
        let x = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        let y = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert!(right.threads[chat].messages[0].reactors.iter().any(|reaction|
        reaction.author == reactor_hex && reaction.emoji == "👍"),
        "foreground must request missing reactions without a reconnect or the 120-second session expiry");
    assert_eq!(right.device_history_session_count_for_test(), 0);
    assert_eq!(left.device_history_session_count_for_test(), 0);
    right.stop_device_sync_now();
}

#[test]
fn device_sync_metadata_refresh_restarts_an_inflight_record_request() {
    sync_metadata_refresh_during_records(Refresh::Receiver, false);
}

#[test]
fn device_sync_record_request_completes_without_refresh() {
    sync_metadata_refresh_during_records(Refresh::None, false);
}

#[test]
fn device_sync_history_survives_a_direct_carrier_disconnect() {
    sync_metadata_refresh_during_records(Refresh::None, true);
}

#[test]
fn linked_device_pulling_its_own_history_does_not_cancel_our_missing_message() {
    sync_metadata_refresh_during_records(Refresh::Sender, false);
}

#[derive(Debug)]
enum Refresh {
    None,
    Receiver,
    Sender,
}

fn sync_metadata_refresh_during_records(refresh: Refresh, carrier_changed: bool) {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let contact = Keys::generate();
    let chat = contact.public_key().to_hex();
    let (mut left, _, _left_dir) = logged_in_test_core_with_updates("refresh-left", &owner, &a);
    let (mut right, _, _right_dir) = logged_in_test_core_with_updates("refresh-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    left.push_incoming_message_from(
        &chat,
        Some("target".into()),
        "message".into(),
        200,
        None,
        None,
        Some(chat.clone()),
        None,
    );
    assert!(left.capture_device_sync_control(
        &chat,
        "removed",
        &chat,
        202,
        REACTION_KIND,
        "",
        &[nostr::Tag::parse(["e", "target"]).unwrap()],
    ));
    left.persist_best_effort_inner();
    let endpoint = Arc::new(
        left.runtime
            .block_on(
                fips_core::FipsEndpoint::builder()
                    .without_system_tun()
                    .bind(),
            )
            .unwrap(),
    );
    let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    let request = serde_json::to_vec(&serde_json::json!({
        "type":"request", "v":1, "rosterAt":100, "recordReconcile":1,
    }))
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    assert!(right.device_history_session_count_for_test() > 0);
    if carrier_changed {
        // The end-to-end stream remains usable through the routed transport;
        // losing a direct link must not cancel the active record exchange.
        right.fips_nearby_links = vec![crate::updates::FipsNearbyLinkSnapshot {
            device_pubkey_hex: a.public_key().to_hex(),
            transport_type: "websocket".into(),
            transport_addr: None,
        }];
        right.update_fips_connection_links(Vec::new());
        assert!(right.device_history_session_count_for_test() > 0);
    }
    // A metadata refresh can arrive between the inventory response and its
    // record request. The other device must not wait on a silently lost round.
    match refresh {
        Refresh::None => {}
        Refresh::Receiver => {
            left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request)
        }
        Refresh::Sender => left.refresh_device_sync_on_foreground(),
    }
    for _ in 0..64 {
        let x = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        let y = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert!(
        has_device_sync_message(&right, &chat, "target"),
        "refresh={refresh:?} stranded pending history: {}",
        serde_json::to_string(&trace).unwrap()
    );
    assert!(trace
        .iter()
        .filter(|packet| packet["type"] == "historyRecords")
        .flat_map(|packet| packet["records"].as_array().unwrap())
        .any(|record| record["type"] == "reaction" && record["reaction"]["id"] == "removed"));
    assert_eq!(right.device_history_session_count_for_test(), 0);
    assert_eq!(left.device_history_session_count_for_test(), 0);
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn opening_a_chat_recovers_a_missing_direct_message_from_a_linked_device() {
    let mut pair = chat_read_sync_pair("chat-open-missing-message");
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    for core in [&mut pair.a, &mut pair.b] {
        core.preferences.accepted_owner_pubkeys.push(chat.clone());
    }
    pair.a.push_outgoing_message_with_id(
        "phone-only".into(),
        &chat,
        "sent from the phone".into(),
        200,
        None,
        DeliveryState::Sent,
    );
    pair.a.persist_best_effort_inner();
    pair.a.threads.get_mut(&chat).unwrap().messages.clear();
    let (phone, mac) = install_receipt_test_link(&mut pair);

    pair.b.open_chat(&chat);
    pair.b.open_chat_finalize(&chat);
    let trace = pump_receipt_test_link(&mut pair, &phone, &mac);
    let request = trace
        .iter()
        .find(|packet| packet["type"] == "request")
        .expect("history request on chat open");
    assert_eq!(request["recordReconcile"], 1);
    assert!(!request.to_string().contains(&chat));
    assert!(trace
        .iter()
        .any(|packet| packet["type"] == "historyOpen" && packet["scope"] == "history"));
    let recovered = pair
        .b
        .message_for_mutation(&chat, "phone-only")
        .expect("phone message on the Mac");
    assert!(recovered.is_outgoing);
    assert_eq!(recovered.created_at_secs, 200);
    assert_eq!(recovered.body, "sent from the phone");
    pair.b.open_chat_finalize(&chat);
    let repeat = pump_receipt_test_link(&mut pair, &phone, &mac);
    assert!(!repeat.iter().any(|packet| packet["type"] == "request"));
}
