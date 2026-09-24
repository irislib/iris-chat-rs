use super::*;

#[test]
fn wake_is_authenticated_encrypted_recipient_bound_and_offer_only() {
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let other = Keys::generate();
    let signal = Signal::new("offer", "00112233445566778899aabbccddeeff", true, false);
    let event = wake_event(&sender, recipient.public_key(), &signal).unwrap();
    assert!(!event.content.contains(&signal.call_id));
    assert_eq!(
        wake_signal(&event, &recipient).unwrap().call_id,
        signal.call_id
    );
    assert!(wake_signal(&event, &other).is_none());
    let mut tampered = event.clone();
    tampered.content.push('a');
    assert!(wake_signal(&tampered, &recipient).is_none());
    let stale = EventBuilder::new(event.kind, &event.content)
        .tags(event.tags.clone())
        .custom_created_at(Timestamp::from(unix_now().get() - 41))
        .sign_with_keys(&sender)
        .unwrap();
    assert!(wake_signal(&stale, &recipient).is_none());
    let end = wake_event(
        &sender,
        recipient.public_key(),
        &Signal::new("end", &signal.call_id, false, false),
    )
    .unwrap();
    assert!(wake_signal(&end, &recipient).is_none());
}

#[test]
fn call_push_subscription_uses_dedicated_device_filter_and_voip_topic() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let caller = Keys::generate();
    let request = build_call_push_subscription_request(
        owner.secret_key().to_secret_hex(),
        device.public_key().to_hex(),
        vec![caller.public_key().to_hex()],
        None,
        "ios".into(),
        "test-token".into(),
        Some("to.iris.chat".into()),
        false,
        None,
    )
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&request.body_json.unwrap()).unwrap();
    assert_eq!(body["apns_topic"], "to.iris.chat.voip");
    assert_eq!(body["apns_environment"], "development");
    assert_eq!(body["filter"]["kinds"][0], CALL_WAKE_KIND);
    assert_eq!(body["filter"]["#p"][0], device.public_key().to_hex());
    assert_eq!(body["filter"]["authors"][0], caller.public_key().to_hex());
    assert_eq!(body["fcm_tokens"], serde_json::json!([]));
}

#[test]
fn wake_does_not_ring_unknown_blocked_disabled_or_replayed_calls() {
    let mut fixture = super::super::tests::Fixture::new();
    let sender = Keys::generate();
    let local = fixture.core.logged_in.as_ref().unwrap().device_keys.clone();
    let signal = Signal::new("offer", "00112233445566778899aabbccddeeff", false, false);
    let event = wake_event(&sender, local.public_key(), &signal).unwrap();
    fixture.core.receive_call_push(&event);
    assert!(fixture.core.state.call.is_none());
    let keys = fixture.core.app_keys.get_mut(&fixture.owner).unwrap();
    keys.devices[0].identity_pubkey_hex = sender.public_key().to_hex();
    fixture
        .core
        .preferences
        .blocked_owner_pubkeys
        .push(fixture.owner.clone());
    fixture.core.receive_call_push(&event);
    assert!(fixture.core.state.call.is_none());
    fixture.core.preferences.blocked_owner_pubkeys.clear();
    fixture.core.preferences.voice_calls_enabled = false;
    fixture.core.receive_call_push(&event);
    assert!(fixture.core.state.call.is_none());
    fixture.core.preferences.voice_calls_enabled = true;
    let next = wake_event(
        &sender,
        local.public_key(),
        &Signal::new("offer", "112233445566778899aabbccddeeff00", false, false),
    )
    .unwrap();
    fixture.core.receive_call_push(&next);
    assert_eq!(fixture.core.state.call.as_ref().unwrap().phase, "incoming");
    let id = fixture.core.state.call.as_ref().unwrap().call_id.clone();
    fixture.core.handle_action(AppAction::EndCall {
        call_id: id.clone(),
    });
    fixture
        .core
        .handle_action(AppAction::EndCall { call_id: id });
    fixture.core.receive_call_push(&next);
    assert!(fixture.core.state.call.is_none());
}
