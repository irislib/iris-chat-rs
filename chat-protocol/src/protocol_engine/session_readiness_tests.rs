fn established_readiness_fixture() -> RemoteSendFixture {
    let mut fixture = remote_send_fixture();
    let peer = fixture.peer_owner.public_key();
    assert!(
        !fixture.sender.has_usable_direct_session_for_owner(peer),
        "a public invite alone is not an existing session"
    );
    let sent = fixture
        .sender
        .send_direct_unsigned_event_to_peer_only(
            peer,
            &peer.to_hex(),
            own_seen_rumor(&fixture.owner),
            UnixSeconds(10),
        )
        .unwrap();
    assert_eq!(
        decrypt_own_sync_effects(&mut fixture.ready, sent.effects).len(),
        1
    );
    assert!(fixture.sender.has_usable_direct_session_for_owner(peer));
    fixture
}

#[test]
fn usable_direct_session_reuses_projection_and_restores_it_after_rollback() {
    let mut fixture = established_readiness_fixture();
    let peer = fixture.peer_owner.public_key();
    let builds = fixture
        .sender
        .known_message_author_cache_build_count_for_test();
    for _ in 0..100 {
        assert!(fixture.sender.has_usable_direct_session_for_owner(peer));
        fixture.sender.known_message_author_pubkeys();
    }
    assert_eq!(
        fixture
            .sender
            .known_message_author_cache_build_count_for_test(),
        builds
    );

    let failed: anyhow::Result<()> = fixture.sender.with_state_checkpoint(|engine| {
        engine.verified_app_keys_owners.remove(&ndr_owner(peer));
        engine.invalidate_known_message_author_cache();
        assert!(!engine.has_usable_direct_session_for_owner(peer));
        anyhow::bail!("roll back simulated incomplete roster update")
    });
    assert!(failed.is_err());
    assert!(fixture.sender.has_usable_direct_session_for_owner(peer));
    assert_eq!(
        fixture
            .sender
            .known_message_author_cache_build_count_for_test(),
        builds + 2
    );
}

#[test]
fn usable_direct_session_includes_retained_inactive_sessions_after_restart() {
    let mut fixture = established_readiness_fixture();
    let peer = fixture.peer_owner.public_key();
    let mut snapshot = fixture.sender.session_manager.snapshot();
    let device = snapshot
        .users
        .iter_mut()
        .find(|user| user.owner_pubkey == ndr_owner(peer))
        .unwrap()
        .devices
        .iter_mut()
        .find(|device| device.device_pubkey == ndr_device(fixture.ready_device.public_key()))
        .unwrap();
    let sendable = device.active_session.take().unwrap();
    let mut receive_only = sendable.clone();
    receive_only.our_current_nostr_key = None;
    device.active_session = Some(receive_only);
    device.inactive_sessions.push(sendable);
    // Represent a retained older send-capable session beside a newer receive-only one.
    fixture.sender.session_manager =
        SessionManager::from_snapshot(snapshot, fixture.device.secret_key().to_secret_bytes())
            .unwrap();
    fixture.sender.invalidate_known_message_author_cache();
    fixture.sender.persist_now().unwrap();
    fixture.sender = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert!(fixture.sender.has_usable_direct_session_for_owner(peer));
}

#[test]
fn usable_direct_session_revocation_invalidates_cached_and_persisted_readiness() {
    let mut fixture = established_readiness_fixture();
    let peer = fixture.peer_owner.public_key();
    fixture
        .sender
        .ingest_app_keys_event(&signed_app_keys(&fixture.peer_owner, &[], 30))
        .unwrap();
    assert!(!fixture.sender.has_usable_direct_session_for_owner(peer));
    fixture.sender = ProtocolEngine::load_or_create_for_local_device(
        fixture.store,
        fixture.owner.public_key(),
        &fixture.device,
    )
    .unwrap();
    assert!(!fixture.sender.has_usable_direct_session_for_owner(peer));
}

#[test]
fn sender_read_queries_preserve_snapshot_owner_resolution_after_revocation() {
    let mut fixture = established_readiness_fixture();
    let snapshot = fixture.sender.session_manager_snapshot();
    let mut senders = std::collections::BTreeSet::new();
    for user in &snapshot.users {
        for device in &user.devices {
            for session in device.active_session.iter().chain(&device.inactive_sessions) {
                senders.extend(session.their_current_nostr_public_key);
                senders.extend(session.their_next_nostr_public_key);
                senders.extend(session.skipped_keys.keys().copied());
            }
        }
    }
    assert!(!senders.is_empty());
    senders.insert(ndr_device(Keys::generate().public_key()));
    for revoked in [false, true] {
        if revoked {
            fixture.sender.ingest_app_keys_event(&signed_app_keys(&fixture.peer_owner, &[], 30)).unwrap();
        }
        let snapshot = fixture.sender.session_manager_snapshot();
        for sender in &senders {
            assert_eq!(
                fixture.sender.resolve_message_sender_owner_for_sender(*sender),
                fixture.sender.resolve_message_sender_owner_with_snapshot(*sender, &snapshot),
            );
        }
    }
}
