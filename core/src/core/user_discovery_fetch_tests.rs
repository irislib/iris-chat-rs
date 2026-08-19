use super::*;
use nostr::{EventBuilder, Keys, Tag, Timestamp};
use nostr_double_ratchet::DeviceEntry;
use tempfile::TempDir;

const CLEARLY_FUTURE_OFFSET_SECS: u64 = 60 * 60;

fn follow_event(keys: &Keys, created_at: u64, followed: &[PublicKey]) -> Event {
    EventBuilder::new(Kind::ContactList, "")
        .tags(followed.iter().copied().map(Tag::public_key))
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn app_keys_event(keys: &Keys, created_at: u64) -> Event {
    AppKeys::new(vec![DeviceEntry::new(
        Keys::generate().public_key(),
        created_at,
    )])
    .get_event_at(keys.public_key(), created_at)
    .sign_with_keys(keys)
    .unwrap()
}

fn positions(cache: &UserDiscoveryCache, owners: &[&Keys]) -> Vec<u32> {
    owners
        .iter()
        .map(|owner| cache.users[&owner.public_key().to_hex()].follow_position)
        .collect()
}

#[test]
fn relay_fetch_persists_social_order_and_preserves_it_until_root_changes() {
    let relay = crate::local_relay::TestRelay::start();
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
    let app_keys = owners
        .iter()
        .map(|owner| app_keys_event(owner, 100))
        .collect::<Vec<_>>();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    let publisher = Client::new(Keys::generate());
    let discovery_client = Client::new(Keys::generate());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let first = runtime.block_on(async {
        ensure_session_relays_configured(&publisher, &relay_urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher.send_event(&root_follow).await.unwrap();
        publisher.send_event(&alice_follows_carol).await.unwrap();
        for event in &app_keys {
            publisher.send_event(event).await.unwrap();
        }
        fetch_user_discovery(
            discovery_client.clone(),
            relay_urls.clone(),
            root.public_key(),
            UserDiscoveryCache::default(),
        )
        .await
    });
    assert_eq!(first.cache.users.len(), owners.len());
    assert_eq!(positions(&first.cache, &owners), [1, 2, 0]);

    let temp = TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(temp.path()).unwrap());
    store.replace_user_discovery(&first.cache).unwrap();
    assert_eq!(
        positions(&store.load_user_discovery().unwrap(), &owners),
        [1, 2, 0]
    );

    let second = runtime.block_on(async {
        let future = follow_event(
            &alice,
            unix_now().get() + CLEARLY_FUTURE_OFFSET_SECS,
            &[carol.public_key()],
        );
        publisher.send_event(&future).await.unwrap();
        let result = fetch_user_discovery(
            discovery_client.clone(),
            relay_urls.clone(),
            root.public_key(),
            first.cache,
        )
        .await;
        result
    });
    assert_eq!(second.cache.users.len(), owners.len());
    assert_eq!(positions(&second.cache, &owners), [1, 2, 0]);
    assert!(second.detail.contains("social_failed_chunks=1"));
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
        publisher.send_event(&changed_root).await.unwrap();
        publisher.send_event(&future).await.unwrap();
        let result = fetch_user_discovery(
            discovery_client.clone(),
            relay_urls,
            root.public_key(),
            second.cache,
        )
        .await;
        let _ = publisher.shutdown().await;
        let _ = discovery_client.shutdown().await;
        result
    });
    assert_eq!(third.cache.users.len(), changed_owners.len());
    assert_eq!(positions(&third.cache, &changed_owners), [0, 1]);
    assert_eq!(third.cache.follow_event_id, Some(changed_root.id.to_hex()));
    assert!(third.detail.contains("social_failed_chunks=1"));
}

#[test]
fn local_relay_ignores_future_root_and_preserves_metadata_and_unfollow() {
    let relay = crate::local_relay::TestRelay::start();
    let local_owner = Keys::generate();
    let followed_owner = Keys::generate();
    let followed_device = Keys::generate().public_key();
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
    let app_keys = AppKeys::new(vec![DeviceEntry::new(followed_device, 100)])
        .get_event_at(followed_owner.public_key(), 100)
        .sign_with_keys(&followed_owner)
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
        publisher.send_event(&app_keys).await.unwrap();
        publisher.send_event(&metadata).await.unwrap();
        fetch_user_discovery(
            discovery_client.clone(),
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
            discovery_client.clone(),
            relay_urls.clone(),
            local_owner.public_key(),
            poisoned,
        )
        .await
    });
    assert_eq!(
        second.detail,
        "follows=1 eligible=1 failed_chunks=0 metadata_failed_chunks=0 social_failed_chunks=0"
    );
    assert_eq!(second.cache.users.len(), 1);
    assert_eq!(second.cache.follow_event_id, Some(follow.id.to_hex()));

    let unfollow = follow_event(&local_owner, 101, &[]);
    let third = runtime.block_on(async {
        publisher.send_event(&unfollow).await.unwrap();
        let result = fetch_user_discovery(
            discovery_client.clone(),
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
