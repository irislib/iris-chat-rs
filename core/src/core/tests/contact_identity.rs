#[test]
fn contact_identity_keeps_first_name_until_exact_approval_and_persists_private_history() {
    let mut pair = chat_read_receipt_pair("contact-identity");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    let metadata = |name: &str, time| {
        EventBuilder::new(Kind::Metadata, serde_json::json!({"name":name}).to_string())
            .custom_created_at(Timestamp::from_secs(time))
            .sign_with_keys(&peer)
            .unwrap()
    };
    pair.a.apply_profile_metadata_event(&metadata("Alice", 10));
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.apply_profile_metadata_event(&metadata("Bob", 11));
    pair.a.rebuild_state();
    assert_eq!(
        pair.a.state.current_chat.as_ref().unwrap().display_name,
        "Alice"
    );
    assert_eq!(
        pair.a
            .state
            .current_chat
            .as_ref()
            .unwrap()
            .contact_identity
            .as_ref()
            .unwrap()
            .pending_name
            .as_deref(),
        Some("Bob")
    );
    pair.a.apply_profile_metadata_event(&metadata("Carol", 12));
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Bob".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Alice");
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Carol".into(),
    });
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Carol".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Carol");
    assert_eq!(
        pair.a.threads[&id]
            .messages
            .iter()
            .filter(|m| m.body == "Name change approved: Alice → Carol")
            .count(),
        1
    );
    let pending = pair.a.pending_relay_publishes.len();
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    // Favorites queue authenticated sibling ratchet events, never public follow metadata.
    assert!(pair.a.pending_relay_publishes.len() > pending);
    assert!(pending_events_with_kind(&pair.a, 30078).is_empty());
    assert!(
        pair.a.private_contacts.state.as_ref().unwrap().contacts[&id]["favorite"]
            .value
            .as_bool()
            .unwrap()
    );
    let stored = pair.a.load_persisted().unwrap().unwrap();
    let memory = &stored.owner_profiles[&id].contact_memory;
    assert_eq!(memory.first_seen_name.as_deref(), Some("Alice"));
    assert_eq!(memory.accepted_name.as_deref(), Some("Carol"));
    assert_eq!(memory.name_changes.len(), 1);
    assert!(memory.favorite);
    let public = build_profile_metadata_json(&stored.owner_profiles[&id]);
    assert!(!public.contains("Alice"));
    assert!(!public.contains("favorite"));
    save_contact_details(&mut pair.a, &peer, "Work friend", "");
    pair.a.apply_profile_metadata_event(&metadata("Dana", 13));
    pair.a.handle_action(AppAction::ApproveContactName {
        owner_pubkey_hex: id.clone(),
        name: "Dana".into(),
    });
    assert_eq!(pair.a.owner_display_label(&id), "Work friend");
    assert_eq!(
        pair.a.owner_profiles[&id]
            .contact_memory
            .first_seen_name
            .as_deref(),
        Some("Alice")
    );
}

#[test]
fn contact_identity_read_only_snapshot_keeps_unread_and_pending_name_off_route() {
    let mut pair = chat_read_sync_pair("contact-read-only");
    let peer = Keys::generate();
    let id = peer.public_key().to_hex();
    let profile = |name: &str, time| {
        EventBuilder::new(
            Kind::Metadata,
            serde_json::json!({"name": name}).to_string(),
        )
        .custom_created_at(Timestamp::from_secs(time))
        .sign_with_keys(&peer)
        .unwrap()
    };
    pair.a.apply_profile_metadata_event(&profile("Alice", 10));
    pair.a.handle_action(AppAction::CreateChat {
        peer_input: id.clone(),
    });
    pair.a.apply_profile_metadata_event(&profile("Alicia", 11));
    pair.a.handle_action(AppAction::SetContactFavorite {
        owner_pubkey_hex: id.clone(),
        favorite: true,
    });
    pair.a.threads.get_mut(&id).unwrap().unread_count = 3;
    pair.a.rebuild_persist_and_emit_state();
    let mut state = pair.a.state.clone();
    state.current_chat = None;
    let bounded = chat_read_state(&state, &id, Some(1));
    let shared = pair.a.app_store.shared();
    let read = chat_snapshot_from_state_and_db(&bounded, Some(&shared), &id, 1).unwrap();
    let memory = read.contact_identity.unwrap();
    assert_eq!(memory.saved_name.as_deref(), Some("Alice"));
    assert_eq!(memory.pending_name.as_deref(), Some("Alicia"));
    assert!(memory.is_favorite);
    assert_eq!(pair.a.threads[&id].unread_count, 3);
    assert!(bounded.current_chat.is_none());
}
