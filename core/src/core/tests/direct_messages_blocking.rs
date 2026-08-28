#[test]
fn blocking_a_peer_removes_them_from_chat_list_and_subscribable_set() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("block-drops-from-subs", &owner, &device);

    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hi",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("e".repeat(64)));
    let peer_hex = sender.public_key().to_hex();
    core.rebuild_state();
    assert!(
        core.state
            .chat_list
            .iter()
            .any(|chat| chat.chat_id == peer_hex),
        "fresh stranger thread is visible before blocking"
    );

    let revision_before_block = core.user_discovery_revision;
    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: peer_hex.clone(),
        blocked: true,
    });
    assert!(core.user_discovery_revision > revision_before_block);

    assert!(
        !core
            .state
            .chat_list
            .iter()
            .any(|chat| chat.chat_id == peer_hex),
        "blocked peer's thread must disappear from the chat list"
    );
    assert!(
        !core.subscribable_message_author_hexes().contains(&peer_hex),
        "blocked peer must never be in the subscribable author set"
    );
    let push = core.build_mobile_push_sync_snapshot();
    assert!(
        !push.message_author_pubkeys.contains(&peer_hex),
        "blocked peer must be dropped from the mobile push sub"
    );

    let blocked_revision = core.user_discovery_revision;
    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: peer_hex.clone(),
        blocked: true,
    });
    assert_eq!(core.user_discovery_revision, blocked_revision);

    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: peer_hex.clone(),
        blocked: false,
    });
    assert!(core.user_discovery_revision > blocked_revision);
    assert!(core
        .state
        .chat_list
        .iter()
        .any(|chat| chat.chat_id == peer_hex));
}

#[test]
fn blocked_peer_subsequent_message_is_dropped_at_ingest() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("block-ingest-guard", &owner, &device);

    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: sender.public_key().to_hex(),
        blocked: true,
    });

    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "still pestering you",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("f".repeat(64)));

    assert!(
        !core.threads.contains_key(&sender.public_key().to_hex()),
        "blocked peer's message must not create or grow a thread"
    );
}
