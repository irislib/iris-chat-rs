use super::*;

pub(in crate::core) fn accept_notification_peer_in_store(conn: &rusqlite::Connection, peer: &str) {
    conn.execute_batch("INSERT OR IGNORE INTO preferences(id, send_typing_indicators, send_read_receipts, desktop_notifications_enabled, startup_at_login_enabled, nostr_relay_urls_json, image_proxy_enabled, image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex, mobile_push_server_url)
        VALUES (1,1,1,1,0,'[]',1,'','','','');").unwrap();
    conn.execute("UPDATE preferences SET accepted_owner_pubkeys_json=json_insert(accepted_owner_pubkeys_json, '$[#]', ?1) WHERE id=1", [peer]).unwrap();
}

pub(super) fn request_network_peer(core: &mut AppCore, owner: &Keys, peer: PublicKey) {
    let friend = Keys::generate();
    let mut graph = nostr_social_graph::SocialGraph::new(&owner.public_key().to_hex());
    request_graph_event(&mut graph, owner, &[friend.public_key()], 3, 1);
    request_graph_event(&mut graph, &friend, &[peer], 3, 1);
    core.user_discovery.owner_pubkey_hex = Some(owner.public_key().to_hex());
    core.user_discovery.social_graph = Some(graph.to_binary().unwrap());
    core.app_store
        .replace_user_discovery(&core.user_discovery)
        .unwrap();
    core.refresh_social_graph();
    core.persist_best_effort_inner();
}

fn request_graph_event(
    graph: &mut nostr_social_graph::SocialGraph,
    author: &Keys,
    peers: &[PublicKey],
    kind: u32,
    at: u64,
) {
    graph.handle_event(
        &nostr_social_graph::NostrEvent {
            pubkey: author.public_key().to_hex(),
            kind,
            created_at: at,
            tags: peers
                .iter()
                .map(|peer| vec!["p".into(), peer.to_hex()])
                .collect(),
            content: String::new(),
            id: format!("{at:064x}"),
            sig: String::new(),
        },
        true,
        1.0,
    );
}

fn deliver_request(core: &mut AppCore, sender: &Keys, id: &str, body: &str) {
    core.apply_runtime_text_message(
        sender.public_key(),
        None,
        body.into(),
        unix_now().get(),
        None,
        Some(id.into()),
        None,
    );
}

#[test]
fn request_policy_full_graph_following_overmuted_and_switches() {
    use message_requests::RequestAccess::*;
    let owner = Keys::generate();
    let mut core = logged_in_test_core("request-policy-tiers", &owner, &Keys::generate());
    let peers: Vec<_> = (0..6).map(|_| Keys::generate()).collect();
    let mut graph = nostr_social_graph::SocialGraph::new(&owner.public_key().to_hex());
    request_graph_event(&mut graph, &owner, &[peers[0].public_key()], 3, 1);
    for n in 0..4 {
        request_graph_event(&mut graph, &peers[n], &[peers[n + 1].public_key()], 3, 1);
    }
    core.social_graph = Some(graph);
    core.preferences.accept_unknown_direct_messages = false;
    core.preferences.accept_social_graph_direct_messages = false;
    let followed = peers[0].public_key().to_hex();
    let distant = peers[4].public_key().to_hex();
    let stranger = peers[5].public_key().to_hex();
    assert_eq!(core.direct_request_access(&followed), Accepted);
    assert_eq!(core.direct_request_access(&distant), Denied);
    assert_eq!(core.direct_request_access(&stranger), Denied);
    core.preferences.accept_social_graph_direct_messages = true;
    assert_eq!(
        core.social_graph
            .as_ref()
            .unwrap()
            .get_follow_distance(&distant),
        5
    );
    assert_eq!(core.direct_request_access(&distant), Network);
    core.preferences.accept_unknown_direct_messages = true;
    assert_eq!(core.direct_request_access(&stranger), Unconnected);
    request_graph_event(
        core.social_graph.as_mut().unwrap(),
        &peers[0],
        &[peers[4].public_key()],
        10_000,
        2,
    );
    request_graph_event(
        core.social_graph.as_mut().unwrap(),
        &peers[1],
        &[peers[4].public_key()],
        10_000,
        2,
    );
    assert!(core
        .social_graph
        .as_ref()
        .unwrap()
        .is_overmuted(&distant, 1.0));
    assert_eq!(core.direct_request_access(&distant), Denied);
    core.preferences.accept_overmuted_direct_messages = true;
    assert_eq!(core.direct_request_access(&distant), Network);
    deliver_request(&mut core, &peers[0], "followed-first", "hello");
    assert!(!core.thread_is_message_request(&followed));
    core.social_graph = None;
    core.preferences.accept_unknown_direct_messages = false;
    assert_eq!(
        core.direct_request_access(&followed),
        Accepted,
        "unfollowing preserves accepted chats"
    );
    core.set_user_blocked(&followed, true);
    assert_eq!(core.direct_request_access(&followed), Denied);
}

#[test]
fn request_policy_pending_threads_do_not_bypass_disabled_requests() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("request-policy-pending", &owner, &Keys::generate());
    deliver_request(&mut core, &peer, "first", "hello");
    let chat = peer.public_key().to_hex();
    assert!(core.thread_is_message_request(&chat));
    core.preferences.accept_unknown_direct_messages = false;
    deliver_request(&mut core, &peer, "second", "should be ignored");
    assert_eq!(core.threads[&chat].messages.len(), 1);
    core.accept_message_request(&chat);
    deliver_request(&mut core, &peer, "third", "accepted");
    assert_eq!(core.threads[&chat].messages.len(), 2);
}

#[test]
fn request_policy_newest_fifty_survive_restart_without_bumping_or_evicting_accepted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("request-policy-fifo", &owner, &device);
    let peers: Vec<_> = (0..52).map(|_| Keys::generate()).collect();
    for (n, peer) in peers.iter().take(50).enumerate() {
        deliver_request(&mut core, peer, &format!("first-{n}"), "hello");
    }
    let oldest = peers[0].public_key().to_hex();
    let accepted = peers[1].public_key().to_hex();
    core.accept_message_request(&accepted);
    deliver_request(&mut core, &peers[50], "fifty-first", "hello");
    deliver_request(
        &mut core,
        &peers[0],
        "oldest-again",
        "more messages do not bump me",
    );
    let order = core.app_store.request_arrivals().unwrap();
    core.persist_best_effort_inner();
    drop(core);
    let mut restored =
        restart_block_history_core(&owner, &device, dir.path().to_string_lossy().into_owned());
    restored.restore_device_sync_record_projection().unwrap();
    assert_eq!(restored.app_store.request_arrivals().unwrap(), order);
    deliver_request(&mut restored, &peers[51], "fifty-second", "hello");
    assert!(!restored.threads.contains_key(&oldest));
    assert!(restored.threads.contains_key(&accepted));
    assert!(restored
        .threads
        .contains_key(&peers[51].public_key().to_hex()));
    let pending = restored
        .threads
        .keys()
        .filter(|peer| {
            restored.direct_request_access(peer) == message_requests::RequestAccess::Unconnected
        })
        .count();
    assert_eq!(pending, 50);
    let cutoff = restored.chat_deletions[&oldest];
    restored.apply_runtime_text_message(
        peers[0].public_key(),
        None,
        "replay".into(),
        cutoff,
        None,
        Some("replayed-old".into()),
        None,
    );
    assert!(!restored.threads.contains_key(&oldest));
}

#[test]
fn request_policy_bounds_messages_and_bytes_per_pending_sender() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let large = Keys::generate();
    let mut core = logged_in_test_core("request-policy-bounds", &owner, &Keys::generate());
    for n in 0..30 {
        deliver_request(&mut core, &peer, &format!("message-{n}"), "hello");
    }
    assert_eq!(core.threads[&peer.public_key().to_hex()].messages.len(), 20);
    for n in 0..5 {
        deliver_request(
            &mut core,
            &large,
            &format!("large-{n}"),
            &"x".repeat(30 * 1024),
        );
    }
    assert_eq!(core.threads[&large.public_key().to_hex()].messages.len(), 2);
}

#[test]
fn request_policy_preferences_persist_and_push_stays_quiet_until_accepted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    let (mut core, _, dir) =
        logged_in_test_core_with_updates("request-policy-prefs", &owner, &device);
    core.handle_action(AppAction::SetAcceptSocialGraphDirectMessages { enabled: false });
    core.handle_action(AppAction::SetAcceptOvermutedDirectMessages { enabled: true });
    deliver_request(&mut core, &peer, "hello", "quiet request");
    core.rebuild_persist_and_emit_state();
    let notifications = crate::core::notifications::decide_notifications(
        &[],
        &core.state.chat_list,
        &core.preferences,
        false,
        None,
    );
    assert!(notifications.is_empty());
    let conn = core.app_store.shared();
    assert!(!crate::core::mobile_push::request_policy::allows(
        &conn.lock().unwrap(),
        &owner.public_key().to_hex(),
        &chat
    ));
    core.accept_message_request(&chat);
    assert!(crate::core::mobile_push::request_policy::allows(
        &conn.lock().unwrap(),
        &owner.public_key().to_hex(),
        &chat
    ));
    core.persist_best_effort_inner();
    drop(core);
    let restored =
        restart_block_history_core(&owner, &device, dir.path().to_string_lossy().into_owned());
    assert!(!restored.preferences.accept_social_graph_direct_messages);
    assert!(restored.preferences.accept_overmuted_direct_messages);
}

#[test]
fn request_policy_encrypted_first_contact_and_subscription_wire_stay_private() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    for allowed in [false, true] {
        let mut core = logged_in_test_core("request-policy-wire", &owner, &device);
        core.preferences.accept_unknown_direct_messages = allowed;
        let (event, _) = appcore_direct_message_event_with_author_keys_for_test(
            core.protocol_engine.as_mut().unwrap(),
            &sender,
            "secret request text",
            unix_now().get(),
        );
        let wire = serde_json::to_string(&event).unwrap();
        for private in [
            owner.public_key().to_hex(),
            sender.public_key().to_hex(),
            "secret request text".into(),
        ] {
            assert!(!wire.contains(&private));
        }
        core.pending_relay_publishes.clear();
        core.handle_relay_event(event.clone());
        core.handle_relay_event(event);
        let chat = sender.public_key().to_hex();
        assert_eq!(
            core.threads
                .get(&chat)
                .map_or(0, |thread| thread.messages.len()),
            usize::from(allowed)
        );
        assert!(
            pending_events_with_kind(&core, MESSAGE_EVENT_KIND).is_empty(),
            "requests must not send receipts"
        );
        let authors = core.subscribable_message_author_hexes();
        assert_eq!(authors.is_empty(), !allowed);
        let push = core.build_mobile_push_sync_snapshot();
        let request = build_mobile_push_create_subscription_request(
            owner.secret_key().to_secret_hex(),
            "ios".into(),
            "test-token".into(),
            Some("to.iris.chat".into()),
            push.message_author_pubkeys,
            Vec::new(),
            vec![Keys::generate().public_key().to_hex()],
            false,
            None,
            Vec::new(),
        )
        .expect("serialized notification subscription")
        .body_json
        .unwrap();
        assert!(!request.contains(&owner.public_key().to_hex()));
        assert!(!request.contains(&sender.public_key().to_hex()));
    }
}
