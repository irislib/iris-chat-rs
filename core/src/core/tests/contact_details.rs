fn save_contact_details(core: &mut AppCore, peer: &Keys, nickname: &str, note: &str) {
    core.handle_action(AppAction::SetContactDetails {
        owner_pubkey_hex: peer.public_key().to_hex(),
        nickname: nickname.to_string(),
        note: note.to_string(),
    });
}

#[test]
fn contact_details_persist_privately_and_survive_public_profile_changes() {
    let mut pair = chat_read_sync_pair("contact-details");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    let pending = pair.a.pending_relay_publishes.len();
    save_contact_details(
        &mut pair.a,
        &peer,
        "  Work   Alice  ",
        "  Met at lunch.\nLikes tea.  ",
    );
    let current = pair.a.state.current_chat.as_ref().unwrap();
    assert_eq!(current.nickname.as_deref(), Some("Work Alice"));
    assert_eq!(
        current.contact_note.as_deref(),
        Some("Met at lunch.\nLikes tea.")
    );
    assert_eq!(pair.a.pending_relay_publishes.len(), pending);
    let profile = &pair.a.owner_profiles[&id];
    let public = build_profile_metadata_json(profile);
    assert!(!public.contains("Work Alice"));
    assert!(!public.contains("Likes tea"));
    let event = EventBuilder::new(Kind::Metadata, r#"{"name":"Alice Public","about":"Hello"}"#)
        .sign_with_keys(&peer)
        .unwrap();
    assert!(pair.a.apply_profile_metadata_event(&event));
    pair.a.persist_best_effort();
    let stored = pair.a.load_persisted().unwrap().unwrap();
    assert_eq!(
        stored.owner_profiles[&id].nickname.as_deref(),
        Some("Work Alice")
    );
    assert_eq!(
        stored.owner_profiles[&id].contact_note.as_deref(),
        Some("Met at lunch.\nLikes tea.")
    );
    assert_eq!(
        stored.owner_profiles[&id].name.as_deref(),
        Some("Alice Public")
    );
    // The legacy nickname action must preserve a saved note.
    pair.a.handle_action(AppAction::SetContactNickname {
        owner_pubkey_hex: id.clone(),
        nickname: String::new(),
    });
    assert_eq!(
        pair.a.owner_profiles[&id].contact_note,
        stored.owner_profiles[&id].contact_note
    );
    save_contact_details(&mut pair.a, &peer, "", " \n ");
    assert!(pair.a.owner_profiles[&id].contact_note.is_none());
    assert_eq!(
        pair.a.state.current_chat.as_ref().unwrap().display_name,
        "Alice Public"
    );
}

#[test]
fn contact_details_sync_only_to_authorized_devices_and_removal_wins_over_stale_data() {
    let mut pair = chat_read_sync_pair("contact-details-sync");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    save_contact_details(&mut pair.a, &peer, "Alice", "Private note");
    let stale = pair.a.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.b, &peer, &stale);
    assert!(!pair.b.owner_profiles.contains_key(&id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    assert_eq!(
        pair.b.owner_profiles[&id].contact_note.as_deref(),
        Some("Private note")
    );
    save_contact_details(&mut pair.b, &peer, "", "");
    sync_chat_reads(&pair.b, &mut pair.a, &pair.b_device, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    for core in [&mut pair.a, &mut pair.b] {
        assert!(core.owner_profiles[&id].nickname.is_none());
        assert!(core.owner_profiles[&id].contact_note.is_none());
        let saved = core.load_persisted().unwrap().unwrap();
        assert!(saved.owner_profiles[&id].contact_note.is_none());
        assert!(saved.owner_profiles[&id].contact_updated_at_ms > 0);
    }
}

#[test]
fn contact_details_validation_is_atomic_and_accepts_note_without_nickname() {
    let mut pair = chat_read_sync_pair("contact-details-validation");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    save_contact_details(&mut pair.a, &peer, "", "Note only");
    let expected = pair.a.owner_profiles[&id].clone();
    save_contact_details(&mut pair.a, &peer, "Changed", &"x".repeat(241));
    assert_eq!(pair.a.owner_profiles[&id], expected);
    save_contact_details(&mut pair.a, &peer, &"x".repeat(81), "Changed");
    assert_eq!(pair.a.owner_profiles[&id], expected);
    save_contact_details(&mut pair.a, &peer, "", &"🌻".repeat(240));
    assert_eq!(
        pair.a.owner_profiles[&id]
            .contact_note
            .as_ref()
            .unwrap()
            .chars()
            .count(),
        240
    );
    let stranger = Keys::generate();
    save_contact_details(&mut pair.a, &stranger, "Unknown", "No chat");
    assert!(!pair
        .a
        .owner_profiles
        .contains_key(&stranger.public_key().to_hex()));
}

#[test]
fn contact_details_converge_after_offline_edits_and_ignore_older_clients() {
    let mut pair = chat_read_sync_pair("contact-details-conflicts");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    for core in [&mut pair.a, &mut pair.b] {
        core.handle_action(AppAction::CreateChat {
            peer_input: id.clone(),
        });
    }
    save_contact_details(&mut pair.a, &peer, "Alice", "First edit");
    save_contact_details(&mut pair.b, &peer, "Alice", "Second edit");
    let timestamp = pair.a.owner_profiles[&id].contact_updated_at_ms;
    pair.b
        .owner_profiles
        .get_mut(&id)
        .unwrap()
        .contact_updated_at_ms = timestamp;
    let from_a = pair.a.build_device_sync_packets_for_test(100, false);
    let from_b = pair.b.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.a, &pair.b_device, &from_b);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &from_a);
    assert_eq!(pair.a.owner_profiles[&id], pair.b.owner_profiles[&id]);
    assert_eq!(
        pair.a.owner_profiles[&id].contact_note.as_deref(),
        Some("Second edit")
    );
    let without_details = from_a
        .iter()
        .map(|packet| {
            let mut json: serde_json::Value = serde_json::from_slice(packet).unwrap();
            if let Some(chats) = json["chats"].as_array_mut() {
                for chat in chats {
                    chat.as_object_mut().unwrap().remove("contactDetails");
                }
            }
            serde_json::to_vec(&json).unwrap()
        })
        .collect::<Vec<_>>();
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &without_details);
    assert_eq!(
        pair.b.owner_profiles[&id].contact_note.as_deref(),
        Some("Second edit")
    );
}
