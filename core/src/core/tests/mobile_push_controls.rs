#[test]
fn mobile_push_control_labels_survive_foreground_decryption() {
    for (kind, content, tags, expected) in [
        (RECEIPT_KIND, "delivered", vec![], "Delivered"),
        (RECEIPT_KIND, "seen", vec![], "Seen"),
        (TYPING_KIND, "typing", vec![], "Typing…"),
        (
            TYPING_KIND,
            "typing",
            vec![vec!["expiration".into(), "1".into()]],
            "Stopped typing",
        ),
    ] {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, dir) = logged_in_test_core_with_updates("push-control", &owner, &owner);
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
        let now = unix_now().get();
        let (rumor, _) = runtime_rumor_json(peer.public_key(), kind, content, now, tags);
        let plan = session
            .plan_send(rumor.as_bytes(), NdrUnixSeconds(now))
            .unwrap();
        let event = message_event(&session.apply_send(plan).envelope).unwrap();
        let payload =
            serde_json::json!({"event": event, "title": "Iris Chat", "body": "New message"})
                .to_string();
        let resolve = || {
            decrypt_mobile_push_notification(
                dir.path().to_string_lossy().into_owned(),
                owner.public_key().to_hex(),
                owner.secret_key().to_secret_hex(),
                payload.clone(),
            )
        };
        let before = resolve();
        assert!(
            !before.should_show,
            "Android/foreground suppression stays enabled"
        );
        assert_eq!(before.body, expected);
        core.handle_relay_event(event);
        core.persist_best_effort();
        let after = resolve();
        assert!(!after.should_show);
        assert_eq!(
            after.body, expected,
            "NSE must recover the exact control after the live ratchet advances"
        );
        let decoded: serde_json::Value = serde_json::from_str(&after.payload_json).unwrap();
        assert_eq!(decoded["chat_id"], peer.public_key().to_hex());
        let other_account = decrypt_mobile_push_notification(
            dir.path().to_string_lossy().into_owned(),
            Keys::generate().public_key().to_hex(),
            owner.secret_key().to_secret_hex(),
            payload.clone(),
        );
        assert!(
            other_account.body.is_empty(),
            "cached controls stay scoped to their account"
        );
        assert!(
            core.threads
                .values()
                .all(|thread| thread.messages.is_empty()),
            "controls must not become chat messages"
        );
    }
}

#[test]
fn mobile_push_control_cache_is_bounded_and_cleared_with_account_data() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("push-control-bounds", &owner, &owner);
    let (json, _) = runtime_rumor_json(
        peer.public_key(),
        RECEIPT_KIND,
        "seen",
        unix_now().get(),
        vec![],
    );
    let rumor = parse_runtime_rumor(&json).unwrap();
    for index in 0..270 {
        core.cache_mobile_push_control(
            Some(&format!("{index:064x}")),
            peer.public_key(),
            &peer.public_key().to_hex(),
            &rumor,
        );
    }
    let count = || {
        let conn =
            crate::core::mobile_push::open_lookup_connection(dir.path().to_str().unwrap()).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM app_meta WHERE key LIKE 'mobile_push_control:%'",
            [],
            |row| row.get::<_, usize>(0),
        )
        .unwrap()
    };
    assert_eq!(count(), 256);
    core.app_store.clear().unwrap();
    assert_eq!(
        count(),
        0,
        "normal account-data clearing also removes notification previews"
    );
}

#[test]
fn mobile_push_control_cache_does_not_label_old_typing_as_current() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, dir) = logged_in_test_core_with_updates("push-old-typing", &owner, &owner);
    let (json, _) = runtime_rumor_json(
        peer.public_key(),
        TYPING_KIND,
        "typing",
        unix_now().get() - 60,
        vec![],
    );
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "unavailable")
        .sign_with_keys(&peer)
        .unwrap();
    // Use a real outer-event id so the notification takes its usual cache path.
    core.cache_mobile_push_control(
        Some(&event.id.to_hex()),
        peer.public_key(),
        &peer.public_key().to_hex(),
        &parse_runtime_rumor(&json).unwrap(),
    );
    let result = decrypt_mobile_push_notification(
        dir.path().to_string_lossy().into_owned(),
        owner.public_key().to_hex(),
        owner.secret_key().to_secret_hex(),
        serde_json::json!({"event":event}).to_string(),
    );
    assert_eq!(result.body, "Typing update");
    assert!(!result.should_show);
}
