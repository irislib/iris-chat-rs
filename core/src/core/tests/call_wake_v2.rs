fn prepare_first_call_for_test(
    a: &mut AppCore,
    b: &mut AppCore,
    ao: &Keys,
    bo: &Keys,
    ad: &Keys,
    bd: &Keys,
) {
    for core in [&mut *a, &mut *b] {
        for (owner, device) in [(ao, ad), (bo, bd)] {
            call_test_peer(core, owner, device);
            observe_peer_appkeys_for_test(
                core.protocol_engine.as_mut().unwrap(),
                owner,
                &[device.public_key()],
                unix_now().get(),
            );
        }
    }
    let invite = nostr_double_ratchet::invite_unsigned_event(
        &b.protocol_engine.as_ref().unwrap().local_invite().unwrap(),
    )
    .unwrap()
    .sign_with_keys(bd)
    .unwrap();
    a.protocol_engine
        .as_mut()
        .unwrap()
        .observe_invite_event(&invite)
        .unwrap();
    a.persist_best_effort();
    b.persist_best_effort();
}

fn install_call_ratchet_for_test(
    a: &mut AppCore,
    b: &mut AppCore,
    ao: &Keys,
    bo: &Keys,
    ad: &Keys,
    bd: &Keys,
) {
    let now = unix_now().get();
    for core in [&mut *a, &mut *b] {
        for (owner, device) in [(ao, ad), (bo, bd)] {
            call_test_peer(core, owner, device);
            observe_peer_appkeys_for_test(
                core.protocol_engine.as_mut().unwrap(),
                owner,
                &[device.public_key()],
                now,
            );
        }
    }
    let invite = b.protocol_engine.as_ref().unwrap().local_invite().unwrap();
    let (session, response) = invite
        .accept_with_owner(
            ad.public_key(),
            ad.secret_key().to_secret_bytes(),
            Some(ad.public_key().to_hex()),
            Some(ao.public_key()),
        )
        .unwrap();
    a.protocol_engine
        .as_mut()
        .unwrap()
        .import_session_state(
            bo.public_key(),
            Some(bd.public_key().to_hex()),
            session.state,
            unix_now(),
        )
        .unwrap();
    let response = nostr_double_ratchet::process_invite_response_event(
        &invite,
        &nostr_double_ratchet::invite_response_event(&response).unwrap(),
        bd.secret_key().to_secret_bytes(),
    )
    .unwrap()
    .unwrap();
    b.protocol_engine
        .as_mut()
        .unwrap()
        .import_session_state(
            ao.public_key(),
            Some(ad.public_key().to_hex()),
            response.session.state,
            unix_now(),
        )
        .unwrap();
    a.persist_best_effort();
    b.persist_best_effort();
}

fn ratcheted_call_wake_for_test(
    a: &mut AppCore,
    recipient: &Keys,
    sender: &Keys,
    id: &str,
) -> Event {
    let owner = a.logged_in.as_ref().unwrap().owner_pubkey;
    let signal = serde_json::json!({"v":3,"type":"offer","call_id":id,"video":true,"muted":false,"codec":"opus-h264-v3"});
    let rumor = EventBuilder::new(Kind::from(21112), signal.to_string()).build(owner);
    let result = a
        .protocol_engine
        .as_mut()
        .unwrap()
        .send_direct_unsigned_event_to_peer_only(
            recipient.public_key(),
            &recipient.public_key().to_hex(),
            rumor,
            unix_now(),
        )
        .unwrap();
    let encrypted = protocol_effect_events(&result.effects)
        .into_iter()
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap()
        .clone();
    EventBuilder::new(
        Kind::from(21111),
        serde_json::json!({"type":"call-wake","v":2,"events":[encrypted]}).to_string(),
    )
    .tags(
        encrypted
            .tags
            .public_keys()
            .copied()
            .map(nostr::Tag::public_key),
    )
    .sign_with_keys(sender)
    .unwrap()
}

#[test]
fn call_wake_v2_cold_preview_preserves_ratchet_and_foreground_enforces_contact_policy() {
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let (mut a, _, _adir) = logged_in_test_core_with_updates("wake-v2-a", &ao, &ad);
    let (mut b, _, _bdir) = logged_in_test_core_with_updates("wake-v2-b", &bo, &bd);
    install_call_ratchet_for_test(&mut a, &mut b, &ao, &bo, &ad, &bd);
    let id = "00112233445566778899aabbccddeeff";
    let event = ratcheted_call_wake_for_test(&mut a, &bo, &ad, id);
    assert!(!event.content.contains(id));
    assert!(!event.content.contains("opus-h264-v3"));
    let payload = serde_json::json!({"event":event}).to_string();
    let resolve = |core: &AppCore| {
        calls::push::resolve_call_push_invite(
            core.data_dir.to_string_lossy().into(),
            bd.secret_key().to_secret_hex(),
            payload.clone(),
        )
    };
    b.preferences.accepted_owner_pubkeys.clear();
    b.persist_best_effort();
    assert!(resolve(&b).is_none());
    b.receive_call_push(&event);
    assert!(b.calls.active.is_none());
    b.preferences
        .accepted_owner_pubkeys
        .push(ao.public_key().to_hex());
    b.preferences
        .blocked_owner_pubkeys
        .push(ao.public_key().to_hex());
    b.persist_best_effort();
    assert!(resolve(&b).is_none());
    b.receive_call_push(&event);
    assert!(b.calls.active.is_none());
    b.preferences.blocked_owner_pubkeys.clear();
    b.preferences.video_calls_enabled = false;
    b.preferences.voice_calls_enabled = false;
    b.persist_best_effort();
    assert!(resolve(&b).is_none());
    b.preferences.video_calls_enabled = true;
    b.preferences.voice_calls_enabled = true;
    b.persist_best_effort();
    for _ in 0..2 {
        assert_eq!(
            resolve(&b).unwrap().call_id,
            id,
            "cold preview never advances durable ratchet"
        );
    }
    let forged = EventBuilder::new(event.kind, &event.content)
        .tags(event.tags.clone())
        .sign_with_keys(&Keys::generate())
        .unwrap();
    b.receive_call_push(&forged);
    assert!(b.calls.active.is_none());
    b.receive_call_push(&event);
    assert_eq!(b.state.call.as_ref().unwrap().call_id, id);
    b.handle_action(AppAction::EndCall { call_id: id.into() });
    b.receive_call_push(&event);
    assert!(b.calls.active.is_none());
    assert!(b.threads[&ao.public_key().to_hex()]
        .messages
        .iter()
        .all(|message| !message.body.contains("call_id")));
}

#[test]
fn first_call_wake_fetches_acked_bootstrap_and_reordered_ciphertext_recovers_without_static_encryption(
) {
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let (mut a, _, _adir) = logged_in_test_core_with_updates("first-wake-a", &ao, &ad);
    let (mut b, _, _bdir) = logged_in_test_core_with_updates("first-wake-b", &bo, &bd);
    prepare_first_call_for_test(&mut a, &mut b, &ao, &bo, &ad, &bd);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    a.preferences.mobile_push_server_url = format!("http://{}", listener.local_addr().unwrap());
    let silent_peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    a.reconcile_calls_udp_for_test(
        "127.0.0.1:0".parse().unwrap(),
        silent_peer.local_addr().unwrap(),
        &test_fips_peer(&bd).npub(),
    );
    a.start_call(&bo.public_key().to_hex(), true);
    let id = a.state.call.as_ref().unwrap().call_id.clone();
    let events: Vec<Event> = a
        .pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str(&pending.event_json).ok())
        .collect();
    let response = events
        .iter()
        .find(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
        .unwrap();
    let encrypted = events
        .iter()
        .find(|event| event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
        .unwrap();
    let relay = crate::local_relay::TestRelay::start();
    b.preferences.nostr_relay_urls = vec![relay.url().into()];
    b.persist_best_effort();
    let inline = EventBuilder::new(
        Kind::from(21111),
        serde_json::json!({"type":"call-wake","v":2,"events":[response,encrypted]}).to_string(),
    )
    .tag(nostr::Tag::public_key(bd.public_key()))
    .sign_with_keys(&ad)
    .unwrap();
    assert!(
        inline.as_json().len() > 4096,
        "owner proof must never be dropped to fit push"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "reference wake cannot precede bootstrap relay acceptance"
    );
    publish_signer_test_event(&a, &relay, response);
    a.handle_relay_publish_finished(
        response.id.to_hex(),
        true,
        vec![relay.url().into()],
        "accepted".into(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "accepted bootstrap must release the wake"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("wake connection failed: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let event = read_call_wake_http_for_test(&mut stream);
    assert!(event.as_json().len() <= 4096);
    let envelope: serde_json::Value = serde_json::from_str(&event.content).unwrap();
    assert_eq!(envelope["bootstrapEventId"], response.id.to_hex());
    assert_eq!(envelope["events"].as_array().unwrap().len(), 1);
    let payload = serde_json::json!({"event":event}).to_string();
    let resolve = |core: &AppCore| {
        calls::push::resolve_call_push_invite(
            core.data_dir.to_string_lossy().into(),
            bd.secret_key().to_secret_hex(),
            payload.clone(),
        )
    };
    assert_eq!(
        resolve(&b).unwrap().call_id,
        id,
        "cold first call needs no earlier message"
    );
    // Relay delivery can put the encrypted offer before the bootstrap response.
    b.handle_relay_event(encrypted.clone());
    assert!(b.state.call.is_none());
    assert_eq!(
        resolve(&b).unwrap().call_id,
        id,
        "preview also handles a parked ciphertext"
    );
    let (tx, rx) = flume::unbounded();
    b.core_sender = tx;
    b.receive_call_push(&event);
    pump_signer_core_until(&mut b, &rx, |core| core.state.call.is_some());
    assert_eq!(b.state.call.as_ref().unwrap().call_id, id);
    assert!(b.threads[&ao.public_key().to_hex()]
        .messages
        .iter()
        .all(|message| !message.body.contains("call_id")));
}

fn read_call_wake_http_for_test(stream: &mut std::net::TcpStream) -> Event {
    use std::io::{Read, Write};
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

#[test]
fn call_wake_v2_discovers_earlier_unread_bootstrap_after_sender_restart() {
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let co = Keys::generate();
    let cd = Keys::generate();
    let (mut a, _, _adir) = logged_in_test_core_with_updates("older-wake-a", &ao, &ad);
    let (mut b, _, _bdir) = logged_in_test_core_with_updates("older-wake-b", &bo, &bd);
    let (mut c, _, _cdir) = logged_in_test_core_with_updates("older-wake-c", &co, &cd);
    prepare_first_call_for_test(&mut a, &mut b, &ao, &bo, &ad, &bd);
    prepare_first_call_for_test(&mut c, &mut b, &co, &bo, &cd, &bd);
    let earlier_response = |sender: &mut AppCore, owner: &Keys| {
        let rumor =
            EventBuilder::new(Kind::from(1), "Earlier unread message").build(owner.public_key());
        let result = sender
            .protocol_engine
            .as_mut()
            .unwrap()
            .send_direct_unsigned_event_to_peer_only(
                bo.public_key(),
                &bo.public_key().to_hex(),
                rumor,
                unix_now(),
            )
            .unwrap();
        protocol_effect_events(&result.effects)
            .into_iter()
            .find(|event| event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND)
            .unwrap()
            .clone()
    };
    let response = earlier_response(&mut a, &ao);
    let unrelated = earlier_response(&mut c, &co);
    // Reload only durable sender protocol state. No active-call bootstrap cache
    // survives, and the recipient still has never received the first message.
    a.protocol_engine = None;
    let storage = Arc::new(SqliteStorageAdapter::new(
        a.app_store.shared(),
        ao.public_key().to_hex(),
        ad.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut restored =
        ProtocolEngine::load_or_create_for_local_device(storage, ao.public_key(), &ad).unwrap();
    restored.authenticate_local_owner_for_sending(&ao).unwrap();
    a.protocol_engine = Some(restored);
    let id = "aabbccddeeff00112233445566778899";
    let wake = ratcheted_call_wake_for_test(&mut a, &bo, &ad, id);
    let envelope: serde_json::Value = serde_json::from_str(&wake.content).unwrap();
    assert!(envelope.get("bootstrapEventId").is_none());
    let payload = serde_json::json!({"event":wake}).to_string();
    let relay = crate::local_relay::TestRelay::start();
    b.preferences.nostr_relay_urls = vec![relay.url().into()];
    b.persist_best_effort();
    let resolve = || {
        calls::push::resolve_call_push_invite(
            b.data_dir.to_string_lossy().into(),
            bd.secret_key().to_secret_hex(),
            payload.clone(),
        )
    };
    publish_signer_test_event(&c, &relay, &unrelated);
    assert!(
        resolve().is_none(),
        "another valid bootstrap cannot authenticate this caller's ciphertext"
    );
    publish_signer_test_event(&a, &relay, &response);
    for _ in 0..2 {
        assert_eq!(
            resolve().unwrap().call_id,
            id,
            "normal recipient discovery recovers the older session without advancing durable state"
        );
    }
    assert!(b.state.call.is_none(), "cold preview only reads state");
}
