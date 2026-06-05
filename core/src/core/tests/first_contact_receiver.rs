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

#[test]
fn cold_app_key_first_direct_message_is_recipient_scoped() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "cold-appkey-first-message-alice",
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

    let mut bob_acceptor = logged_in_test_core(
        "cold-appkey-first-message-bob-acceptor",
        &bob_owner,
        &bob_device,
    );
    bob_acceptor.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    let response = pending_events_with_kind(&bob_acceptor, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("bob invite response");
    let bob_bootstrap_messages = pending_events_with_kind(&bob_acceptor, MESSAGE_EVENT_KIND);
    alice.handle_relay_event(response);
    for event in bob_bootstrap_messages {
        alice.handle_relay_event(event);
    }
    assert!(
        alice.protocol_engine.as_ref().is_some_and(|engine| {
            engine.active_session_count_for_owner(bob_owner.public_key()) > 0
        }),
        "Alice should have a Bob session before sending"
    );

    let mut bob = logged_in_test_core(
        "cold-appkey-first-message-bob-receiver",
        &bob_owner,
        &bob_device,
    );
    alice.pending_relay_publishes.clear();
    bob.pending_relay_publishes.clear();

    let alice_app_keys = AppKeys::new(vec![DeviceEntry::new(alice_device.public_key(), 1)]);
    let bob_app_keys = AppKeys::new(vec![DeviceEntry::new(bob_device.public_key(), 1)]);
    for (core, owner, app_keys) in [
        (&mut alice, bob_owner.public_key(), bob_app_keys.clone()),
        (&mut bob, alice_owner.public_key(), alice_app_keys.clone()),
    ] {
        let batch = core
            .protocol_engine
            .as_mut()
            .expect("protocol engine")
            .ingest_app_keys_snapshot(owner, app_keys.clone(), 1)
            .expect("ingest peer app keys");
        core.process_protocol_engine_retry_batch("test_app_keys", batch);
        core.app_keys
            .insert(owner.to_hex(), known_app_keys_from_ndr(owner, &app_keys, 1));
    }

    bob.active_chat_id = Some(alice_owner.public_key().to_hex());
    alice.handle_action(AppAction::SendMessage {
        chat_id: bob_owner.public_key().to_hex(),
        text: "cold app-key hello".to_string(),
    });

    let message_events = pending_events_with_kind(&alice, MESSAGE_EVENT_KIND);
    assert!(
        !message_events.is_empty(),
        "cold app-key direct send should publish at least one message event; pending={} debug={:?}",
        alice.pending_relay_publishes.len(),
        alice
            .debug_log
            .iter()
            .map(|entry| format!("{}:{}", entry.category, entry.detail))
            .collect::<Vec<_>>()
    );
    assert!(
        message_events
            .iter()
            .all(|event| event_has_pubkey_tag_for_test(event, bob_device.public_key())),
        "cold app-key direct sends must tag Bob's device as the relay-visible recipient"
    );
    let filters = bob.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        has_filter_with_kind_pubkey_for_test(&filters, MESSAGE_EVENT_KIND, bob_device.public_key()),
        "Bob must ask relays for cold first messages addressed to his device; filters={:?}",
        filters
            .iter()
            .map(|filter| serde_json::to_value(filter).expect("filter json"))
            .collect::<Vec<_>>()
    );
}

fn event_has_pubkey_tag_for_test(event: &Event, pubkey: PublicKey) -> bool {
    let pubkey_hex = pubkey.to_hex();
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(|value| value.as_str()) == Some("p")
            && values.get(1).map(|value| value.as_str()) == Some(pubkey_hex.as_str())
    })
}

fn has_filter_with_kind_pubkey_for_test(filters: &[Filter], kind: u32, pubkey: PublicKey) -> bool {
    let pubkey_hex = pubkey.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| kinds.iter().any(|value| value.as_u64() == Some(kind as u64)));
            let has_pubkey = filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|value| value.as_str() == Some(pubkey_hex.as_str()))
                });
            has_kind && has_pubkey
        })
}
