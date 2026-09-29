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

#[test]
fn blocking_an_owner_rejects_all_devices_live_and_after_cold_start() {
    let mut fixture = super::super::tests::Fixture::new();
    let callers = [Keys::generate(), Keys::generate()];
    let local = fixture.core.logged_in.as_ref().unwrap().device_keys.clone();
    let devices = &mut fixture
        .core
        .app_keys
        .get_mut(&fixture.owner)
        .unwrap()
        .devices;
    for (device, keys) in devices.iter_mut().zip(&callers) {
        device.identity_pubkey_hex = keys.public_key().to_hex();
    }
    fixture.core.persist_best_effort();
    let signal = Signal::new("offer", "00112233445566778899aabbccddeeff", true, false);
    let payloads: Vec<_> = callers
        .iter()
        .map(|caller| {
            let event = wake_event(caller, local.public_key(), &signal).unwrap();
            serde_json::json!({"event": event}).to_string()
        })
        .collect();
    let resolve = |core: &AppCore, payload: &str| {
        resolve_call_push_invite(
            core.data_dir.to_string_lossy().into(),
            local.secret_key().to_secret_hex(),
            payload.into(),
        )
    };
    for payload in &payloads {
        assert!(resolve(&fixture.core, payload).is_some());
    }
    assert_eq!(
        fixture
            .core
            .build_mobile_push_sync_snapshot()
            .call_author_pubkeys
            .len(),
        2
    );
    fixture.core.handle_action(AppAction::SetUserBlocked {
        owner_pubkey_hex: fixture.owner.clone(),
        blocked: true,
    });
    assert!(fixture
        .core
        .build_mobile_push_sync_snapshot()
        .call_author_pubkeys
        .is_empty());
    for payload in &payloads {
        assert!(
            resolve(&fixture.core, payload).is_none(),
            "cold-start lookup must use the persisted block"
        );
        fixture.core.ingest_mobile_push_payload(payload);
        assert!(fixture.core.calls.active.is_none());
        assert!(fixture.core.state.call.is_none());
    }
    fixture.core.handle_action(AppAction::StartCall {
        chat_id: fixture.owner.clone(),
        video: true,
    });
    assert!(fixture.core.calls.active.is_none());
    assert!(fixture.core.state.call.is_none());
}

#[test]
fn incoming_message_requests_cannot_call_until_accepted_even_when_unknown_messages_are_allowed() {
    for video in [false, true] {
        for accept_unknown in [false, true] {
            let mut fixture = super::super::tests::Fixture::new();
            let caller = Keys::generate();
            let local = fixture.core.logged_in.as_ref().unwrap().device_keys.clone();
            fixture
                .core
                .app_keys
                .get_mut(&fixture.owner)
                .unwrap()
                .devices[0]
                .identity_pubkey_hex = caller.public_key().to_hex();
            fixture.core.preferences.accepted_owner_pubkeys.clear();
            let peer = PublicKey::from_hex(&fixture.owner).unwrap();
            let mut message = UnsignedEvent::new(
                peer,
                Timestamp::now(),
                Kind::Custom(CHAT_MESSAGE_KIND as u16),
                vec![],
                "Hello",
            );
            message.ensure_id();
            fixture.core.apply_decrypted_runtime_message(
                peer,
                None,
                serde_json::to_string(&message).unwrap(),
                None,
            );
            assert!(fixture.core.threads[&fixture.owner]
                .messages
                .iter()
                .any(|message| !message.is_outgoing));
            assert!(!fixture.core.threads[&fixture.owner]
                .messages
                .iter()
                .any(|message| message.is_outgoing));
            fixture
                .core
                .handle_action(AppAction::SetAcceptUnknownDirectMessages {
                    enabled: accept_unknown,
                });
            fixture.core.persist_best_effort();
            let signal = Signal::new("offer", "00112233445566778899aabbccddeeff", video, false);
            let event = wake_event(&caller, local.public_key(), &signal).unwrap();
            let payload = serde_json::json!({"event": event}).to_string();
            let resolve = |core: &AppCore| {
                resolve_call_push_invite(
                    core.data_dir.to_string_lossy().into(),
                    local.secret_key().to_secret_hex(),
                    payload.clone(),
                )
            };
            assert!(resolve(&fixture.core).is_none());
            fixture.core.ingest_mobile_push_payload(&payload);
            fixture.core.handle_call_packet(
                &caller.public_key().to_hex(),
                PORT,
                &serde_json::to_vec(&signal).unwrap(),
            );
            assert!(fixture.core.state.call.is_none());
            assert!(fixture
                .core
                .build_mobile_push_sync_snapshot()
                .call_author_pubkeys
                .is_empty());

            fixture
                .core
                .handle_action(AppAction::SetMessageRequestAccepted {
                    chat_id: fixture.owner.clone(),
                });
            assert!(resolve(&fixture.core).is_some());
            fixture.core.ingest_mobile_push_payload(&payload);
            assert_eq!(fixture.core.state.call.as_ref().unwrap().phase, "incoming");
            fixture.core.handle_action(AppAction::SetUserBlocked {
                owner_pubkey_hex: fixture.owner.clone(),
                blocked: true,
            });
            assert!(
                resolve(&fixture.core).is_none(),
                "blocking overrides prior chat acceptance"
            );
            assert!(fixture.core.calls.active.is_none());
        }
    }
}
