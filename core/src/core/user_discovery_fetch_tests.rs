use super::*;
use nostr::{EventBuilder, Keys, Tag, Timestamp};
use tempfile::TempDir;

const CLEARLY_FUTURE_OFFSET_SECS: u64 = 60 * 60;

fn follow_event(keys: &Keys, created_at: u64, followed: &[PublicKey]) -> Event {
    EventBuilder::new(Kind::ContactList, "")
        .tags(followed.iter().copied().map(Tag::public_key))
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn positions(cache: &UserDiscoveryCache, owners: &[&Keys]) -> Vec<u32> {
    owners
        .iter()
        .map(|owner| cache.users[&owner.public_key().to_hex()].follow_position)
        .collect()
}

async fn fetch_reconciled_fixture(
    events: Vec<Event>,
    owner: PublicKey,
    previous: UserDiscoveryCache,
    expected_ids: &[nostr::EventId],
) -> UserDiscoveryFetchResult {
    let relay = crate::core::protocol::public_discovery::test_support::graph_fixture(events).await;
    let result = fetch_user_discovery(
        Arc::new(DiscoveryRelays::default()),
        vec![relay.url.clone()],
        owner,
        previous,
    )
    .await;
    let frames = relay.wire.lock().unwrap();
    for kind in [3, 10000] {
        assert!(frames
            .iter()
            .any(|frame| frame[0] == "NEG-OPEN" && frame[2]["kinds"] == serde_json::json!([kind])));
    }
    for id in expected_ids {
        assert!(frames.iter().any(|frame| frame[0] == "REQ"
            && frame[2]["ids"]
                .as_array()
                .is_some_and(|ids| ids.contains(&serde_json::json!(id.to_hex())))));
    }
    assert!(frames.iter().all(|frame| frame[0] != "EVENT"));
    result
}

#[test]
fn relay_people_graph_fetches_mutes_and_restores_them_offline() {
    let owner = Keys::generate();
    let friend = Keys::generate();
    let target = Keys::generate();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cache = runtime.block_on(async {
        let muted = EventBuilder::new(Kind::from(10_000), "")
            .tags([Tag::public_key(target.public_key())])
            .custom_created_at(Timestamp::from(2))
            .sign_with_keys(&friend)
            .unwrap();
        fetch_reconciled_fixture(
            vec![
                follow_event(&owner, 1, &[friend.public_key()]),
                muted.clone(),
            ],
            owner.public_key(),
            UserDiscoveryCache::default(),
            &[muted.id],
        )
        .await
        .cache
    });
    let temp = TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(temp.path()).unwrap());
    store.replace_user_discovery(&cache).unwrap();
    let restored = store.load_user_discovery().unwrap();
    assert_eq!(restored, cache);
    let graph = nostr_social_graph::SocialGraph::from_binary(
        &owner.public_key().to_hex(),
        restored.social_graph.as_deref().unwrap(),
    )
    .unwrap();
    assert!(graph.is_overmuted(&target.public_key().to_hex(), 1.0));
}

#[test]
fn relay_fetch_keeps_all_follows_and_preserves_social_order_until_root_changes() {
    let root = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let carol = Keys::generate();
    let owners = [&alice, &bob, &carol];
    let root_follow = follow_event(
        &root,
        100,
        &owners
            .iter()
            .map(|owner| owner.public_key())
            .collect::<Vec<_>>(),
    );
    let alice_follows_carol = follow_event(&alice, 100, &[carol.public_key()]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let first = runtime.block_on(fetch_reconciled_fixture(
        vec![root_follow.clone(), alice_follows_carol.clone()],
        root.public_key(),
        UserDiscoveryCache::default(),
        &[alice_follows_carol.id],
    ));
    assert_eq!(first.cache.users.len(), owners.len());
    assert_eq!(positions(&first.cache, &owners), [1, 2, 0]);
    assert!(first.cache.social_rank_ready);
    assert_eq!(
        first.cache.social_friend_support[&carol.public_key().to_hex()],
        1
    );

    let temp = TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(temp.path()).unwrap());
    store.replace_user_discovery(&first.cache).unwrap();
    assert_eq!(
        positions(&store.load_user_discovery().unwrap(), &owners),
        [1, 2, 0]
    );
    let first_social_support = first.cache.social_friend_support.clone();

    let second = runtime.block_on(async {
        let future = follow_event(
            &alice,
            unix_now().get() + CLEARLY_FUTURE_OFFSET_SECS,
            &[carol.public_key()],
        );
        fetch_reconciled_fixture(
            vec![root_follow.clone(), future.clone()],
            root.public_key(),
            first.cache,
            &[future.id],
        )
        .await
    });
    assert_eq!(second.cache.users.len(), owners.len());
    assert_eq!(positions(&second.cache, &owners), [1, 2, 0]);
    assert_eq!(second.cache.social_friend_support, first_social_support);
    // The retained signed head remains usable when a relay only returns a future revision.
    assert!(second.detail.contains("social_failed_chunks=0"));
    assert!(!second.cache.social_opinion_events.is_empty());
    store.replace_user_discovery(&second.cache).unwrap();
    assert_eq!(
        positions(&store.load_user_discovery().unwrap(), &owners),
        [1, 2, 0]
    );

    let changed_owners = [&bob, &carol];
    let changed_root = follow_event(
        &root,
        101,
        &changed_owners
            .iter()
            .map(|owner| owner.public_key())
            .collect::<Vec<_>>(),
    );
    let third = runtime.block_on(async {
        let future = follow_event(
            &bob,
            unix_now().get() + CLEARLY_FUTURE_OFFSET_SECS,
            &[carol.public_key()],
        );
        fetch_reconciled_fixture(
            vec![changed_root.clone(), future.clone()],
            root.public_key(),
            second.cache,
            &[future.id],
        )
        .await
    });
    assert_eq!(third.cache.users.len(), changed_owners.len());
    assert_eq!(positions(&third.cache, &changed_owners), [0, 1]);
    assert_eq!(third.cache.follow_event_id, Some(changed_root.id.to_hex()));
    assert!(!third.cache.social_rank_ready);
    assert!(third.cache.social_friend_support.is_empty());
    assert!(third.detail.contains("social_failed_chunks=1"));
}

#[test]
fn local_relay_ignores_future_root_and_preserves_metadata_and_unfollow() {
    let relay = crate::local_relay::TestRelay::start();
    let local_owner = Keys::generate();
    let followed_owner = Keys::generate();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    let follow = EventBuilder::new(Kind::ContactList, "")
        .tags([Tag::parse([
            "p",
            followed_owner.public_key().to_hex().as_str(),
            "",
            "Relay friend",
        ])
        .unwrap()])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&local_owner)
        .unwrap();
    let metadata = EventBuilder::new(
        Kind::Metadata,
        r#"{"name":"relay-alice","display_name":"Relay Alice","about":"Local test"}"#,
    )
    .custom_created_at(Timestamp::from(100))
    .sign_with_keys(&followed_owner)
    .unwrap();
    let publisher = Client::new(Keys::generate());
    let discovery_client = Client::new(Keys::generate());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let first = runtime.block_on(async {
        ensure_session_relays_configured(&publisher, &relay_urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher.send_event(&follow).await.unwrap();
        publisher.send_event(&metadata).await.unwrap();
        fetch_user_discovery(
            Arc::new(DiscoveryRelays::default()),
            relay_urls.clone(),
            local_owner.public_key(),
            UserDiscoveryCache::default(),
        )
        .await
    });
    assert_eq!(first.cache.users.len(), 1);
    assert_eq!(
        first
            .cache
            .users
            .get(&followed_owner.public_key().to_hex())
            .and_then(|user| user.petname.as_deref()),
        Some("Relay friend")
    );
    assert_eq!(first.metadata_events.len(), 1);
    assert_eq!(first.cache.follow_event_id, Some(follow.id.to_hex()));

    let future = follow_event(
        &local_owner,
        unix_now().get() + CLEARLY_FUTURE_OFFSET_SECS,
        &[],
    );
    let mut poisoned = first.cache;
    poisoned.follow_event_id = Some(future.id.to_hex());
    poisoned.follow_created_at_secs = future.created_at.as_secs();
    poisoned.users.clear();
    let second = runtime.block_on(async {
        publisher.send_event(&future).await.unwrap();
        fetch_user_discovery(
            Arc::new(DiscoveryRelays::default()),
            relay_urls.clone(),
            local_owner.public_key(),
            poisoned,
        )
        .await
    });
    // This ordinary relay has no NIP-77 support. Root/profile reads still
    // succeed, while graph opinions honestly remain incomplete.
    assert_eq!(
        second.detail,
        "follows=1 metadata_failed_chunks=0 social_failed_chunks=1"
    );
    assert_eq!(second.cache.users.len(), 1);
    assert_eq!(second.cache.follow_event_id, Some(follow.id.to_hex()));

    let unfollow = follow_event(&local_owner, 101, &[]);
    let third = runtime.block_on(async {
        publisher.send_event(&unfollow).await.unwrap();
        let result = fetch_user_discovery(
            Arc::new(DiscoveryRelays::default()),
            relay_urls,
            local_owner.public_key(),
            second.cache,
        )
        .await;
        let _ = publisher.shutdown().await;
        let _ = discovery_client.shutdown().await;
        result
    });
    assert!(third.cache.users.is_empty());
    assert_eq!(third.cache.follow_event_id, Some(unfollow.id.to_hex()));
}

#[test]
fn opinion_cache_filters_future_mutes_before_replacement_and_keeps_valid_head() {
    let owner = Keys::generate();
    let valid = EventBuilder::new(Kind::MuteList, "valid")
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&owner)
        .unwrap();
    let future = EventBuilder::new(Kind::MuteList, "future")
        .custom_created_at(Timestamp::from(
            unix_now().get() + CLEARLY_FUTURE_OFFSET_SECS,
        ))
        .sign_with_keys(&owner)
        .unwrap();
    let records = encode_opinion_cache(
        [valid.clone(), future].into_iter(),
        &HashSet::from([owner.public_key()]),
    );
    assert_eq!(decode_opinion_cache(&records), vec![valid]);
}
