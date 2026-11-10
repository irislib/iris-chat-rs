use super::*;

mod invite_notification_regressions {
    use super::*;
    use nostr::JsonUtil;

    fn response_with_proof(
        invite: &Invite,
        owner: &Keys,
        device: &Keys,
        proof: Option<&Event>,
    ) -> Event {
        let (_, response) = invite
            .accept_with_owner(
                device.public_key(),
                device.secret_key().to_secret_bytes(),
                Some(device.public_key().to_hex()),
                Some(owner.public_key()),
            )
            .unwrap();
        let event = invite_response_event(&response).unwrap();
        let Some(proof) = proof else {
            return event;
        };
        let keys = Keys::new(nostr::SecretKey::from_slice(&response.signer_secret_key).unwrap());
        let recipient = PublicKey::from_slice(&response.recipient.to_bytes()).unwrap();
        let encrypted = nostr::nips::nip44::encrypt(
            keys.secret_key(),
            &recipient,
            proof.as_json(),
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        EventBuilder::new(event.kind, event.content.clone())
            .tags(event.tags.iter().cloned())
            .tag(nostr::Tag::parse(["owner-proof", encrypted.as_str()]).unwrap())
            .custom_created_at(event.created_at)
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn preview(
        core: &AppCore,
        owner: &Keys,
        device: &Keys,
        event: &Event,
    ) -> MobilePushNotificationResolution {
        decrypt_mobile_push_notification(
            core.data_dir.to_string_lossy().into_owned(),
            owner.public_key().to_hex(),
            device.secret_key().to_secret_hex(),
            serde_json::json!({"event": event}).to_string(),
        )
    }

    fn candidates(
        before: &[ChatThreadSnapshot],
        core: &AppCore,
    ) -> Vec<notifications::NotificationCandidate> {
        notifications::decide_notifications(
            before,
            &core.state.chat_list,
            &core.state.preferences,
            false,
            None,
        )
    }

    #[test]
    fn new_chat_notifications_use_bundled_proof_without_mutating_storage_or_leaking_identities() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let mut core = isolated_invite_ui_core("new-chat-bundled-proof", &owner, &device);
        request_network_peer(&mut core, &owner, peer_owner.public_key());
        let url = create_private_invite_for_test(&mut core);
        let invite = crate::core::invites::parse_public_invite_input(&url).unwrap();
        let proof = signed_app_keys_authorization_event(&peer_owner, peer_device.public_key(), 10);
        let event = response_with_proof(&invite, &peer_owner, &peer_device, Some(&proof));
        core.persist_app_state().unwrap();
        let storage = core.private_chat_invite_storage().unwrap();
        let snapshot = || {
            storage
                .list("")
                .unwrap()
                .into_iter()
                .map(|key| {
                    let value = storage.get(&key).unwrap();
                    (key, value)
                })
                .collect::<BTreeMap<_, _>>()
        };
        let saved = snapshot();
        for _ in 0..2 {
            let resolution = preview(&core, &owner, &device, &event);
            assert!(resolution.should_show);
            assert_eq!(resolution.title, "New chat");
            assert_eq!(resolution.body, "");
            let payload: serde_json::Value =
                serde_json::from_str(&resolution.payload_json).unwrap();
            assert_eq!(payload["chat_id"], peer_owner.public_key().to_hex());
        }
        assert_eq!(snapshot(), saved, "notification preview must be read-only");
        let wire = nostr::ClientMessage::event(event.clone()).as_json();
        let sync = core.build_mobile_push_sync_snapshot();
        assert!(sync
            .invite_response_pubkeys
            .contains(&invite.inviter_ephemeral_public_key.to_string()));
        let request = crate::core::mobile_push::build_mobile_push_create_subscription_request(
            owner.secret_key().to_secret_hex(),
            "ios".into(),
            "test-token".into(),
            Some("test.iris".into()),
            sync.message_author_pubkeys,
            sync.background_message_author_pubkeys,
            sync.invite_response_pubkeys,
            false,
            Some("https://example.invalid".into()),
            sync.delayed_message_authors,
        )
        .unwrap()
        .body_json
        .unwrap();
        for identity in [&owner, &device, &peer_owner, &peer_device] {
            assert!(!wire.contains(&identity.public_key().to_hex()));
            assert!(!request.contains(&identity.public_key().to_hex()));
        }
        let before = core.state.chat_list.clone();
        core.handle_relay_event(event.clone());
        let notices = candidates(&before, &core);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "New chat");
        assert_eq!(notices[0].body, "");
        assert_eq!(notices[0].chat_id, peer_owner.public_key().to_hex());
        assert!(notices[0]
            .message_id
            .as_ref()
            .unwrap()
            .starts_with("invite:"));
        assert_eq!(
            core.threads[&peer_owner.public_key().to_hex()].unread_count,
            0
        );
        // One-use private invites are gone by the time APNs may decrypt.
        assert!(preview(&core, &owner, &device, &event).should_show);
        let after = core.state.chat_list.clone();
        core.handle_relay_event(event);
        assert!(candidates(&after, &core).is_empty());
    }

    #[test]
    fn new_chat_notifications_reject_missing_wrong_and_revoked_proof_and_respect_settings() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let other = Keys::generate();
        let mut core = isolated_invite_ui_core("new-chat-proof-policy", &owner, &device);
        request_network_peer(&mut core, &owner, peer_owner.public_key());
        let url = create_private_invite_for_test(&mut core);
        let invite = crate::core::invites::parse_public_invite_input(&url).unwrap();
        let good = signed_app_keys_authorization_event(&peer_owner, peer_device.public_key(), 10);
        let wrong = signed_app_keys_authorization_event(&other, peer_device.public_key(), 10);
        for proof in [None, Some(&wrong)] {
            let event = response_with_proof(&invite, &peer_owner, &peer_device, proof);
            assert!(!preview(&core, &owner, &device, &event).should_show);
        }
        let event = response_with_proof(&invite, &peer_owner, &peer_device, Some(&good));
        for mode in [
            "disabled",
            "invites-disabled",
            "muted",
            "blocked",
            "enabled",
        ] {
            core.preferences.desktop_notifications_enabled = mode != "disabled";
            core.preferences.invite_acceptance_notifications_enabled = mode != "invites-disabled";
            core.preferences.muted_chat_ids = if mode == "muted" {
                vec![peer_owner.public_key().to_hex()]
            } else {
                vec![]
            };
            core.preferences.blocked_owner_pubkeys = if mode == "blocked" {
                vec![peer_owner.public_key().to_hex()]
            } else {
                vec![]
            };
            core.persist_app_state().unwrap();
            assert_eq!(
                preview(&core, &owner, &device, &event).should_show,
                mode == "enabled",
                "{mode}"
            );
        }
        core.handle_relay_event(signed_app_keys_authorization_event(
            &peer_owner,
            other.public_key(),
            20,
        ));
        assert!(
            !preview(&core, &owner, &device, &event).should_show,
            "older bundled proof cannot undo revocation"
        );
    }

    #[test]
    fn automatic_device_handshakes_stay_silent_before_and_after_proof() {
        for bundled in [false, true] {
            let owner = Keys::generate();
            let device = Keys::generate();
            let peer_owner = Keys::generate();
            let peer_device = Keys::generate();
            let mut core = isolated_invite_ui_core("silent-transport-handshake", &owner, &device);
            request_network_peer(&mut core, &owner, peer_owner.public_key());
            let invite = core
                .protocol_engine
                .as_ref()
                .unwrap()
                .local_invite()
                .unwrap();
            let proof =
                signed_app_keys_authorization_event(&peer_owner, peer_device.public_key(), 10);
            let event = response_with_proof(
                &invite,
                &peer_owner,
                &peer_device,
                bundled.then_some(&proof),
            );
            core.persist_app_state().unwrap();
            assert!(!preview(&core, &owner, &device, &event).should_show);
            let before = core.state.chat_list.clone();
            core.handle_relay_event(event.clone());
            core.handle_relay_event(proof);
            assert!(candidates(&before, &core).is_empty());
            assert!(!core.threads.contains_key(&peer_owner.public_key().to_hex()));
            assert!(
                core.protocol_engine
                    .as_ref()
                    .unwrap()
                    .active_session_count_for_owner(peer_owner.public_key())
                    > 0
            );
            assert!(!preview(&core, &owner, &device, &event).should_show);
            core.handle_relay_event(event);
            assert!(candidates(&before, &core).is_empty());
        }
    }
}
