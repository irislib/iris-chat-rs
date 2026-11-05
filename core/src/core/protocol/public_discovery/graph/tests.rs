use super::test_support::*;
use super::*;
use nostr::Tag;
use std::sync::atomic::Ordering as AtomicOrdering;

#[tokio::test]
async fn complete_graph_flow_uses_shared_anonymous_auth_and_real_negentropy_without_uploads() {
    let owner = Keys::generate();
    let friend = Keys::generate();
    let device = Keys::generate();
    let root = EventBuilder::new(Kind::ContactList, "")
        .tags([Tag::public_key(friend.public_key())])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&owner)
        .unwrap();
    let follow = event(&friend, Kind::ContactList, "");
    let mute = event(&friend, Kind::MuteList, "");
    let fixture = fixture(vec![
        root,
        follow.clone(),
        mute.clone(),
        event(&friend, Kind::Metadata, "{}"),
    ])
    .await;
    let pool = Arc::new(DiscoveryRelays::default());
    let keeper = pool.open(std::slice::from_ref(&fixture.url)).await.unwrap();
    let result = crate::core::user_discovery::fetch_user_discovery(
        pool.clone(),
        vec![fixture.url.clone()],
        owner.public_key(),
        UserDiscoveryCache::default(),
    )
    .await;
    assert_eq!(result.cache.users.len(), 1);
    assert!(result.cache.social_graph.is_some());
    assert_eq!(result.metadata_events.len(), 1);
    let scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    let reqs = count(&fixture, "REQ");
    for head in [&follow, &mute] {
        let same = scope
            .opinions(
                head.kind,
                &[friend.public_key()],
                std::slice::from_ref(head),
                2,
            )
            .await
            .unwrap();
        assert!(same.complete);
        assert_eq!(same.events, vec![head.clone()]);
    }
    assert_eq!(
        count(&fixture, "REQ"),
        reqs,
        "unchanged NIP-77 inventories download no events"
    );
    until(|| {
        count(&fixture, "AUTH") > 0 && count(&fixture, "NEG-CLOSE") == count(&fixture, "NEG-OPEN")
    })
    .await;
    let frames = fixture.wire.lock().unwrap().clone();
    let kinds = frames
        .iter()
        .filter(|frame| frame[0] == "NEG-OPEN")
        .flat_map(|frame| {
            frame[2]["kinds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|kind| kind.as_u64().unwrap())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(kinds, BTreeSet::from([3, 10000]));
    for frame in &frames {
        assert_ne!(frame[0], "EVENT", "download-only graph scope cannot upload");
        if frame[0] == "AUTH" {
            let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
            assert_ne!(auth.pubkey, owner.public_key());
            assert_ne!(auth.pubkey, device.public_key());
            auth.verify().unwrap();
        }
        if frame[0] == "NEG-OPEN" {
            assert!(frame[2]["authors"]
                .as_array()
                .unwrap()
                .iter()
                .all(
                    |author| author == &serde_json::json!(friend.public_key().to_hex())
                        || author == &serde_json::json!(owner.public_key().to_hex())
                ));
        }
    }
    assert_eq!(fixture.connections.load(AtomicOrdering::SeqCst), 1);
    assert_closed(&fixture, "NEG-OPEN", "NEG-CLOSE");
    until(|| {
        let frames = fixture.wire.lock().unwrap();
        frames.iter().filter(|f| f[0] == "CLOSE").count()
            == frames.iter().filter(|f| f[0] == "REQ").count()
    })
    .await;
    assert_closed(&fixture, "REQ", "CLOSE");
    drop(scope);
    drop(keeper);
}

#[tokio::test]
async fn graph_cancellation_closes_negotiation_and_request_without_dropping_shared_connection() {
    let fixture = fixture(Vec::new()).await;
    let pool = Arc::new(DiscoveryRelays::default());
    let keeper = pool.open(std::slice::from_ref(&fixture.url)).await.unwrap();
    let scope = Arc::new(
        pool.graph_scope(std::slice::from_ref(&fixture.url))
            .await
            .unwrap(),
    );
    let author = Keys::generate().public_key();
    fixture.mode.store(2, AtomicOrdering::SeqCst);
    let running = {
        let scope = scope.clone();
        tokio::spawn(async move { scope.opinions(Kind::ContactList, &[author], &[], 2).await })
    };
    until(|| count(&fixture, "NEG-OPEN") > 0).await;
    running.abort();
    let _ = running.await;
    until(|| count(&fixture, "NEG-CLOSE") == 1).await;
    fixture.mode.store(1, AtomicOrdering::SeqCst);
    fixture.hold_eose.store(true, Ordering::Release);
    let running = {
        let scope = scope.clone();
        tokio::spawn(async move { scope.opinions(Kind::MuteList, &[author], &[], 2).await })
    };
    until(|| count(&fixture, "REQ") > 0).await;
    running.abort();
    let _ = running.await;
    until(|| count(&fixture, "CLOSE") > 0).await;
    assert_closed(&fixture, "NEG-OPEN", "NEG-CLOSE");
    assert_closed(&fixture, "REQ", "CLOSE");
    fixture.hold_eose.store(false, Ordering::Release);
    assert!(
        scope
            .opinions(Kind::MuteList, &[author], &[], 2)
            .await
            .unwrap()
            .complete
    );
    assert_eq!(fixture.connections.load(AtomicOrdering::SeqCst), 1);
    drop(scope);
    drop(keeper);
}

#[tokio::test]
async fn graph_budget_counts_frames_across_follow_and_mute_queries_and_closes_on_overflow() {
    let author = Keys::generate();
    let fixture = fixture(vec![
        event(&author, Kind::ContactList, &"x".repeat(3000)),
        event(&author, Kind::MuteList, &"y".repeat(3000)),
    ])
    .await;
    let pool = Arc::new(DiscoveryRelays::default());
    let mut scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    until(|| count(&fixture, "AUTH") > 0).await;
    scope.budget.max_bytes = 5000;
    let first = scope
        .fetch(
            vec![Filter::new()
                .kind(Kind::ContactList)
                .author(author.public_key())],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    let second = scope
        .fetch(
            vec![Filter::new()
                .kind(Kind::MuteList)
                .author(author.public_key())],
            Duration::from_secs(2),
        )
        .await;
    assert!(second.unwrap_err().contains("safety bound"));
    until(|| count(&fixture, "CLOSE") == 2).await;
    assert_closed(&fixture, "REQ", "CLOSE");
}

#[tokio::test]
async fn graph_scope_covers_every_configured_relay_and_reports_incomplete_coverage() {
    let fixtures = [
        fixture(Vec::new()).await,
        fixture(Vec::new()).await,
        fixture(Vec::new()).await,
    ];
    let pool = Arc::new(DiscoveryRelays::default());
    let mut urls = fixtures
        .iter()
        .map(|fixture| fixture.url.clone())
        .collect::<Vec<_>>();
    urls.reverse();
    urls.push(urls[0].clone());
    let scope = pool.graph_scope(&urls).await.unwrap();
    assert!(scope.lease.urls.windows(2).all(|pair| pair[0] < pair[1]));
    let author = Keys::generate().public_key();
    assert!(
        scope
            .opinions(Kind::ContactList, &[author], &[], 2)
            .await
            .unwrap()
            .complete
    );
    for fixture in &fixtures {
        assert_eq!(count(fixture, "NEG-OPEN"), 1);
    }
    fixtures[1].mode.store(1, AtomicOrdering::SeqCst);
    fixtures[1].hold_eose.store(true, Ordering::Release);
    assert!(
        !scope
            .opinions(Kind::MuteList, &[author], &[], 2)
            .await
            .unwrap()
            .complete
    );
}

#[tokio::test]
async fn graph_deadline_includes_shared_query_admission() {
    let fixture = fixture(Vec::new()).await;
    let pool = Arc::new(DiscoveryRelays::default());
    let mut scope = pool
        .graph_scope(std::slice::from_ref(&fixture.url))
        .await
        .unwrap();
    scope.budget.deadline = tokio::time::Instant::now() + Duration::from_millis(50);
    let _held = pool.queries.acquire().await.unwrap();
    let result = scope
        .opinions(Kind::ContactList, &[Keys::generate().public_key()], &[], 2)
        .await;
    assert!(result.unwrap_err().contains("deadline"));
    assert_eq!(count(&fixture, "NEG-OPEN"), 0);
}
