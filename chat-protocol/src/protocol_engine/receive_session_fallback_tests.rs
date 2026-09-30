#[test]
fn direct_receive_after_restart_tries_new_session_after_stale_same_invite_session() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    let mut receiver = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    receiver
        .ingest_app_keys_event(&signed_app_keys(
            &peer_owner,
            &[peer_device.public_key()],
            10,
        ))
        .unwrap();
    let peer = test_engine(&peer_owner, &peer_device);
    let mut invite = peer.local_invite().unwrap().clone();
    let mut states = Vec::new();
    let mut replies = Vec::new();
    for (index, message_count) in [2, 1].into_iter().enumerate() {
        let (mut session, response) = invite
            .accept_with_owner(
                device.public_key(),
                device.secret_key().to_secret_bytes(),
                Some(device.public_key().to_hex()),
                Some(owner.public_key()),
            )
            .unwrap();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(20), &mut rng);
        let mut peer_session = invite
            .process_response(
                &mut ctx,
                &response,
                peer_device.secret_key().to_secret_bytes(),
            )
            .unwrap()
            .session;
        for _ in 0..message_count {
            let plan = session.plan_send(b"bootstrap", NdrUnixSeconds(20)).unwrap();
            let envelope = session.apply_send(plan).envelope;
            let receive_plan = peer_session.plan_receive(&mut ctx, &envelope).unwrap();
            peer_session.apply_receive(receive_plan);
        }
        let text = format!("reply on session {index}");
        let plan = peer_session
            .plan_send(text.as_bytes(), NdrUnixSeconds(21))
            .unwrap();
        let reply = peer_session.apply_send(plan).envelope;
        replies.push(nostr_double_ratchet::message_event(&reply).unwrap());
        states.push(session.state.clone());
        receiver
            .import_session_state(
                peer_owner.public_key(),
                Some(peer_device.public_key().to_hex()),
                session.state,
                UnixSeconds(20),
            )
            .unwrap();
    }
    assert_eq!(replies[0].pubkey, replies[1].pubkey);

    // Exercise the shipped native dependency and persisted protocol engine,
    // including the signed owner/device proof required to accept the reply.
    let mut receiver =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
            .unwrap();
    let snapshot = receiver.session_manager_snapshot();
    let record = snapshot
        .users
        .iter()
        .find(|user| user.owner_pubkey == ndr_owner(peer_owner.public_key()))
        .unwrap()
        .devices
        .iter()
        .find(|record| record.device_pubkey == ndr_device(peer_device.public_key()))
        .unwrap();
    assert_eq!(record.active_session.as_ref(), Some(&states[0]));
    assert_eq!(record.inactive_sessions, vec![states[1].clone()]);

    for index in [1, 0] {
        let received = receiver
            .process_direct_message_event(&replies[index])
            .expect("a stale session must not hide a valid later candidate")
            .expect("authenticated reply must reach the native chat protocol");
        assert_eq!(received.sender, peer_owner.public_key());
        assert_eq!(received.sender_device, Some(peer_device.public_key()));
        assert_eq!(received.content, format!("reply on session {index}"));
    }
}
