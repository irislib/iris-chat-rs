use super::test_support::*;
use super::*;
use nostr::Tag;
use std::sync::atomic::Ordering as AtomicOrdering;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn missing_negentropy_bodies_after_eose_preserve_cached_head_and_rank() {
    let owner = Keys::generate();
    let friend = Keys::generate();
    let root = EventBuilder::new(Kind::ContactList, "")
        .tags([Tag::public_key(friend.public_key())])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&owner)
        .unwrap();
    let old = event(&friend, Kind::ContactList, "old");
    let new = EventBuilder::new(Kind::ContactList, "new")
        .custom_created_at(Timestamp::from(101))
        .sign_with_keys(&friend)
        .unwrap();
    let fixture = fixture(vec![root.clone(), new.clone()]).await;
    fixture.omitted.lock().unwrap().insert(new.id);
    let mut cache = UserDiscoveryCache::default();
    cache.owner_pubkey_hex = Some(owner.public_key().to_hex());
    cache.follow_event_id = Some(root.id.to_hex());
    cache.follow_event_json = Some(serde_json::to_string(&root).unwrap());
    cache.follow_created_at_secs = 100;
    cache.users.insert(
        friend.public_key().to_hex(),
        DiscoveredUserRecord {
            owner_pubkey_hex: friend.public_key().to_hex(),
            follow_position: 0,
            petname: None,
        },
    );
    cache.social_rank_ready = true;
    cache
        .social_friend_support
        .insert(friend.public_key().to_hex(), 9);
    cache.social_opinion_events = vec![serde_json::to_string(&old).unwrap()];
    cache.social_graph = crate::core::user_discovery_graph::update_people_graph(
        owner.public_key(),
        Some(&root),
        &[friend.public_key()],
        std::slice::from_ref(&old),
        vec![],
        None,
        200,
    );
    let result = crate::core::user_discovery::fetch_user_discovery(
        Arc::new(DiscoveryRelays::default()),
        vec![fixture.url.clone()],
        owner.public_key(),
        cache,
    )
    .await;
    assert!(
        result.detail.contains("social_failed_chunks=1"),
        "{}",
        result.detail
    );
    assert_eq!(
        result.cache.social_friend_support[&friend.public_key().to_hex()],
        9
    );
    let retained = result
        .cache
        .social_opinion_events
        .iter()
        .map(|record| serde_json::from_str::<Event>(record).unwrap())
        .collect::<Vec<_>>();
    assert!(retained.iter().any(|event| event.id == old.id));
    assert!(!retained.iter().any(|event| event.id == new.id));
    let graph = nostr_social_graph::SocialGraph::from_binary(
        &owner.public_key().to_hex(),
        result.cache.social_graph.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        graph
            .get_list_revision(&friend.public_key().to_hex(), 3)
            .unwrap()
            .id,
        old.id.to_hex()
    );
    let frames = fixture.wire.lock().unwrap();
    assert!(frames.iter().any(|frame| frame[0] == "REQ"
        && frame[2]["ids"]
            .as_array()
            .is_some_and(|ids| ids.contains(&serde_json::json!(new.id.to_hex())))));
}

#[tokio::test]
async fn supported_five_thousand_follow_root_above_old_frame_limit_refreshes_and_unfollows() {
    let owner = Keys::generate();
    let follows = (0..5000)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    let root = EventBuilder::new(Kind::ContactList, "")
        .tags(follows.iter().copied().map(Tag::public_key))
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&owner)
        .unwrap();
    assert!(serde_json::to_vec(&root).unwrap().len() > 128 * 1024);
    let first = fixture(vec![root.clone()]).await;
    let pool = Arc::new(DiscoveryRelays::default());
    let result = crate::core::user_discovery::fetch_user_discovery(
        pool.clone(),
        vec![first.url.clone()],
        owner.public_key(),
        UserDiscoveryCache::default(),
    )
    .await;
    assert_eq!(result.cache.users.len(), 5000, "{}", result.detail);
    assert_eq!(result.cache.follow_event_id, Some(root.id.to_hex()));
    let empty = EventBuilder::new(Kind::ContactList, "")
        .custom_created_at(Timestamp::from(101))
        .sign_with_keys(&owner)
        .unwrap();
    let second = fixture(vec![empty.clone()]).await;
    let result = crate::core::user_discovery::fetch_user_discovery(
        pool,
        vec![second.url.clone()],
        owner.public_key(),
        result.cache,
    )
    .await;
    assert!(result.cache.users.is_empty());
    assert_eq!(result.cache.follow_event_id, Some(empty.id.to_hex()));
}

#[tokio::test]
async fn raw_padding_duplicates_invalid_json_and_binary_exhaust_shared_budget_and_close() {
    let author = Keys::generate();
    let head = event(&author, Kind::ContactList, "");
    let fixture = fixture(Vec::new()).await;
    fixture.hold_eose.store(true, Ordering::Release);
    let pool = Arc::new(DiscoveryRelays::default());
    let mut scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    until(|| count(&fixture, "AUTH") > 0).await;
    let counter = pool.wire.counter(&fixture.url).unwrap();
    let before = counter.load(Ordering::Relaxed);
    let padded = format!(
        " {}{}",
        serde_json::json!(["EVENT", "__graph_req__", head]),
        " ".repeat(2000)
    );
    let invalid = "not JSON ".repeat(200);
    *fixture.raw.lock().unwrap() = vec![
        Message::Text(padded.clone()),
        Message::Text(padded.clone()),
        Message::Text(invalid.clone()),
        Message::Binary(padded.clone().into_bytes()),
    ];
    scope.budget.max_bytes = 5000;
    let result = scope
        .fetch(
            vec![Filter::new()
                .kind(Kind::ContactList)
                .author(author.public_key())],
            Duration::from_secs(2),
        )
        .await;
    assert!(result.unwrap_err().contains("safety bound"));
    until(|| count(&fixture, "CLOSE") > 0).await;
    let id = fixture
        .wire
        .lock()
        .unwrap()
        .iter()
        .find(|frame| frame[0] == "REQ")
        .unwrap()[1]
        .as_str()
        .unwrap()
        .to_owned();
    let expected = padded.replace("__graph_req__", &id).len() * 3 + invalid.len();
    assert!(counter.load(Ordering::Relaxed) - before >= expected);
    assert_closed(&fixture, "REQ", "CLOSE");
}

#[tokio::test]
async fn oversized_negentropy_and_excess_rounds_stop_without_fallback_downloads() {
    for mode in [3, 4] {
        let fixture = fixture(Vec::new()).await;
        fixture.mode.store(mode, AtomicOrdering::SeqCst);
        let pool = Arc::new(DiscoveryRelays::default());
        let scope = pool
            .graph_scope(std::slice::from_ref(&fixture.url))
            .await
            .unwrap();
        let result = scope
            .opinions(Kind::ContactList, &[Keys::generate().public_key()], &[], 2)
            .await;
        assert!(result.unwrap_err().contains("safety bound"));
        until(|| count(&fixture, "NEG-CLOSE") == 1).await;
        assert_eq!(count(&fixture, "REQ"), 0);
        assert_eq!(
            count(&fixture, "NEG-MSG"),
            if mode == 4 { NEG_ROUNDS } else { 0 }
        );
    }
}
