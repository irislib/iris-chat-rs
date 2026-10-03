fn drain_history_wire(
    source: &mut AppCore,
    source_key: &Keys,
    target: &mut AppCore,
    target_key: &Keys,
    records: &flume::Receiver<super::device_sync_tcp::SendBatch>,
    trace: &mut Vec<serde_json::Value>,
) -> bool {
    let mut progress = false;
    while let Ok(batch) = records.try_recv() {
        assert_eq!(batch.peer, test_fips_peer(target_key));
        for record in batch.records {
            trace.push(serde_json::from_slice(&record).unwrap());
            target.handle_device_sync_packet(
                &source_key.public_key().to_hex(),
                DEVICE_SYNC_PORT,
                &record,
            );
            progress = true;
        }
    }
    if let Some(record) = source.take_device_sync_control_for_test(test_fips_peer(target_key)) {
        trace.push(serde_json::from_slice(&record).unwrap());
        target.handle_device_sync_packet(
            &source_key.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            &record,
        );
        progress = true;
    }
    progress
}

#[test]
fn device_sync_negentropy_repairs_persisted_offline_gaps_and_preserves_deletions() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let (mut left, _, _left_dir) = logged_in_test_core_with_updates("negentropy-left", &owner, &a);
    let (mut right, _, _right_dir) =
        logged_in_test_core_with_updates("negentropy-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    left.create_device_history_transfer(b.public_key(), true, "ab".repeat(32))
        .unwrap();
    right
        .record_device_history_approver(&a.public_key().to_hex(), 100, "ab".repeat(32))
        .unwrap();
    for index in 0..180 {
        let id = format!("record-{index:03}");
        left.push_incoming_message_from(
            &chat_id,
            Some(id.clone()),
            format!("offline {index}"),
            20 + index,
            None,
            None,
            Some(chat_id.clone()),
            None,
        );
        if index % 3 != 0 {
            right.push_incoming_message_from(
                &chat_id,
                Some(id),
                format!("offline {index}"),
                20 + index,
                None,
                None,
                Some(chat_id.clone()),
                None,
            );
        }
    }
    left.persist_best_effort_inner();
    right.persist_best_effort_inner();
    left.threads.get_mut(&chat_id).unwrap().messages.clear(); // Exercise durable rows beyond the loaded projection.
    right
        .app_store
        .delete_message_locally(&chat_id, "record-003", None)
        .unwrap();
    let endpoint = left
        .runtime
        .block_on(
            fips_core::FipsEndpoint::builder()
                .without_system_tun()
                .bind(),
        )
        .unwrap();
    let endpoint = Arc::new(endpoint);
    let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    let request = serde_json::to_vec(
        &serde_json::json!({"type":"request", "v":1, "rosterAt":100, "historyReconcile":1}),
    )
    .unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    // Inventory is immutable, but a removed record must not be sent afterward.
    left.app_store
        .delete_message_locally(&chat_id, "record-006", None)
        .unwrap();
    for _ in 0..1024 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert!(
        trace
            .iter()
            .any(|packet| packet["type"] == "historyOpen" && packet["since"] == 0),
        "history did not start; local policy {:?}",
        right
            .device_history_transfer(&a.public_key().to_hex())
            .map(|record| record.since)
    );
    let transferred = trace
        .iter()
        .filter(|packet| packet["type"] == "historyMessages")
        .flat_map(|packet| packet["messages"].as_array().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        transferred.len(),
        58,
        "only absent, nondeleted records travel, not the full history"
    );
    assert!(transferred
        .iter()
        .all(|message| message["id"].as_str().unwrap()[7..].parse::<u64>().unwrap() % 3 == 0));
    assert!(
        !has_device_sync_message(&right, &chat_id, "record-003"),
        "a local deletion must not resurrect"
    );
    assert!(
        has_device_sync_message(&right, &chat_id, "record-000"),
        "private authorizer choice permits pre-link history"
    );
    assert_eq!(
        right
            .app_store
            .load_messages_before(&chat_id, "record-179", 500)
            .unwrap()
            .len(),
        177
    );
    assert!(
        !has_device_sync_message(&right, &chat_id, "record-006"),
        "withdrawn after inventory stays absent"
    );
    assert_eq!(right.device_history_session_count_for_test(), 0);
    assert_eq!(left.device_history_session_count_for_test(), 0);
    assert_eq!(
        right.state.device_history_sync.as_ref().unwrap().phase,
        crate::DeviceHistorySyncPhase::Waiting
    );
    assert_eq!(
        right
            .state
            .device_history_sync
            .as_ref()
            .unwrap()
            .imported_messages,
        25
    );
    assert!(
        !right
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .complete,
        "withheld record keeps initial transfer resumable"
    );
    trace.clear();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    for _ in 0..1024 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert_eq!(
        right.threads[&chat_id]
            .messages
            .iter()
            .filter(|message| message.id == "record-000")
            .count(),
        1
    );
    assert_eq!(
        right.state.device_history_sync.as_ref().unwrap().phase,
        crate::DeviceHistorySyncPhase::Complete
    );
    assert_eq!(
        right
            .state
            .device_history_sync
            .as_ref()
            .unwrap()
            .imported_messages,
        25
    );
    assert_eq!(
        right
            .state
            .device_history_sync
            .as_ref()
            .unwrap()
            .total_messages,
        Some(25)
    );
    assert!(
        left.device_history_transfer(&b.public_key().to_hex())
            .unwrap()
            .complete
    );
    assert!(
        right
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .complete
    );
    assert!(trace
        .iter()
        .filter(|packet| packet["type"] == "historyOpen" && packet["since"] == 0)
        .all(|packet| packet["until"] == 99 && packet["linkId"] == "ab".repeat(32)));
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn device_sync_history_pair_policy_survives_restart_and_excludes_other_siblings() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let c = Keys::generate();
    let (mut core, _, dir) = logged_in_test_core_with_updates("history-policy", &owner, &b);
    configure_test_device_sync_profile(&mut core, &owner, &a, &b, None);
    let mut extra = core.app_keys[&owner.public_key().to_hex()].devices[0].clone();
    extra.identity_pubkey_hex = c.public_key().to_hex();
    core.app_keys
        .get_mut(&owner.public_key().to_hex())
        .unwrap()
        .devices
        .push(extra);
    core.record_device_history_approver(&a.public_key().to_hex(), 100, "ab".repeat(32))
        .unwrap();
    let policy = serde_json::json!({"type":"historyPolicy","v":1,"linkAt":100,"linkId":"ab".repeat(32),"since":0});
    core.handle_device_sync_packet(
        &c.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&policy).unwrap(),
    );
    assert!(
        !core
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .policy_known
    );
    core.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&policy).unwrap(),
    );
    assert_eq!(
        core.device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .since,
        0
    );
    let keys = core.app_keys.clone();
    drop(core);
    let mut reopened =
        logged_in_test_core_at_data_dir(&owner, &b, dir.path().to_string_lossy().into_owned());
    reopened.app_keys = keys;
    reopened.restore_device_history_progress();
    assert_eq!(
        reopened.state.device_history_sync.as_ref().unwrap().phase,
        crate::DeviceHistorySyncPhase::Waiting
    );
    let packet = serde_json::json!({"type":"snapshot","v":1,"rosterAt":0,"chats":[{"id":a.public_key().to_hex(),"updatedAt":10}],"appKeys":[],"groups":[{"id":"history-policy-group","name":"Friends","createdBy":owner.public_key().to_hex(),"members":[owner.public_key().to_hex(),a.public_key().to_hex()],"admins":[owner.public_key().to_hex()],"protocol":"pairwise_fanout_v1","revision":1,"createdAt":5,"updatedAt":10,"accepted":true}],"messages":[{"chatId":a.public_key().to_hex(),"id":"old-history","body":"aGk=","author":a.public_key().to_hex(),"createdAt":99}]});
    reopened.handle_device_sync_packet(
        &c.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&packet).unwrap(),
    );
    assert!(
        !has_device_sync_message(&reopened, &a.public_key().to_hex(), "old-history"),
        "other siblings never receive old-history admission"
    );
    assert!(reopened.threads.contains_key(&a.public_key().to_hex()));
    assert!(reopened.groups.contains_key("history-policy-group"));
    let mut new_message = packet.clone();
    new_message["messages"][0]["createdAt"] = serde_json::json!(100);
    new_message["messages"][0]["id"] = serde_json::json!("future-history");
    reopened.handle_device_sync_packet(
        &c.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&new_message).unwrap(),
    );
    assert!(has_device_sync_message(
        &reopened,
        &a.public_key().to_hex(),
        "future-history"
    ));
    // Exact pairing operation changes even when a revoked/relinked key has the same second timestamp.
    reopened
        .record_device_history_approver(&a.public_key().to_hex(), 100, "cd".repeat(32))
        .unwrap();
    reopened.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&policy).unwrap(),
    );
    assert!(
        !reopened
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .policy_known
    );
    let list_only = serde_json::json!({"type":"historyPolicy","v":1,"linkAt":100,"linkId":"cd".repeat(32),"since":100});
    reopened.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&list_only).unwrap(),
    );
    reopened.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&packet).unwrap(),
    );
    assert!(!has_device_sync_message(
        &reopened,
        &a.public_key().to_hex(),
        "old-history"
    ));
    let mut broaden = list_only;
    broaden["since"] = serde_json::json!(0);
    reopened.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&broaden).unwrap(),
    );
    assert_eq!(
        reopened
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .since,
        100
    );
}

#[test]
fn device_sync_history_choice_is_private_pair_only_and_cancels_revoked_sessions() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("history-session-policy", &owner, &a);
    configure_test_device_sync_profile(&mut core, &owner, &a, &b, None);
    core.pending_relay_publishes.clear();
    core.create_device_history_transfer(b.public_key(), true, "ab".repeat(32))
        .unwrap();
    assert!(
        core.pending_relay_publishes.is_empty(),
        "history choice is never a public announcement"
    );
    for packet in core.build_device_sync_packets_for_test(100, false) {
        let packet = serde_json::from_slice::<serde_json::Value>(&packet).unwrap();
        assert!(packet.get("historyGrants").is_none());
        assert!(packet.get("historyPolicy").is_none());
        assert_eq!(packet["messages"], serde_json::json!([]));
    }
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
    core.install_device_sync_sender_for_test(endpoint.clone(), sender, vec![test_fips_peer(&b)]);
    let source = b.public_key().to_hex();
    let request = serde_json::json!({"type":"request","v":1,"rosterAt":100,"historyReconcile":1,"historySince":0});
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&request).unwrap(),
    );
    let metadata = records
        .try_recv()
        .unwrap()
        .records
        .into_iter()
        .map(|record| serde_json::from_slice::<serde_json::Value>(&record).unwrap())
        .collect::<Vec<_>>();
    assert!(metadata
        .iter()
        .any(|packet| packet["type"] == "historyPolicy" && packet["since"] == 0));
    let mut engine = nostr_pubsub_reconcile::Session::new(
        [],
        nostr_pubsub_reconcile::Filter {
            since: 0,
            until: 99,
        },
        Default::default(),
    )
    .unwrap();
    let frame = engine
        .initiate()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let session = "aa".repeat(16);
    let mut open = serde_json::json!({"type":"historyOpen","v":1,"session":session,"since":0,"until":99,"frame":frame,"linkId":"cd".repeat(32)});
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&open).unwrap(),
    );
    assert_eq!(core.device_history_session_count_for_test(), 0);
    open["linkId"] = serde_json::json!("ab".repeat(32));
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&open).unwrap(),
    );
    assert_eq!(core.device_history_session_count_for_test(), 1);
    records.try_recv().unwrap();
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(
        core.device_history_session_count_for_test(),
        0,
        "reconnect cancels old transcript"
    );
    while records.try_recv().is_ok() {}
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&open).unwrap(),
    );
    assert_eq!(core.device_history_session_count_for_test(), 1);
    while records.try_recv().is_ok() {}
    core.app_keys
        .get_mut(&owner.public_key().to_hex())
        .unwrap()
        .devices
        .retain(|device| device.identity_pubkey_hex != source);
    let need =
        serde_json::json!({"type":"historyNeed","v":1,"session":session,"ids":["00".repeat(32)]});
    core.handle_device_sync_packet(
        &source,
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&need).unwrap(),
    );
    assert_eq!(core.device_history_session_count_for_test(), 0);
    assert!(records.try_recv().is_err());
    core.runtime.block_on(endpoint.shutdown()).unwrap();
}

#[test]
fn device_sync_history_bounded_page_fallback_completes_only_after_durable_import() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let chat = Keys::generate().public_key().to_hex();
    let (mut left, _, _left_dir) =
        logged_in_test_core_with_updates("history-page-source", &owner, &a);
    let (mut right, _, _right_dir) =
        logged_in_test_core_with_updates("history-page-target", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    for known in left.app_keys.values_mut() {
        known
            .devices
            .sort_by(|a, b| a.identity_pubkey_hex.cmp(&b.identity_pubkey_hex));
    }
    right.app_keys = left.app_keys.clone();
    left.create_device_history_transfer(b.public_key(), true, "ab".repeat(32))
        .unwrap();
    right
        .record_device_history_approver(&a.public_key().to_hex(), 100, "ab".repeat(32))
        .unwrap();
    for index in 0..40 {
        left.push_incoming_message_from(
            &chat,
            Some(format!("old-{index}")),
            format!("old {index}"),
            20 + index,
            None,
            None,
            Some(chat.clone()),
            None,
        );
    }
    left.push_incoming_message_from(
        &chat,
        Some("future".to_string()),
        "future".to_string(),
        102,
        None,
        None,
        Some(chat.clone()),
        None,
    );
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
    let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(128, 64 * 1024);
    let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(128, 64 * 1024);
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    let request = serde_json::json!({"type":"request","v":1,"rosterAt":100,"historyReconcile":1});
    left.handle_device_sync_packet(
        &b.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&request).unwrap(),
    );
    let mut trace = Vec::new();
    drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
    drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
    // A responder whose bounded inventory cannot be built ends the transcript.
    // The recipient must switch to authenticated cursor pages and await its terminal marker.
    let response = left_rx.try_recv().unwrap();
    let session = serde_json::from_slice::<serde_json::Value>(&response.records[0]).unwrap()
        ["session"]
        .clone();
    let done = serde_json::json!({"type":"historyDone","v":1,"session":session});
    left.handle_device_sync_packet(
        &b.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&done).unwrap(),
    );
    right.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&done).unwrap(),
    );
    assert!(
        !right
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .complete
    );
    // Live traffic must remain usable while the old cursor copy is still pending.
    let live = serde_json::json!({"type":"snapshot","v":1,
        "rosterAt":100,"messages":[{"chatId":chat,"id":"future","body":"ZnV0dXJl","createdAt":102,"author":chat}]
    });
    right.handle_device_sync_packet(
        &a.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&live).unwrap(),
    );
    assert!(has_device_sync_message(&right, &chat, "future"));
    assert_eq!(
        right
            .state
            .device_history_sync
            .as_ref()
            .unwrap()
            .imported_messages,
        0
    );
    for _ in 0..1024 {
        let x = drain_history_wire(&mut left, &a, &mut right, &b, &left_rx, &mut trace);
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y {
            break;
        }
    }
    assert!(trace
        .iter()
        .any(|packet| packet["type"] == "historyPageEnd"));
    assert!(trace.iter().any(|packet| packet["type"] == "request"
        && packet["page"]["kind"] == "messages"
        && packet["linkId"] == "ab".repeat(32)));
    assert_eq!(
        right
            .app_store
            .load_messages_before(&chat, "future", 100)
            .unwrap()
            .len(),
        40
    );
    assert!(has_device_sync_message(&right, &chat, "future"));
    assert!(
        right
            .device_history_transfer(&a.public_key().to_hex())
            .unwrap()
            .complete
    );
    assert!(
        left.device_history_transfer(&b.public_key().to_hex())
            .unwrap()
            .complete
    );
    assert_eq!(
        right
            .state
            .device_history_sync
            .as_ref()
            .unwrap()
            .imported_messages,
        40
    );
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}
