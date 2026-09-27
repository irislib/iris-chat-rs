#[test]
fn forwarded_content_survives_encryption_projection_and_storage_without_source_author() {
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let source_owner = Keys::generate();
    let mut sender = logged_in_test_core("forward-sender", &sender_owner, &sender_device);
    let mut receiver = logged_in_test_core("forward-receiver", &receiver_owner, &receiver_device);
    let invite = create_private_invite_for_test(&mut receiver);
    prove_invite_owner(&mut sender, &receiver_owner, &receiver_device, 10);
    sender.upsert_local_app_key_device_with_labels(
        sender_owner.public_key(),
        sender_device.public_key(),
        None,
        true,
    );
    sender.publish_local_app_keys_snapshot_only("test_forward");
    sender.pending_relay_publishes.clear();
    sender.handle_action(AppAction::AcceptInvite {
        invite_input: invite,
    });

    let source = test_chat_message(
        &source_owner.public_key().to_hex(),
        "source-message",
        "Bring a picnic blanket.",
        unix_now().get(),
        false,
    );
    let text = crate::format_forwarded_message(source.body.clone());
    let target = receiver_owner.public_key().to_hex();
    sender.handle_action(AppAction::SendMessage {
        chat_id: target.clone(),
        text: text.clone(),
    });
    let outgoing = sender
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.body == text)
        .unwrap();
    assert_eq!(
        outgoing.author_owner_pubkey_hex,
        Some(sender_owner.public_key().to_hex())
    );
    assert!(outgoing.is_outgoing);

    for event in pending_events_with_kind(&sender, INVITE_RESPONSE_KIND)
        .into_iter()
        .chain(pending_events_with_kind(&sender, MESSAGE_EVENT_KIND))
    {
        receiver.handle_relay_event(event);
    }
    let chat_id = sender_owner.public_key().to_hex();
    receiver.handle_action(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    });
    let received = receiver
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.body == text)
        .expect("decrypted forward in UI projection");
    assert_eq!(received.body, "Forwarded:\n\nBring a picnic blanket.");
    assert_eq!(received.author_owner_pubkey_hex, Some(chat_id.clone()));
    assert!(!received.is_outgoing);
    assert!(!received.body.contains(&source_owner.public_key().to_hex()));
    assert!(!received.body.contains(&source.id));
    let received_id = received.id.clone();
    let persisted = receiver.load_persisted().unwrap().unwrap();
    let stored = persisted
        .threads
        .iter()
        .find(|t| t.chat_id == chat_id)
        .unwrap()
        .messages
        .iter()
        .find(|m| m.id == received_id)
        .unwrap();
    assert_eq!(super::chats::chat_message_from_persisted(stored).body, text);
    assert_eq!(
        crate::format_forwarded_message(text.clone()),
        text,
        "forwarding again keeps one heading"
    );
}

#[test]
fn forwarded_attachment_keeps_heading_in_group_timeline_and_persistence() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("forward-group", &owner, &device);
    core.handle_action(AppAction::CreateGroup {
        name: "Notes".into(),
        member_inputs: vec![],
    });
    let chat_id = core.state.current_chat.as_ref().unwrap().chat_id.clone();
    let url = "htree://nhash1example/photo.jpg";
    core.handle_action(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: crate::format_forwarded_message(format!("A picnic\n{url}")),
    });
    let message = core
        .state
        .current_chat
        .as_ref()
        .unwrap()
        .messages
        .iter()
        .find(|m| m.body.starts_with("Forwarded:"))
        .unwrap()
        .clone();
    assert_eq!(message.body, "Forwarded:\n\nA picnic");
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].htree_url, url);
    let persisted = core.load_persisted().unwrap().unwrap();
    let stored = persisted
        .threads
        .iter()
        .find(|t| t.chat_id == chat_id)
        .unwrap()
        .messages
        .iter()
        .find(|m| m.id == message.id)
        .unwrap();
    let restored = super::chats::chat_message_from_persisted(stored);
    assert_eq!(restored.body, message.body);
    assert_eq!(restored.attachments, message.attachments);
}
