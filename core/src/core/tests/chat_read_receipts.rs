fn chat_read_receipt_pair(label: &str) -> ChatReadSyncPair {
    let mut pair = chat_read_sync_pair(label);
    install_two_way_local_sibling_state_for_test(
        &mut pair.a,
        &mut pair.b,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
    );
    pair.a.pending_relay_publishes.clear();
    pair.b.pending_relay_publishes.clear();
    pair
}

#[test]
fn chat_read_receipt_before_message_persists_progress_for_delayed_delivery() {
    let mut pair = chat_read_receipt_pair("read-receipt-before-message");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let message_id = "a".repeat(64);
    chat_read_sync_incoming(&mut pair.a, &peer, &message_id, 200);
    pair.a
        .mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));
    assert!(!pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND).is_empty());

    // No device-sync snapshot and no original message reaches the sibling.
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    let expected = pair.a.chat_read_states[&chat_id].clone();
    assert_eq!(pair.b.chat_read_states.get(&chat_id), Some(&expected));
    assert_eq!(
        pair.b
            .app_store
            .load_chat_read_states()
            .unwrap()
            .get(&chat_id),
        Some(&expected),
        "the encrypted receipt must durably record progress before history exists"
    );
    assert!(pair
        .b
        .threads
        .get(&chat_id)
        .is_none_or(|thread| thread.messages.is_empty()));

    drop(pair.b);
    let mut restarted = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.b_device,
        pair._b_dir.path().to_string_lossy().into_owned(),
    );
    restarted.load_persisted().unwrap();
    chat_read_sync_incoming(&mut restarted, &peer, &message_id, 200);
    assert_eq!(restarted.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&restarted, &chat_id, &message_id, true);

    let later_id = "b".repeat(64);
    chat_read_sync_incoming(&mut restarted, &peer, &later_id, 200);
    assert_eq!(restarted.threads[&chat_id].unread_count, 1);
    assert_chat_read_delivery(&restarted, &chat_id, &later_id, false);
    assert_eq!(protocol_send_log_count(&pair.a, "receipt"), 0);
}

#[test]
fn chat_read_receipt_clears_sibling_and_gossips_progress_to_another_device() {
    let mut pair = chat_read_receipt_pair("read-receipt-gossip");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let message_id = "c".repeat(64);
    for core in [&mut pair.a, &mut pair.b] {
        chat_read_sync_incoming(core, &peer, &message_id, 200);
    }
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    pair.a
        .mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));
    assert!(!pending_events_with_kind(&pair.a, MESSAGE_EVENT_KIND).is_empty());
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);

    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_eq!(
        pair.b
            .state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat list row")
            .unread_count,
        0
    );
    assert_chat_read_delivery(&pair.b, &chat_id, &message_id, true);
    assert_eq!(
        pair.b.chat_read_states.get(&chat_id),
        pair.a.chat_read_states.get(&chat_id),
        "only the encrypted receipt supplied the read frontier"
    );
    assert_eq!(protocol_send_log_count(&pair.a, "receipt"), 0);
    assert_eq!(protocol_send_log_count(&pair.b, "receipt"), 0);

    let c_device = Keys::generate();
    let (mut c, _, _c_dir) =
        logged_in_test_core_with_updates("read-receipt-gossip-c", &pair.owner, &c_device);
    configure_test_device_sync_profile(&mut c, &pair.owner, &c_device, &pair.b_device, None);
    let roster = AppKeys::new(vec![
        DeviceEntry::new(pair.a_device.public_key(), 1),
        DeviceEntry::new(pair.b_device.public_key(), 1),
        DeviceEntry::new(c_device.public_key(), 1),
    ]);
    for core in [&mut pair.b, &mut c] {
        core.apply_known_app_keys_snapshot(pair.owner.public_key(), &roster, 100);
    }
    chat_read_sync_incoming(&mut c, &peer, &message_id, 200);
    assert_eq!(c.threads[&chat_id].unread_count, 1);
    assert!(!c.chat_read_states.contains_key(&chat_id));

    // The original reader is unavailable; a receipt recipient must be able
    // to forward the durable frontier in its normal recovery snapshot.
    sync_chat_reads(&pair.b, &mut c, &pair.b_device, false);
    assert_eq!(c.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&c, &chat_id, &message_id, true);
    assert_eq!(
        c.chat_read_states[&chat_id],
        pair.b.chat_read_states[&chat_id]
    );
}

#[test]
fn chat_read_receipt_from_remote_peer_cannot_apply_an_own_read_state_tag() {
    let mut pair = chat_read_sync_pair("read-receipt-foreign-tag");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let message_id = "d".repeat(64);
    chat_read_sync_incoming(&mut pair.b, &peer, &message_id, 200);
    let forged = ChatReadState {
        updated_at_ms: unix_now().get().saturating_mul(1000),
        device_id: pair.a_device.public_key().to_hex(),
        seen_through_secs: 200,
        seen_at_boundary: [message_id.clone()].into_iter().collect(),
    };
    let (content, _) = runtime_rumor_json(
        peer.public_key(),
        RECEIPT_KIND,
        "seen",
        unix_now().get(),
        vec![
            vec!["e".to_string(), message_id.clone()],
            vec![
                "iris-read-state".to_string(),
                serde_json::to_string(&forged).unwrap(),
            ],
        ],
    );
    pair.b
        .apply_decrypted_runtime_message(peer.public_key(), None, content, None);

    assert!(!pair.b.chat_read_states.contains_key(&chat_id));
    assert!(!pair
        .b
        .app_store
        .load_chat_read_states()
        .unwrap()
        .contains_key(&chat_id));
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    assert_chat_read_delivery(&pair.b, &chat_id, &message_id, false);
}

#[test]
fn chat_read_receipt_without_history_gossips_to_a_third_device() {
    let mut pair = chat_read_receipt_pair("read-receipt-before-history-gossip");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let message_id = "d".repeat(64);
    chat_read_sync_incoming(&mut pair.a, &peer, &message_id, 200);
    pair.a
        .mark_messages_seen(&chat_id, std::slice::from_ref(&message_id));
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    assert!(!pair.b.threads.contains_key(&chat_id));

    let c_device = Keys::generate();
    let (mut c, _, _c_dir) =
        logged_in_test_core_with_updates("read-before-history-c", &pair.owner, &c_device);
    configure_test_device_sync_profile(&mut c, &pair.owner, &c_device, &pair.b_device, None);
    let roster = AppKeys::new(vec![
        DeviceEntry::new(pair.a_device.public_key(), 1),
        DeviceEntry::new(pair.b_device.public_key(), 1),
        DeviceEntry::new(c_device.public_key(), 1),
    ]);
    for core in [&mut pair.b, &mut c] {
        core.apply_known_app_keys_snapshot(pair.owner.public_key(), &roster, 100);
    }
    chat_read_sync_incoming(&mut c, &peer, &message_id, 200);
    assert_eq!(c.threads[&chat_id].unread_count, 1);
    sync_chat_reads(&pair.b, &mut c, &pair.b_device, false);
    assert_eq!(c.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&c, &chat_id, &message_id, true);
}

#[test]
fn encrypted_receipts_update_stored_messages_outside_the_loaded_chat_window() {
    for linked_reader in [false, true] {
        let alice_owner = Keys::generate();
        let alice_device = Keys::generate();
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let mut alice = logged_in_test_core("receipt-window-alice", &alice_owner, &alice_device);
        let mut bob = logged_in_test_core("receipt-window-bob", &bob_owner, &bob_device);
        let alice_chat = bob_owner.public_key().to_hex();
        let bob_chat = alice_owner.public_key().to_hex();
        for (core, owner, device) in [
            (&mut alice, &alice_owner, &alice_device),
            (&mut bob, &bob_owner, &bob_device),
        ] {
            core.upsert_local_app_key_device_with_labels(
                owner.public_key(),
                device.public_key(),
                None,
                true,
            );
            core.publish_local_app_keys_snapshot_only("receipt_window_test");
        }
        let invite = create_private_invite_for_test(&mut bob);
        prove_invite_owner(&mut alice, &bob_owner, &bob_device, 10);
        bob.preferences.send_read_receipts = true;
        bob.accept_direct_peer(&bob_chat);
        if linked_reader {
            bob.logged_in.as_mut().unwrap().owner_keys = None;
        }
        alice.handle_action(AppAction::AcceptInvite {
            invite_input: invite,
        });
        for text in ["first waiting for receipt", "second waiting for receipt"] {
            alice.handle_action(AppAction::SendMessage {
                chat_id: alice_chat.clone(),
                text: text.to_string(),
            });
        }
        deliver_pending_relay_events_for_test(&alice, &mut bob);
        let receipt_ids = bob.threads[&bob_chat]
            .messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(receipt_ids.len(), 2);
        assert_eq!(bob.pending_delivered_receipts.len(), 2);

        // A newer outgoing message becomes the only loaded preview after an
        // inactive chat is restored. The older messages still exist in SQLite.
        alice.handle_action(AppAction::SendMessage {
            chat_id: alice_chat.clone(),
            text: "newer unsent-to-peer preview".to_string(),
        });
        alice.active_chat_id = None;
        alice.persist_best_effort();
        let restored = alice.load_persisted().unwrap().unwrap();
        let preview = restored
            .threads
            .iter()
            .find(|thread| thread.chat_id == alice_chat)
            .unwrap();
        assert_eq!(preview.messages.len(), 1);
        let newest_id = preview.messages[0].id.clone();
        alice.threads.get_mut(&alice_chat).unwrap().messages = preview
            .messages
            .iter()
            .map(super::chats::chat_message_from_persisted)
            .collect();
        assert!(receipt_ids.iter().all(|id| id != &newest_id));

        bob.pending_relay_publishes.clear();
        bob.flush_all_pending_delivered_receipts_for_test();
        assert!(!pending_events_with_kind(&bob, MESSAGE_EVENT_KIND).is_empty());
        deliver_pending_relay_events_for_test(&bob, &mut alice);
        for id in &receipt_ids {
            let stored = alice
                .app_store
                .load_messages_around(&alice_chat, id, 0, 0)
                .unwrap();
            assert_eq!(DeliveryState::from(stored[0].delivery.clone()), DeliveryState::Received,
                "encrypted delivered receipt was lost for unloaded message; linked_reader={linked_reader}");
            assert_eq!(
                stored[0].recipient_deliveries[0].owner_pubkey_hex,
                alice_chat
            );
        }

        // Seen must also reach disk when the acknowledged rows are unloaded.
        alice
            .threads
            .get_mut(&alice_chat)
            .unwrap()
            .messages
            .retain(|message| message.id == newest_id);
        bob.pending_relay_publishes.clear();
        bob.mark_messages_seen(&bob_chat, &receipt_ids[..1]);
        deliver_pending_relay_events_for_test(&bob, &mut alice);
        let first = alice
            .app_store
            .load_messages_around(&alice_chat, &receipt_ids[0], 0, 0)
            .unwrap();
        assert_eq!(
            DeliveryState::from(first[0].delivery.clone()),
            DeliveryState::Seen
        );
        let second = alice
            .app_store
            .load_messages_around(&alice_chat, &receipt_ids[1], 0, 0)
            .unwrap();
        assert_eq!(
            DeliveryState::from(second[0].delivery.clone()),
            DeliveryState::Received
        );
        let newest = alice
            .app_store
            .load_messages_around(&alice_chat, &newest_id, 0, 0)
            .unwrap();
        assert!(!matches!(
            DeliveryState::from(newest[0].delivery.clone()),
            DeliveryState::Received | DeliveryState::Seen
        ));
        assert!(
            alice.active_chat_id.is_none(),
            "receipt processing must not navigate or mark the chat read"
        );
    }
}
