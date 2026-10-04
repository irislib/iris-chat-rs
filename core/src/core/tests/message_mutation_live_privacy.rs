#[test]
fn message_mutations_own_live_delivery_uses_guarded_record_sync_only() {
    for grouped in [false, true] {
        for target_time in [None, Some(50), Some(200)] {
            let owner = Keys::generate();
            let a = Keys::generate();
            let b = Keys::generate();
            let contact = Keys::generate();
            let (mut core, _, _directory) =
                logged_in_test_core_with_updates("mutation-live-privacy", &owner, &b);
            configure_test_device_sync_profile(&mut core, &owner, &b, &a, None);
            let chat = if grouped {
                core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(
                    test_group_snapshot(
                        "live-privacy",
                        "Private",
                        owner.public_key(),
                        vec![owner.public_key(), contact.public_key()],
                        vec![owner.public_key()],
                        1,
                    ),
                ));
                group_chat_id("live-privacy")
            } else {
                contact.public_key().to_hex()
            };
            if let Some(time) = target_time {
                core.apply_runtime_text_message(
                    owner.public_key(),
                    Some(chat.clone()),
                    "Original".into(),
                    time,
                    None,
                    Some("original".into()),
                    None,
                );
            }
            let mut tags = vec![nostr::Tag::parse(["e", "original"]).unwrap()];
            if grouped {
                tags.push(nostr::Tag::parse(["l", "live-privacy"]).unwrap());
            } else {
                tags.push(nostr::Tag::parse(["p", chat.as_str()]).unwrap());
            }
            let event = UnsignedEvent::new(
                owner.public_key(),
                Timestamp::from_secs(201),
                Kind::Custom(MESSAGE_EDIT_KIND as u16),
                tags,
                "UNSHARED_LIVE_REPLACEMENT",
            );
            if grouped {
                assert!(
                    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
                        nostr_double_ratchet::GroupReceivedMessage {
                            group_id: "live-privacy".into(),
                            revision: 1,
                            body: serde_json::to_vec(&event).unwrap(),
                            sender_owner: NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
                            sender_device: Some(NdrDevicePubkey::from_bytes(
                                a.public_key().to_bytes()
                            )),
                        }
                    ))
                );
            } else {
                assert!(core.apply_decrypted_runtime_message_with_metadata(
                    owner.public_key(),
                    Some(a.public_key()),
                    Some(contact.public_key()),
                    serde_json::to_string(&event).unwrap(),
                    None,
                    201
                ));
            }
            assert!(core.message_mutation_records(&chat, "original").is_empty());
            if target_time.is_some() {
                assert_eq!(
                    core.message_for_mutation(&chat, "original").unwrap().body,
                    "Original"
                );
            }
            let stored: u64 = core
                .app_store
                .shared()
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM app_meta WHERE value LIKE '%UNSHARED_LIVE_REPLACEMENT%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored, 0);
        }
    }
}

#[test]
fn message_mutations_send_to_participants_and_sync_note_to_self_without_sibling_fanout() {
    for self_chat in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let sibling = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, _dir) =
            logged_in_test_core_with_updates("mutation-send-privacy", &owner, &device);
        configure_test_device_sync_profile(&mut core, &owner, &device, &sibling, None);
        let chat = if self_chat {
            owner.public_key().to_hex()
        } else {
            peer.public_key().to_hex()
        };
        core.apply_runtime_text_message(
            owner.public_key(),
            Some(chat.clone()),
            "Original".into(),
            50,
            None,
            Some("original".into()),
            None,
        );
        let peer_device = Keys::generate();
        let engine = core.protocol_engine.as_mut().unwrap();
        observe_peer_appkeys_for_test(
            engine,
            &owner,
            &[device.public_key(), sibling.public_key()],
            1,
        );
        if !self_chat {
            observe_peer_appkeys_for_test(engine, &peer, &[peer_device.public_key()], 1);
            observe_peer_device_invite_for_test(engine, &peer, &peer_device, 2);
        }
        core.mutate_own_message(&chat, "original", Some("Edited here"));
        assert_eq!(
            core.message_for_mutation(&chat, "original").unwrap().body,
            "Edited here"
        );
        assert_eq!(core.message_mutation_records(&chat, "original").len(), 1);
        let storage = crate::core::storage::SqliteStorageAdapter::new(
            core.app_store.shared(),
            owner.public_key().to_hex(),
            device.public_key().to_hex(),
        );
        let state: serde_json::Value = serde_json::from_str(
            &storage
                .get("appcore/protocol-engine-state-v1")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            state["pending_local_sibling_sends"]
                .as_array()
                .unwrap()
                .len(),
            0,
            "Own-device replacement content must only use target-aware record sync"
        );
        assert_eq!(state["pending_remote_sends"].as_array().unwrap().len(), 0);
    }
}
