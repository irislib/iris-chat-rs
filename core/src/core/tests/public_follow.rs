use nostr::JsonUtil;

#[test]
fn public_follow_round_trip_fetches_preserves_publishes_and_restores_the_list() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let peer = Keys::generate();
    let existing = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("public-follow", &owner, &Keys::generate());
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    core.logged_in.as_mut().unwrap().relay_urls = relay_urls_from_strings(&[relay.url().into()]);
    core.preferences.nostr_relay_urls = vec![relay.url().into()];
    let previous = EventBuilder::new(Kind::ContactList, r#"{"kept":"legacy relay settings"}"#)
        .tag(
            nostr::Tag::parse([
                "p",
                &existing.public_key().to_hex(),
                "wss://example.com",
                "Old friend",
            ])
            .unwrap(),
        )
        .tag(nostr::Tag::parse(["custom", "keep"]).unwrap())
        .custom_created_at(Timestamp::from_secs(unix_now().get() - 10))
        .sign_with_keys(&owner)
        .unwrap();
    publish_signer_test_event(&core, &relay, &previous);
    core.handle_action(AppAction::CreateChat {
        peer_input: peer.public_key().to_hex(),
    });
    let read_head = |core: &AppCore| {
        Event::from_json(core.user_discovery.follow_event_json.as_deref().unwrap()).unwrap()
    };
    for following in [true, false] {
        core.handle_action(AppAction::SetPublicFollow {
            owner_pubkey_hex: peer.public_key().to_hex(),
            following,
        });
        assert!(core.pending_follow.is_some());
        pump_signer_core_until(&mut core, &rx, |core| core.pending_follow.is_none());
        let head = read_head(&core);
        assert_eq!(head.pubkey, owner.public_key());
        assert!(head.verify().is_ok());
        assert_eq!(head.content, previous.content);
        assert!(previous
            .tags
            .iter()
            .all(|tag| head.tags.iter().any(|kept| kept == tag)));
        assert_eq!(
            head.tags.public_keys().any(|key| *key == peer.public_key()),
            following
        );
        assert_eq!(
            core.state
                .current_chat
                .as_ref()
                .unwrap()
                .contact_identity
                .as_ref()
                .unwrap()
                .is_following,
            following
        );
        pump_signer_core_until(&mut core, &rx, |_| {
            relay
                .events()
                .iter()
                .any(|event| event["id"] == head.id.to_hex())
        });
        assert!(core
            .app_store
            .load_user_discovery()
            .unwrap()
            .follow_event_json
            .is_some());
    }
}

#[test]
fn public_follow_failure_or_stale_response_cannot_replace_a_known_list() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("follow-failure", &owner, &Keys::generate());
    core.user_discovery.follow_event_id = Some("known-newer-head".into());
    core.user_discovery.follow_created_at_secs = 100;
    let pending = || {
        Some((
            "request".into(),
            owner.public_key().to_hex(),
            peer.public_key().to_hex(),
            true,
        ))
    };
    core.pending_follow = pending();
    core.finish_follow_update("another-request", Ok(Vec::new()));
    assert_eq!(core.pending_follow, pending());
    core.finish_follow_update("request", Ok(Vec::new()));
    assert!(core.pending_follow.is_none());
    assert_eq!(
        core.user_discovery.follow_event_id.as_deref(),
        Some("known-newer-head")
    );
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Couldn’t load your latest follow list. Try again.")
    );
    core.pending_follow = pending();
    core.finish_follow_update("request", Err("Server unavailable".into()));
    assert_eq!(core.state.toast.as_deref(), Some("Server unavailable"));
    assert!(pending_events_with_kind(&core, 3).is_empty());
}

#[test]
fn public_follow_uses_canonical_same_timestamp_head_and_reports_partial_save() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("follow-head", &owner, &Keys::generate());
    let time = unix_now().get() - 10;
    let mut heads = ["first", "second"].map(|content| {
        EventBuilder::new(Kind::ContactList, content)
            .custom_created_at(Timestamp::from_secs(time))
            .sign_with_keys(&owner)
            .unwrap()
    });
    heads.sort_by_key(|event| event.id);
    core.user_discovery.follow_event_id = Some(heads[1].id.to_hex());
    core.user_discovery.follow_event_json = Some(heads[1].as_json());
    core.user_discovery.follow_created_at_secs = time;
    core.pending_follow = Some((
        "canonical".into(),
        owner.public_key().to_hex(),
        peer.public_key().to_hex(),
        true,
    ));
    core.finish_follow_update("canonical", Ok(vec![heads[0].clone()]));
    let saved = Event::from_json(core.user_discovery.follow_event_json.as_ref().unwrap()).unwrap();
    assert_eq!(saved.content, heads[0].content);
    core.app_store.shared().lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_follow_cache BEFORE INSERT ON user_discovery_state BEGIN SELECT RAISE(FAIL, 'test cache write failure'); END;"
    ).unwrap();
    core.pending_follow = Some((
        "partial".into(),
        owner.public_key().to_hex(),
        peer.public_key().to_hex(),
        false,
    ));
    core.finish_follow_update("partial", Ok(vec![saved]));
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Follow change queued, but couldn’t save the local profile. Try refreshing.")
    );
    let live = Event::from_json(core.user_discovery.follow_event_json.as_ref().unwrap()).unwrap();
    assert!(!live.tags.public_keys().any(|key| *key == peer.public_key()));
    assert!(pending_events_with_kind(&core, 3)
        .iter()
        .any(|event| event.id == live.id));
}
