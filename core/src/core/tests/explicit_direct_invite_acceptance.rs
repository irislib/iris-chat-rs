mod explicit_direct_invite_acceptance {
    use super::*;
    use nostr::{ClientMessage, JsonUtil, SubscriptionId};

    struct Pair {
        inviter: AppCore,
        joiner: AppCore,
        inviter_owner: Keys,
        inviter_device: Keys,
        joiner_owner: Keys,
        joiner_device: Keys,
        _inviter_dir: tempfile::TempDir,
        joiner_dir: tempfile::TempDir,
        url: String,
    }

    impl Pair {
        fn new() -> Self {
            let inviter_owner = Keys::generate();
            let inviter_device = Keys::generate();
            let joiner_owner = Keys::generate();
            let joiner_device = Keys::generate();
            let (mut inviter, _, inviter_dir) = logged_in_test_core_with_updates(
                "explicit-invite-sender",
                &inviter_owner,
                &inviter_device,
            );
            let (mut joiner, _, joiner_dir) = logged_in_test_core_with_updates(
                "explicit-invite-receiver",
                &joiner_owner,
                &joiner_device,
            );
            for (core, owner, device) in [
                (&mut inviter, &inviter_owner, &inviter_device),
                (&mut joiner, &joiner_owner, &joiner_device),
            ] {
                core.preferences.nearby_enabled = false;
                core.preferences.nostr_relay_urls.clear();
                core.preferences.send_read_receipts = true;
                core.upsert_local_app_key_device_with_labels(
                    owner.public_key(),
                    device.public_key(),
                    None,
                    true,
                );
                core.publish_local_app_keys_snapshot_only("explicit_invite_test");
                core.pending_relay_publishes.clear();
            }
            inviter.handle_action(AppAction::CreateFullInvite);
            let url = inviter
                .state
                .public_invite
                .as_ref()
                .unwrap()
                .legacy_url
                .clone();
            Self {
                inviter,
                joiner,
                inviter_owner,
                inviter_device,
                joiner_owner,
                joiner_device,
                _inviter_dir: inviter_dir,
                joiner_dir,
                url,
            }
        }

        fn accept(&mut self, delayed: bool) {
            if !delayed {
                prove_invite_owner(
                    &mut self.joiner,
                    &self.inviter_owner,
                    &self.inviter_device,
                    10,
                );
            }
            self.joiner.handle_action(AppAction::AcceptInvite {
                invite_input: self.url.clone(),
            });
            if delayed {
                assert!(self
                    .joiner
                    .request_decision(&self.inviter_owner.public_key().to_hex())
                    .is_none_or(|state| state.decision != "accepted"));
                assert!(self.joiner.pending_outgoing_invite_acceptance.is_some());
                prove_invite_owner(
                    &mut self.joiner,
                    &self.inviter_owner,
                    &self.inviter_device,
                    10,
                );
            }
            assert!(self.joiner.pending_outgoing_invite_acceptance.is_none());
            assert_eq!(
                self.joiner
                    .request_decision(&self.inviter_owner.public_key().to_hex())
                    .unwrap()
                    .decision,
                "accepted"
            );
            let invite = super::super::invites::parse_public_invite_input(&self.url).unwrap();
            let responses = pending_events_with_kind(&self.joiner, INVITE_RESPONSE_KIND);
            assert!(!responses.is_empty());
            for response in responses {
                let wire = ClientMessage::event(response.clone()).as_json();
                for identity in [
                    &self.inviter_owner,
                    &self.inviter_device,
                    &self.joiner_owner,
                    &self.joiner_device,
                ] {
                    assert!(!wire.contains(&identity.public_key().to_hex()));
                }
                assert_eq!(
                    response
                        .tags
                        .public_keys()
                        .map(|key| key.to_hex())
                        .collect::<Vec<_>>(),
                    vec![invite.inviter_ephemeral_public_key.to_string()]
                );
            }
            deliver_pending_relay_events_for_test(&self.joiner, &mut self.inviter);
            prove_invite_owner(
                &mut self.inviter,
                &self.joiner_owner,
                &self.joiner_device,
                10,
            );
            settle_pending_relay_events_for_test(&mut self.inviter, &mut self.joiner);
            self.inviter.pending_relay_publishes.clear();
            self.joiner.pending_relay_publishes.clear();
        }

        fn first_message_and_receipt(&mut self) {
            let sender_chat = self.joiner_owner.public_key().to_hex();
            let receiver_chat = self.inviter_owner.public_key().to_hex();
            assert!(
                self.joiner.threads[&receiver_chat].messages.is_empty(),
                "no outgoing reply/manual request acceptance"
            );
            self.inviter.handle_action(AppAction::SendMessage {
                chat_id: sender_chat.clone(),
                text: "first message after explicit invite".into(),
            });
            let id = self.inviter.threads[&sender_chat]
                .messages
                .last()
                .unwrap()
                .id
                .clone();
            deliver_pending_relay_events_for_test(&self.inviter, &mut self.joiner);
            assert!(!self.joiner.thread_is_message_request(&receiver_chat));
            let incoming = self.joiner.threads[&receiver_chat]
                .messages
                .iter()
                .find(|m| m.id == id)
                .unwrap();
            assert!(!incoming.is_outgoing);
            assert_eq!(incoming.body, "first message after explicit invite");
            assert!(self
                .joiner
                .pending_delivered_receipts
                .contains_key(&(receiver_chat, id.clone())));
            self.joiner.flush_all_pending_delivered_receipts_for_test();
            let events = pending_events_with_kind(&self.joiner, MESSAGE_EVENT_KIND);
            assert!(
                !events.is_empty(),
                "authenticated delivery receipt must be published"
            );
            for event in &events {
                let wire = ClientMessage::event(event.clone()).as_json();
                let frame: serde_json::Value = serde_json::from_str(&wire).unwrap();
                let received = Event::from_json(frame[1].to_string()).unwrap();
                assert!(nostr_double_ratchet::has_metadata_privacy_version(
                    &received
                ));
                for identity in [&self.inviter_owner, &self.joiner_owner, &self.joiner_device] {
                    assert!(
                        !wire.contains(&identity.public_key().to_hex()),
                        "receipt exposes a participant identity"
                    );
                }
                assert_ne!(received.pubkey, self.inviter_device.public_key());
                assert!(
                    !wire.contains(&id),
                    "receipt targets belong inside the ratchet ciphertext"
                );
                self.inviter.handle_relay_event(received);
            }
            let message = self.inviter.threads[&sender_chat]
                .messages
                .iter()
                .find(|m| m.id == id)
                .unwrap();
            assert_eq!(message.delivery, DeliveryState::Received);
            assert!(message
                .recipient_deliveries
                .iter()
                .any(|delivery| delivery.owner_pubkey_hex == sender_chat
                    && delivery.delivery == DeliveryState::Received));
            let filters = self
                .inviter
                .recent_protocol_filters(unix_now())
                .into_iter()
                .filter(|filter| {
                    serde_json::to_value(filter).unwrap()["kinds"]
                        .as_array()
                        .is_some_and(|kinds| {
                            kinds
                                .iter()
                                .any(|kind| kind.as_u64() == Some(u64::from(MESSAGE_EVENT_KIND)))
                        })
                })
                .collect::<Vec<_>>();
            assert!(!filters.is_empty());
            let wire =
                ClientMessage::req(SubscriptionId::new("receipt-privacy"), filters).as_json();
            for identity in [&self.inviter_owner, &self.joiner_owner, &self.joiner_device] {
                assert!(
                    !wire.contains(&identity.public_key().to_hex()),
                    "receipt subscription links peer identity"
                );
            }
            // Retrying optional server persistence cannot replace the accepted receipt
            // with identity-signed ciphertext or change the receipt's ratchet envelope.
            self.joiner
                .retry_pending_relay_publishes("explicit_invite_receipt_retry");
            assert_eq!(
                pending_events_with_kind(&self.joiner, MESSAGE_EVENT_KIND),
                events
            );
        }
    }

    #[test]
    fn explicit_direct_invite_first_receipt_is_authenticated_and_private() {
        for delayed in [false, true] {
            let mut pair = Pair::new();
            pair.accept(delayed);
            pair.first_message_and_receipt();
        }
    }

    #[test]
    fn explicit_direct_invite_acceptance_survives_restart_before_first_message() {
        let mut pair = Pair::new();
        pair.accept(true);
        pair.joiner.persist_best_effort_inner();
        drop(pair.joiner);
        pair.joiner = logged_in_test_core_at_data_dir(
            &pair.joiner_owner,
            &pair.joiner_device,
            pair.joiner_dir.path().to_string_lossy().into_owned(),
        );
        pair.joiner
            .start_session(
                pair.joiner_owner.public_key(),
                Some(pair.joiner_owner.clone()),
                pair.joiner_device.clone(),
                true,
                true,
            )
            .unwrap();
        pair.joiner.pending_relay_publishes.clear();
        let peer = pair.inviter_owner.public_key().to_hex();
        assert_eq!(
            pair.joiner.request_decision(&peer).unwrap().decision,
            "accepted"
        );
        assert!(pair
            .joiner
            .preferences
            .accepted_owner_pubkeys
            .contains(&peer));
        pair.first_message_and_receipt();
    }

    fn decision(core: &mut AppCore, chat: &str, choice: &str, time: u64) {
        let owner = core.logged_in.as_ref().unwrap().owner_pubkey;
        let event = nostr_identity::build_private_setting(
            owner,
            "requestDecision",
            chat,
            &serde_json::json!({"chatId":chat, "decision":choice, "updatedAtMs":time}),
        )
        .unwrap();
        assert!(core.merge_private_setting(&event));
    }

    #[test]
    fn explicit_direct_invite_delayed_proof_preserves_later_rejection_or_reset() {
        for (choice, prior_logical_decision) in [
            ("rejected", false),
            ("none", false),
            ("rejected", true),
            ("none", true),
        ] {
            let mut pair = Pair::new();
            let peer = pair.inviter_owner.public_key().to_hex();
            let previous = unix_now_ms() + 1000;
            if prior_logical_decision {
                decision(&mut pair.joiner, &peer, "rejected", previous);
            }
            pair.joiner.handle_action(AppAction::AcceptInvite {
                invite_input: pair.url.clone(),
            });
            decision(
                &mut pair.joiner,
                &peer,
                choice,
                if prior_logical_decision {
                    previous + 1
                } else {
                    unix_now_ms()
                },
            );
            prove_invite_owner(
                &mut pair.joiner,
                &pair.inviter_owner,
                &pair.inviter_device,
                10,
            );
            assert_eq!(
                pair.joiner.request_decision(&peer).unwrap().decision,
                choice
            );
            assert!(!pair
                .joiner
                .preferences
                .accepted_owner_pubkeys
                .contains(&peer));
            assert!(pair.joiner.thread_is_message_request(&peer));
            assert!(pair.joiner.pending_delivered_receipts.is_empty());
        }
    }

    #[test]
    fn explicit_direct_invite_record_lookup_preserves_original_choice_time() {
        for (choice, prior_logical_decision) in [
            ("rejected", false),
            ("none", false),
            ("rejected", true),
            ("none", true),
        ] {
            let mut pair = Pair::new();
            let peer = pair.inviter_owner.public_key().to_hex();
            let previous = unix_now_ms() + 1000;
            if prior_logical_decision {
                decision(&mut pair.joiner, &peer, "rejected", previous);
            }
            let (_, issued) = pair
                .inviter
                .prepare_issued_invite(None, None, None)
                .unwrap();
            let intent = pair.joiner.capture_invite_acceptance(None).unwrap();
            // Supply only the external fetch completion, retaining the intent
            // captured when the user originally opened the private link.
            pair.joiner.pending_private_invite_lookup = Some(("private-fetch".into(), intent));
            decision(
                &mut pair.joiner,
                &peer,
                choice,
                if prior_logical_decision {
                    previous + 1
                } else {
                    unix_now_ms()
                },
            );
            pair.joiner.finish_private_invite_lookup(
                "private-fetch".into(),
                pair.joiner_owner.public_key().to_hex(),
                Ok(issued.record),
            );
            prove_invite_owner(
                &mut pair.joiner,
                &pair.inviter_owner,
                &pair.inviter_device,
                10,
            );
            assert_eq!(
                pair.joiner.request_decision(&peer).unwrap().decision,
                choice
            );
            assert!(!pair
                .joiner
                .preferences
                .accepted_owner_pubkeys
                .contains(&peer));
            assert!(pair.joiner.thread_is_message_request(&peer));
        }
    }

    #[test]
    fn explicit_direct_invite_fetched_choice_can_replace_previous_rejection() {
        let mut pair = Pair::new();
        let peer = pair.inviter_owner.public_key().to_hex();
        let previous = unix_now_ms() + 1000;
        decision(&mut pair.joiner, &peer, "rejected", previous);
        let (_, issued) = pair
            .inviter
            .prepare_issued_invite(None, None, None)
            .unwrap();
        let intent = pair.joiner.capture_invite_acceptance(None).unwrap();
        pair.joiner.pending_private_invite_lookup = Some(("fetched-choice".into(), intent));
        pair.joiner.finish_private_invite_lookup(
            "fetched-choice".into(),
            pair.joiner_owner.public_key().to_hex(),
            Ok(issued.record),
        );
        prove_invite_owner(
            &mut pair.joiner,
            &pair.inviter_owner,
            &pair.inviter_device,
            10,
        );
        let accepted = pair.joiner.request_decision(&peer).unwrap();
        assert_eq!(accepted.decision, "accepted");
        assert!(
            serde_json::to_value(accepted).unwrap()["updatedAtMs"]
                .as_u64()
                .unwrap()
                > previous
        );
        deliver_pending_relay_events_for_test(&pair.joiner, &mut pair.inviter);
        prove_invite_owner(
            &mut pair.inviter,
            &pair.joiner_owner,
            &pair.joiner_device,
            10,
        );
        settle_pending_relay_events_for_test(&mut pair.inviter, &mut pair.joiner);
        pair.inviter.pending_relay_publishes.clear();
        pair.joiner.pending_relay_publishes.clear();
        pair.first_message_and_receipt();
    }

    #[test]
    fn explicit_direct_invite_new_choice_can_replace_previous_rejection() {
        let mut pair = Pair::new();
        let peer = pair.inviter_owner.public_key().to_hex();
        let previous = unix_now_ms() + 1000;
        decision(&mut pair.joiner, &peer, "rejected", previous);
        pair.accept(true);
        let accepted = pair.joiner.request_decision(&peer).unwrap();
        assert!(
            serde_json::to_value(accepted).unwrap()["updatedAtMs"]
                .as_u64()
                .unwrap()
                > previous
        );
        pair.first_message_and_receipt();
    }

    #[test]
    fn explicit_direct_invite_blocked_peer_is_not_accepted() {
        for block_after_invite in [false, true] {
            let mut pair = Pair::new();
            let peer = pair.inviter_owner.public_key().to_hex();
            if !block_after_invite {
                pair.joiner.set_user_blocked(&peer, true);
            }
            pair.joiner.handle_action(AppAction::AcceptInvite {
                invite_input: pair.url.clone(),
            });
            if block_after_invite {
                pair.joiner.set_user_blocked(&peer, true);
            }
            prove_invite_owner(
                &mut pair.joiner,
                &pair.inviter_owner,
                &pair.inviter_device,
                10,
            );
            assert!(pair.joiner.is_owner_blocked(&peer));
            assert!(!pair
                .joiner
                .preferences
                .accepted_owner_pubkeys
                .contains(&peer));
            assert!(pair
                .joiner
                .request_decision(&peer)
                .is_none_or(|state| state.decision != "accepted"));
            assert!(pair.joiner.pending_delivered_receipts.is_empty());
        }
    }

    #[test]
    fn explicit_direct_invite_unverified_or_unauthorized_owner_is_not_accepted() {
        for mode in ["malformed", "missing", "unlisted"] {
            let mut pair = Pair::new();
            let peer = pair.inviter_owner.public_key().to_hex();
            let url = if mode == "malformed" {
                "not an invite".into()
            } else {
                pair.url.clone()
            };
            pair.joiner
                .handle_action(AppAction::AcceptInvite { invite_input: url });
            if mode == "unlisted" {
                prove_invite_owner(&mut pair.joiner, &pair.inviter_owner, &Keys::generate(), 10);
            }
            assert!(pair.joiner.request_decision(&peer).is_none());
            assert!(!pair
                .joiner
                .preferences
                .accepted_owner_pubkeys
                .contains(&peer));
            assert!(
                active_session_device_pubkeys(&pair.joiner, pair.inviter_owner.public_key())
                    .is_empty()
            );
            assert!(pair.joiner.pending_relay_publishes.is_empty());
        }
    }

    #[test]
    fn explicit_direct_invite_unsolicited_response_keeps_inviter_request_gate() {
        let mut pair = Pair::new();
        pair.accept(true);
        let peer = pair.joiner_owner.public_key().to_hex();
        assert!(pair.inviter.request_decision(&peer).is_none());
        pair.joiner.handle_action(AppAction::SendMessage {
            chat_id: pair.inviter_owner.public_key().to_hex(),
            text: "unsolicited first reply".into(),
        });
        deliver_pending_relay_events_for_test(&pair.joiner, &mut pair.inviter);
        assert!(pair.inviter.thread_is_message_request(&peer));
        assert!(pair.inviter.pending_delivered_receipts.is_empty());
        assert!(!pair
            .inviter
            .preferences
            .accepted_owner_pubkeys
            .contains(&peer));
    }

    #[test]
    fn explicit_direct_invite_group_join_does_not_accept_admin_conversation() {
        let mut pair = Pair::new();
        pair.inviter.create_group("Invite group", &[]);
        let group = pair.inviter.groups.keys().next().unwrap().clone();
        let (_, issued) = pair
            .inviter
            .prepare_issued_invite(Some(&group), None, None)
            .unwrap();
        pair.joiner.pending_private_invite_lookup = Some((
            "group-lookup".into(),
            pair.joiner.capture_invite_acceptance(None).unwrap(),
        ));
        pair.joiner.finish_private_invite_lookup(
            "group-lookup".into(),
            pair.joiner_owner.public_key().to_hex(),
            Ok(issued.record),
        );
        prove_invite_owner(
            &mut pair.joiner,
            &pair.inviter_owner,
            &pair.inviter_device,
            10,
        );
        let peer = pair.inviter_owner.public_key().to_hex();
        assert!(pair.joiner.request_decision(&peer).is_none());
        assert!(!pair
            .joiner
            .preferences
            .accepted_owner_pubkeys
            .contains(&peer));
        assert!(!pair.joiner.threads.contains_key(&peer));
        assert_eq!(
            pair.joiner.state.toast.as_deref(),
            Some("Waiting for the group admin to connect…")
        );
    }
}
