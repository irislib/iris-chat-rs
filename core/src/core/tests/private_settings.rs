#[test]
fn independent_request_choices_and_chat_deletions_merge_after_restart() {
    for reverse in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let first = Keys::generate().public_key().to_hex();
        let second = Keys::generate().public_key().to_hex();
        let (mut core, _, dir) =
            logged_in_test_core_with_updates("private-setting-merge", &owner, &device);
        let mut updates = [
            (
                "chatDeletion",
                first.clone(),
                serde_json::json!({"id":first,"deletedAt":100}),
            ),
            (
                "chatDeletion",
                second.clone(),
                serde_json::json!({"id":second,"deletedAt":120}),
            ),
            (
                "chatDeletion",
                first.clone(),
                serde_json::json!({"id":first,"deletedAt":150}),
            ),
            (
                "requestDecision",
                first.clone(),
                serde_json::json!({"chatId":first,"decision":"accepted","updatedAtMs":1000}),
            ),
            (
                "requestDecision",
                second.clone(),
                serde_json::json!({"chatId":second,"decision":"rejected","updatedAtMs":1000}),
            ),
            (
                "requestDecision",
                first.clone(),
                serde_json::json!({"chatId":first,"decision":"none","updatedAtMs":1001}),
            ),
        ];
        if reverse {
            updates.reverse();
        }
        for (name, target, value) in updates.iter().chain(updates.iter()) {
            let event =
                nostr_identity::build_private_setting(owner.public_key(), name, target, value)
                    .unwrap();
            assert!(core.merge_private_setting(&event));
        }
        core.persist_best_effort_inner();
        drop(core);
        let mut restored = logged_in_test_core_at_data_dir(
            &owner,
            &device,
            dir.path().to_string_lossy().into_owned(),
        );
        restored.load_persisted().unwrap();
        restored.restore_device_sync_record_projection().unwrap();
        assert_eq!(restored.chat_deletions.get(&first), Some(&150));
        assert_eq!(restored.chat_deletions.get(&second), Some(&120));
        assert_eq!(restored.request_decision(&first).unwrap().decision, "none");
        assert_eq!(
            restored.request_decision(&second).unwrap().decision,
            "rejected"
        );
        assert!(!restored.preferences.accepted_owner_pubkeys.contains(&first));
        let foreign = nostr_identity::build_private_setting(
            Keys::generate().public_key(),
            "chatDeletion",
            &first,
            &serde_json::json!({"id":first,"deletedAt":200}),
        )
        .unwrap();
        assert!(!restored.merge_private_setting(&foreign));
    }
}

#[test]
fn authorization_records_keep_signatures_and_reject_forged_proofs() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("authorization-sync", &owner, &device);
    let event = AppKeys::new(vec![DeviceEntry::new(Keys::generate().public_key(), 100)])
        .get_event_at(peer.public_key(), 100)
        .sign_with_keys(&peer)
        .unwrap();
    let record = serde_json::json!({"type":"authorization","event":event});
    assert!(core.apply_app_keys_event(&event).unwrap());
    assert!(core
        .private_state_fixture_for_test()
        .iter()
        .any(|entry| entry["record"] == record));
    let mut forged = serde_json::to_value(&event).unwrap();
    forged["content"] = serde_json::json!("forged");
    assert!(!core
        .apply_app_keys_event(&serde_json::from_value(forged).unwrap())
        .unwrap());
    drop(core);
    let mut restored =
        logged_in_test_core_at_data_dir(&owner, &device, dir.path().to_string_lossy().into_owned());
    assert!(restored
        .private_state_fixture_for_test()
        .iter()
        .any(|entry| entry["record"] == record && entry["id"] == event.id.to_hex()));
    assert!(serde_json::to_value(record).unwrap()["event"]["sig"].is_string());
    restored.restore_device_sync_record_projection().unwrap();
    assert!(restored.app_keys.contains_key(&peer.public_key().to_hex()));
}

#[test]
fn private_state_reconciliation_allows_approved_pre_link_history() {
    check_private_state_link_history(false);
}

#[test]
fn legacy_request_acceptance_reconciles_before_linked_device_history() {
    check_private_state_link_history(true);
}

fn check_private_state_link_history(legacy_acceptance: bool) {
    for include_history in [false, true] {
        let mut pair = chat_read_sync_pair("private-state-link-history");
        pair.b.app_keys = pair.a.app_keys.clone();
        let owner = pair.owner.public_key();
        let old = AppKeys::new(vec![DeviceEntry::new(pair.a_device.public_key(), 1)])
            .get_event_at(owner, 50)
            .sign_with_keys(&pair.owner)
            .unwrap();
        let current = AppKeys::new(vec![
            DeviceEntry::new(pair.a_device.public_key(), 1),
            DeviceEntry::new(pair.b_device.public_key(), 100),
        ])
        .get_event_at(owner, 100)
        .sign_with_keys(&pair.owner)
        .unwrap();
        assert!(pair.a.apply_app_keys_event(&old).unwrap());
        assert!(pair.a.apply_app_keys_event(&current).unwrap());
        assert!(pair.b.apply_app_keys_event(&current).unwrap());
        let chat = owner.to_hex();
        pair.a
            .set_chat_mention_notifications("group:mention-settings", false);
        if legacy_acceptance {
            pair.a.preferences.accepted_owner_pubkeys.push(chat.clone());
        }
        pair.a.push_incoming_message_from(
            &chat,
            Some("before-link".into()),
            "Saved before linking".into(),
            60,
            None,
            None,
            Some(chat.clone()),
            None,
        );
        pair.a
            .create_device_history_transfer(
                pair.b_device.public_key(),
                include_history,
                "ba".repeat(32),
            )
            .unwrap();
        pair.b
            .record_device_history_approver(
                &pair.a_device.public_key().to_hex(),
                100,
                "ba".repeat(32),
            )
            .unwrap();
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
        let (left_tx, left_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        let (right_tx, right_rx) = DeviceSyncTcpSender::test_channel(256, 64 * 1024);
        pair.a.install_device_sync_sender_for_test(
            endpoint.clone(),
            left_tx,
            vec![test_fips_peer(&pair.b_device)],
        );
        pair.b.install_device_sync_sender_for_test(
            endpoint,
            right_tx,
            vec![test_fips_peer(&pair.a_device)],
        );
        pair.a.refresh_device_sync_on_foreground();
        pair.b.refresh_device_sync_on_foreground();
        let mut trace = Vec::new();
        for _ in 0..512 {
            let x = drain_history_wire(
                &mut pair.a,
                &pair.a_device,
                &mut pair.b,
                &pair.b_device,
                &left_rx,
                &mut trace,
            );
            let y = drain_history_wire(
                &mut pair.b,
                &pair.b_device,
                &mut pair.a,
                &pair.a_device,
                &right_rx,
                &mut trace,
            );
            if !x && !y {
                break;
            }
        }
        assert_eq!(
            has_device_sync_message(&pair.b, &chat, "before-link"),
            include_history,
            "wire: {}",
            serde_json::to_string(&trace).unwrap()
        );
        if legacy_acceptance {
            assert_eq!(pair.b.request_decision(&chat).unwrap().decision, "accepted");
        }
        assert_eq!(
            pair.b.preferences.silent_mention_chat_ids,
            vec!["group:mention-settings"],
            "private mention choice must reconcile without sharing chat history"
        );
        if include_history {
            assert!(
                pair.b
                    .device_history_transfer(&pair.a_device.public_key().to_hex())
                    .unwrap()
                    .complete
            );
        }
    }
}
