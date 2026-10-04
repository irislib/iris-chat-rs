#[test]
fn device_sync_mutations_reject_unknown_and_pre_link_targets_on_import() {
    for unknown in [false, true] {
        run_mutation_import_privacy(Some(unknown), false);
    }
}

#[test]
fn device_sync_mutations_import_originals_before_controls_and_retry_split_targets() {
    for partitioned in [false, true] {
        run_mutation_import_privacy(None, partitioned);
    }
}

fn run_mutation_import_privacy(tamper: Option<bool>, partitioned: bool) {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let contact = Keys::generate();
    let chat = contact.public_key().to_hex();
    let (mut left, _, _left_dir) = logged_in_test_core_with_updates("mutation-privacy-left", &owner, &a);
    let (mut right, _, _right_dir) = logged_in_test_core_with_updates("mutation-privacy-right", &owner, &b);
    configure_test_device_sync_profile(&mut left, &owner, &a, &b, None);
    right.app_keys = left.app_keys.clone();
    left.create_device_history_transfer(b.public_key(), false, "bb".repeat(32)).unwrap();
    right.record_device_history_approver(&a.public_key().to_hex(), 100, "bb".repeat(32)).unwrap();
    left.push_incoming_message_from(&chat, Some("original".into()), "Original".into(),
        200, None, None, Some(chat.clone()), None);
    if tamper == Some(false) {
        right.push_incoming_message_from(&chat, Some("private-old".into()), "Old original".into(),
            50, None, None, Some(chat.clone()), None);
    }
    // The control's hash sorts before the original: with one-record partitions
    // the first pass must defer it, then retry after importing the later target.
    let original_id = {
        use sha2::Digest;
        sha2::Sha256::digest(serde_json::json!([chat, "original"]).to_string().as_bytes())
    };
    let control_id = (0..1000).map(|n| format!("edit-{n}")).find(|id| {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(serde_json::json!(["messageMutation", chat, id]).to_string().as_bytes());
        digest.as_slice() < original_id.as_slice()
    }).unwrap();
    assert!(left.capture_device_sync_control(&chat, &control_id, &chat, 201,
        MESSAGE_EDIT_KIND, "PRIVATE REPLACEMENT", &[nostr::Tag::parse(["e", "original"]).unwrap()]));
    assert!(left.capture_device_sync_control(
        &chat, "orphan-control", &chat, 202, MESSAGE_EDIT_KIND, "UNSHARED UNKNOWN TEXT",
        &[nostr::Tag::parse(["e", "source-unknown"]).unwrap()],
    ));
    left.persist_best_effort_inner();
    let endpoint = Arc::new(left.runtime.block_on(fips_core::FipsEndpoint::builder()
        .without_system_tun().bind()).unwrap());
    let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
    left.install_device_sync_sender_for_test(endpoint.clone(), left_tx, vec![test_fips_peer(&b)]);
    right.install_device_sync_sender_for_test(endpoint.clone(), right_tx, vec![test_fips_peer(&a)]);
    if partitioned {
        left.set_device_history_record_limit_for_test(1);
        right.set_device_history_record_limit_for_test(1);
    }
    let request = serde_json::to_vec(&serde_json::json!({"type":"request", "v":1,
        "rosterAt":100, "recordReconcile":1})).unwrap();
    left.handle_device_sync_packet(&b.public_key().to_hex(), DEVICE_SYNC_PORT, &request);
    let mut trace = Vec::new();
    let mut controls_seen = 0;
    for round in 0..2048 {
        let mut packets = Vec::new();
        while let Ok(batch) = left_rx.try_recv() {
            packets.extend(batch.records.into_iter().map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()));
        }
        if let Some(bytes) = left.take_device_sync_control_for_test(test_fips_peer(&b)) {
            packets.push(serde_json::from_slice(&bytes).unwrap());
        }
        // Force control packets ahead of original packets within each response.
        // The final requested-ID acknowledgement must stay last.
        packets.sort_by_key(|packet| if packet["records"].as_array().is_some_and(|records|
            records.iter().any(|record| record["type"] == "messageMutation")) { 0 } else { 1 });
        let x = !packets.is_empty();
        for mut packet in packets {
            if let Some(records) = packet["records"].as_array_mut() {
                for record in records {
                    if record["type"] == "messageMutation" {
                        controls_seen += 1;
                        if let Some(unknown) = tamper {
                            record["mutation"]["messageId"] = if unknown { "unknown" } else { "private-old" }.into();
                        }
                    }
                }
            }
            right.handle_device_sync_packet(&a.public_key().to_hex(), DEVICE_SYNC_PORT,
                &serde_json::to_vec(&packet).unwrap());
            trace.push(packet);
        }
        let y = drain_history_wire(&mut right, &b, &mut left, &a, &right_rx, &mut trace);
        if !x && !y { break; }
        assert!(round < 2047, "Unknown controls must not spin indefinitely");
    }
    assert!(!trace.iter().filter_map(|packet| packet["records"].as_array()).flatten()
        .any(|record| record["mutation"]["messageId"] == "source-unknown"),
        "Export must withhold controls with unknown original entitlement");
    assert!(controls_seen > 0);
    assert_eq!(left.device_history_session_count_for_test(), 0);
    assert_eq!(right.device_history_session_count_for_test(), 0);
    if let Some(unknown) = tamper {
        let target = if unknown { "unknown" } else { "private-old" };
        assert!(right.message_mutation_records(&chat, target).is_empty(), "Rejected replacement must not be stored");
        assert_eq!(right.message_for_mutation(&chat, "original").unwrap().body, "Original");
        if !unknown {
            assert_eq!(right.message_for_mutation(&chat, target).unwrap().body, "Old original");
        }
        let shared = right.app_store.shared();
        let stored: u64 = shared.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM app_meta WHERE value LIKE '%PRIVATE REPLACEMENT%'", [], |row| row.get(0)).unwrap();
        assert_eq!(stored, 0, "Unknown/pre-floor content must not survive in durable metadata");
        assert!(controls_seen <= 2, "No progress means no further automatic retries");
    } else {
        assert_eq!(right.message_for_mutation(&chat, "original").unwrap().body, "PRIVATE REPLACEMENT");
        if partitioned { assert_eq!(controls_seen, 2, "Retry after the later original arrives"); }
    }
    left.device_sync.take(); right.device_sync.take();
    left.runtime.block_on(endpoint.shutdown()).unwrap();
}
