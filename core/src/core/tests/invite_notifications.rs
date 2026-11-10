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
        for private in [false, true] {
            let owner = Keys::generate();
            let device = Keys::generate();
            let peer_owner = Keys::generate();
            let peer_device = Keys::generate();
            let mut core = isolated_invite_ui_core("new-chat-bundled-proof", &owner, &device);
            let invite = if private {
                let url = create_private_invite_for_test(&mut core);
                crate::core::invites::parse_public_invite_input(&url).unwrap()
            } else {
                core.protocol_engine
                    .as_ref()
                    .unwrap()
                    .local_invite()
                    .unwrap()
            };
            let proof =
                signed_app_keys_authorization_event(&peer_owner, peer_device.public_key(), 10);
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
                assert!(resolution.should_show, "private={private}");
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
            assert_eq!(notices.len(), 1, "private={private}");
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
    }

    #[test]
    fn new_chat_notifications_reject_missing_wrong_and_revoked_proof_and_respect_settings() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let other = Keys::generate();
        let mut core = isolated_invite_ui_core("new-chat-proof-policy", &owner, &device);
        let invite = core
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap();
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
    fn new_chat_notifications_accept_proved_arrival_with_full_unproved_queue() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let mut core = isolated_invite_ui_core("new-chat-full-waiting-queue", &owner, &device);
        let invite = core
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap();
        for _ in 0..32 {
            let unproved_owner = Keys::generate();
            let unproved_device = Keys::generate();
            core.handle_relay_event(response_with_proof(
                &invite,
                &unproved_owner,
                &unproved_device,
                None,
            ));
        }
        assert_eq!(core.pending_new_chat_responses.len(), 32);
        let before = core.state.chat_list.clone();

        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let wrong_owner = Keys::generate();
        let wrong_proof =
            signed_app_keys_authorization_event(&wrong_owner, peer_device.public_key(), 10);
        core.handle_relay_event(response_with_proof(
            &invite,
            &peer_owner,
            &peer_device,
            Some(&wrong_proof),
        ));
        assert!(candidates(&before, &core).is_empty());
        assert!(!core.threads.contains_key(&peer_owner.public_key().to_hex()));
        assert_eq!(core.pending_new_chat_responses.len(), 32);

        let proof = signed_app_keys_authorization_event(&peer_owner, peer_device.public_key(), 10);
        let event = response_with_proof(&invite, &peer_owner, &peer_device, Some(&proof));
        core.handle_relay_event(event.clone());
        let notices = candidates(&before, &core);
        assert_eq!(
            notices.len(),
            1,
            "unproved senders must not crowd out a proved new chat"
        );
        assert_eq!(notices[0].chat_id, peer_owner.public_key().to_hex());
        assert_eq!(notices[0].title, "New chat");
        assert!(notices[0].body.is_empty());
        assert_eq!(core.pending_new_chat_responses.len(), 32);
        assert_eq!(
            crate::core::invite_notifications::load_pending_responses(
                &core.private_chat_invite_storage().unwrap()
            )
            .unwrap()
            .len(),
            32
        );
        let after = core.state.chat_list.clone();
        core.handle_relay_event(event);
        assert!(candidates(&after, &core).is_empty());
    }

    #[test]
    fn new_chat_notifications_wait_for_proof_across_restart() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer_owner = Keys::generate();
        let peer_device = Keys::generate();
        let (mut core, _, directory) =
            logged_in_test_core_with_updates("new-chat-delayed-proof", &owner, &device);
        core.preferences.nostr_relay_urls.clear();
        let invite = core
            .protocol_engine
            .as_ref()
            .unwrap()
            .local_invite()
            .unwrap();
        let event = response_with_proof(&invite, &peer_owner, &peer_device, None);
        core.handle_relay_event(event.clone());
        assert_eq!(core.pending_new_chat_responses.len(), 1);
        assert!(!core.threads.contains_key(&peer_owner.public_key().to_hex()));
        drop(core);
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            directory.path().to_string_lossy().into_owned(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.preferences.nostr_relay_urls.clear();
        core.start_primary_session(owner.clone(), device.clone(), true, true)
            .unwrap();
        assert_eq!(core.pending_new_chat_responses.len(), 1);
        let before = core.state.chat_list.clone();
        prove_invite_owner(&mut core, &peer_owner, &peer_device, 10);
        assert_eq!(candidates(&before, &core).len(), 1);
        assert!(core.pending_new_chat_responses.is_empty());
        let after = core.state.chat_list.clone();
        core.handle_relay_event(event);
        assert!(candidates(&after, &core).is_empty());
    }
}
