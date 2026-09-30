use nostr_double_ratchet::InviteNostrExt;

fn deliver_first_contact_effects(
    receiver: &mut ProtocolEngine,
    device: &Keys,
    effects: &[ProtocolEffect],
) -> Vec<ProtocolDecryptedMessage> {
    let invite_author = receiver.local_invite_response_pubkey().unwrap();
    let mut messages = Vec::new();
    for ProtocolEffect::Publish(publish) in effects {
        let target = if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
            invite_author
        } else {
            device.public_key()
        };
        if !publish
            .event
            .tags
            .public_keys()
            .any(|recipient| *recipient == target)
        {
            continue;
        }
        match publish.event.kind.as_u16() as u32 {
            INVITE_RESPONSE_KIND => messages.extend(
                receiver
                    .observe_invite_response_event(&publish.event)
                    .unwrap()
                    .direct_messages,
            ),
            MESSAGE_EVENT_KIND => messages.extend(
                receiver
                    .process_direct_message_event(&publish.event)
                    .unwrap(),
            ),
            _ => panic!("unexpected first-contact event"),
        }
    }
    messages
}

fn first_contact_texts(messages: Vec<ProtocolDecryptedMessage>) -> Vec<String> {
    messages
        .into_iter()
        .filter_map(|message| {
            let inner: UnsignedEvent = serde_json::from_str(&message.content).unwrap();
            (inner.kind.as_u16() == 14).then_some(inner.content)
        })
        .collect()
}

#[test]
fn first_contact_reply_to_existing_four_device_owner_survives_missing_invite() {
    let account_owner = Keys::generate();
    let devices = (0..4).map(|_| Keys::generate()).collect::<Vec<_>>();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let account_store = Arc::new(InMemoryStorage::new());
    let peer_store = Arc::new(InMemoryStorage::new());
    let mut account = ProtocolEngine::load_or_create_for_local_device(
        account_store.clone(),
        account_owner.public_key(),
        &devices[0],
    )
    .unwrap();
    let mut siblings = devices[1..]
        .iter()
        .map(|device| test_engine(&account_owner, device))
        .collect::<Vec<_>>();
    let mut peer = ProtocolEngine::load_or_create_for_local_device(
        peer_store.clone(),
        peer_owner.public_key(),
        &peer_device,
    )
    .unwrap();
    let account_roster = signed_app_keys(
        &account_owner,
        &devices.iter().map(Keys::public_key).collect::<Vec<_>>(),
        10,
    );
    let peer_roster = signed_app_keys(&peer_owner, &[peer_device.public_key()], 10);
    for engine in std::iter::once(&mut account)
        .chain(siblings.iter_mut())
        .chain(std::iter::once(&mut peer))
    {
        engine.ingest_app_keys_event(&account_roster).unwrap();
        engine.ingest_app_keys_event(&peer_roster).unwrap();
    }

    // Open a real chat link from an existing account: both participants have
    // distinct account/device identities. One linked-device invite is absent.
    let public_link = peer
        .local_invite()
        .unwrap()
        .get_url("https://chat.iris.to")
        .unwrap();
    let invite = Invite::from_url(&public_link).unwrap();
    let accepted = match account
        .accept_invite(&invite, Some(peer_owner.public_key()))
        .unwrap()
    {
        ProtocolAcceptInviteOutcome::Accepted(accepted) => accepted,
        ProtocolAcceptInviteOutcome::Blocked(_) => panic!("signed owner proof must allow invite"),
    };
    deliver_first_contact_effects(&mut peer, &peer_device, &accepted.effects);
    for index in 0..2 {
        let invite_event = invite_unsigned_event(&siblings[index].local_invite().unwrap())
            .unwrap()
            .sign_with_keys(&devices[index + 1])
            .unwrap();
        peer.observe_invite_event(&invite_event).unwrap();
    }
    let outgoing = account
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "first contact from existing account",
            None,
            UnixSeconds(20),
        )
        .unwrap();
    assert_eq!(
        first_contact_texts(deliver_first_contact_effects(
            &mut peer,
            &peer_device,
            &outgoing.effects
        )),
        vec!["first contact from existing account"],
    );
    account = ProtocolEngine::load_or_create_for_local_device(
        account_store,
        account_owner.public_key(),
        &devices[0],
    )
    .unwrap();

    let reply = peer
        .send_direct_text(
            account_owner.public_key(),
            &account_owner.public_key().to_hex(),
            "reply while fourth device is unavailable",
            None,
            UnixSeconds(21),
        )
        .expect("one missing device invite must not suppress a reply to reachable devices");
    assert_eq!(
        first_contact_texts(deliver_first_contact_effects(
            &mut account,
            &devices[0],
            &reply.effects
        )),
        vec!["reply while fourth device is unavailable"],
    );
    for index in 0..2 {
        assert_eq!(
            first_contact_texts(deliver_first_contact_effects(
                &mut siblings[index],
                &devices[index + 1],
                &reply.effects
            )),
            vec!["reply while fourth device is unavailable"],
        );
    }
    peer = ProtocolEngine::load_or_create_for_local_device(
        peer_store,
        peer_owner.public_key(),
        &peer_device,
    )
    .unwrap();
    let last_invite = invite_unsigned_event(&siblings[2].local_invite().unwrap())
        .unwrap()
        .sign_with_keys(&devices[3])
        .unwrap();
    let recovered = peer.observe_invite_event(&last_invite).unwrap();
    assert_eq!(
        first_contact_texts(deliver_first_contact_effects(
            &mut siblings[2],
            &devices[3],
            &recovered.effects
        )),
        vec!["reply while fourth device is unavailable"],
    );
    assert!(
        deliver_first_contact_effects(&mut account, &devices[0], &recovered.effects).is_empty()
    );
}
