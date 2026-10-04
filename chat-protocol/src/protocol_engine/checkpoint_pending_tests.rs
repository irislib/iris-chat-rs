#[test]
fn checkpoint_direct_send_reuses_unchanged_pending_ciphertext_serialization() {
    let mut fixture = remote_send_fixture();
    fixture.sender.pending_group_sender_key_messages =
        queued_group_history(2).0.pending_group_sender_key_messages;
    fixture.sender.persist().unwrap();
    let serialized = CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get);
    let before = fixture.sender.pending_group_sender_key_messages.clone();
    let peer = fixture.peer_owner.public_key();
    let sent = fixture
        .sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "A direct send must not re-encode unrelated ciphertext",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    assert!(!sent.event_ids.is_empty());
    assert_eq!(fixture.sender.pending_group_sender_key_messages, before);
    assert_eq!(
        CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get),
        serialized,
        "unchanged pending ciphertext must reuse its serialized checkpoint segment"
    );
    let restored = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert_eq!(restored.pending_group_sender_key_messages, before);
    assert_eq!(
        decrypt_own_sync_effects(&mut fixture.ready, sent.effects).len(),
        1
    );
}

#[test]
fn checkpoint_pending_mutations_detach_clones_and_noop_retain_reuses_cache() {
    let (engine, _, _) = queued_group_history(3);
    let mut pending = engine.pending_group_sender_key_messages;
    let original = pending.clone();
    let cached = pending.serialized().unwrap();
    assert!(Arc::ptr_eq(&pending.values, &original.values));
    let mut visited = 0;
    pending.retain(|_| {
        visited += 1;
        true
    });
    assert_eq!(visited, 3);
    assert!(Arc::ptr_eq(&cached, &pending.serialized().unwrap()));
    assert!(Arc::ptr_eq(&pending.values, &original.values));

    let removed = pending.remove(1);
    assert_ne!(pending.serialized().unwrap().json.get(), cached.json.get());
    assert!(!Arc::ptr_eq(&pending.values, &original.values));
    assert_eq!(original.serialized().unwrap().json.get(), cached.json.get());
    pending.insert(1, removed);
    assert_eq!(pending.serialized().unwrap().json.get(), cached.json.get());
    pending.push(original[0].clone());
    assert_eq!(
        pending.serialized().unwrap().json.get(),
        serde_json::to_string(&*pending).unwrap()
    );
    pending.pop();
    pending[0].encrypted_header = Some("a different cached generation".into());
    assert_eq!(
        pending.serialized().unwrap().json.get(),
        serde_json::to_string(&*pending).unwrap()
    );
    visited = 0;
    pending.retain(|_| {
        visited += 1;
        visited != 2
    });
    assert_eq!(
        visited, 3,
        "retain must evaluate each predicate exactly once"
    );
    assert_eq!(pending.len(), 2);
    assert_eq!(
        pending.serialized().unwrap().json.get(),
        serde_json::to_string(&*pending).unwrap()
    );
    assert_eq!(original.serialized().unwrap().json.get(), cached.json.get());
}

#[test]
fn checkpoint_pending_failed_save_and_rollback_keep_matching_ciphertexts() {
    let (mut engine, _, _) = queued_group_history(3);
    engine.persist().unwrap();
    let before = engine.pending_group_sender_key_messages.clone();
    let cached = before.serialized().unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: true.into(),
    });
    engine.storage = storage.clone();
    let persisted_before = storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap();
    let failure: anyhow::Result<()> = engine.with_state_checkpoint(|engine| {
        engine.pending_group_sender_key_messages.remove(0);
        engine.persist()
    });
    assert!(failure
        .unwrap_err()
        .downcast_ref::<StorageError>()
        .is_some());
    assert_eq!(
        storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(),
        persisted_before
    );
    assert_eq!(engine.pending_group_sender_key_messages, before);
    assert!(Arc::ptr_eq(
        &cached,
        &engine
            .pending_group_sender_key_messages
            .serialized()
            .unwrap()
    ));

    // A failed standalone put does not mark different queue bytes as saved.
    // Explicit retry must write the changed logical state, with its own cache.
    engine.pending_group_sender_key_messages.remove(1);
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(engine.persist().is_err());
    assert_eq!(
        storage.get(PROTOCOL_ENGINE_STATE_KEY).unwrap(),
        persisted_before
    );
    let changed = engine
        .pending_group_sender_key_messages
        .serialized()
        .unwrap();
    engine.persist().unwrap();
    assert!(Arc::ptr_eq(
        &changed,
        &engine
            .pending_group_sender_key_messages
            .serialized()
            .unwrap()
    ));
    let keys = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    let restored =
        ProtocolEngine::load_or_create_for_local_device(storage, engine.owner_pubkey, &keys)
            .unwrap();
    assert_eq!(
        restored.pending_group_sender_key_messages,
        engine.pending_group_sender_key_messages
    );
    assert_eq!(before.len(), 3);
    assert_eq!(restored.pending_group_sender_key_messages.len(), 2);
}

#[test]
fn checkpoint_pending_restores_compact_pretty_and_padded_legacy_json() {
    let (mut engine, _, _) = queued_group_history(3);
    for index in 0..3 {
        engine.pending_group_sender_key_messages[index].encrypted_header =
            Some("quoted field: \"pending_group_sender_key_messages\": [{}] \\\n🦊".repeat(500));
    }
    engine.persist().unwrap();
    let persisted = engine
        .storage
        .get(PROTOCOL_ENGINE_STATE_KEY)
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    let keys = Keys::new(nostr::SecretKey::from_slice(&engine.local_device_secret).unwrap());
    for json in [
        serde_json::to_string(&value).unwrap(),
        serde_json::to_string_pretty(&value).unwrap(),
        persisted,
    ] {
        engine.storage.put(PROTOCOL_ENGINE_STATE_KEY, json).unwrap();
        let mut restored = ProtocolEngine::load_or_create_for_local_device(
            engine.storage.clone(),
            engine.owner_pubkey,
            &keys,
        )
        .unwrap();
        assert_eq!(
            restored.pending_group_sender_key_messages,
            engine.pending_group_sender_key_messages
        );
        let cached = restored
            .pending_group_sender_key_messages
            .serialized()
            .unwrap();
        assert_eq!(
            cached.json.get(),
            serde_json::to_string(&*restored.pending_group_sender_key_messages).unwrap(),
            "restore must compact inherited whitespace before reusing array layout"
        );
        let count = CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get);
        restored.persist().unwrap();
        assert_eq!(
            CHECKPOINT_PENDING_SERIALIZED_ITEMS.with(std::cell::Cell::get),
            count
        );
    }
}
