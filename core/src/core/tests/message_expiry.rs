#[test]
fn expired_history_preserves_newer_unread_badges() {
    for old_delivery in [DeliveryState::Seen, DeliveryState::Received] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let chat_id = Keys::generate().public_key().to_hex();
        let mut core = logged_in_test_core("expired-read-history", &owner, &device);
        let mut old = test_chat_message(&chat_id, "old", "read history", 100, false);
        old.expires_at_secs = Some(200);
        old.delivery = old_delivery;
        let mut unread = test_chat_message(&chat_id, "new", "still unread", 150, false);
        unread.delivery = DeliveryState::Received;
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 1,
                updated_at_secs: 150,
                messages: vec![old, unread],
                draft: String::new(),
            },
        );
        core.persist_best_effort_inner();

        assert_eq!(core.prune_expired_messages(200), 1);
        let thread = &core.threads[&chat_id];
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].id, "new");
        assert_eq!(thread.unread_count, 1);

        core.persist_best_effort_inner();
        let persisted = core.load_persisted().unwrap().unwrap();
        assert_eq!(persisted.threads[0].unread_count, 1);
        assert_eq!(persisted.threads[0].messages.len(), 1);
    }
}

#[test]
fn expired_runtime_messages_are_not_restored_by_direct_or_group_delivery() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("expired-runtime-replay", &owner, &device);
    core.preferences.send_read_receipts = false;
    let now = unix_now().get();

    for expires_at in [now.saturating_sub(1), now] {
        let (payload, _) = runtime_rumor_json(
            sender.public_key(),
            CHAT_MESSAGE_KIND,
            "expired secret",
            now.saturating_sub(60),
            vec![vec!["expiration".to_string(), expires_at.to_string()]],
        );
        core.apply_decrypted_runtime_message(
            sender.public_key(),
            None,
            payload,
            Some("f".repeat(64)),
        );

        let (payload, _) = runtime_rumor_json(
            sender.public_key(),
            CHAT_MESSAGE_KIND,
            "expired group secret",
            now.saturating_sub(60),
            vec![
                vec!["l".to_string(), "expired-group".to_string()],
                vec!["expiration".to_string(), expires_at.to_string()],
            ],
        );
        core.apply_group_decrypted_event(GroupIncomingEvent::Message(
            nostr_double_ratchet::GroupReceivedMessage {
                group_id: "expired-group".to_string(),
                sender_owner: ndr_owner_pubkey(sender.public_key()),
                sender_device: None,
                body: payload.into_bytes(),
                revision: 1,
            },
        ));

        core.apply_runtime_text_message(
            owner.public_key(),
            Some(sender.public_key().to_hex()),
            "expired own-device replay".to_string(),
            now.saturating_sub(60),
            Some(expires_at),
            Some("own-replay".to_string()),
            None,
        );
    }

    assert!(core.threads.is_empty());
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 0);
}
