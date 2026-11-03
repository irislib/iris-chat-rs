#[test]
fn linked_phone_recovers_recipient_receipts_for_known_messages() {
    let mut pair = chat_read_sync_pair("receipt-negentropy");
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    for (id, status) in [
        ("delivered", DeliveryState::Received),
        ("seen", DeliveryState::Seen),
    ] {
        pair.a.push_outgoing_message_with_id(
            id.into(),
            &chat,
            id.into(),
            200,
            None,
            DeliveryState::Sent,
        );
        sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
        pair.a
            .apply_receipt_to_messages(&chat, &[id.into()], status, false, Some(&chat));
    }
    pair.a.persist_best_effort_inner();
    pair.a.threads.get_mut(&chat).unwrap().messages.clear();
    let (a_rx, b_rx) = install_receipt_test_link(&mut pair);
    pair.b.refresh_device_sync_on_foreground();
    let trace = pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
    assert!(trace
        .iter()
        .flat_map(|p| p["records"].as_array().into_iter().flatten())
        .any(|r| r["type"] == "recipientReceipt"));
    for (id, expected) in [
        ("delivered", DeliveryState::Received),
        ("seen", DeliveryState::Seen),
    ] {
        let message = pair.b.message_for_mutation(&chat, id).unwrap();
        assert_eq!(message.delivery, expected, "phone status for {id}");
        assert!(message
            .recipient_deliveries
            .iter()
            .any(|r| r.owner_pubkey_hex == chat && r.delivery == expected));
    }
}

#[test]
fn sibling_message_replay_preserves_received_receipts() {
    let mut pair = chat_read_sync_pair("receipt-replay");
    let chat = Keys::generate().public_key().to_hex();
    pair.a.push_outgoing_message_with_id(
        "m".into(),
        &chat,
        "original".into(),
        200,
        None,
        DeliveryState::Sent,
    );
    pair.a.apply_receipt_to_messages(
        &chat,
        &["m".into()],
        DeliveryState::Seen,
        false,
        Some(&chat),
    );
    // Heal a pre-upgrade row whose summary was already downgraded even
    // though the recipient's confirmed Seen survived on disk.
    pair.a.threads.get_mut(&chat).unwrap().messages[0].delivery = DeliveryState::Sent;
    pair.a.persist_best_effort_inner();
    let old = pair
        .a
        .app_store
        .load_messages_around(&chat, "m", 0, 0)
        .unwrap();
    assert_eq!(
        chats::chat_message_from_persisted(&old[0]).delivery,
        DeliveryState::Seen
    );
    pair.a.threads.get_mut(&chat).unwrap().messages[0] =
        chats::chat_message_from_persisted(&old[0]);
    for unloaded in [false, true] {
        if unloaded {
            pair.a.persist_best_effort_inner();
            pair.a.threads.get_mut(&chat).unwrap().messages.clear();
        }
        pair.a.apply_runtime_text_message(
            pair.owner.public_key(),
            Some(chat.clone()),
            "original".into(),
            200,
            None,
            Some("m".into()),
            None,
        );
        assert_eq!(
            pair.a.message_for_mutation(&chat, "m").unwrap().delivery,
            DeliveryState::Seen
        );
    }
}

type ReceiptTestChannel = flume::Receiver<super::device_sync_tcp::SendBatch>;

fn install_receipt_test_link(
    pair: &mut ChatReadSyncPair,
) -> (ReceiptTestChannel, ReceiptTestChannel) {
    for roster in pair.a.app_keys.values_mut() {
        roster
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    pair.b.app_keys = pair.a.app_keys.clone();
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
        endpoint,
        b_tx,
        vec![test_fips_peer(&pair.a_device)],
    );
    (a_rx, b_rx)
}

fn pump_receipt_test_link(
    pair: &mut ChatReadSyncPair,
    a_rx: &ReceiptTestChannel,
    b_rx: &ReceiptTestChannel,
) -> Vec<serde_json::Value> {
    let mut trace = Vec::new();
    for round in 0..256 {
        let a = drain_chat_read_negentropy(
            &mut pair.a,
            &pair.a_device,
            &mut pair.b,
            &pair.b_device,
            a_rx,
            &mut trace,
            false,
        );
        let b = drain_chat_read_negentropy(
            &mut pair.b,
            &pair.b_device,
            &mut pair.a,
            &pair.a_device,
            b_rx,
            &mut trace,
            false,
        );
        if !a && !b {
            break;
        }
        assert!(round < 255);
    }
    trace
}

#[test]
fn linked_phone_repairs_own_reads_after_sender_foregrounds() {
    let mut pair = chat_read_sync_pair("seen-after-foreground");
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    for core in [&mut pair.a, &mut pair.b] {
        chat_read_sync_incoming(core, &peer, "still unread", 199);
        chat_read_sync_incoming(core, &peer, "read on laptop", 200);
    }
    let (a_rx, b_rx) = install_receipt_test_link(&mut pair);
    pair.b.refresh_device_sync_on_foreground();
    pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
    // Only the laptop resumes. The phone keeps its existing stream and can
    // miss live read metadata, just as when a read falls outside its recent batch.
    pair.a.refresh_device_sync_on_foreground();
    pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
    pair.a.mark_messages_seen(&chat, &["read on laptop".into()]);
    pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
    assert_eq!(pair.b.threads[&chat].unread_count, 1);
    assert_chat_read_delivery(&pair.b, &chat, "read on laptop", true);
    assert_chat_read_delivery(&pair.b, &chat, "still unread", false);
    let row = pair
        .b
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat)
        .unwrap();
    assert_eq!(row.unread_count, 1, "chat-list badge presented to iOS");
    drop(pair.b);
    let mut restarted = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.b_device,
        pair._b_dir.path().to_string_lossy().into_owned(),
    );
    let saved = restarted.load_persisted().unwrap().unwrap();
    assert_eq!(
        saved
            .threads
            .iter()
            .find(|t| t.chat_id == chat)
            .unwrap()
            .unread_count,
        1
    );
    assert!(restarted.message_was_seen_on_own_device(&chat, 200, "read on laptop"));
    assert!(!restarted.message_was_seen_on_own_device(&chat, 199, "still unread"));
}

#[test]
fn live_recipient_receipts_require_a_linked_device_and_resist_stale_replays() {
    let mut pair = chat_read_sync_pair("live-recipient-receipts");
    let chat = Keys::generate().public_key().to_hex();
    pair.a.push_outgoing_message_with_id(
        "m".into(),
        &chat,
        "private body".into(),
        200,
        None,
        DeliveryState::Sent,
    );
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    let (a_rx, _) = install_receipt_test_link(&mut pair);
    let mut packets = Vec::new();
    for state in [DeliveryState::Received, DeliveryState::Seen] {
        pair.a
            .apply_receipt_to_messages(&chat, &["m".into()], state, false, Some(&chat));
        let batch = a_rx.try_recv().unwrap();
        assert_eq!(batch.peer, test_fips_peer(&pair.b_device));
        assert_eq!(batch.records.len(), 1);
        let bytes = batch.records[0].clone();
        let packet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(packet["type"], "recipientReceipts");
        assert!(!packet.to_string().contains("private body"));
        assert_eq!(packet["receipts"][0]["recipient"], chat);
        packets.push(bytes);
    }
    deliver_chat_read_packets(&mut pair.b, &Keys::generate(), &packets);
    assert_eq!(
        pair.b.message_for_mutation(&chat, "m").unwrap().delivery,
        DeliveryState::Sent
    );
    // Apply to a disk-only row. Both stale and duplicate facts can be replayed.
    pair.b.persist_best_effort_inner();
    pair.b.threads.get_mut(&chat).unwrap().messages.clear();
    for index in [1, 0, 1] {
        deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets[index..=index]);
    }
    assert!(pair.b.threads[&chat].messages.is_empty());
    let row = pair.b.message_for_mutation(&chat, "m").unwrap();
    assert_eq!(row.delivery, DeliveryState::Seen);
    assert_eq!(row.recipient_deliveries.len(), 1);
    assert_eq!(row.recipient_deliveries[0].delivery, DeliveryState::Seen);
    assert!(pair.b.chat_read_states.is_empty());
}

#[test]
fn receipt_repair_for_prelink_messages_respects_the_completed_history_grant() {
    for include_history in [false, true] {
        let mut pair = chat_read_sync_pair("prelink-receipt");
        let chat = Keys::generate().public_key().to_hex();
        // Both devices happen to have the old original. Only a full-history
        // grant allows its receipt to cross this link after initial transfer.
        for core in [&mut pair.a, &mut pair.b] {
            core.push_outgoing_message_with_id(
                "old".into(),
                &chat,
                "old body".into(),
                50,
                None,
                DeliveryState::Sent,
            );
        }
        let (a_rx, b_rx) = install_receipt_test_link(&mut pair);
        let link_id = "bd".repeat(32);
        pair.a
            .create_device_history_transfer(
                pair.b_device.public_key(),
                include_history,
                link_id.clone(),
            )
            .unwrap();
        pair.b
            .record_device_history_approver(&pair.a_device.public_key().to_hex(), 100, link_id)
            .unwrap();
        pair.b.refresh_device_sync_on_foreground();
        let setup = pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
        assert!(
            pair.a
                .device_history_transfer(&pair.b_device.public_key().to_hex())
                .unwrap()
                .complete,
            "initial transfer: {setup:?}"
        );
        pair.a.apply_receipt_to_messages(
            &chat,
            &["old".into()],
            DeliveryState::Seen,
            false,
            Some(&chat),
        );
        // Lose the live update, and inspect the actual recovery exchange.
        while a_rx.try_recv().is_ok() {}
        pair.b.refresh_device_sync_on_foreground();
        let trace = pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
        let records: Vec<_> = trace
            .iter()
            .flat_map(|p| p["records"].as_array().into_iter().flatten())
            .collect();
        assert_eq!(
            records.iter().any(|r| r["type"] == "recipientReceipt"),
            include_history
        );
        assert!(records.iter().all(|r| r["type"] != "message"));
        assert_eq!(
            pair.b.message_for_mutation(&chat, "old").unwrap().delivery,
            if include_history {
                DeliveryState::Seen
            } else {
                DeliveryState::Sent
            }
        );
    }
}

#[test]
fn linked_phone_merges_group_receipts_per_recipient_after_original_import() {
    let mut pair = chat_read_sync_pair("group-receipt");
    let chat = "group:receipt-test";
    let first = Keys::generate().public_key().to_hex();
    let second = Keys::generate().public_key().to_hex();
    let (a_rx, b_rx) = install_receipt_test_link(&mut pair);
    pair.a.push_outgoing_message_with_id(
        "group-message".into(),
        chat,
        "private text".into(),
        200,
        None,
        DeliveryState::Sent,
    );
    pair.a.apply_receipt_to_messages(
        chat,
        &["group-message".into()],
        DeliveryState::Seen,
        false,
        Some(&first),
    );
    pair.a.apply_receipt_to_messages(
        chat,
        &["group-message".into()],
        DeliveryState::Received,
        false,
        Some(&second),
    );
    while a_rx.try_recv().is_ok() {}
    pair.a.set_device_history_record_limit_for_test(1);
    pair.b.set_device_history_record_limit_for_test(1);
    pair.b.refresh_device_sync_on_foreground();
    pump_receipt_test_link(&mut pair, &a_rx, &b_rx);
    let row = pair.b.message_for_mutation(chat, "group-message").unwrap();
    assert_eq!(row.delivery, DeliveryState::Seen);
    assert_eq!(row.recipient_deliveries.len(), 2);
    assert!(row
        .recipient_deliveries
        .iter()
        .any(|d| d.owner_pubkey_hex == first && d.delivery == DeliveryState::Seen));
    assert!(row
        .recipient_deliveries
        .iter()
        .any(|d| d.owner_pubkey_hex == second && d.delivery == DeliveryState::Received));
}
