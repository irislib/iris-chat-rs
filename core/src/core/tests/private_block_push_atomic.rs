#[test]
fn private_block_mobile_push_uses_committed_open_interval_before_preferences_save() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let target = peer.public_key().to_hex();
    let (mut core, _, dir) = logged_in_test_core_with_updates("block-push-atomic", &owner, &device);
    core.app_store.bind_account(owner.public_key()).unwrap();
    let invite = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .local_invite()
        .unwrap();
    let (mut session, response) = invite
        .accept_with_owner(
            peer.public_key(),
            peer.secret_key().to_secret_bytes(),
            Some(peer.public_key().to_hex()),
            Some(peer.public_key()),
        )
        .unwrap();
    core.protocol_engine
        .as_mut()
        .unwrap()
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    // A delayed group message may predate the block. The production SQLite
    // fallback must still suppress its authenticated private carrier while the
    // author is blocked.
    let (rumor, _) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "older shared context",
        50,
        vec![vec!["l".into(), "shared".into()]],
    );
    let plan = session
        .plan_send(rumor.as_bytes(), NdrUnixSeconds(unix_now().get()))
        .unwrap();
    let event = message_event(&session.apply_send(plan).envelope).unwrap();
    core.handle_relay_event(event.clone());
    core.persist_best_effort_inner();
    let payload = serde_json::json!({"event":event}).to_string();
    let resolve = || {
        decrypt_mobile_push_notification(
            dir.path().to_string_lossy().into_owned(),
            owner.public_key().to_hex(),
            device.secret_key().to_secret_hex(),
            payload.clone(),
        )
    };
    let before = resolve();
    assert!(before.should_show, "positive cached preview: {before:?}");
    assert_eq!(before.body, "older shared context");
    // Fail only the later preference write. The production signed-event and
    // normalized-interval transaction must already be durable at this boundary.
    let shared = core.app_store.shared();
    shared
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_block_preference_save BEFORE UPDATE OF blocked_owner_pubkeys_json
         ON preferences WHEN NEW.blocked_owner_pubkeys_json <> OLD.blocked_owner_pubkeys_json
         BEGIN SELECT RAISE(FAIL, 'simulated preference storage failure'); END;",
        )
        .unwrap();
    assert!(core.apply_private_block_event(signed_block_transition(
        &owner, &device, &target, 1, 100, 100, true
    )));
    let stored_preferences: String = shared
        .lock()
        .unwrap()
        .query_row(
            "SELECT blocked_owner_pubkeys_json FROM preferences WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_preferences, "[]",
        "exercise the real partial-save boundary"
    );
    let blocked = resolve();
    assert!(!blocked.should_show);
    assert!(blocked.body.is_empty());
    shared
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_block_preference_save;")
        .unwrap();
    assert!(core.apply_private_block_event(signed_block_transition(
        &owner, &device, &target, 2, 200, 100, false
    )));
    let after = resolve();
    assert!(
        after.should_show,
        "closed intervals retain earlier group context"
    );
    assert_eq!(after.body, "older shared context");
}
