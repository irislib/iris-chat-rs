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
        restored.restore_device_sync_record_projection();
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
    restored.restore_device_sync_record_projection();
    assert!(restored.app_keys.contains_key(&peer.public_key().to_hex()));
}
