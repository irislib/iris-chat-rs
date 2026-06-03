#[test]
fn first_contact_receiver_bootstrap_fetches_preexisting_payload() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "receiver-first-contact-bootstrap-alice",
        &alice_owner,
        &alice_device,
    );
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core(
        "receiver-first-contact-bootstrap-bob",
        &bob_owner,
        &bob_device,
    );
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite { invite_input: invite_url });
    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "payload before bootstrap".to_string(),
    });

    let response_event = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");
    let payload_event = bob
        .pending_relay_publishes
        .values()
        .find_map(|pending| {
            let event = serde_json::from_str::<Event>(&pending.event_json).ok()?;
            (event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                && pending.inner_event_id.is_some()
                && pending.chat_id.is_some())
            .then_some(event)
        })
        .expect("payload event");
    let payload_event_id = payload_event.id.to_string();
    let payload_author = payload_event.pubkey;

    alice.handle_relay_event(payload_event.clone());
    assert!(
        !alice.has_seen_event(&payload_event_id),
        "unknown first-contact payload must stay retryable until bootstrap installs the author"
    );
    assert!(
        !alice.threads.contains_key(&bob_owner.public_key().to_hex()),
        "payload must not create a chat before the invite response is processed"
    );
    let pending_inbound = alice
        .protocol_engine
        .as_ref()
        .map(|engine| engine.pending_inbound_for_test())
        .unwrap_or_default();
    assert_eq!(
        pending_inbound.len(),
        1,
        "unknown header payload should stay as scoped retryable work"
    );
    let pending = pending_inbound.first().expect("pending inbound");
    assert_eq!(pending.event_id, payload_event_id);
    assert_eq!(
        pending.sender_message_pubkey_hex.as_deref(),
        Some(payload_author.to_hex().as_str())
    );
    assert!(
        pending.has_envelope && pending.metadata_verified,
        "pending inbound work must keep verified payload metadata"
    );

    alice.handle_relay_event(response_event);
    assert!(
        alice
            .protocol_engine
            .as_ref()
            .is_some_and(|engine| engine.is_known_message_author(payload_author)),
        "invite response must install the sender message author"
    );
    assert!(
        has_filter_with_kind_author(
            &alice.recent_protocol_filters(UnixSeconds(1_777_159_500)),
            MESSAGE_EVENT_KIND,
            payload_author,
        ),
        "receiver must ask relays for messages from the newly discovered author"
    );

    let decrypted = alice
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .process_direct_message_event(&payload_event)
        .expect("process fetched payload")
        .expect("payload decrypts after bootstrap");
    assert_eq!(decrypted.sender, bob_owner.public_key());
    let runtime_rumor = parse_runtime_rumor(&decrypted.content).expect("runtime rumor");
    assert_eq!(runtime_rumor.content, "payload before bootstrap");
}
