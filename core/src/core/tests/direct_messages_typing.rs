#[test]
fn incoming_uncached_direct_message_starts_targeted_profile_fetch() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("incoming-profile-fetch", &owner, &device);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls =
        relay_urls_from_strings(&core.preferences.nostr_relay_urls);
    core.debug_log.clear();

    let (incoming, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hi",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, incoming, Some("c".repeat(64)));

    let sender_hex = sender.public_key().to_hex();
    assert!(
        core.debug_log.iter().any(|entry| {
            entry.category == "profile.metadata.fetch"
                && entry.detail.contains("reason=incoming_message")
                && entry.detail.contains(&sender_hex)
        }),
        "new incoming direct messages should look up uncached sender profile metadata"
    );
}

#[test]
fn self_synced_outgoing_message_from_linked_device_marks_thread_accepted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("stranger-cross-device-accept", &owner, &device);

    // Stranger sends in: request thread.
    let (incoming, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "hi",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, incoming, Some("c".repeat(64)));

    // Linked device replies — arrives as an outgoing self-sync rumor
    // authored by the local owner, conversation-owner = the peer.
    let (outgoing, _) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "replied from my laptop",
        1_777_159_500,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message_with_metadata(
        owner.public_key(),
        None,
        Some(sender.public_key()),
        outgoing,
        Some("d".repeat(64)),
    );
    core.rebuild_state();

    let chat_id = sender.public_key().to_hex();
    let snapshot = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("thread surfaces after cross-device reply");
    assert!(
        !snapshot.is_request,
        "self-synced outgoing reply from another device implicitly accepts"
    );
    assert!(
        core.state
            .preferences
            .accepted_owner_pubkeys
            .contains(&chat_id),
        "cross-device reply also lands the peer in the whitelist so push sub picks them up"
    );
}

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

    core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: peer_hex.clone(),
        blocked: true,
    });

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

#[test]
fn unknown_users_toggle_off_excludes_non_accepted_peers_from_subs() {
    // Drive a real invite handshake so Alice ends up with a session
    // for Bob — that's the only way `known_message_author_pubkeys`
    // gets populated, and the subscription filter we're testing is a
    // post-processing pass over that set.
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core("push-filter-toggle-alice", &alice_owner, &alice_device);
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("push-filter-toggle-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "hi alice".to_string(),
    });
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");
    alice.handle_relay_event(response);
    let messages = pending_events_with_kind(&bob, MESSAGE_EVENT_KIND);
    for event in messages {
        alice.handle_relay_event(event);
    }

    assert!(
        !alice.subscribable_message_author_hexes().is_empty(),
        "alice should have at least one subscribable author after bob's invite handshake"
    );
    let bob_hex = bob_owner.public_key().to_hex();
    let initial_set = alice.subscribable_message_author_hexes();
    let bob_event_authors: Vec<String> = initial_set.iter().cloned().collect();

    // Toggle off without an accept: bob isn't whitelisted, so the
    // sub filter drops his event authors.
    alice.handle_action(AppAction::SetAcceptUnknownDirectMessages { enabled: false });
    let narrowed = alice.subscribable_message_author_hexes();
    for author in &bob_event_authors {
        assert!(
            !narrowed.contains(author),
            "non-accepted peer's event author {author} must drop with unknown-users toggle off"
        );
    }
    let push_narrowed = alice.build_mobile_push_sync_snapshot();
    for author in &bob_event_authors {
        assert!(
            !push_narrowed.message_author_pubkeys.contains(author),
            "mobile push subscription must mirror the filter for {author}"
        );
    }

    // Now accept bob: his event authors come back into the sub.
    alice.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: bob_hex.clone(),
    });
    let after_accept = alice.subscribable_message_author_hexes();
    assert!(
        bob_event_authors
            .iter()
            .all(|author| after_accept.contains(author)),
        "explicit accept reintroduces the peer's authors regardless of the toggle"
    );
}

#[test]
fn remote_runtime_rumor_pubkey_must_match_authenticated_sender() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-remote-route", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        forged_peer.public_key(),
        CHAT_MESSAGE_KIND,
        "forged",
        1_777_159_493,
        vec![vec!["p".to_string(), forged_peer.public_key().to_hex()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("2".repeat(64)));

    let sender_chat_id = sender.public_key().to_hex();
    let forged_chat_id = forged_peer.public_key().to_hex();
    assert!(!core
        .threads
        .get(&sender_chat_id)
        .is_some_and(|thread| { thread.messages.iter().any(|message| message.id == inner_id) }));
    assert!(!core.threads.contains_key(&forged_chat_id));
}

#[test]
fn self_sync_runtime_metadata_overrides_malicious_inner_p_tag() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let real_peer = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-self-sync-route", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "self sync route comes from runtime metadata",
        1_777_159_500,
        vec![vec!["p".to_string(), forged_peer.public_key().to_hex()]],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        owner.public_key(),
        Some(sibling_device.public_key()),
        Some(real_peer.public_key()),
        content,
        Some("4".repeat(64)),
    );

    let real_chat_id = real_peer.public_key().to_hex();
    let forged_chat_id = forged_peer.public_key().to_hex();
    assert!(core
        .threads
        .get(&real_chat_id)
        .is_some_and(|thread| thread.messages.iter().any(|message| message.id == inner_id)));
    assert!(
        !core.threads.contains_key(&forged_chat_id),
        "self-sync plaintext p tag must not override authenticated runtime conversation metadata"
    );
}

#[test]
fn self_synced_direct_message_from_device_claim_routes_to_peer_owner() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-device-claim-route", &owner, &linked_device);
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(linked_device.public_key(), 1),
        DeviceEntry::new(primary_device.public_key(), 1),
    ]);
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(owner.public_key(), &local_app_keys, 1),
    );
    let peer_chat_id = peer.public_key().to_hex();
    let primary_device_chat_id = primary_device.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from primary device",
        1_777_159_501,
        vec![vec!["p".to_string(), peer_chat_id.clone()]],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        primary_device.public_key(),
        Some(primary_device.public_key()),
        Some(peer.public_key()),
        content,
        Some("9".repeat(64)),
    );

    let thread = core.threads.get(&peer_chat_id).expect("peer thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.id, inner_id);
    assert_eq!(message.body, "sent from primary device");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert!(
        !core.threads.contains_key(&primary_device_chat_id),
        "known local device metadata must not create a direct chat with the device identity"
    );
}

#[test]
fn incoming_direct_message_from_known_peer_device_routes_to_owner_thread() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("remote-known-device-route", &owner, &local_device);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    core.app_keys.insert(
        peer_owner.public_key().to_hex(),
        known_app_keys_from_ndr(peer_owner.public_key(), &peer_app_keys, 1),
    );
    let peer_owner_chat_id = peer_owner.public_key().to_hex();
    let peer_device_chat_id = peer_device.public_key().to_hex();
    core.ensure_thread_record(&peer_owner_chat_id, 1);
    let (content, inner_id) = runtime_rumor_json(
        peer_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from peer device",
        1_777_159_502,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message_with_metadata(
        peer_device.public_key(),
        Some(peer_device.public_key()),
        None,
        content,
        Some("b".repeat(64)),
    );

    let thread = core
        .threads
        .get(&peer_owner_chat_id)
        .expect("peer owner thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].id, inner_id);
    assert_eq!(thread.messages[0].body, "sent from peer device");
    assert!(!thread.messages[0].is_outgoing);
    assert!(
        !core.threads.contains_key(&peer_device_chat_id),
        "known peer device metadata must not create a direct chat with the device identity"
    );
}

#[test]
fn receipt_runtime_rumor_uses_authenticated_sender_chat_not_malicious_tags() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-receipt-route", &owner, &device);
    let sender_chat_id = sender.public_key().to_hex();
    let message_id = "5".repeat(64);
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &sender_chat_id,
        "pending outbound".to_string(),
        1_777_159_492,
        None,
        DeliveryState::Sent,
    );
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        RECEIPT_KIND,
        "seen",
        1_777_159_493,
        vec![
            vec!["e".to_string(), message_id.clone()],
            vec!["p".to_string(), forged_peer.public_key().to_hex()],
        ],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("7".repeat(64)));

    let message = core
        .threads
        .get(&sender_chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("sender chat message");
    assert_eq!(message.delivery, DeliveryState::Seen);
    assert!(!core
        .threads
        .contains_key(&forged_peer.public_key().to_hex()));
}

#[test]
fn self_synced_seen_receipt_marks_incoming_message_seen() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-seen-receipt", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "6".repeat(64);

    core.push_incoming_message_from(
        &chat_id,
        Some(message_id.clone()),
        "read on another device".to_string(),
        1_777_159_492,
        None,
        Some(chat_id.clone()),
        Some(chat_id.clone()),
        Some("outer-incoming".to_string()),
    );
    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 1);

    let (content, _) = runtime_rumor_json(
        owner.public_key(),
        RECEIPT_KIND,
        "seen",
        1_777_159_493,
        vec![
            vec!["e".to_string(), message_id.clone()],
            vec!["p".to_string(), chat_id.clone()],
        ],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        owner.public_key(),
        Some(sibling_device.public_key()),
        Some(peer.public_key()),
        content,
        Some("8".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 0);
    let message = thread
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .expect("incoming message");
    assert_eq!(message.delivery, DeliveryState::Seen);
}

fn protocol_send_log_count(core: &AppCore, reason: &str) -> usize {
    let needle = format!("reason={reason} ");
    core.debug_log
        .iter()
        .filter(|entry| entry.category == "appcore.protocol.send" && entry.detail.contains(&needle))
        .count()
}

#[test]
fn delivered_receipt_waits_for_debounce_before_sending() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("delivered-debounce-flushes", &owner, &device);

    let chat_id = peer.public_key().to_hex();
    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: chat_id.clone(),
    });
    let (content, _) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "wait a beat",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(peer.public_key(), None, content, Some("c".repeat(64)));

    assert_eq!(core.pending_delivered_receipts.len(), 1);
    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        0,
        "delivered should not be sent synchronously with message ingest"
    );

    core.flush_all_pending_delivered_receipts_for_test();

    assert!(core.pending_delivered_receipts.is_empty());
    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        1,
        "delivered should send once the debounce expires"
    );
}

#[test]
fn seen_cancels_pending_delivered_receipt_before_debounce_flush() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("delivered-debounce-seen-cancels", &owner, &device);

    let chat_id = peer.public_key().to_hex();
    core.handle_action(AppAction::SetMessageRequestAccepted {
        chat_id: chat_id.clone(),
    });
    let (content, message_id) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "opened immediately",
        1_777_159_493,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(peer.public_key(), None, content, Some("d".repeat(64)));

    assert_eq!(core.pending_delivered_receipts.len(), 1);
    assert_eq!(protocol_send_log_count(&core, "receipt"), 0);

    core.mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));

    assert!(core.pending_delivered_receipts.is_empty());
    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        1,
        "seen should still send immediately"
    );

    core.flush_all_pending_delivered_receipts_for_test();

    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        1,
        "cancelled delivered should not send after the debounce flush"
    );
}

#[test]
fn mark_seen_syncs_to_local_siblings_when_sender_receipts_are_disabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("seen-sync-receipts-disabled", &owner, &device);
    install_local_sibling_session_for_test(&mut core, &owner, &device, &sibling_device);
    core.pending_relay_publishes.clear();
    core.preferences.send_read_receipts = false;

    let chat_id = peer.public_key().to_hex();
    let message_id = "a".repeat(64);
    core.push_incoming_message_from(
        &chat_id,
        Some(message_id.clone()),
        "sync this read privately".to_string(),
        1_777_159_492,
        None,
        Some(chat_id.clone()),
        Some(chat_id.clone()),
        Some("outer-disabled".to_string()),
    );

    core.mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 0);
    assert_eq!(
        protocol_send_log_count(&core, "receipt.self_sync"),
        1,
        "local sibling should still learn that the chat was seen"
    );
    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        0,
        "disabled sender receipts must not leak a seen receipt to the peer"
    );
}

#[test]
fn mark_seen_syncs_message_requests_to_local_siblings_without_peer_receipt() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("seen-sync-request", &owner, &device);
    install_local_sibling_session_for_test(&mut core, &owner, &device, &sibling_device);
    core.pending_relay_publishes.clear();

    let chat_id = peer.public_key().to_hex();
    let message_id = "b".repeat(64);
    core.push_incoming_message_from(
        &chat_id,
        Some(message_id.clone()),
        "request read on this device".to_string(),
        1_777_159_492,
        None,
        Some(chat_id.clone()),
        Some(chat_id.clone()),
        Some("outer-request".to_string()),
    );
    assert!(
        core.thread_is_message_request(&chat_id),
        "test needs an unaccepted request thread"
    );

    core.mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 0);
    assert_eq!(
        protocol_send_log_count(&core, "receipt.self_sync"),
        1,
        "local sibling should clear its unread count for the request"
    );
    assert_eq!(
        protocol_send_log_count(&core, "receipt"),
        0,
        "message requests must not send sender-visible seen receipts"
    );
}

#[test]
fn self_synced_direct_message_is_rendered_as_outgoing_on_linked_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-missing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from sibling",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(sibling_device.public_key()),
        content,
        Some("e".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.id, inner_id);
    assert_eq!(message.body, "sent from sibling");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn retry_batch_self_synced_direct_message_updates_open_chat_projection() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("retry-self-sync-open-chat", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent while open",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );

    core.open_chat(&chat_id);
    // `open_chat` defers the thread-load to an `OpenChatFinalize`
    // InternalEvent so the screen flip is instant; in the real app the
    // event loop drains the message right after. The test has no
    // message pump, so run the finalize step inline.
    core.open_chat_finalize(&chat_id);
    assert!(core
        .state
        .current_chat
        .as_ref()
        .expect("open chat")
        .messages
        .is_empty());

    core.process_protocol_engine_retry_batch(
        "test_self_sync",
        ProtocolRetryBatch {
            direct_messages: vec![ProtocolDecryptedMessage {
                sender: owner.public_key(),
                sender_device: Some(sibling_device.public_key()),
                conversation_owner: Some(peer.public_key()),
                content,
                event_id: Some("c".repeat(64)),
            }],
            ..ProtocolRetryBatch::default()
        },
    );

    let message = core
        .state
        .current_chat
        .as_ref()
        .expect("open chat after retry")
        .messages
        .iter()
        .find(|message| message.id == inner_id)
        .expect("self-synced message in open chat projection");
    assert_eq!(message.body, "sent while open");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(stored_message_count(&core), 1);
}

/// Notification-tap regression: when the iOS Notification Service
/// Extension writes a preview row to SQLite while the app is
/// suspended, the in-memory `threads` map doesn't know about that
/// chat. Tapping the notification used to flip the screen to
/// `Screen::Chat { chat_id }` before any thread record existed, so
/// `state.current_chat` came back `None` on the first paint and the
/// SwiftUI ChatScreen sat on its "Loading chat…" placeholder until
/// the deferred `OpenChatFinalize` event landed.
///
/// `open_chat` now stubs the thread record + loads its persisted
/// page inline, so `current_chat` is populated on the same emit that
/// flips the screen.
#[test]
fn open_chat_populates_current_chat_for_preview_only_thread() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("open-chat-preview", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    // Simulate the NSE preview-write side-effect: a message row lives
    // in SQLite under the peer's chat_id without any in-memory thread
    // record yet (the running core only learns about it on next relay
    // event ingest, which the suspended app didn't have a chance to
    // run).
    let preview = ChatMessageSnapshot {
        id: "1".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: peer.public_key().to_hex(),
        author_owner_pubkey_hex: Some(peer.public_key().to_hex()),
        author_picture_url: None,
        body: "ping from nse".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 1_777_159_500,
        expires_at_secs: None,
        delivery: DeliveryState::Received,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("0".repeat(64)),
    };
    core.app_store
        .upsert_notification_preview_message(&chat_id, 1, 1_777_159_500, &preview)
        .expect("preview upsert");
    assert!(core.threads.get(&chat_id).is_none(), "preconditions");

    core.open_chat(&chat_id);

    // First emit after open_chat must already carry `current_chat` —
    // that's what stops the SwiftUI shell from rendering "Loading
    // chat…" while the (background) `OpenChatFinalize` runs.
    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current_chat present on first paint");
    assert_eq!(current.chat_id, chat_id);
    assert_eq!(current.messages.len(), 1, "preview message loaded inline");
    assert_eq!(current.messages[0].body, "ping from nse");
    assert!(matches!(
        core.state.router.screen_stack.last(),
        Some(Screen::Chat { chat_id: shown }) if shown == &chat_id
    ));
}

#[test]
fn duplicate_persisted_incoming_message_surfaces_missing_chat_row() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("incoming-duplicate-missing-row", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let outer_event_id = "1".repeat(64);
    let created_at_secs = 1_777_159_501;
    let (content, inner_id) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "hidden until manual open",
        created_at_secs,
        Vec::new(),
    );

    let preview = ChatMessageSnapshot {
        id: inner_id.clone(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: peer.public_key().to_hex(),
        author_owner_pubkey_hex: Some(peer.public_key().to_hex()),
        author_picture_url: None,
        body: "hidden until manual open".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs,
        expires_at_secs: None,
        delivery: DeliveryState::Received,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some(outer_event_id.clone()),
    };
    core.app_store
        .upsert_notification_preview_message(&chat_id, 1, created_at_secs, &preview)
        .expect("preview upsert");
    assert!(
        core.threads.get(&chat_id).is_none(),
        "precondition: live core has no chat row yet"
    );

    core.apply_decrypted_runtime_message(peer.public_key(), None, content, Some(outer_event_id));
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("duplicate persisted message still creates a visible chat row");
    assert_eq!(
        row.last_message_preview.as_deref(),
        Some("hidden until manual open")
    );
    assert_eq!(row.unread_count, 1);
}

#[test]
fn open_chat_preserves_same_second_message_insert_order() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("open-chat-same-second-order", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let created_at_secs = 1_777_159_500;
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: created_at_secs,
            messages: vec![
                test_chat_message(
                    &chat_id,
                    "z-first-random-event-id",
                    "first",
                    created_at_secs,
                    true,
                ),
                test_chat_message(
                    &chat_id,
                    "a-second-random-event-id",
                    "second",
                    created_at_secs,
                    true,
                ),
                test_chat_message(
                    &chat_id,
                    "m-last-random-event-id",
                    "last",
                    created_at_secs,
                    true,
                ),
            ],
            draft: String::new(),
        },
    );
    core.persist_best_effort();
    core.threads.remove(&chat_id);

    core.open_chat(&chat_id);

    let current = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(
        current
            .messages
            .iter()
            .map(|message| message.body.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "last"]
    );
}

/// Draft persistence (Signal-iOS parity): SetChatDraft saves the
/// composer's unsent text on the thread record, send_message clears
/// it. Both states survive a reload by way of the regular persist
/// pipeline, but the in-memory snapshot is enough for the contract.
#[test]
fn set_chat_draft_persists_until_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("draft-persist", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    observe_peer_device_invite_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &peer,
        &peer_device,
        1,
    );

    core.set_chat_draft(&chat_id, "ping…");
    let snap = core
        .threads
        .get(&chat_id)
        .expect("draft stubs a thread record");
    assert_eq!(snap.draft, "ping…");
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|c| c.chat_id == chat_id)
            .map(|c| c.draft.as_str()),
        Some("ping…")
    );

    // Idempotent: same text is a no-op (no extra emit) so the chat
    // list stays equal.
    let rev_before = core.state.rev;
    core.set_chat_draft(&chat_id, "ping…");
    assert_eq!(core.state.rev, rev_before);

    // Sending wipes the draft.
    core.send_message(&chat_id, "ping…", None);
    assert!(core
        .threads
        .get(&chat_id)
        .expect("thread still present after send")
        .draft
        .is_empty());
}

#[test]
fn contact_nickname_overrides_direct_chat_title_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir);
    core.owner_profiles.insert(
        chat_id.clone(),
        OwnerProfileRecord {
            nickname: None,
            name: Some("alice".to_string()),
            display_name: Some("Alice Actual".to_string()),
            picture: None,
            about: None,
            updated_at_secs: 1,
            ..OwnerProfileRecord::default()
        },
    );
    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });

    core.handle_action(AppAction::SetContactNickname {
        owner_pubkey_hex: chat_id.clone(),
        nickname: "  Work Alice  ".to_string(),
    });

    let thread = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .expect("direct chat row");
    assert_eq!(thread.display_name, "Work Alice");
    assert_eq!(thread.nickname.as_deref(), Some("Work Alice"));
    assert_eq!(thread.profile_name.as_deref(), Some("Alice Actual"));
    assert_eq!(thread.subtitle.as_deref(), Some("Alice Actual"));

    let current = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(current.display_name, "Work Alice");
    assert_eq!(current.nickname.as_deref(), Some("Work Alice"));
    assert_eq!(current.profile_name.as_deref(), Some("Alice Actual"));

    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(
        loaded
            .owner_profiles
            .get(&chat_id)
            .and_then(|profile| profile.nickname.as_deref()),
        Some("Work Alice")
    );

    core.handle_action(AppAction::SetContactNickname {
        owner_pubkey_hex: chat_id.clone(),
        nickname: String::new(),
    });
    let current = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(current.display_name, "Alice Actual");
    assert_eq!(current.nickname, None);
    assert_eq!(current.profile_name.as_deref(), Some("Alice Actual"));
    assert_eq!(current.subtitle, None);
}

#[test]
fn self_synced_direct_message_updates_existing_local_outgoing_without_duplicate() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-existing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from this device",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );
    core.push_outgoing_message_with_id(
        inner_id.clone(),
        &chat_id,
        "local optimistic".to_string(),
        1_777_159_499,
        None,
        DeliveryState::Pending,
    );

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(device.public_key()),
        content,
        Some("a".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.body, "local optimistic");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn web_runtime_typing_rumors_do_not_become_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing", &owner, &device);
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&sender)
        .expect("outer event");
    let outer_event_id = outer_event.id.to_string();
    let now_secs = unix_now().get();
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        TYPING_KIND,
        "typing",
        now_secs,
        vec![
            vec![
                "ms".to_string(),
                format!("{}", now_secs.saturating_mul(1000)),
            ],
            vec!["expiration".to_string(), format!("{}", now_secs + 60)],
        ],
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        content,
        Some(outer_event_id.clone()),
    );

    let chat_id = sender.public_key().to_hex();
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
    assert!(core.typing_indicators.values().any(|record| {
        record.chat_id == chat_id && record.author_owner_hex == sender.public_key().to_hex()
    }));

    // Typing rumors aren't durable, so the SQLite-backed
    // notification preview path doesn't find them and falls through
    // to the generic resolver — which suppresses non-message kinds.
    // Only durable chat messages get a real preview after the
    // foreground app has consumed the rumor.
    let payload = serde_json::json!({
        "event": outer_event,
        "title": "Iris Chat",
        "body": "New message",
    })
    .to_string();
    let resolution = decrypt_mobile_push_notification(
        core.data_dir.to_string_lossy().to_string(),
        "invalid-owner".to_string(),
        "invalid-device".to_string(),
        payload,
    );
    assert!(!resolution.should_show);
}

#[test]
fn web_runtime_typing_stop_clears_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing-stop", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 1);
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        TYPING_KIND,
        "typing",
        1_777_159_484,
        vec![vec!["expiration".to_string(), "1".to_string()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("b".repeat(64)));

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
}

#[test]
fn newer_chat_message_clears_stale_typing_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-newer-message", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 10);
    core.push_outgoing_message_with_id(
        "newer-local-message".to_string(),
        &chat_id,
        "ok".to_string(),
        11,
        None,
        DeliveryState::Sent,
    );
    core.rebuild_state();

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
}

#[test]
fn first_received_message_clears_typing_indicator_in_chat_list() {
    // Repro for the "chat row stuck on Typing after the very first
    // peer message" complaint: the chat starts with no messages,
    // peer types, peer sends, the chat list row must drop the
    // typing badge as soon as the message lands. Goes through the
    // production path (`apply_runtime_text_message`), which is what
    // the relay event handler calls.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let row_before = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id);
    if let Some(row) = row_before {
        assert!(
            row.is_typing,
            "precondition: typing badge visible before the first message"
        );
    }

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(
        !row.is_typing,
        "chat list must drop the typing badge once the peer's first message lands"
    );
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn first_received_message_clears_typing_indicator_in_open_chat() {
    // Same case as the chat-list version, but with the chat actively
    // open. The in-chat typing badge reads from
    // `current_chat.typing_indicators`, which goes through the same
    // projection filter — must drop the indicator after the message.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg-open", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    // Open the chat: set active + create the (empty) thread record so
    // `current_chat` actually projects.
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 0,
            messages: Vec::new(),
            draft: String::new(),
        },
    );
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        !current.typing_indicators.is_empty(),
        "precondition: indicator visible in open chat"
    );

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        current.typing_indicators.is_empty(),
        "open chat must drop the typing badge after the first message"
    );
}

#[test]
fn first_received_message_at_same_second_clears_typing_indicator() {
    // Same bug, edge case: typing event and the chat message share
    // a one-second wire-clock tick. Production path again.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-same-second", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(!row.is_typing);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn typing_floor_blocks_late_typing_after_message() {
    // Bug shape from peer apps that don't send a stop-typing event:
    // a typing rumor with `created_at_secs` strictly greater than the
    // latest message slips through the projection's `>` filter and
    // re-arms the indicator after we've already shown the message.
    //
    // The floor is bumped when the message lands and gates any
    // subsequent typing event with `event_secs <= floor`. Even though
    // the typing rumor here has T=200 > the message's T=100, the
    // floor is also at 100 (it's the same chat) — the typing must
    // strictly exceed the floor *and* the floor itself was set by
    // the message we already saw, so a new typing event has to be
    // genuinely after that message's wire-clock second to surface.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-late", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();
    let t0 = unix_now().get();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        t0,
        None,
        Some("msg-1".to_string()),
        None,
    );

    // Typing rumor races in after the message with the *same* wire
    // second — the floor at t0 keeps it suppressed.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), t0, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));

    // A genuinely newer typing event (peer is typing again) does
    // arm the indicator.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), t0 + 1, None);
    assert!(core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn stale_typing_rumor_is_not_shown_as_current_typing_status() {
    // A typing rumor whose expiration is in the past for our wall clock —
    // the invite-bootstrap pattern, or a late-arriving rumor whose sender
    // had a 60-second window that's already elapsed by the time it reaches
    // us — must not arm the indicator.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-stale-wall-clock", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();
    let now_secs = unix_now().get();

    // Bootstrap pattern: event timestamped now, expiration already past.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), now_secs, Some(1));
    assert!(
        !core
            .typing_indicators
            .values()
            .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex),
        "typing rumor whose expiration precedes the wire timestamp must not arm the indicator"
    );

    // Late delivery: sender-supplied expiration was in the future for them
    // (event_secs + 60 > event_secs) but is in the past for us now.
    core.apply_typing_event(
        chat_id.clone(),
        sender_hex.clone(),
        now_secs - 600,
        Some(now_secs - 540),
    );
    assert!(
        !core
            .typing_indicators
            .values()
            .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex),
        "typing rumor expired against our wall clock must not arm the indicator"
    );

    // Sanity: a fresh typing rumor still arms.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), now_secs, None);
    assert!(
        core.typing_indicators
            .values()
            .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex),
        "fresh typing rumor must arm the indicator"
    );
}

#[test]
fn typing_floor_persists_across_message_deletion() {
    // iris-chat (JS) keeps `lastMessageAt` monotonic so that deleting
    // a message doesn't let a stale typing rumor slip through. Same
    // contract here: once the floor reaches a given second, deleting
    // the message that put it there leaves the floor in place.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-delete", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.delete_local_message(&chat_id, "msg-1");

    // The thread is now empty (latest_message_secs would be 0) but
    // the floor must stay at 100 from the message we already saw.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 100, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn web_runtime_control_rumors_do_not_create_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let controls = [
        (
            RECEIPT_KIND,
            "seen",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
        (
            REACTION_KIND,
            "+",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
    ];

    for (index, (kind, body, tags)) in controls.into_iter().enumerate() {
        let mut core =
            logged_in_test_core(&format!("web-runtime-control-{index}"), &owner, &device);
        let (content, _) = runtime_rumor_json(
            sender.public_key(),
            kind,
            body,
            1_777_159_483 + index as u64,
            tags,
        );

        core.apply_decrypted_runtime_message(
            sender.public_key(),
            None,
            content,
            Some(format!("{:064x}", index + 20)),
        );

        let chat_id = sender.public_key().to_hex();
        assert!(
            core.threads
                .get(&chat_id)
                .map(|thread| thread.messages.is_empty())
                .unwrap_or(true),
            "control kind {kind} created a chat message"
        );
    }
}

#[test]
fn web_runtime_chat_settings_create_system_notice() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-chat-settings", &owner, &device);
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        "60",
        1_777_159_483,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("1".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert!(thread.messages[0]
        .body
        .contains("set disappearing messages timer"));
}

#[test]
fn web_runtime_chat_settings_update_and_clear_ttl() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-chat-settings-ttl", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let (set_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        &serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        })
        .to_string(),
        1_777_159_483,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        set_content,
        Some("b".repeat(64)),
    );

    assert_eq!(
        core.chat_message_ttl_seconds.get(&chat_id),
        Some(&3600),
        "incoming settings set chat ttl"
    );
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    let thread = core.threads.get(&chat_id).expect("thread");
    assert!(thread
        .messages
        .last()
        .expect("system notice")
        .body
        .contains("1 hour"));

    let (clear_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        "0",
        1_777_159_484,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        clear_content,
        Some("d".repeat(64)),
    );

    assert!(
        !core.chat_message_ttl_seconds.contains_key(&chat_id),
        "incoming settings clear chat ttl"
    );
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let thread = core.threads.get(&chat_id).expect("thread after clear");
    assert!(thread
        .messages
        .last()
        .expect("clear notice")
        .body
        .contains("Off"));
}

#[test]
fn web_runtime_chat_message_expiration_tag_is_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-expiring-message", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "secret",
        1_777_159_483,
        vec![vec!["expiration".to_string(), "1777159543".to_string()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("f".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "secret");
    assert_eq!(thread.messages[0].expires_at_secs, Some(1_777_159_543));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &inner_id),
        Some(1_777_159_543)
    );
}

#[test]
fn runtime_controls_settings_reactions_and_expiration_flow() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let sender_device = Keys::generate();
    let mut core = logged_in_test_core("runtime-controls-flow", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    observe_peer_device_invite_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &sender,
        &sender_device,
        1,
    );
    let (message_content, message_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "runtime message",
        1_777_159_483,
        vec![vec!["expiration".to_string(), "1777159543".to_string()]],
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        message_content,
        Some("2".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread after message");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "runtime message");
    assert_eq!(thread.messages[0].expires_at_secs, Some(1_777_159_543));

    core.apply_typing_event(
        chat_id.clone(),
        sender.public_key().to_hex(),
        unix_now().get(),
        None,
    );
    assert!(core.typing_indicators.values().any(|record| {
        record.chat_id == chat_id && record.author_owner_hex == sender.public_key().to_hex()
    }));

    core.apply_incoming_reaction_to_chat(&chat_id, &message_id, &sender.public_key().to_hex(), "+");
    let reacted = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.first())
        .expect("reacted message");
    assert_eq!(reacted.reactions.len(), 1);
    assert_eq!(reacted.reactions[0].emoji, "+");

    let (settings_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        &serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        })
        .to_string(),
        1_777_159_485,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        settings_content,
        Some("4".repeat(64)),
    );
    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "local expiring reply".to_string(),
        expires_at_secs: 1_777_160_000,
    });
    let reply = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.body == "local expiring reply")
        })
        .expect("local expiring reply");
    assert_eq!(reply.expires_at_secs, Some(1_777_160_000));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &reply.id),
        Some(1_777_160_000)
    );
}

fn ndr_owner_pubkey(pubkey: PublicKey) -> nostr_double_ratchet::OwnerPubkey {
    nostr_double_ratchet::OwnerPubkey::from_bytes(pubkey.to_bytes())
}

fn ndr_device_pubkey(pubkey: PublicKey) -> nostr_double_ratchet::DevicePubkey {
    nostr_double_ratchet::DevicePubkey::from_bytes(pubkey.to_bytes())
}

fn test_group_snapshot(
    group_id: &str,
    name: &str,
    created_by: PublicKey,
    members: Vec<PublicKey>,
    admins: Vec<PublicKey>,
    revision: u64,
) -> GroupSnapshot {
    GroupSnapshot {
        group_id: group_id.to_string(),
        protocol: nostr_double_ratchet::GroupProtocol::sender_key_v1(),
        name: name.to_string(),
        picture: None,
        about: None,
        created_by: ndr_owner_pubkey(created_by),
        members: members.into_iter().map(ndr_owner_pubkey).collect(),
        admins: admins.into_iter().map(ndr_owner_pubkey).collect(),
        revision,
        created_at: nostr_double_ratchet::UnixSeconds(1),
        updated_at: nostr_double_ratchet::UnixSeconds(revision),
    }
}

#[test]
fn appcore_mutual_groups_snapshot_filters_to_visible_shared_groups() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let other = Keys::generate();
    let mut core = logged_in_test_core("mutual-groups", &owner, &device);

    let shared = test_group_snapshot(
        "shared-group",
        "Shared group",
        owner.public_key(),
        vec![owner.public_key(), peer.public_key()],
        vec![owner.public_key()],
        2,
    );
    let not_shared = test_group_snapshot(
        "other-group",
        "Other group",
        owner.public_key(),
        vec![owner.public_key(), other.public_key()],
        vec![owner.public_key()],
        3,
    );
    let hidden = test_group_snapshot(
        "hidden-group",
        "Hidden group",
        owner.public_key(),
        vec![owner.public_key(), peer.public_key()],
        vec![owner.public_key()],
        4,
    );

    for group in [&shared, &not_shared, &hidden] {
        core.groups.insert(group.group_id.clone(), group.clone());
    }
    for group in [&shared, &not_shared] {
        let chat_id = group_chat_id(&group.group_id);
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 0,
                updated_at_secs: group.updated_at.get(),
                messages: vec![test_chat_message(
                    &chat_id,
                    &format!("msg-{}", group.group_id),
                    "hello",
                    group.updated_at.get(),
                    false,
                )],
                draft: String::new(),
            },
        );
    }

    let snapshot = core.mutual_groups_snapshot(&peer.public_key().to_hex());

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].chat_id, group_chat_id("shared-group"));
    assert_eq!(snapshot.groups[0].display_name, "Shared group");
    assert_eq!(snapshot.groups[0].member_count, 2);
}

#[test]
fn group_runtime_chat_message_is_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let mut core = logged_in_test_core("group-runtime-message", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-runtime-message".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        sender_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "group secret",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    let payload = payload.into_bytes();

    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
            body: payload,
            revision: 1,
        },
    ));

    let thread = core.threads.get(&chat_id).expect("group thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "group secret");
    assert_eq!(thread.messages[0].id, rumor_id);
    assert_eq!(thread.messages[0].expires_at_secs, None);
}

#[test]
fn group_delivered_receipt_is_queued_directly_to_message_author() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = logged_in_test_core(
        "group-delivered-author-only-alice",
        &alice_owner,
        &alice_device,
    );
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("group-delivered-author-only-bob", &bob_owner, &bob_device);
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    bob.pending_relay_publishes.clear();

    let group_id = "group-delivered-author-only".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        alice_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "author-only delivered receipt",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );

    bob.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(alice_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(alice_device.public_key())),
            body: payload.into_bytes(),
            revision: 1,
        },
    ));

    let debug = bob
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert_eq!(debug.pending_group_fanout_count, 0);
    assert_eq!(bob.pending_delivered_receipts.len(), 1);
    assert_eq!(
        protocol_send_log_count(&bob, "receipt"),
        0,
        "delivered receipt should wait for its short debounce"
    );

    bob.flush_all_pending_delivered_receipts_for_test();

    assert!(
        bob.debug_log.iter().any(|entry| {
            entry.category == "appcore.protocol.send"
                && entry.detail.contains("reason=receipt ")
                && entry.detail.contains(&format!("chat_id={chat_id} "))
                && entry.detail.contains("event_ids=1")
        }),
        "delivered receipt for group message {rumor_id} should be sent as a direct payload to the message author"
    );
}

#[test]
fn group_seen_receipt_sent_directly_to_author_updates_sender_copy() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core("group-seen-author-alice", &alice_owner, &alice_device);
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("group-seen-author-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "direct bootstrap".to_string(),
    });
    for event in pending_events_with_kind(&bob, INVITE_RESPONSE_KIND) {
        alice.handle_relay_event(event);
    }
    for event in pending_events_with_kind(&bob, MESSAGE_EVENT_KIND) {
        alice.handle_relay_event(event);
    }
    alice.pending_relay_publishes.clear();
    bob.pending_relay_publishes.clear();

    let group_id = "group-seen-author-only".to_string();
    let chat_id = group_chat_id(&group_id);
    let message_id = "group-message-seen-author-only".to_string();
    alice.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "seen receipt should be private".to_string(),
        1_777_159_483,
        None,
        DeliveryState::Sent,
    );
    bob.push_incoming_message_from(
        &chat_id,
        Some(message_id.clone()),
        "seen receipt should be private".to_string(),
        1_777_159_483,
        None,
        Some("Alice".to_string()),
        Some(alice_owner.public_key().to_hex()),
        Some("group-outer".to_string()),
    );
    bob.mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));

    let receipt_publishes = bob
        .pending_relay_publishes
        .values()
        .filter(|pending| {
            serde_json::from_str::<Event>(&pending.event_json)
                .ok()
                .is_some_and(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        })
        .count();
    assert_eq!(receipt_publishes, 1);
    for event in pending_events_with_kind(&bob, MESSAGE_EVENT_KIND) {
        alice.handle_relay_event(event);
    }

    let message = alice
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("alice group message");
    assert_eq!(message.delivery, DeliveryState::Seen);
    assert_eq!(message.recipient_deliveries.len(), 1);
    assert_eq!(
        message.recipient_deliveries[0].owner_pubkey_hex,
        bob_owner.public_key().to_hex()
    );
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Seen
    );
}

#[test]
fn group_rumor_id_dedupes_pairwise_fanout_copies() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let mut core = logged_in_test_core("group-pairwise-dedupe", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-pairwise-dedupe".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        sender_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "deduped group body",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    let payload = payload.into_bytes();

    for _ in 0..2 {
        core.apply_group_decrypted_event(GroupIncomingEvent::Message(
            nostr_double_ratchet::GroupReceivedMessage {
                group_id: group_id.clone(),
                sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
                sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
                body: payload.clone(),
                revision: 1,
            },
        ));
    }

    let thread = core.threads.get(&chat_id).expect("group thread");
    let matching = thread
        .messages
        .iter()
        .filter(|message| message.body == "deduped group body")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, rumor_id);
}

#[test]
fn group_runtime_rumor_pubkey_must_match_authenticated_sender() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let forged_owner = Keys::generate();
    let mut core = logged_in_test_core("group-runtime-forged-author", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-forged-author".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        forged_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "forged group body",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );

    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
            body: payload.into_bytes(),
            revision: 1,
        },
    ));

    assert!(!core
        .threads
        .get(&chat_id)
        .is_some_and(|thread| thread.messages.iter().any(|message| message.id == rumor_id)));
}
