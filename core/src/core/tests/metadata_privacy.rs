#[test]
fn metadata_privacy_blocks_identity_signers_even_with_a_version_marker() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("metadata-identity-guard", &owner, &device);
    let recipient = Keys::generate().public_key();
    let invite = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .local_invite()
        .unwrap();
    let invite_signer = Keys::new(
        nostr::SecretKey::from_slice(&invite.inviter_ephemeral_private_key.unwrap()).unwrap(),
    );
    let before = core.pending_relay_publishes.len();
    for signer in [&owner, &device, &invite_signer] {
        for kind in [MESSAGE_EVENT_KIND, INVITE_RESPONSE_KIND] {
            let event = EventBuilder::new(Kind::from(kind as u16), "encrypted")
                .tags([
                    nostr::Tag::public_key(recipient),
                    nostr::Tag::parse(["v", "2"]).unwrap(),
                ])
                .sign_with_keys(signer)
                .unwrap();
            assert!(!core.publish_runtime_event(event.clone(), "privacy-test", None));
            core.emit_nearby_published_event(&event);
        }
    }
    assert_eq!(core.pending_relay_publishes.len(), before);
    assert!(!updates
        .try_iter()
        .any(|update| matches!(update, AppUpdate::NearbyPublishedEvent { .. })));
}

#[test]
fn metadata_privacy_quarantines_saved_old_ciphertext_and_preserves_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("metadata-restore-guard", &owner, &device);
    let chat = Keys::generate().public_key().to_hex();
    core.push_outgoing_message_with_id(
        "inner".into(),
        &chat,
        "keep this history".into(),
        unix_now().get(),
        None,
        DeliveryState::Queued,
    );
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "old ciphertext")
        .tag(nostr::Tag::public_key(PublicKey::from_hex(&chat).unwrap()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let pending = PendingRelayPublish {
        owner_pubkey_hex: owner.public_key().to_hex(),
        event_id: event.id.to_hex(),
        label: APPCORE_PROTOCOL_LABEL.into(),
        event_json: event.as_json(),
        inner_event_id: Some("inner".into()),
        chat_id: Some(chat.clone()),
        created_at_secs: event.created_at.as_secs(),
        attempt_count: 1,
        last_error: None,
    };
    core.app_store
        .upsert_pending_relay_publish(&pending)
        .unwrap();
    core.persist_best_effort();
    core.start_session(owner.public_key(), Some(owner.clone()), device, true, true)
        .unwrap();
    core.run_session_startup_follow_up();
    assert!(!core
        .pending_relay_publishes
        .contains_key(&event.id.to_hex()));
    let retained = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    let old = retained
        .iter()
        .find(|row| row.event_id == event.id.to_hex())
        .unwrap();
    assert_eq!(old.event_json, pending.event_json);
    assert!(old
        .last_error
        .as_deref()
        .unwrap()
        .contains("will not be resent"));
    let message = core.threads[&chat]
        .messages
        .iter()
        .find(|message| message.id == "inner")
        .unwrap();
    assert_eq!(message.body, "keep this history");
    assert_eq!(message.delivery, DeliveryState::Failed);
}
