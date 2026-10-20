#[test]
fn direct_message_and_handshake_wire_hide_sender_identity() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    let sent = f
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "private wire identity regression",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let mut messages = 0;
    for effect in &sent.effects {
        let ProtocolEffect::Publish(publish) = effect;
        if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND {
            messages += 1;
            assert_eq!(
                publish
                    .event
                    .tags
                    .public_keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![f.ready_device.public_key()]
            );
        } else if publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
            assert_eq!(
                publish
                    .event
                    .tags
                    .public_keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![
                    public_device(f.ready.local_invite().unwrap().inviter_ephemeral_public_key)
                        .unwrap()
                ]
            );
        } else {
            panic!("unexpected private publish kind");
        }
        let wire = serde_json::to_string(&publish.event).unwrap();
        for identity in [&f.owner, &f.device, &f.peer_owner, &f.late_device] {
            assert!(
                !wire.contains(&identity.public_key().to_hex()),
                "private exchange exposed a sender identity or recipient account"
            );
        }
        for identity in [
            &f.owner,
            &f.device,
            &f.peer_owner,
            &f.ready_device,
            &f.late_device,
        ] {
            assert_ne!(
                publish.event.pubkey,
                identity.public_key(),
                "private event signed by an identity"
            );
        }
        assert!(publish.event.verify().is_ok());
    }
    assert!(messages > 0);
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
        1
    );
}

#[test]
fn public_invite_cannot_identify_the_sender_of_a_private_reply() {
    let mut f = remote_send_fixture();
    // A relay can read the public invite and its device/account authorization.
    let public_invite_key =
        public_device(f.ready.local_invite().unwrap().inviter_ephemeral_public_key).unwrap();
    let peer = f.peer_owner.public_key();
    let sent = f
        .sender
        .send_direct_text(peer, &peer.to_hex(), "hello", None, UnixSeconds(10))
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
        1
    );
    let owner = f.owner.public_key();
    let reply = f
        .ready
        .send_direct_text(owner, &owner.to_hex(), "reply", None, UnixSeconds(11))
        .unwrap();
    let messages = reply
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND =>
            {
                Some(&publish.event)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!messages.is_empty());
    for event in messages {
        assert_ne!(
            event.pubkey, public_invite_key,
            "public invite links this reply signer to the sender identity"
        );
        let wire = serde_json::to_string(event).unwrap();
        for identity in [f.peer_owner.public_key(), f.ready_device.public_key()] {
            assert!(!wire.contains(&identity.to_hex()));
        }
        assert!(event.verify().is_ok());
    }
    assert_eq!(
        decrypt_own_sync_effects(&mut f.sender, reply.effects).len(),
        1
    );
}
