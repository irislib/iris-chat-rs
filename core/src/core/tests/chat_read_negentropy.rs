#[test]
fn chat_read_negentropy_recovers_private_progress_before_history_after_restart() {
    for include_history in [false, true] {
        run_chat_read_negentropy(include_history, true);
    }
}

#[test]
fn chat_read_negentropy_keeps_legacy_peers_on_snapshot_sync() {
    run_chat_read_negentropy(false, false);
}

fn run_chat_read_negentropy(include_history: bool, supports_read_states: bool) {
    let mut pair = chat_read_sync_pair("read-negentropy");
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    let group_chat = "group:read-negentropy";
    for (id, time) in [("old-read", 50), ("recent-read", 200)] {
        chat_read_sync_incoming(&mut pair.a, &peer, id, time);
    }
    pair.a.set_chat_unread(&chat, false);
    pair.a.push_incoming_message_from(
        group_chat,
        Some("group-read".into()),
        "Group message".into(),
        200,
        None,
        None,
        Some(chat.clone()),
        None,
    );
    pair.a
        .mark_messages_seen(group_chat, &["group-read".into()]);
    let expected = pair.a.chat_read_states.clone();
    pair.a
        .create_device_history_transfer(
            pair.b_device.public_key(),
            include_history,
            "af".repeat(32),
        )
        .unwrap();
    pair.b
        .record_device_history_approver(&pair.a_device.public_key().to_hex(), 100, "af".repeat(32))
        .unwrap();
    pair.a.persist_best_effort_inner();
    assert_eq!(protocol_send_log_count(&pair.a, "receipt"), 0);
    drop(pair.a);
    let mut left = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.a_device,
        pair.a_dir.path().to_string_lossy().into_owned(),
    );
    left.load_persisted().unwrap();
    assert!(!left.preferences.send_read_receipts);
    assert_eq!(left.chat_read_states, expected);
    // This fixture loads durable data without running full account restoration.
    configure_test_device_sync_profile(
        &mut left,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
        None,
    );
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    left.ensure_thread_record(&chat, 200);
    left.ensure_thread_record(group_chat, 200);
    pair.b.app_keys = left.app_keys.clone();
    assert!(
        left.device_sync_peer_is_authorized(&pair.b_device.public_key().to_hex()),
        "restarted reader must authorize sibling"
    );
    assert!(
        pair.b
            .device_sync_peer_is_authorized(&pair.a_device.public_key().to_hex()),
        "sibling must authorize reader"
    );
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
    left.install_device_sync_sender_for_test(
        endpoint.clone(),
        left_tx,
        vec![test_fips_peer(&pair.b_device)],
    );
    pair.b.install_device_sync_sender_for_test(
        endpoint,
        right_tx,
        vec![test_fips_peer(&pair.a_device)],
    );
    // Exercise partitioning as well as state-before-history ordering. Omit
    // privateEvents so ordering must follow the read-state capability itself.
    left.set_device_history_record_limit_for_test(1);
    pair.b.set_device_history_record_limit_for_test(1);
    let request = serde_json::to_vec(&serde_json::json!({
        "type":"request", "v":1, "rosterAt":100, "recordReconcile":1,
        "readStates": supports_read_states.then_some(1),
    }))
    .unwrap();
    left.handle_device_sync_packet(
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
    for round in 0..2048 {
        let a = drain_chat_read_negentropy(
            &mut left,
            &pair.a_device,
            &mut pair.b,
            &pair.b_device,
            &left_rx,
            &mut trace,
            supports_read_states,
        );
        let b = drain_chat_read_negentropy(
            &mut pair.b,
            &pair.b_device,
            &mut left,
            &pair.a_device,
            &right_rx,
            &mut trace,
            supports_read_states,
        );
        if !a && !b {
            break;
        }
        assert!(round < 2047, "reconciliation did not finish");
    }
    let read_records = trace
        .iter()
        .flat_map(|packet| packet["records"].as_array().into_iter().flatten())
        .filter(|record| record["type"] == "chatRead")
        .count();
    if !supports_read_states {
        assert_eq!(
            read_records, 0,
            "old peers cannot decode new record variants"
        );
        assert!(
            pair.b.chat_read_states.is_empty(),
            "snapshot progress was intentionally dropped"
        );
        sync_chat_reads(&left, &mut pair.b, &pair.a_device, false);
        assert_eq!(
            pair.b.chat_read_states, expected,
            "legacy snapshots still carry read progress"
        );
        return;
    }
    assert!(
        read_records > 0,
        "negentropy must repair dropped snapshot read progress: {trace:?}"
    );
    assert_eq!(pair.b.chat_read_states, expected);
    assert_eq!(pair.b.app_store.load_chat_read_states().unwrap(), expected);
    assert_eq!(pair.b.threads[&chat].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat, "recent-read", true);
    assert_eq!(
        pair.b
            .app_store
            .message_exists_or_deleted(&chat, Some("old-read"), None)
            .unwrap(),
        include_history
    );
    assert_eq!(protocol_send_log_count(&pair.b, "receipt"), 0);
    drop(pair.b);
    let mut right = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.b_device,
        pair._b_dir.path().to_string_lossy().into_owned(),
    );
    right.load_persisted().unwrap();
    // The group original can arrive after the read record and another restart.
    right.push_incoming_message_from(
        group_chat,
        Some("group-read".into()),
        "Group message".into(),
        200,
        None,
        None,
        Some(chat.clone()),
        None,
    );
    assert_chat_read_delivery(&right, group_chat, "group-read", true);
    assert_eq!(right.threads[group_chat].unread_count, 0);
    chat_read_sync_incoming(&mut right, &peer, "unseen-same-second", 200);
    assert_chat_read_delivery(&right, &chat, "unseen-same-second", false);
    assert_eq!(right.threads[&chat].unread_count, 1);
}

fn drain_chat_read_negentropy(
    source: &mut AppCore,
    source_key: &Keys,
    target: &mut AppCore,
    target_key: &Keys,
    receiver: &flume::Receiver<super::device_sync_tcp::SendBatch>,
    trace: &mut Vec<serde_json::Value>,
    suppress_seen_notifications: bool,
) -> bool {
    let mut packets = Vec::new();
    while let Ok(batch) = receiver.try_recv() {
        assert_eq!(batch.peer, test_fips_peer(target_key));
        packets.extend(batch.records);
    }
    if let Some(packet) = source.take_device_sync_control_for_test(test_fips_peer(target_key)) {
        packets.push(packet);
    }
    let progress = !packets.is_empty();
    for packet in packets {
        let mut packet: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        // Simulate missed/stale read metadata while delivering the rest of the
        // normal negotiation. Only negentropy can provide the missing progress.
        if packet["type"] == "snapshot" {
            for chat in packet["chats"].as_array_mut().into_iter().flatten() {
                chat.as_object_mut().unwrap().remove("readState");
            }
        }
        let previous = target.state.chat_list.clone();
        target.handle_device_sync_packet(
            &source_key.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &serde_json::to_vec(&packet).unwrap(),
        );
        if suppress_seen_notifications {
            let mut preferences = target.state.preferences.clone();
            preferences.desktop_notifications_enabled = true;
            assert!(
                super::notifications::decide_notifications(
                    &previous,
                    &target.state.chat_list,
                    &preferences,
                    false,
                    None,
                )
                .is_empty(),
                "already-read history must not notify during catch-up: {packet}"
            );
        }
        trace.push(packet);
    }
    progress
}
