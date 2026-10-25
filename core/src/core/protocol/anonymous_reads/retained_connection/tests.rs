use super::*;
use crate::core::protocol::anonymous_reads::tls_tests as tls;
use serde_json::json;

mod wire_fixture;
use wire_fixture::{relay, Behavior};
mod idle_control_tests;

fn filters(author: PublicKey) -> Vec<Filter> {
    vec![Filter::new().author(author).kind(Kind::Metadata)]
}

fn new_connection(
    url: &RelayUrl,
    filters: &[Filter],
    slots: &Arc<Semaphore>,
) -> ScopedReadConnection {
    let mut connection = ScopedReadConnection::new(url, filters);
    connection.slots = slots.clone();
    connection
}

async fn read(
    connection: &mut ScopedReadConnection,
    tls: Option<&ScopedTlsConnector>,
) -> Result<Vec<Event>, String> {
    let relay = connection.relay.clone();
    let filters = connection.filters.clone();
    background_read(&relay, &filters, tls, None, Some(connection))
        .await
        .map_err(ReadFailure::message)
}

async fn idle_for(connection: &mut ScopedReadConnection, duration: Duration) {
    assert!(
        tokio::time::timeout(duration, idle_connections(std::slice::from_mut(connection)))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn repeated_tls_queries_reuse_only_the_exact_author_scope() {
    tokio::time::timeout(Duration::from_secs(8), async {
        let alice = Keys::generate();
        let bob = Keys::generate();
        let events = [&alice, &bob].map(|keys| {
            EventBuilder::new(Kind::Metadata, "profile")
                .sign_with_keys(keys)
                .unwrap()
        });
        let relay = tls::persistent_relay(events.to_vec()).await;
        let slots = Arc::new(Semaphore::new(2));
        let a = filters(alice.public_key());
        let b = filters(bob.public_key());
        let a_tls = ScopedTlsConnector::new_with_roots(&relay.url, &a, tls::roots()).unwrap();
        let b_tls = ScopedTlsConnector::new_with_roots(&relay.url, &b, tls::roots()).unwrap();
        let mut first = new_connection(&relay.url, &a, &slots);
        let mut second = new_connection(&relay.url, &b, &slots);
        let mut cache = VerificationCache::default();
        for _ in 0..8 {
            assert_eq!(
                poll_retained(
                    &relay.url,
                    a.clone(),
                    Some(&a_tls),
                    cache.relay(0),
                    Some(&mut first)
                )
                .await
                .unwrap(),
                vec![events[0].clone()]
            );
            assert_eq!(
                read(&mut second, Some(&b_tls)).await.unwrap(),
                vec![events[1].clone()]
            );
        }
        assert_eq!(
            cache
                .signature_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            relay.handshakes.lock().unwrap().len(),
            2,
            "sixteen queries need only two TLS handshakes"
        );
        tls::assert_wire(&relay, &[alice.public_key(), bob.public_key()], 2);
        assert_eq!(slots.available_permits(), 0);

        drop(first);
        assert_eq!(slots.available_permits(), 1);
        let mut restarted = new_connection(&relay.url, &a, &slots);
        let fresh_tls = ScopedTlsConnector::new_with_roots(&relay.url, &a, tls::roots()).unwrap();
        assert_eq!(
            read(&mut restarted, Some(&fresh_tls)).await.unwrap(),
            vec![events[0].clone()]
        );
        assert_eq!(relay.handshakes.lock().unwrap().len(), 3);
        tls::assert_wire(&relay, &[alice.public_key(), bob.public_key()], 3);
        drop((restarted, second));
        assert_eq!(slots.available_permits(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn scope_changes_dispose_the_connection_before_any_new_request() {
    let author = Keys::generate().public_key();
    let relay = relay(Behavior::Keep).await;
    let original = filters(author);
    let slots = Arc::new(Semaphore::new(1));
    for (url, changed) in [
        (relay.url.clone(), filters(Keys::generate().public_key())),
        (
            relay.url.clone(),
            vec![Filter::new().author(author).kind(Kind::Metadata).limit(1)],
        ),
        (
            RelayUrl::parse(&format!(
                "{}/other",
                relay.url.as_str().trim_end_matches('/')
            ))
            .unwrap(),
            original.clone(),
        ),
        (
            relay.url.clone(),
            vec![Filter::new()
                .author(author)
                .kind(Kind::Custom(MESSAGE_EVENT_KIND as u16))],
        ),
    ] {
        let mut connection = new_connection(&relay.url, &original, &slots);
        read(&mut connection, None).await.unwrap();
        let before = relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, value)| value[0] == "REQ")
            .count();
        assert!(connection
            .read(&url, &changed, &mut Collection::new(false), None, None)
            .await
            .is_err());
        assert!(connection.session.is_none());
        assert_eq!(slots.available_permits(), 1);
        let after = relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, value)| value[0] == "REQ")
            .count();
        assert_eq!(before, after, "scope changes must not reach the old socket");
    }
}

#[tokio::test]
async fn full_retention_budget_keeps_queries_working_without_holding_extra_sockets() {
    let relay = relay(Behavior::Keep).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut first = new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    let mut second = new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut first, None).await.unwrap();
    for _ in 0..2 {
        read(&mut second, None).await.unwrap();
        assert!(second.session.is_none());
        assert_eq!(slots.available_permits(), 0);
    }
    assert_eq!(relay.auth_keys().len(), 3);
    drop(first);
    read(&mut second, None).await.unwrap();
    assert!(second.session.is_some());
    drop(second);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test]
async fn idle_ping_is_answered_without_reconnecting_or_opening_a_subscription() {
    let relay = relay(Behavior::Ping).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut connection =
        new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut connection, None).await.unwrap();
    tokio::select! {
        _ = idle_connections(std::slice::from_mut(&mut connection)) => unreachable!(),
        result = tokio::time::timeout(Duration::from_secs(2), relay.pong.notified()) => result.unwrap(),
    }
    assert!(connection.session.is_some());
    read(&mut connection, None).await.unwrap();
    assert_eq!(relay.auth_keys().len(), 1);
}

#[tokio::test]
async fn closed_idle_socket_recovers_once_with_fresh_anonymous_auth() {
    let relay = relay(Behavior::Close).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut connection =
        new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    for _ in 0..3 {
        read(&mut connection, None).await.unwrap();
    }
    assert_eq!(
        relay.auth_keys().len(),
        3,
        "stale-connection recovery must create fresh AUTH keys"
    );
}

#[tokio::test]
async fn auth_failure_is_not_retried_and_idle_flood_does_not_wake_reconnection() {
    let relay = relay(Behavior::Reauthenticate).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut connection =
        new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut connection, None).await.unwrap();
    assert!(read(&mut connection, None)
        .await
        .unwrap_err()
        .contains("repeated anonymous authentication"));
    assert!(connection.session.is_none());
    assert_eq!(relay.auth_keys().len(), 1);

    let flood = wire_fixture::relay(Behavior::Flood).await;
    let mut connection =
        new_connection(&flood.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut connection, None).await.unwrap();
    idle_for(&mut connection, Duration::from_millis(100)).await;
    assert!(connection.session.is_none());
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(flood.auth_keys().len(), 1);
}

#[tokio::test]
async fn expiry_and_cancelled_query_release_retention_without_losing_valid_partial_results() {
    let relay = relay(Behavior::Keep).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut connection =
        new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut connection, None).await.unwrap();
    connection.session.as_mut().unwrap().opened = tokio::time::Instant::now() - MAX_CONNECTION_AGE;
    idle_for(&mut connection, Duration::from_millis(5)).await;
    assert_eq!(slots.available_permits(), 1);
    read(&mut connection, None).await.unwrap();
    assert_eq!(relay.auth_keys().len(), 2);
    drop(connection);

    let partial = wire_fixture::relay(Behavior::NoEose).await;
    let filters = filters(partial.author);
    let mut connection = new_connection(&partial.url, &filters, &slots);
    let result = read_one_with_connector(
        &partial.url,
        &filters,
        Duration::from_millis(150),
        None,
        None,
        Some(&mut connection),
    )
    .await;
    assert!(matches!(result, Ok(ref events) if events.len() == 1));
    assert!(
        connection.session.is_none(),
        "a timeout must discard the incomplete transaction"
    );
    assert_eq!(slots.available_permits(), 1);
}
