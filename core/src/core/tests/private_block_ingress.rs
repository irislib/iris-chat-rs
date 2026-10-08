#[test]
fn private_block_same_second_transition_survives_out_of_order_replay() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let (mut core, _, dir) = logged_in_test_core_with_updates("block-second", &owner, &device);
    let block = signed_block_transition(&owner, &device, &target, 1, 100, 100, true);
    let unblock = signed_block_transition(&owner, &device, &target, 2, 100, 100, false);
    for event in [unblock, block] {
        assert!(core.apply_private_block_event(event));
        assert!(!core.block_allows_history("group:shared", &target, 100));
        assert!(core.block_allows_history("group:shared", &target, 99));
        assert!(core.block_allows_history("group:shared", &target, 101));
        assert!(!core.is_owner_blocked(&target));
    }
    core.persist_best_effort_inner();
    drop(core);
    let mut restored =
        logged_in_test_core_at_data_dir(&owner, &device, dir.path().to_string_lossy().into_owned());
    restored.restore_device_sync_record_projection();
    assert!(!restored.block_allows_history("group:shared", &target, 100));
    assert!(restored.block_allows_history("group:shared", &target, 101));
}

#[test]
fn private_block_snapshot_does_not_restore_blocked_direct_chat_metadata() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("block-snapshot", &owner, &device);
    configure_test_device_sync_profile(&mut core, &owner, &device, &sibling, None);
    assert!(core.apply_private_block_event(signed_block_transition(
        &owner, &device, &target, 1, 100, 100, true
    )));
    let send = |core: &mut AppCore, at| {
        let packet = serde_json::json!({"type":"snapshot","v":1,"rosterAt":100,"chats":[{"id":target,"updatedAt":at}]}).to_string();
        core.handle_device_sync_packet(
            &sibling.public_key().to_hex(),
            DEVICE_SYNC_PORT,
            packet.as_bytes(),
        );
    };
    send(&mut core, 150);
    assert!(!core.threads.contains_key(&target));
    assert!(core.apply_private_block_event(signed_block_transition(
        &owner, &device, &target, 2, 200, 100, false
    )));
    send(&mut core, 150);
    assert!(!core.threads.contains_key(&target));
    send(&mut core, 250);
    assert!(
        core.threads.contains_key(&target),
        "post-unblock metadata remains allowed"
    );
}

#[test]
fn private_block_late_direct_ingress_never_redelivers_blocked_period_after_unblock() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let target = peer.public_key().to_hex();
    let mut core = logged_in_test_core("block-direct-late", &owner, &device);
    for (revision, at, blocked) in [(1, 100, true), (2, 200, false)] {
        assert!(core.apply_private_block_event(signed_block_transition(
            &owner, &device, &target, revision, at, 100, blocked
        )));
    }
    for kind in [CHAT_MESSAGE_KIND, TYPING_KIND, REACTION_KIND] {
        let (rumor, _) =
            runtime_rumor_json(peer.public_key(), kind, "blocked period", 150, Vec::new());
        core.apply_decrypted_runtime_message(peer.public_key(), None, rumor, None);
        assert!(!core.threads.contains_key(&target));
    }
    assert!(core.apply_decrypted_runtime_message_with_metadata(
        peer.public_key(),
        None,
        None,
        "legacy blocked period".into(),
        None,
        150
    ));
    assert!(!core.threads.contains_key(&target));
    let (rumor, _) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "allowed after unblock",
        250,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(peer.public_key(), None, rumor, None);
    assert_eq!(core.threads[&target].messages.len(), 1);
    assert_eq!(
        core.threads[&target].messages[0].body,
        "allowed after unblock"
    );
}

#[test]
fn private_block_mobile_push_decryption_and_cache_suppress_blocked_period() {
    for group in [false, true] {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, dir) = logged_in_test_core_with_updates("block-push", &owner, &owner);
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
        let now = unix_now().get();
        let tags = if group {
            vec![vec!["l".into(), "shared".into()]]
        } else {
            Vec::new()
        };
        let (rumor, _) = runtime_rumor_json(
            peer.public_key(),
            CHAT_MESSAGE_KIND,
            "blocked notification body",
            now,
            tags,
        );
        let plan = session
            .plan_send(rumor.as_bytes(), NdrUnixSeconds(now))
            .unwrap();
        let event = message_event(&session.apply_send(plan).envelope).unwrap();
        let payload = serde_json::json!({"event": event}).to_string();
        let resolve = || {
            decrypt_mobile_push_notification(
                dir.path().to_string_lossy().into_owned(),
                owner.public_key().to_hex(),
                owner.secret_key().to_secret_hex(),
                payload.clone(),
            )
        };
        let before = resolve();
        assert!(before.should_show, "positive decrypt control: {before:?}");
        assert_eq!(before.body, "blocked notification body");
        // A lagging sibling can already have this post in its SQLite cache.
        // Prove that cache is readable before introducing the block policy.
        let chat_id = if group {
            "group:shared".to_string()
        } else {
            peer.public_key().to_hex()
        };
        core.push_incoming_message_from(
            &chat_id,
            Some("lagging-copy".into()),
            "blocked cached body".into(),
            now,
            None,
            None,
            Some(peer.public_key().to_hex()),
            Some(event.id.to_hex()),
        );
        core.persist_best_effort_inner();
        let unavailable_device = Keys::generate();
        let resolve_cache = || {
            decrypt_mobile_push_notification(
                dir.path().to_string_lossy().into_owned(),
                owner.public_key().to_hex(),
                unavailable_device.secret_key().to_secret_hex(),
                payload.clone(),
            )
        };
        let cached_before = resolve_cache();
        assert!(
            cached_before.should_show,
            "positive SQLite fallback: {cached_before:?}"
        );
        assert_eq!(cached_before.body, "blocked cached body");
        // Fixed signed timestamps keep this boundary regression independent of
        // whether the wall clock ticks during the cryptographic fixture setup.
        assert!(core.apply_private_block_event(signed_block_transition(
            &owner,
            &owner,
            &peer.public_key().to_hex(),
            1,
            now,
            now,
            true
        )));
        core.persist_best_effort_inner();
        let blocked = resolve();
        assert!(!blocked.should_show);
        assert!(blocked.body.is_empty());
        assert!(core.apply_private_block_event(signed_block_transition(
            &owner,
            &owner,
            &peer.public_key().to_hex(),
            2,
            now,
            now,
            false
        )));
        core.persist_best_effort_inner();
        let unblocked = resolve();
        assert!(
            !unblocked.should_show,
            "same-second blocked period cannot appear after unblock"
        );
        assert!(unblocked.body.is_empty());
        let cached = resolve_cache();
        assert!(!cached.should_show);
        assert!(cached.body.is_empty());
    }
}
