#[test]
fn retry_batch_publish_registration_blocks_delivery_until_relay_success() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("retry-batch-publish-blocks-delivery", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    let chat_id = peer.public_key().to_hex();
    let message_id = "retry-batch-message".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "retry payload".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );

    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "payload")
        .sign_with_keys(&device)
        .expect("payload event");
    let event_id = event.id.to_string();

    core.process_protocol_engine_retry_batch(
        "test_retry_publish_ordering",
        ProtocolRetryBatch {
            effects: vec![ProtocolEffect::Publish(ProtocolPublish {
                event,
                chat_id: chat_id.clone(),
                inner_event_id: Some(message_id.clone()),
            })],
            ..ProtocolRetryBatch::default()
        },
    );

    assert!(
        core.pending_relay_publishes.contains_key(&event_id),
        "retry publish must be persisted before delivery reconciliation"
    );
    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.iter().find(|message| message.id == message_id))
        .expect("message after retry batch");
    assert_eq!(message.delivery, DeliveryState::Pending);

    core.handle_relay_publish_finished(
        event_id,
        true,
        vec!["wss://relay.example".to_string()],
        "retry batch publish accepted".to_string(),
    );

    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.iter().find(|message| message.id == message_id))
        .expect("message after relay ack");
    assert_eq!(message.delivery, DeliveryState::Sent);
}
