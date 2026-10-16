#[test]
fn request_acceptance_negentropy_survives_restart_without_message_history() {
    for engagement in ["accept", "reply", "reaction"] {
        let mut pair = chat_read_sync_pair("request-acceptance-negentropy");
        let peer = Keys::generate();
        let chat = peer.public_key().to_hex();
        for core in [&mut pair.a, &mut pair.b] {
            chat_read_sync_incoming(core, &peer, "request", 200);
            assert!(core.thread_is_message_request(&chat));
        }
        match engagement {
            "accept" => pair.a.handle_action(AppAction::SetMessageRequestAccepted {
                chat_id: chat.clone(),
            }),
            "reply" => pair.a.handle_action(AppAction::SendMessage {
                chat_id: chat.clone(),
                text: "Hello".into(),
            }),
            _ => pair.a.toggle_reaction(&chat, "request", "❤"),
        }
        assert_eq!(pair.a.request_decision(&chat).unwrap().decision, "accepted");
        pair.a.persist_best_effort_inner();
        drop(pair.a);
        let mut left = logged_in_test_core_at_data_dir(
            &pair.owner,
            &pair.a_device,
            pair.a_dir.path().to_string_lossy().into_owned(),
        );
        left.load_persisted().unwrap();
        left.restore_device_sync_record_projection();
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
        pair.b.app_keys = left.app_keys.clone();
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
        left.set_device_history_record_limit_for_test(1);
        pair.b.set_device_history_record_limit_for_test(1);
        let request = serde_json::to_vec(&serde_json::json!({
            "type":"request", "v":1, "rosterAt":100, "recordReconcile":1, "privateEvents":2,
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
        let mut private_records = 0;
        for round in 0..2048 {
            let mut progress = false;
            for reverse in [false, true] {
                let (source, key, target, target_key, receiver) = if reverse {
                    (
                        &mut pair.b,
                        &pair.b_device,
                        &mut left,
                        &pair.a_device,
                        &right_rx,
                    )
                } else {
                    (
                        &mut left,
                        &pair.a_device,
                        &mut pair.b,
                        &pair.b_device,
                        &left_rx,
                    )
                };
                let mut packets = Vec::new();
                while let Ok(batch) = receiver.try_recv() {
                    packets.extend(batch.records);
                }
                if let Some(packet) =
                    source.take_device_sync_control_for_test(test_fips_peer(target_key))
                {
                    packets.push(packet);
                }
                progress |= !packets.is_empty();
                for packet in packets {
                    let packet: serde_json::Value = serde_json::from_slice(&packet).unwrap();
                    // Only reconciliation can deliver the missing acceptance:
                    // drop snapshot notifications and all message history.
                    if packet["type"] == "snapshot" {
                        continue;
                    }
                    if packet["type"] == "historyRecords" {
                        private_records += packet["records"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|record| record["type"] == "privateEvent")
                            .count();
                    }
                    target.handle_device_sync_packet(
                        &key.public_key().to_hex(),
                        DEVICE_SYNC_PORT,
                        &serde_json::to_vec(&packet).unwrap(),
                    );
                }
            }
            if !progress {
                break;
            }
            assert!(round < 2047, "acceptance reconciliation must terminate");
        }
        assert!(
            private_records > 0,
            "{engagement}: missing private record exchange"
        );
        assert_eq!(pair.b.request_decision(&chat).unwrap().decision, "accepted");
        assert!(!pair.b.thread_is_message_request(&chat));
        // The request decision must survive even when messages disappear.
        pair.b.threads.get_mut(&chat).unwrap().messages.clear();
        chat_read_sync_incoming(&mut pair.b, &peer, "later", unix_now().get());
        assert!(!pair.b.thread_is_message_request(&chat));
    }
}

#[test]
fn sibling_engagement_uses_original_time_and_cannot_override_newer_rejection() {
    for reaction in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let chat = peer.public_key().to_hex();
        let (mut core, _, dir) =
            logged_in_test_core_with_updates("engagement-time", &owner, &device);
        chat_read_sync_incoming(&mut core, &peer, "request", 100);
        let rejection = nostr_identity::build_private_setting(
            owner.public_key(),
            "requestDecision",
            &chat,
            &serde_json::json!({"chatId":chat,"decision":"rejected","updatedAtMs":250_000}),
        )
        .unwrap();
        assert!(core.merge_private_setting(&rejection));
        for time in [200, 250, 300] {
            if reaction {
                assert!(core.capture_device_sync_control(
                    &chat,
                    &format!("reaction-{time}"),
                    &owner.public_key().to_hex(),
                    time,
                    REACTION_KIND,
                    "❤",
                    &[nostr::Tag::parse(["e", "request"]).unwrap()]
                ));
            } else {
                core.apply_runtime_text_message(
                    owner.public_key(),
                    Some(chat.clone()),
                    "My reply".into(),
                    time,
                    None,
                    Some(format!("reply-{time}")),
                    None,
                );
            }
            assert_eq!(core.thread_is_message_request(&chat), time <= 250);
            assert_eq!(
                core.request_decision(&chat).unwrap().decision,
                if time <= 250 { "rejected" } else { "accepted" }
            );
        }
        core.persist_best_effort_inner();
        drop(core);
        let mut restored = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            dir.path().to_string_lossy().into_owned(),
        );
        restored.load_persisted().unwrap();
        restored.restore_device_sync_record_projection();
        assert_eq!(
            restored.request_decision(&chat).unwrap().decision,
            "accepted"
        );
        restored.set_user_blocked(&chat, true);
        assert!(restored.observe_chat_engagement(
            &chat,
            &owner.public_key().to_hex(),
            unix_now_ms()
        ));
        assert!(restored.is_owner_blocked(&chat));
        assert!(!restored.is_owner_blocked(&owner.public_key().to_hex()));
    }
}
