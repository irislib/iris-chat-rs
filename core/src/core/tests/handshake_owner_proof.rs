#[test]
fn private_chat_handshake_carries_locally_signed_approval_without_server_echo() {
    for linked in [false, true] {
        let sender_owner = Keys::generate();
        let sender_device = Keys::generate();
        let receiver_owner = Keys::generate();
        let receiver_device = Keys::generate();
        let mut sender =
            logged_in_test_core("handshake-proof-sender", &sender_owner, &sender_device);
        let mut receiver = logged_in_test_core(
            "handshake-proof-receiver",
            &receiver_owner,
            &receiver_device,
        );
        let invite = create_private_invite_for_test(&mut receiver);
        prove_invite_owner(&mut sender, &receiver_owner, &receiver_device, 10);

        // Use the production publication path but never deliver its standalone
        // event to either client. The account's approval must exist locally.
        sender.upsert_local_app_key_device_with_labels(
            sender_owner.public_key(),
            sender_device.public_key(),
            None,
            true,
        );
        sender.publish_local_app_keys_snapshot_only("test_local_handshake_proof");
        if linked {
            // A linked device retains the public signed approval on restart,
            // while holding only its own device secret key.
            sender.logged_in.as_mut().unwrap().owner_keys = None;
            let storage = Arc::new(SqliteStorageAdapter::new(
                sender.app_store.shared(),
                sender_owner.public_key().to_hex(),
                sender_device.public_key().to_hex(),
            )) as Arc<dyn StorageAdapter>;
            sender.protocol_engine = Some(
                ProtocolEngine::load_or_create_for_local_device(
                    storage,
                    sender_owner.public_key(),
                    &sender_device,
                )
                .unwrap(),
            );
        }
        sender.pending_relay_publishes.clear();
        sender.handle_action(AppAction::AcceptInvite {
            invite_input: invite,
        });
        sender.handle_action(AppAction::SendMessage {
            chat_id: receiver_owner.public_key().to_hex(),
            text: "delivered with the device approval".to_string(),
        });
        let response = pending_events_with_kind(&sender, INVITE_RESPONSE_KIND)
            .into_iter()
            .next()
            .expect("outgoing handshake");
        assert!(response
            .tags
            .iter()
            .any(|tag| tag.as_slice()[0] == "owner-proof"));
        assert!(!receiver
            .app_keys
            .contains_key(&sender_owner.public_key().to_hex()));
        // Reorder the message ahead of the handshake, as real servers can do.
        let messages = pending_events_with_kind(&sender, MESSAGE_EVENT_KIND);
        assert!(
            messages.len() >= 2,
            "text must be queued for publication; readiness={:?}",
            sender
                .protocol_engine
                .as_ref()
                .unwrap()
                .direct_send_readiness(receiver_owner.public_key())
        );
        for event in messages {
            receiver.handle_relay_event(event);
        }
        receiver.handle_relay_event(response);
        assert_eq!(
            receiver
                .protocol_engine
                .as_ref()
                .unwrap()
                .active_session_count_for_owner(sender_owner.public_key()),
            1
        );
        assert!(
            receiver
                .threads
                .get(&sender_owner.public_key().to_hex())
                .is_some_and(|thread| {
                    thread
                        .messages
                        .iter()
                        .any(|message| message.body == "delivered with the device approval")
                }),
            "the private-invite path must release the original message, linked={linked}"
        );
    }
}
