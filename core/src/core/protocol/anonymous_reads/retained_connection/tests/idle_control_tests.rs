use super::*;
use wire_fixture::IdleControl;

async fn wait_idle_pong(connection: &mut ScopedReadConnection, relay: &wire_fixture::Relay) {
    tokio::select! {
        _ = idle_connections(std::slice::from_mut(connection)) => unreachable!(),
        result = tokio::time::timeout(Duration::from_secs(2), relay.pong.notified()) => {
            result.expect("inert relay control must not discard the scoped socket");
        },
    }
    assert!(connection.session.is_some());
}

#[tokio::test]
async fn public_read_eose_after_auth_ok_does_not_force_another_connection() {
    let relay = relay(Behavior::PublicBeforeAuth).await;
    let slots = Arc::new(Semaphore::new(1));
    let author = Keys::generate().public_key();
    let mut connection = new_connection(&relay.url, &filters(author), &slots);
    for _ in 0..3 {
        assert!(read(&mut connection, None).await.unwrap().is_empty());
        wait_idle_pong(&mut connection, &relay).await;
    }
    let keys = relay.auth_keys();
    assert_eq!(
        keys.len(),
        1,
        "idle EOSE must not force fresh anonymous AUTH"
    );
    assert!(!keys.contains(&author));
    let rows = relay.wire.lock().unwrap();
    let requests = rows
        .iter()
        .filter(|(_, value)| value[0] == "REQ")
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        4,
        "initial public REQ, AUTH retry, two polls"
    );
    assert_eq!(requests[0].1[1], requests[1].1[1]);
    assert_eq!(
        rows.iter().filter(|(_, value)| value[0] == "CLOSE").count(),
        3
    );
    assert!(rows.iter().all(|(id, _)| *id == 1));
}

#[tokio::test]
async fn inert_idle_controls_retain_only_the_existing_anonymous_author_scope() {
    for control in [
        IdleControl::Notice,
        IdleControl::Closed,
        IdleControl::Eose,
        IdleControl::AcceptedAuth,
    ] {
        let relay = relay(Behavior::IdleControl(control)).await;
        let slots = Arc::new(Semaphore::new(2));
        let alice = Keys::generate().public_key();
        let bob = Keys::generate().public_key();
        let mut first = new_connection(&relay.url, &filters(alice), &slots);
        let mut second = new_connection(&relay.url, &filters(bob), &slots);
        for _ in 0..2 {
            for connection in [&mut first, &mut second] {
                read(connection, None).await.unwrap();
                wait_idle_pong(connection, &relay).await;
            }
        }
        let auth = relay.auth_keys();
        assert_eq!(
            auth.len(),
            2,
            "each author keeps its own authenticated socket"
        );
        assert!(!auth.contains(&alice) && !auth.contains(&bob));
        assert_eq!(slots.available_permits(), 0);
    }
}

#[tokio::test]
async fn foreign_rejected_and_malformed_idle_controls_still_dispose_without_retry() {
    for control in [
        IdleControl::ForeignClosed,
        IdleControl::ForeignEose,
        IdleControl::ForeignAuth,
        IdleControl::RejectedAuth,
        IdleControl::Challenge,
        IdleControl::Event,
        IdleControl::Malformed,
    ] {
        let relay = relay(Behavior::IdleControl(control)).await;
        let slots = Arc::new(Semaphore::new(1));
        let mut connection =
            new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
        read(&mut connection, None).await.unwrap();
        idle_for(&mut connection, Duration::from_millis(100)).await;
        assert!(connection.session.is_none());
        assert_eq!(slots.available_permits(), 1);
        assert_eq!(
            relay.auth_keys().len(),
            1,
            "idle controls cannot trigger a retry"
        );
        read(&mut connection, None).await.unwrap();
        assert_eq!(
            relay.auth_keys().len(),
            2,
            "next scheduled read uses fresh AUTH"
        );
    }
}

#[tokio::test]
async fn inert_text_flood_remains_bounded_without_waking_reconnection() {
    let relay = relay(Behavior::TextFlood).await;
    let slots = Arc::new(Semaphore::new(1));
    let mut connection =
        new_connection(&relay.url, &filters(Keys::generate().public_key()), &slots);
    read(&mut connection, None).await.unwrap();
    idle_for(&mut connection, Duration::from_millis(100)).await;
    assert!(connection.session.is_none());
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(relay.auth_keys().len(), 1);
}
