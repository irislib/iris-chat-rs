use super::test_support::*;
use super::*;
use nostr::Tag;
use std::sync::atomic::Ordering as AtomicOrdering;

#[tokio::test]
async fn cancelling_graph_setup_before_handshake_queues_no_filters() {
    let fixture = fixture(Vec::new()).await;
    fixture.hold_handshake.store(true, Ordering::Release);
    let pool = Arc::new(DiscoveryRelays::default());
    let keeper = pool.open(std::slice::from_ref(&fixture.url)).await.unwrap();
    let scope = Arc::new(
        pool.graph_scope(std::slice::from_ref(&fixture.url))
            .await
            .unwrap(),
    );
    let running = {
        let scope = scope.clone();
        tokio::spawn(async move {
            scope
                .fetch(
                    vec![Filter::new()
                        .kind(Kind::ContactList)
                        .author(Keys::generate().public_key())],
                    Duration::from_secs(3),
                )
                .await
        })
    };
    until(|| {
        fixture.connections.load(AtomicOrdering::SeqCst) == 1
            && pool.queries.available_permits() == 0
    })
    .await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    fixture.hold_handshake.store(false, Ordering::Release);
    fixture.release_handshake.notify_one();
    until(|| count(&fixture, "AUTH") > 0).await;
    let relay = pool.client.relay(fixture.url.clone()).await.unwrap();
    assert!(relay.subscriptions().await.is_empty());
    assert_eq!(count(&fixture, "REQ"), 0);
    assert_eq!(count(&fixture, "NEG-OPEN"), 0);
    assert_eq!(pool.queries.available_permits(), 1);
    drop(scope);
    drop(keeper);
}

#[tokio::test]
async fn auth_rejection_preserves_cached_graph_and_never_uses_account_or_device_identity() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let cached = event(&owner, Kind::ContactList, "cached");
    let fixture = fixture(Vec::new()).await;
    fixture.reject_auth.store(true, Ordering::Release);
    let pool = Arc::new(DiscoveryRelays::default());
    let scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    let result = scope
        .opinions(
            Kind::ContactList,
            &[owner.public_key()],
            std::slice::from_ref(&cached),
            2,
        )
        .await
        .unwrap();
    assert!(!result.complete);
    assert_eq!(result.events, vec![cached]);
    until(|| count(&fixture, "AUTH") > 0).await;
    let frames = fixture.wire.lock().unwrap();
    for frame in frames.iter() {
        assert_ne!(frame[0], "EVENT");
        if frame[0] == "AUTH" {
            let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
            assert_ne!(auth.pubkey, owner.public_key());
            assert_ne!(auth.pubkey, device.public_key());
        }
    }
}

#[tokio::test]
async fn graph_reconnect_reuses_anonymous_pool_and_keeps_raw_counter_monotonic() {
    let author = Keys::generate();
    let device = Keys::generate();
    let cached = event(&author, Kind::ContactList, "cached");
    let current = EventBuilder::new(Kind::ContactList, "current")
        .custom_created_at(Timestamp::from(101))
        .sign_with_keys(&author)
        .unwrap();
    let fixture = fixture(vec![current.clone()]).await;
    fixture.disconnect_once.store(true, Ordering::Release);
    let pool = Arc::new(DiscoveryRelays::default());
    let keeper = pool.open(std::slice::from_ref(&fixture.url)).await.unwrap();
    let scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    let counter = pool.wire.counter(&fixture.url).unwrap();
    let first = scope
        .opinions(
            Kind::ContactList,
            &[author.public_key()],
            std::slice::from_ref(&cached),
            2,
        )
        .await
        .unwrap();
    assert!(first.events.iter().any(|event| event.id == cached.id));
    let before = counter.load(Ordering::Relaxed);
    tokio::time::timeout(Duration::from_secs(20), async {
        while fixture.connections.load(AtomicOrdering::SeqCst) < 2 || count(&fixture, "AUTH") < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let again = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    let second = again
        .opinions(
            Kind::ContactList,
            &[author.public_key()],
            std::slice::from_ref(&cached),
            2,
        )
        .await
        .unwrap();
    assert!(second.complete);
    assert!(second.events.iter().any(|event| event.id == current.id));
    assert!(Arc::ptr_eq(
        &counter,
        &pool.wire.counter(&fixture.url).unwrap()
    ));
    assert!(counter.load(Ordering::Relaxed) > before);
    let frames = fixture.wire.lock().unwrap().clone();
    let identities = frames
        .iter()
        .filter(|frame| frame[0] == "AUTH")
        .map(|frame| {
            let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
            auth.pubkey
        })
        .collect::<HashSet<_>>();
    assert_eq!(identities.len(), 1);
    assert!(
        !identities.contains(&author.public_key()) && !identities.contains(&device.public_key())
    );
    assert!(frames.iter().all(|frame| frame[0] != "EVENT"));
    drop(scope);
    drop(again);
    drop(keeper);
}

#[test]
fn logout_and_relay_changes_abort_the_real_graph_refresh() {
    for logout in [false, true] {
        let directory = tempfile::TempDir::new().unwrap();
        let owner = Keys::generate();
        let friend = Keys::generate();
        let device = Keys::generate();
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            directory.path().to_string_lossy().to_string(),
            Arc::new(std::sync::RwLock::new(AppState::empty())),
        );
        let root = EventBuilder::new(Kind::ContactList, "")
            .tags([Tag::public_key(friend.public_key())])
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&owner)
            .unwrap();
        let fixture = core.runtime.block_on(fixture(vec![root]));
        fixture.mode.store(2, AtomicOrdering::SeqCst);
        core.preferences.nostr_relay_urls = vec![fixture.url.to_string()];
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: vec![fixture.url.clone()],
            authorization_state: LocalAuthorizationState::Authorized,
        });
        let keeper = core
            .runtime
            .block_on(
                core.discovery_relays
                    .open(std::slice::from_ref(&fixture.url)),
            )
            .unwrap();
        core.request_user_discovery_refresh(true);
        let abort = core
            .user_discovery_runtime
            .fetch_abort
            .as_ref()
            .unwrap()
            .clone();
        core.runtime
            .block_on(until(|| count(&fixture, "NEG-OPEN") > 0));
        let token = core.user_discovery_runtime.token;
        if logout {
            core.logout();
        } else {
            core.set_nostr_relays(&[]);
        }
        core.runtime.block_on(async {
            until(|| abort.is_finished()).await;
            if logout {
                until(|| fixture.active.load(AtomicOrdering::SeqCst) == 0).await;
            } else {
                until(|| count(&fixture, "NEG-CLOSE") == count(&fixture, "NEG-OPEN")).await;
                assert_closed(&fixture, "NEG-OPEN", "NEG-CLOSE");
            }
            drop(keeper);
        });
        assert!(!core.user_discovery_runtime.in_flight);
        assert!(core.user_discovery_runtime.token > token);
        let frames = fixture.wire.lock().unwrap();
        assert!(frames.iter().all(|frame| frame[0] != "EVENT"));
        for frame in frames.iter().filter(|frame| frame[0] == "AUTH") {
            let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
            assert_ne!(auth.pubkey, owner.public_key());
            assert_ne!(auth.pubkey, device.public_key());
        }
    }
}
