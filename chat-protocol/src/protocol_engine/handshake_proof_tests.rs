#[test]
fn linked_device_handshake_authorizes_messages_without_separate_roster_delivery() {
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let mut sender = test_engine(&sender_owner, &sender_device);
    let mut receiver = test_engine(&receiver_owner, &receiver_device);
    // These engines receive only account public keys and device secret keys.
    // The signed authorization is the public proof obtained when linking.
    sender
        .ingest_app_keys_event(&signed_app_keys(
            &sender_owner,
            &[sender_device.public_key()],
            10,
        ))
        .unwrap();
    sender
        .ingest_app_keys_event(&signed_app_keys(
            &receiver_owner,
            &[receiver_device.public_key()],
            10,
        ))
        .unwrap();
    let ProtocolAcceptInviteOutcome::Accepted(accepted) = sender
        .accept_invite(
            &receiver.local_invite().unwrap(),
            Some(receiver_owner.public_key()),
        )
        .unwrap()
    else {
        panic!("acceptance should succeed");
    };
    let response = accepted
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND =>
            {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .unwrap();
    // Deliver only the handshake, never the sender's standalone registration.
    receiver.observe_invite_response_event(&response).unwrap();
    assert_eq!(
        receiver.active_session_count_for_owner(sender_owner.public_key()),
        1,
        "the encrypted handshake must carry sufficient signed device proof"
    );
}

#[test]
fn handshake_proof_delivers_out_of_order_messages_and_survives_restart() {
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let sender_store = Arc::new(InMemoryStorage::new());
    let receiver_store = Arc::new(InMemoryStorage::new());
    let mut sender = ProtocolEngine::load_or_create_for_local_device(
        sender_store.clone(),
        sender_owner.public_key(),
        &sender_device,
    )
    .unwrap();
    let mut receiver = ProtocolEngine::load_or_create_for_local_device(
        receiver_store.clone(),
        receiver_owner.public_key(),
        &receiver_device,
    )
    .unwrap();
    let proof = signed_app_keys(&sender_owner, &[sender_device.public_key()], 10);
    sender.ingest_app_keys_event(&proof).unwrap();
    sender
        .ingest_app_keys_event(&signed_app_keys(
            &receiver_owner,
            &[receiver_device.public_key()],
            10,
        ))
        .unwrap();
    // Restart the linked device with no account secret key.
    sender = ProtocolEngine::load_or_create_for_local_device(
        sender_store,
        sender_owner.public_key(),
        &sender_device,
    )
    .unwrap();
    let ProtocolAcceptInviteOutcome::Accepted(accepted) = sender
        .accept_invite(
            &receiver.local_invite().unwrap(),
            Some(receiver_owner.public_key()),
        )
        .unwrap()
    else {
        panic!("accept invite");
    };
    let response = accepted
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND =>
            {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .unwrap();
    let outer = serde_json::to_string(&response).unwrap();
    assert!(!outer.contains(&sender_owner.public_key().to_hex()));
    assert!(!outer.contains(&sender_device.public_key().to_hex()));
    assert!(!outer.contains(&proof.sig.to_string()));
    let rumor = nostr::EventBuilder::new(Kind::Custom(14), "message before handshake")
        .build(sender_owner.public_key());
    let sent = sender
        .send_direct_unsigned_event_to_peer_only(
            receiver_owner.public_key(),
            &receiver_owner.public_key().to_hex(),
            rumor,
            unix_now(),
        )
        .unwrap();
    let message = sent
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND =>
            {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(receiver
        .process_direct_message_event(&message)
        .unwrap()
        .is_none());
    let batch = receiver.observe_invite_response_event(&response).unwrap();
    assert_eq!(batch.direct_messages.len(), 1);
    assert_eq!(batch.direct_messages[0].sender, sender_owner.public_key());
    assert!(batch.direct_messages[0]
        .content
        .contains("message before handshake"));
    receiver = ProtocolEngine::load_or_create_for_local_device(
        receiver_store,
        receiver_owner.public_key(),
        &receiver_device,
    )
    .unwrap();
    assert_eq!(
        receiver.active_session_count_for_owner(sender_owner.public_key()),
        1
    );
    assert!(receiver.has_verified_device_owner_claim(
        ndr_owner(sender_owner.public_key()),
        ndr_device(sender_device.public_key())
    ));
}

#[test]
fn handshake_proof_rejects_forgery_wrong_bindings_and_known_revocations() {
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let receiver_owner = Keys::generate();
    let receiver_device = Keys::generate();
    let stranger = Keys::generate();
    let valid = signed_app_keys(&sender_owner, &[sender_device.public_key()], 10);
    let mut forged_json = serde_json::to_value(&valid).unwrap();
    forged_json["content"] = serde_json::json!("forged");
    let forged: Event = serde_json::from_value(forged_json).unwrap();
    let wrong_owner = signed_app_keys(&stranger, &[sender_device.public_key()], 10);
    let wrong_device = signed_app_keys(&sender_owner, &[stranger.public_key()], 10);
    let future = signed_app_keys(
        &sender_owner,
        &[sender_device.public_key()],
        unix_now().get() + 3600,
    );
    for (proof, revoked) in [
        (&forged, false),
        (&wrong_owner, false),
        (&wrong_device, false),
        (&future, false),
        (&valid, true),
    ] {
        let mut receiver = test_engine(&receiver_owner, &receiver_device);
        if revoked {
            receiver
                .ingest_app_keys_event(&signed_app_keys(&sender_owner, &[], 20))
                .unwrap();
        }
        let invite = receiver.local_invite().unwrap();
        let (_, response) = invite
            .accept_with_owner(
                sender_device.public_key(),
                sender_device.secret_key().to_secret_bytes(),
                None,
                Some(sender_owner.public_key()),
            )
            .unwrap();
        let event = invite_response_with_owner_proof(&response, Some(proof)).unwrap();
        receiver.observe_invite_response_event(&event).unwrap();
        assert_eq!(
            receiver.active_session_count_for_owner(sender_owner.public_key()),
            0
        );
    }
}
