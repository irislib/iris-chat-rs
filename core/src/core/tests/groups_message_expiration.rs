#[test]
fn chat_ttl_applies_to_outgoing_message_expiration() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("outgoing-message-ttl", &owner, &device);
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 60);

    let before = unix_now().get();
    core.send_message(&chat_id, "secret", None);
    let after = unix_now().get();

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("sent message");
    let expires_at = message.expires_at_secs.expect("message expiration");
    assert!(expires_at >= before.saturating_add(60));
    assert!(expires_at <= after.saturating_add(60));
}

#[test]
fn set_chat_message_ttl_action_sets_clears_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());

    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });

    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .message_ttl_seconds,
        Some(3600)
    );
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(loaded.chat_message_ttl_seconds.get(&chat_id), Some(&3600));

    let notice_count = core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.len())
        .unwrap_or_default();
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });
    assert_eq!(
        core.threads
            .get(&chat_id)
            .map(|thread| thread.messages.len())
            .unwrap_or_default(),
        notice_count,
        "reselecting the active timer must not publish another chat-settings notice"
    );

    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: None,
    });

    assert!(!core.chat_message_ttl_seconds.contains_key(&chat_id));
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let loaded = core
        .load_persisted()
        .expect("load persisted after clear")
        .expect("persisted state after clear");
    assert!(
        !loaded.chat_message_ttl_seconds.contains_key(&chat_id),
        "cleared ttl is not restored"
    );
}

#[test]
fn send_disappearing_message_action_uses_explicit_expiration_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("send-disappearing-message-action", &owner, &device);
    let expires_at = unix_now().get().saturating_add(600);

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "secret".to_string(),
        expires_at_secs: expires_at,
    });

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("disappearing message");
    assert_eq!(message.expires_at_secs, Some(expires_at));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &message.id),
        Some(expires_at)
    );
    assert!(
        core.message_expiry_token > 0,
        "expiring sends schedule message pruning"
    );
}

