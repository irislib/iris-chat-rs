#[test]
fn private_block_retires_late_sealed_controls_by_authored_time_after_reload() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let ready = Keys::generate();
    let late = Keys::generate();
    let target = peer.public_key().to_hex();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("block-late-controls", &owner, &device);
    core.app_store.bind_account(owner.public_key()).unwrap();
    core.preferences.nostr_relay_urls.clear();
    core.preferences.nearby_enabled = false;
    let engine = core.protocol_engine.as_mut().unwrap();
    observe_current_device_appkeys_for_test(engine, &owner, &device);
    observe_peer_device_invite_for_test(engine, &peer, &ready, 2);
    observe_peer_appkeys_for_test(engine, &peer, &[ready.public_key(), late.public_key()], 3);
    for at in [50, 150, 250] {
        let rumor = EventBuilder::new(Kind::from(RECEIPT_KIND as u16), "seen")
            .custom_created_at(Timestamp::from_secs(at))
            .build(owner.public_key());
        let sent = engine
            .send_direct_unsigned_event_to_peer_only(
                peer.public_key(),
                &target,
                rumor,
                UnixSeconds(at),
            )
            .unwrap();
        assert!(
            !sent.effects.is_empty(),
            "ready device permits partial preparation"
        );
    }
    let mut rng = OsRng;
    let mut context = ProtocolContext::new(NdrUnixSeconds(300), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut context,
        ndr_device_pubkey(late.public_key()),
        Some(ndr_owner_pubkey(peer.public_key())),
        None,
    )
    .unwrap();
    let invite = nostr_double_ratchet::invite_unsigned_event(&invite)
        .unwrap()
        .sign_with_keys(&late)
        .unwrap();
    assert_eq!(invite.created_at.as_secs(), 300);
    let effects = engine.observe_invite_event(&invite).unwrap().effects;
    let mut published = Vec::new();
    for effect in effects {
        let ProtocolEffect::Publish(publish) = effect;
        if publish.inner_event_id.is_some() {
            assert_eq!(publish.event.created_at.as_secs(), 300);
            let authored = publish
                .authored_at_secs
                .expect("plaintext origin retained before sealing");
            published.push((authored, publish.event.id.to_hex()));
            assert!(core.publish_protocol_event(publish));
        }
    }
    published.sort();
    assert_eq!(
        published.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
        vec![50, 150, 250]
    );
    assert!(
        core.threads
            .get(&target)
            .is_none_or(|thread| thread.messages.is_empty()),
        "controls deliberately have no visible message row to recover timestamps from"
    );
    let legacy = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "legacy control")
        .custom_created_at(Timestamp::from_secs(300))
        .sign_with_keys(&device)
        .unwrap();
    assert!(core.publish_runtime_event(
        legacy.clone(),
        "legacy-control",
        Some(("legacy-control-id".into(), target.clone())),
    ));
    core.persist_best_effort_inner();
    drop(core);

    let (tx, _rx) = flume::unbounded();
    let mut restored = AppCore::new(
        flume::unbounded().0,
        tx,
        dir.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    restored
        .start_session(
            owner.public_key(),
            Some(owner.clone()),
            device.clone(),
            true,
            true,
        )
        .unwrap();
    restored.run_session_startup_follow_up();
    let shared = restored.app_store.shared();
    for (at, id) in &published {
        let authored: Option<u64> = shared
            .lock()
            .unwrap()
            .query_row(
                "SELECT authored_at_secs FROM pending_relay_publishes WHERE event_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            authored,
            Some(*at),
            "authored time survives durable outbox reload"
        );
    }
    let legacy_authored: Option<u64> = shared
        .lock()
        .unwrap()
        .query_row(
            "SELECT authored_at_secs FROM pending_relay_publishes WHERE event_id=?1",
            [legacy.id.to_hex()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_authored, None);
    for (revision, at, blocked) in [(1, 100, true), (2, 200, false)] {
        assert!(restored.apply_private_block_event(signed_block_transition(
            &owner, &device, &target, revision, at, 100, blocked
        )));
    }
    let persisted = restored
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    for (at, id) in published {
        assert_eq!(
            restored.pending_relay_publishes.contains_key(&id),
            at == 250
        );
        assert_eq!(
            persisted.iter().any(|pending| pending.event_id == id),
            at == 250
        );
    }
    assert!(!restored
        .pending_relay_publishes
        .contains_key(&legacy.id.to_hex()));
    assert!(
        !persisted
            .iter()
            .any(|pending| pending.event_id == legacy.id.to_hex()),
        "legacy controls with unknowable origin must not escape a retained block period"
    );
}
