#[test]
fn private_invite_capability_response_ignores_unknown_dm_setting_after_owner_roster() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "private-invite-block-unknown-alice",
        &alice_owner,
        &alice_device,
    );
    alice.pending_relay_publishes.clear();
    alice.preferences.accept_unknown_direct_messages = false;
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("private-invite-block-unknown-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    prove_invite_owner(&mut bob, &alice_owner, &alice_device, 10);
    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "hello from stranger".to_string(),
    });
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");

    alice.handle_relay_event(response);
    prove_invite_owner(&mut alice, &bob_owner, &bob_device, 10);

    assert!(
        alice.threads.contains_key(&bob_owner.public_key().to_hex()),
        "possession of the private invite capability plus owner roster admits the response"
    );
    assert!(
        alice.private_chat_invites.is_empty(),
        "the verified response consumes the single-use invite"
    );
}

#[test]
fn stranger_message_creates_is_request_thread_with_default_settings() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("stranger-request-default", &owner, &device);
    let (content, _inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hello from a stranger",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("a".repeat(64)));
    core.rebuild_state();

    let chat_id = sender.public_key().to_hex();
    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("stranger thread must surface in the chat list");
    assert!(
        snapshot.is_request,
        "stranger thread without accept is a request"
    );
}

#[test]
fn explicit_accept_clears_is_request_without_outgoing_message() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("stranger-explicit-accept", &owner, &device);
    let (content, _inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hi",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("b".repeat(64)));
    let chat_id = sender.public_key().to_hex();

    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: chat_id.clone(),
    });

    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("thread visible after accept");
    assert!(
        !snapshot.is_request,
        "explicit accept must clear the request gate even without a reply"
    );
    assert!(
        core.state
            .preferences
            .accepted_owner_pubkeys
            .contains(&chat_id),
        "accept persists the peer in the whitelist"
    );
}
