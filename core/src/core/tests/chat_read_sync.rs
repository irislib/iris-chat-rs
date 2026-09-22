struct ChatReadSyncPair {
    owner: Keys,
    a_device: Keys,
    b_device: Keys,
    a: AppCore,
    b: AppCore,
    a_dir: tempfile::TempDir,
    _b_dir: tempfile::TempDir,
}

fn chat_read_sync_pair(label: &str) -> ChatReadSyncPair {
    let owner = Keys::generate();
    let a_device = Keys::generate();
    let b_device = Keys::generate();
    let (mut a, _, a_dir) =
        logged_in_test_core_with_updates(&format!("{label}-a"), &owner, &a_device);
    let (mut b, _, b_dir) =
        logged_in_test_core_with_updates(&format!("{label}-b"), &owner, &b_device);
    configure_test_device_sync_profile(&mut a, &owner, &a_device, &b_device, None);
    configure_test_device_sync_profile(&mut b, &owner, &b_device, &a_device, None);
    a.preferences.send_read_receipts = false;
    b.preferences.send_read_receipts = false;
    ChatReadSyncPair {
        owner,
        a_device,
        b_device,
        a,
        b,
        a_dir,
        _b_dir: b_dir,
    }
}

fn chat_read_sync_incoming(core: &mut AppCore, peer: &Keys, id: &str, created_at: u64) {
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        id.to_string(),
        created_at,
        None,
        Some(id.to_string()),
        None,
    );
}

fn deliver_chat_read_packets(receiver: &mut AppCore, sender: &Keys, packets: &[Vec<u8>]) {
    for packet in packets {
        receiver.handle_device_sync_packet(&sender.public_key().to_hex(), DEVICE_SYNC_PORT, packet);
    }
}

fn sync_chat_reads(sender: &AppCore, receiver: &mut AppCore, device: &Keys, history: bool) {
    deliver_chat_read_packets(
        receiver,
        device,
        &sender.build_device_sync_packets_for_test(100, history),
    );
}

fn assert_chat_read_delivery(core: &AppCore, chat_id: &str, id: &str, seen: bool) {
    let message = core.threads[chat_id]
        .messages
        .iter()
        .find(|message| message.id == id)
        .expect("expected message");
    assert_eq!(
        matches!(message.delivery, DeliveryState::Seen),
        seen,
        "message {id} delivery: {:?}",
        message.delivery
    );
}

#[test]
fn chat_read_sync_clears_sibling_unread_without_peer_receipts() {
    let mut pair = chat_read_sync_pair("read-sync-disabled-receipts");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "read-on-a", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    assert_chat_read_delivery(&pair.b, &chat_id, "read-on-a", false);

    pair.a
        .mark_messages_seen(&chat_id, &["read-on-a".to_string()]);
    let packets = pair.a.build_device_sync_packets_for_test(100, true);
    deliver_chat_read_packets(&mut pair.b, &Keys::generate(), &packets);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    assert!(!pair.b.chat_read_states.contains_key(&chat_id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets);

    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, "read-on-a", true);
    assert_eq!(protocol_send_log_count(&pair.a, "receipt"), 0);
    assert_eq!(protocol_send_log_count(&pair.b, "receipt"), 0);
    assert!(pair.a.pending_outgoing_receipts.is_empty());
    assert!(pair.b.pending_outgoing_receipts.is_empty());
}

#[test]
fn chat_read_sync_clears_group_unread_without_peer_receipts() {
    let mut pair = chat_read_sync_pair("read-sync-group");
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let group_id = "read-sync-friends".to_string();
    let chat_id = group_chat_id(&group_id);
    let (body, id) = runtime_rumor_json(
        peer.public_key(),
        CHAT_MESSAGE_KIND,
        "read this group message",
        200,
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    pair.a
        .apply_group_decrypted_event(GroupIncomingEvent::Message(
            nostr_double_ratchet::GroupReceivedMessage {
                group_id,
                sender_owner: ndr_owner_pubkey(peer.public_key()),
                sender_device: Some(ndr_device_pubkey(peer_device.public_key())),
                body: body.into_bytes(),
                revision: 1,
            },
        ));
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);

    pair.a
        .mark_messages_seen(&chat_id, std::slice::from_ref(&id));
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);

    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, &id, true);
    assert!(pair.a.pending_outgoing_receipts.is_empty());
    assert!(pair.b.pending_outgoing_receipts.is_empty());
}

#[test]
fn chat_read_sync_opening_an_already_seen_chat_clears_sibling_badge_after_upgrade() {
    let mut pair = chat_read_sync_pair("read-sync-open-upgrade");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "seen-before-upgrade", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    let thread = pair.a.threads.get_mut(&chat_id).unwrap();
    thread.messages[0].delivery = DeliveryState::Seen;
    thread.unread_count = 0;
    pair.a.persist_best_effort();
    assert!(!pair.a.chat_read_states.contains_key(&chat_id));
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);

    pair.a.handle_action(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    });
    pair.a.open_chat_finalize(&chat_id);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);

    assert!(pair.a.chat_read_states.contains_key(&chat_id));
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, "seen-before-upgrade", true);
}

#[test]
fn chat_read_sync_manual_mark_read_syncs_but_mark_unread_stays_local() {
    let mut pair = chat_read_sync_pair("read-sync-manual");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "manual-read", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);

    pair.a.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: false,
    });
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, "manual-read", true);

    let read_state = pair.a.chat_read_states[&chat_id].clone();
    pair.a.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: true,
    });
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.a.threads[&chat_id].unread_count, 1);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_eq!(pair.a.chat_read_states[&chat_id], read_state);
    assert_chat_read_delivery(&pair.a, &chat_id, "manual-read", true);
}

#[test]
fn chat_read_sync_preserves_newer_and_unlisted_same_second_messages() {
    let mut pair = chat_read_sync_pair("read-sync-boundary");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "read-boundary", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    chat_read_sync_incoming(&mut pair.b, &peer, "unlisted-boundary", 200);
    chat_read_sync_incoming(&mut pair.b, &peer, "newer-unseen", 201);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 3);

    pair.a
        .mark_messages_seen(&chat_id, &["read-boundary".to_string()]);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);

    assert_eq!(pair.b.threads[&chat_id].unread_count, 2);
    assert_chat_read_delivery(&pair.b, &chat_id, "read-boundary", true);
    assert_chat_read_delivery(&pair.b, &chat_id, "unlisted-boundary", false);
    assert_chat_read_delivery(&pair.b, &chat_id, "newer-unseen", false);
}

#[test]
fn chat_read_sync_does_not_recount_old_history_or_clear_newer_unread() {
    let mut pair = chat_read_sync_pair("read-sync-legacy-count");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "legacy-first", 197);
    chat_read_sync_incoming(&mut pair.a, &peer, "legacy-boundary", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    chat_read_sync_incoming(&mut pair.b, &peer, "legacy-middle-a", 198);
    chat_read_sync_incoming(&mut pair.b, &peer, "legacy-middle-b", 199);
    // Older builds could clear a badge while stored deliveries remained
    // Received. Those messages must not be counted as newly unread again.
    pair.b.threads.get_mut(&chat_id).unwrap().unread_count = 0;
    chat_read_sync_incoming(&mut pair.b, &peer, "same-second-unread", 200);
    chat_read_sync_incoming(&mut pair.b, &peer, "newer-unread", 201);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 2);

    pair.a
        .mark_messages_seen(&chat_id, &["legacy-first".to_string()]);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 2);
    assert_chat_read_delivery(&pair.b, &chat_id, "legacy-middle-a", false);

    pair.a
        .mark_messages_seen(&chat_id, &["legacy-boundary".to_string()]);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 2);
    assert_chat_read_delivery(&pair.b, &chat_id, "legacy-boundary", true);
    assert_chat_read_delivery(&pair.b, &chat_id, "same-second-unread", false);
    assert_chat_read_delivery(&pair.b, &chat_id, "newer-unread", false);
}

#[test]
fn chat_read_sync_stale_and_repeated_snapshots_preserve_later_local_unread() {
    let mut pair = chat_read_sync_pair("read-sync-stale");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "first-read", 200);
    pair.a
        .mark_messages_seen(&chat_id, &["first-read".to_string()]);
    let older = pair.a.build_device_sync_packets_for_test(100, true);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &older);
    chat_read_sync_incoming(&mut pair.a, &peer, "second-read", 201);
    pair.a
        .mark_messages_seen(&chat_id, &["second-read".to_string()]);
    let latest = pair.a.build_device_sync_packets_for_test(100, true);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &latest);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);

    pair.b.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: true,
    });
    for packets in [&older, &latest, &latest] {
        deliver_chat_read_packets(&mut pair.b, &pair.a_device, packets);
    }
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    assert_chat_read_delivery(&pair.b, &chat_id, "second-read", true);

    chat_read_sync_incoming(&mut pair.b, &peer, "later-unseen", 202);
    let unread_before_replay = pair.b.threads[&chat_id].unread_count;
    for packets in [&older, &latest] {
        deliver_chat_read_packets(&mut pair.b, &pair.a_device, packets);
    }
    assert_eq!(pair.b.threads[&chat_id].unread_count, unread_before_replay);
    assert_chat_read_delivery(&pair.b, &chat_id, "later-unseen", false);
}

#[test]
fn chat_read_sync_before_message_prevents_delayed_history_becoming_unread() {
    let mut pair = chat_read_sync_pair("read-sync-before-message");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "boundary-read", 200);
    pair.a
        .mark_messages_seen(&chat_id, &["boundary-read".to_string()]);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    assert!(pair.b.threads[&chat_id].messages.is_empty());
    assert!(pair.b.chat_read_states.contains_key(&chat_id));

    chat_read_sync_incoming(&mut pair.b, &peer, "delayed-older", 199);
    chat_read_sync_incoming(&mut pair.b, &peer, "boundary-read", 200);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, "delayed-older", true);
    assert_chat_read_delivery(&pair.b, &chat_id, "boundary-read", true);

    chat_read_sync_incoming(&mut pair.b, &peer, "unlisted-same-second", 200);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    assert_chat_read_delivery(&pair.b, &chat_id, "unlisted-same-second", false);
}

#[test]
fn chat_read_sync_survives_restart_and_catches_up_an_offline_sibling() {
    let mut pair = chat_read_sync_pair("read-sync-restart");
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer, "read-while-offline", 200);
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, true);
    pair.a
        .mark_messages_seen(&chat_id, &["read-while-offline".to_string()]);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 1);
    let expected_state = pair.a.chat_read_states[&chat_id].clone();
    drop(pair.a);

    let mut restarted = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.a_device,
        pair.a_dir.path().to_string_lossy().into_owned(),
    );
    let persisted = restarted.load_persisted().unwrap().unwrap();
    assert_eq!(restarted.chat_read_states[&chat_id], expected_state);
    let persisted_thread = persisted
        .threads
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .unwrap();
    assert_eq!(persisted_thread.unread_count, 0);
    assert!(persisted_thread.messages.iter().any(|message| {
        message.id == "read-while-offline"
            && matches!(message.delivery, PersistedDeliveryState::Seen)
    }));
    configure_test_device_sync_profile(
        &mut restarted,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
        None,
    );
    // The fixture does not restore the full session; supply the known chat's
    // metadata while history still comes from the persisted database.
    restarted.ensure_thread_record(&chat_id, 200);
    sync_chat_reads(&restarted, &mut pair.b, &pair.a_device, true);
    assert_eq!(pair.b.threads[&chat_id].unread_count, 0);
    assert_chat_read_delivery(&pair.b, &chat_id, "read-while-offline", true);

    let stored = pair
        .b
        .app_store
        .load_thread(&chat_id, 100)
        .unwrap()
        .unwrap();
    assert_eq!(stored.unread_count, 0);
    assert!(stored
        .messages
        .iter()
        .all(|message| matches!(message.delivery, PersistedDeliveryState::Seen)));
}

#[test]
fn chat_read_sync_merges_device_alias_progress_without_reviving_old_chat() {
    let mut pair = chat_read_sync_pair("read-sync-alias");
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let owner_id = peer.public_key().to_hex();
    let alias_id = peer_device.public_key().to_hex();
    chat_read_sync_incoming(&mut pair.a, &peer_device, "alias-read", 200);
    pair.a
        .mark_messages_seen(&alias_id, &["alias-read".to_string()]);
    chat_read_sync_incoming(&mut pair.a, &peer, "owner-read", 201);
    pair.a
        .mark_messages_seen(&owner_id, &["owner-read".to_string()]);
    let expected = pair.a.chat_read_states[&owner_id].clone();
    let devices = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    pair.a
        .migrate_verified_device_owner_threads(peer.public_key(), &devices);
    assert!(!pair.a.threads.contains_key(&alias_id));
    assert!(!pair.a.chat_read_states.contains_key(&alias_id));
    assert!(!pair
        .a
        .app_store
        .load_chat_read_states()
        .unwrap()
        .contains_key(&alias_id));
    assert_eq!(pair.a.chat_read_states[&owner_id], expected);
    let packets = pair.a.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets);
    assert!(!pair.b.threads.contains_key(&alias_id));
    assert_eq!(pair.b.chat_read_states[&owner_id], expected);
}
