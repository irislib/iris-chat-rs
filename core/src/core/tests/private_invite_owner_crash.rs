#[test]
fn private_invite_delete_failure_blocks_sibling_imports_in_same_retry_pass() {
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let first_peer_device = Keys::generate();
    let second_peer_device = Keys::generate();
    let mut inviter = logged_in_test_core(
        "private-response-delete-blocks-siblings",
        &inviter_owner,
        &inviter_device,
    );
    let invite_url = create_private_invite_for_test(&mut inviter);
    let invite_key = inviter
        .private_chat_invites
        .keys()
        .next()
        .cloned()
        .expect("private invite key");
    for peer_device in [&first_peer_device, &second_peer_device] {
        inviter.handle_relay_event(crafted_private_invite_response(
            &invite_url,
            peer_device,
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        ));
    }
    assert_eq!(inviter.pending_private_invite_responses.len(), 2);

    {
        let shared = inviter.app_store.shared();
        let conn = shared.lock().expect("storage connection");
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER fail_private_invite_delete_siblings
             BEFORE DELETE ON ndr_kv
             WHEN OLD.key = '{invite_key}'
             BEGIN SELECT RAISE(ABORT, 'injected invite delete failure'); END;"
        ))
        .expect("install delete failure trigger");
    }

    let roster = AppKeys::new(vec![
        DeviceEntry::new(first_peer_device.public_key(), 10),
        DeviceEntry::new(second_peer_device.public_key(), 10),
    ])
    .get_event_at(peer_owner.public_key(), 10)
    .sign_with_keys(&peer_owner)
    .expect("signed sibling roster");
    ingest_invite_owner_app_keys(
        &mut inviter,
        peer_owner.public_key(),
        vec![roster.clone()],
    );

    assert_eq!(
        stored_session_count(&inviter, peer_owner.public_key()),
        1,
        "one failed cleanup must stop sibling responses from reusing the invite"
    );
    assert_eq!(inviter.pending_private_invite_responses.len(), 2);
    assert!(inviter.private_chat_invites.contains_key(&invite_key));

    {
        let shared = inviter.app_store.shared();
        let conn = shared.lock().expect("storage connection");
        conn.execute_batch("DROP TRIGGER fail_private_invite_delete_siblings;")
            .expect("remove delete failure trigger");
    }
    ingest_invite_owner_app_keys(
        &mut inviter,
        peer_owner.public_key(),
        vec![roster],
    );

    assert_eq!(stored_session_count(&inviter, peer_owner.public_key()), 1);
    assert!(inviter.pending_private_invite_responses.is_empty());
    assert!(inviter.private_chat_invites.is_empty());
}

#[test]
fn local_relay_normal_app_keys_subscription_releases_staged_response_end_to_end() {
    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    let inviter_owner = Keys::generate();
    let inviter_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let (core_tx, core_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut inviter = AppCore::new(
        flume::unbounded().0,
        core_tx,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    inviter.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    inviter
        .start_primary_session(inviter_owner.clone(), inviter_device, false, false)
        .expect("start inviter");
    while let Ok(message) = core_rx.try_recv() {
        inviter.handle_message(message);
    }
    let inviter_client = inviter.logged_in.as_ref().unwrap().client.clone();
    inviter.runtime.block_on(async {
        ensure_session_relays_configured(&inviter_client, &relay_urls).await;
        connect_client_with_timeout(&inviter_client, Duration::from_secs(2)).await;
    });

    let roster = signed_app_keys_authorization_event(
        &peer_owner,
        peer_device.public_key(),
        unix_now().get(),
    );
    let publisher = Client::new(peer_owner.clone());
    inviter.runtime.block_on(async {
        ensure_session_relays_configured(&publisher, &relay_urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher
            .send_event(&roster)
            .await
            .expect("publish owner roster");
    });

    let invite_url = create_private_invite_for_test(&mut inviter);
    let response = crafted_private_invite_response(
        &invite_url,
        &peer_device,
        Some(peer_device.public_key().to_hex()),
        Some(peer_owner.public_key()),
    );
    inviter.handle_relay_event(response);
    assert_eq!(inviter.pending_private_invite_responses.len(), 1);

    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        while let Ok(message) = core_rx.try_recv() {
            inviter.handle_message(message);
        }
        if inviter.pending_private_invite_responses.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(inviter.pending_private_invite_responses.is_empty());
    assert!(inviter.private_chat_invites.is_empty());
    assert!(inviter
        .threads
        .contains_key(&peer_owner.public_key().to_hex()));
    assert_eq!(
        active_session_device_pubkeys(&inviter, peer_owner.public_key()),
        vec![peer_device.public_key()]
    );
}
