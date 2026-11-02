use super::*;

#[tokio::test]
async fn populated_account_idle_discovery_reuses_tls_connections() {
    const TEST: &str = "core::protocol::anonymous_reads::retained_connection::tests::populated::populated_account_idle_discovery_reuses_tls_connections";
    if crate::core::protocol::anonymous_reads::tests::capacity_experiment_with_resource_limits(
        TEST,
        Duration::from_secs(90),
    ) {
        return;
    }
    tokio::time::timeout(Duration::from_secs(80), async {
        exercise(120, 3, 0).await;
        // Exercise the full bounded pool with TLS on both ends, rather than
        // inferring TLS memory use from the existing plain-WebSocket fixture.
        // The isolated Linux child has a 768 MiB address-space limit; every
        // Unix child has 2048 descriptors including the local server ends.
        exercise(128, 4, 0).await;
        if cfg!(target_os = "linux") {
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            let peak = status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .unwrap();
            println!("POPULATED_TLS_MEMORY {peak}");
        }
    })
    .await
    .expect("populated TLS discovery deadline");
}

#[test]
fn retention_budget_leaves_descriptor_headroom() {
    for (limit, expected) in [
        (0, 0),
        (3, 0),
        (256, 64),
        (1024, 256),
        (2048, 512),
        (usize::MAX, 512),
    ] {
        assert_eq!(capacity_for_descriptor_limit(Some(limit)), expected);
    }
    assert_eq!(capacity_for_descriptor_limit(None), 64);
}

#[tokio::test]
async fn low_descriptor_limit_preserves_bounded_discovery_progress() {
    const TEST: &str = "core::protocol::anonymous_reads::retained_connection::tests::populated::low_descriptor_limit_preserves_bounded_discovery_progress";
    if crate::core::protocol::anonymous_reads::tests::capacity_experiment_with_resource_limits(
        TEST,
        Duration::from_secs(30),
    ) {
        return;
    }
    // Change only this isolated test child. A restricted app must continue to
    // make progress without relying on a platform limit increase succeeding.
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: a valid mutable rlimit is supplied to each synchronous libc call.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    limit.rlim_cur = 256;
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    assert_eq!(retention_capacity(), 64);
    tokio::time::timeout(Duration::from_secs(20), exercise(120, 1, 56))
        .await
        .unwrap();
}

async fn exercise(author_count: usize, relay_count: usize, reconnects_per_sweep: usize) {
    // Match the real account's discovery population and healthy servers.
    // Each author/server pair still has its own TLS and anonymous AUTH.
    let authors = (0..author_count)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    let mut relays = Vec::new();
    for _ in 0..relay_count {
        relays.push(tls::persistent_relay(vec![]).await);
    }
    let mut connections = Vec::new();
    for author in &authors {
        for relay in &relays {
            let filters = filters(*author);
            connections.push((
                ScopedReadConnection::new(&relay.url, &filters),
                ScopedTlsConnector::new_with_roots(&relay.url, &filters, tls::roots()).unwrap(),
            ));
        }
    }
    let slots = connections[0].0.slots.clone();
    let capacity = slots.available_permits();
    let mut opens = Vec::new();
    for sweep in 0..3 {
        // Exercise the real background admission and reader, including
        // the shared production retention budget, without timer sleeps.
        let results = futures_util::future::join_all(
            connections
                .iter_mut()
                .map(|(connection, tls)| async move { read(connection, Some(tls)).await }),
        )
        .await;
        assert!(results
            .iter()
            .all(|result| matches!(result, Ok(events) if events.is_empty())));
        opens.push(
            relays
                .iter()
                .map(|relay| relay.handshakes.lock().unwrap().len())
                .sum::<usize>(),
        );
        println!(
            "POPULATED_TLS sweep={sweep} scopes={} capacity={capacity} handshakes={} retained={}",
            connections.len(),
            opens[sweep],
            capacity - slots.available_permits(),
        );
        let snapshot = diagnostics::snapshot();
        assert_eq!(snapshot.retained_capacity, capacity as u64);
        assert_eq!(
            snapshot.retained_active,
            (capacity - slots.available_permits()) as u64
        );
    }
    let mut all_auth_keys = HashSet::new();
    for relay in &relays {
        tls::assert_wire(relay, &authors, relay.handshakes.lock().unwrap().len());
        for (_, value) in relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, value)| value[0] == "AUTH")
        {
            let event: Event = serde_json::from_value(value[1].clone()).unwrap();
            assert!(
                all_auth_keys.insert(event.pubkey),
                "separate servers must never share anonymous AUTH"
            );
        }
    }
    drop(connections);
    assert_eq!(
        slots.available_permits(),
        capacity,
        "dropping scopes releases all retention permits"
    );
    assert_eq!(diagnostics::snapshot().retained_active, 0);
    assert_eq!(opens[0], author_count * relay_count);
    assert_eq!(
        opens[1],
        opens[0] + reconnects_per_sweep,
        "idle discovery must not repeat TLS and AUTH for unchanged scopes"
    );
    assert_eq!(opens[2], opens[0] + 2 * reconnects_per_sweep);
}
