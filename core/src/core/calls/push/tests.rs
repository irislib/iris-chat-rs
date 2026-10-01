use super::*;

#[test]
fn wake_contains_only_recipient_bound_signed_ratchet_ciphertext() {
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let ratchet = Keys::generate();
    let encrypted = EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        "already-ratcheted-ciphertext",
    )
    .tag(Tag::public_key(recipient.public_key()))
    .sign_with_keys(&ratchet)
    .unwrap();
    let event = wake_event(&sender, &encrypted, None).unwrap();
    assert_eq!(
        wake_messages(&event, recipient.public_key())
            .unwrap()
            .events,
        vec![encrypted]
    );
    assert!(wake_messages(&event, Keys::generate().public_key()).is_none());
    let mut tampered = event.clone();
    tampered.content.push('a');
    assert!(wake_messages(&tampered, recipient.public_key()).is_none());
    let stale = EventBuilder::new(event.kind, &event.content)
        .tags(event.tags.clone())
        .custom_created_at(Timestamp::from(unix_now().get() - 41))
        .sign_with_keys(&sender)
        .unwrap();
    assert!(wake_messages(&stale, recipient.public_key()).is_none());
    for content in [
        "legacy-static-ciphertext".to_string(),
        serde_json::json!({"type":"call-wake","v":2,"events":[],"call_id":"private"}).to_string(),
    ] {
        let old = EventBuilder::new(event.kind, content)
            .tags(event.tags.clone())
            .sign_with_keys(&sender)
            .unwrap();
        assert!(wake_messages(&old, recipient.public_key()).is_none());
    }
}

#[test]
fn call_wake_v2_oversize_bootstrap_uses_only_exact_id_and_rejects_mixed_forms() {
    let sender = Keys::generate();
    let recipient = Keys::generate();
    let response = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "x".repeat(4096))
        .tag(Tag::public_key(Keys::generate().public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let encrypted = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "ratchet-ciphertext")
        .tag(Tag::public_key(recipient.public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let event = wake_event(&sender, &encrypted, Some(&response)).unwrap();
    let decoded = wake_messages(&event, recipient.public_key()).unwrap();
    assert_eq!(decoded.bootstrap_event_id, Some(response.id.to_hex()));
    assert_eq!(decoded.events, vec![encrypted.clone()]);
    assert!(event.as_json().len() <= MAX_WAKE_BYTES);
    for content in [
        serde_json::json!({"type":"call-wake","v":2,"events":[response,encrypted],"bootstrapEventId":response.id.to_hex()}),
        serde_json::json!({"type":"call-wake","v":2,"events":[encrypted],"bootstrapEventId":"not-an-event-id"}),
    ] {
        let invalid = EventBuilder::new(event.kind, content.to_string())
            .tags(event.tags.clone())
            .sign_with_keys(&sender)
            .unwrap();
        assert!(wake_messages(&invalid, recipient.public_key()).is_none());
    }
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
