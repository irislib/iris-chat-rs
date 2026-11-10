use super::*;
use nostr::{EventBuilder, Keys, Tag, Timestamp};
use tempfile::TempDir;

const CLEARLY_FUTURE_OFFSET_SECS: u64 = 60 * 60;

async fn graph_wire_relay(
    events: Vec<Event>,
) -> (
    RelayUrl,
    std::sync::Arc<std::sync::Mutex<Vec<(usize, serde_json::Value)>>>,
    tokio::task::JoinHandle<()>,
) {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let frames = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = frames.clone();
    let task = tokio::spawn(async move {
        let mut connection = 0;
        while let Ok((socket, _)) = listener.accept().await {
            connection += 1;
            let captured = captured.clone();
            let events = events.clone();
            tokio::spawn(async move {
                let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
                socket
                    .send(Message::Text(
                        serde_json::json!(["AUTH", "graph-wire-challenge"])
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    let text = match message {
                        Message::Text(text) => text,
                        Message::Ping(data) => {
                            let _ = socket.send(Message::Pong(data)).await;
                            continue;
                        }
                        Message::Close(_) => break,
                        _ => continue,
                    };
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    captured.lock().unwrap().push((connection, frame.clone()));
                    if frame[0] == "AUTH" {
                        let _ = socket
                            .send(Message::Text(
                                serde_json::json!(["OK", frame[1]["id"], true, ""])
                                    .to_string()
                                    .into(),
                            ))
                            .await;
                    } else if frame[0] == "NEG-OPEN" {
                        let _ = socket
                            .send(Message::Text(
                                serde_json::json!(["NEG-ERR", frame[1], "unsupported"])
                                    .to_string()
                                    .into(),
                            ))
                            .await;
                    } else if frame[0] == "REQ" {
                        for event in &events {
                            if frame.as_array().unwrap()[2..].iter().any(|filter| {
                                filter.get("kinds").is_none_or(|kinds| {
                                    kinds.as_array().unwrap().iter().any(|kind| {
                                        kind.as_u64() == Some(event.kind.as_u16() as u64)
                                    })
                                }) && filter.get("authors").is_none_or(|authors| {
                                    authors.as_array().unwrap().iter().any(|author| {
                                        author.as_str() == Some(event.pubkey.to_hex().as_str())
                                    })
                                }) && filter.get("ids").is_none_or(|ids| {
                                    ids.as_array()
                                        .unwrap()
                                        .iter()
                                        .any(|id| id.as_str() == Some(event.id.to_hex().as_str()))
                                })
                            }) {
                                let value = serde_json::to_value(event).unwrap();
                                let _ = socket
                                    .send(Message::Text(
                                        serde_json::json!(["EVENT", frame[1], value])
                                            .to_string()
                                            .into(),
                                    ))
                                    .await;
                            }
                        }
                        let _ = socket
                            .send(Message::Text(
                                serde_json::json!(["EOSE", frame[1]]).to_string().into(),
                            ))
                            .await;
                    }
                }
            });
        }
    });
    (url, frames, task)
}

#[test]
fn graph_wire_negentropy_has_public_scope_and_never_authenticates_or_uploads() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = Keys::generate();
        let friend = Keys::generate();
        let private_device = Keys::generate();
        let mutes = EventBuilder::new(Kind::from(10_000), "")
            .tags([Tag::public_key(owner.public_key())])
            .custom_created_at(Timestamp::from(3))
            .sign_with_keys(&friend)
            .unwrap();
        let (url, frames, server) = graph_wire_relay(vec![
            follow_event(&owner, 1, &[friend.public_key()]),
            follow_event(&friend, 2, &[owner.public_key()]),
            mutes,
        ])
        .await;
        let private_client = Client::new(private_device.clone());
        let result = fetch_user_discovery(
            private_client.clone(),
            vec![url],
            owner.public_key(),
            UserDiscoveryCache::default(),
        )
        .await;
        private_client.disconnect().await;
        assert!(result.cache.social_graph.is_some());
        let frames = frames.lock().unwrap();
        let public_connections = frames
            .iter()
            .filter(|(_, frame)| frame[0] == "NEG-OPEN")
            .map(|(connection, _)| *connection)
            .collect::<HashSet<_>>();
        assert!(
            !public_connections.is_empty(),
            "must exercise actual NIP-77 wire requests"
        );
        let mut kinds = std::collections::BTreeSet::new();
        for (connection, frame) in frames
            .iter()
            .filter(|(connection, _)| public_connections.contains(connection))
        {
            assert!(
                frame[0] != "AUTH" && frame[0] != "EVENT",
                "anonymous graph connection {connection} sent {frame}"
            );
            assert!(!frame
                .to_string()
                .contains(&private_device.public_key().to_hex()));
            if frame[0] == "NEG-OPEN" {
                assert!(frame.to_string().len() < 16_384);
                let filter = &frame[2];
                for kind in filter["kinds"].as_array().unwrap() {
                    kinds.insert(kind.as_u64().unwrap());
                }
                let authors = filter["authors"].as_array().unwrap();
                assert!(authors.len() <= DISCOVERY_AUTHOR_CHUNK);
                assert!(authors.iter().all(|author| [
                    owner.public_key().to_hex(),
                    friend.public_key().to_hex()
                ]
                .iter()
                .any(|allowed| author.as_str() == Some(allowed.as_str()))));
            }
        }
        assert_eq!(kinds, std::collections::BTreeSet::from([3, 10_000]));
        server.abort();
    });
}

#[test]
fn graph_wire_follow_and_mute_downloads_share_one_byte_budget() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let author = Keys::generate();
        let heads = [3, 10_000]
            .into_iter()
            .map(|kind| {
                EventBuilder::new(Kind::from(kind), "x".repeat(1500))
                    .custom_created_at(Timestamp::from(10))
                    .sign_with_keys(&author)
                    .unwrap()
            })
            .collect();
        let (url, _, server) = graph_wire_relay(heads).await;
        let client = Client::builder().build();
        ensure_session_relays_configured(&client, &[url]).await;
        connect_client_with_timeout(&client, DISCOVERY_REQUEST_TIMEOUT).await;
        let budget = std::sync::atomic::AtomicUsize::new(2048);
        let (follows, mutes) = tokio::join!(
            fetch_graph_opinion_chunks(
                &client,
                &[],
                &budget,
                Kind::ContactList,
                vec![author.public_key()],
                None
            ),
            fetch_graph_opinion_chunks(
                &client,
                &[],
                &budget,
                Kind::from(10_000),
                vec![author.public_key()],
                None
            ),
        );
        let results = follows
            .into_iter()
            .chain(mutes)
            .map(|(_, result)| result)
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(results.iter().any(|result| result
            .as_ref()
            .err()
            .is_some_and(|error| error.contains("byte or event budget"))));
        let retained_bytes = results
            .into_iter()
            .filter_map(Result::ok)
            .flatten()
            .map(|event| serde_json::to_vec(&event).unwrap().len())
            .sum::<usize>();
        assert!(retained_bytes <= 2048);
        client.disconnect().await;
        server.abort();
    });
}

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

#[test]
fn relay_people_graph_fetches_mutes_and_restores_them_offline() {
    let relay = crate::local_relay::TestRelay::start();
    let owner = Keys::generate();
    let friend = Keys::generate();
    let target = Keys::generate();
    let client = Client::new(Keys::generate());
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cache = runtime.block_on(async {
        ensure_session_relays_configured(&client, &relay_urls).await;
        connect_client_with_timeout(&client, Duration::from_secs(2)).await;
        client
            .send_event(&follow_event(&owner, 1, &[friend.public_key()]))
            .await
            .unwrap();
        let muted = EventBuilder::new(Kind::from(10_000), "")
            .tags([Tag::public_key(target.public_key())])
            .custom_created_at(Timestamp::from(2))
            .sign_with_keys(&friend)
            .unwrap();
        client.send_event(&muted).await.unwrap();
        let result = fetch_user_discovery(
            client.clone(),
            relay_urls,
            owner.public_key(),
            UserDiscoveryCache::default(),
        )
        .await;
        client.disconnect().await;
        result.cache
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
        "follows=1 metadata_failed_chunks=0 social_failed_chunks=0"
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
