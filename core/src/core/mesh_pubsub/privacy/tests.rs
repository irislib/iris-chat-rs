use super::*;
use crate::core::protocol::public_discovery::{test_support, PublicDiscovery};
use std::sync::atomic::Ordering;
type DiscoveryWire = Arc<std::sync::Mutex<Vec<(usize, serde_json::Value)>>>;

#[test]
fn mesh_advert_quiet_delay_respects_expiry_without_expired_spin() {
    let keys = Keys::generate();
    let now = unix_now().get();
    let event = EventBuilder::new(Kind::Custom(37195), "signed fixture")
        .custom_created_at(Timestamp::from(now))
        .tags([nostr::Tag::expiration(Timestamp::from(now + 75))])
        .sign_with_keys(&keys)
        .unwrap();
    let mut observations = DiscoveryObservations::default();
    assert!(observations.remember(&event, 3_600));
    assert!(!observations.remember(&event, 3_600));
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now),
        Duration::from_secs(75)
    );
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now + 70),
        Duration::from_secs(5)
    );
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now + 75),
        Duration::from_secs(120)
    );
    assert_eq!(observations.advert_valid_until, None);
    assert_eq!(
        observations.delay(Duration::from_secs(120), false, now),
        Duration::from_secs(120)
    );
    assert_eq!(
        DiscoveryObservations::default().delay(Duration::from_secs(120), true, now),
        Duration::from_secs(120)
    );

    let old = EventBuilder::new(Kind::Custom(37195), "old signed fixture")
        .custom_created_at(Timestamp::from(now.saturating_sub(7_195)))
        .tags([nostr::Tag::expiration(Timestamp::from(now + 1_000))])
        .sign_with_keys(&keys)
        .unwrap();
    observations.remember(&old, 3_600);
    assert_eq!(
        observations.delay(Duration::from_secs(120), true, now),
        Duration::from_secs(5)
    );
}

#[test]
fn mesh_nonadvert_cadence_changes_only_on_new_event_ids_and_explicit_demand() {
    let keys = Keys::generate();
    let first = EventBuilder::new(Kind::Metadata, "first")
        .sign_with_keys(&keys)
        .unwrap();
    let late = EventBuilder::new(Kind::Metadata, "late arrival")
        .custom_created_at(Timestamp::from(
            first.created_at.as_secs().saturating_sub(10),
        ))
        .sign_with_keys(&keys)
        .unwrap();
    let mut observations = DiscoveryObservations::default();
    let mut cadence = DiscoveryCadence::default();
    assert_eq!(
        cadence.after_poll(observations.remember(&first, 3_600)),
        Duration::from_secs(30)
    );
    assert_eq!(
        cadence.after_poll(observations.remember(&first, 3_600)),
        Duration::from_secs(60)
    );
    assert_eq!(cadence.after_poll(false), Duration::from_secs(120));
    assert_eq!(cadence.after_poll(false), Duration::from_secs(120));
    assert_eq!(
        cadence.after_poll(observations.remember(&late, 3_600)),
        Duration::from_secs(30)
    );
    cadence.after_poll(false);
    cadence.reset();
    assert_eq!(cadence.after_poll(false), Duration::from_secs(30));
}

fn subject_queries(
    wire: &DiscoveryWire,
    author: PublicKey,
) -> std::collections::BTreeSet<(usize, String)> {
    let mut authenticated = HashSet::new();
    let mut requested = std::collections::BTreeSet::new();
    for (connection, value) in wire.lock().unwrap().iter() {
        if value[0] == "AUTH" {
            authenticated.insert(*connection);
        }
        if authenticated.contains(connection)
            && value[0] == "REQ"
            && value.as_array().unwrap().iter().skip(2).any(|filter| {
                filter["authors"]
                    .as_array()
                    .is_some_and(|authors| authors.contains(&serde_json::json!(author.to_hex())))
            })
        {
            requested.insert((*connection, value[1].as_str().unwrap().to_owned()));
        }
    }
    requested
}

async fn wait_subject(wire: &DiscoveryWire, author: PublicKey, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while subject_queries(wire, author).len() < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing anonymous subject query"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn linked_device_relay_discovery_and_protocol_share_one_connection_without_provider_admission() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let peer = Keys::generate();
        let sibling = Keys::generate();
        let relay = test_support::relay(vec![]).await;
        let pool = test_support::pool();
        let providers = Arc::new(
            PrivateMeshProviders::new(vec![], None, vec![], vec![relay.url.clone()])
                .with_relay_pool(pool.clone())
                .with_device_sync_authors([sibling.public_key()]),
        );
        let _ordinary = providers.discovery_slots.acquire_many(2).await.unwrap();
        let _reserved = providers.device_sync_slots.acquire().await.unwrap();
        let live = tls_tests::endpoint().await;
        let mut discovery = PrivateMeshDiscovery::default();
        discovery.update(
            providers.clone(),
            vec![Filter::new()
                .kind(Kind::Custom(37195))
                .authors([peer.public_key(), sibling.public_key()])],
            live.clone(),
            flume::unbounded().0,
            &runtime,
        );
        let (send, incoming) = flume::unbounded();
        let protocol = PublicDiscovery::new(runtime.handle(), send, pool);
        protocol
            .configure(
                std::slice::from_ref(&relay.url),
                vec![Filter::new().kind(Kind::Metadata).author(peer.public_key())],
            )
            .unwrap();
        wait_subject(&relay.wire, sibling.public_key(), 1).await;
        test_support::until(|| {
            relay
                .wire
                .lock()
                .unwrap()
                .iter()
                .any(|(_, row)| row[0] == "REQ" && row[2]["kinds"] == serde_json::json!([0]))
        })
        .await;
        assert!(
            discovery.workers.is_empty(),
            "no FIPS provider means no polling workers"
        );
        assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
        let profile = EventBuilder::new(Kind::Metadata, "profile")
            .sign_with_keys(&peer)
            .unwrap();
        relay.publish(profile);
        tokio::time::timeout(Duration::from_secs(3), incoming.recv_async())
            .await
            .unwrap()
            .unwrap();
        let before = relay.requests();
        sleep(Duration::from_millis(250)).await;
        assert_eq!(relay.requests(), before);
        discovery.refresh_author(sibling.public_key());
        wait_subject_closed(&relay.wire, sibling.public_key(), 2).await;
        assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
        drop(discovery);
        drop(protocol);
        test_support::until(|| relay.active.load(Ordering::SeqCst) == 0).await;
        live.shutdown().await.unwrap();
    });
}

#[test]
fn provider_discovery_still_splits_authors_and_preserves_reserved_sibling_admission() {
    let a = Keys::generate().public_key();
    let b = Keys::generate().public_key();
    let filters = split_provider_authors(Filter::new().kind(Kind::Custom(37195)).authors([a, b]));
    assert_eq!(filters.len(), 2);
    assert!(filters
        .iter()
        .all(|filter| FipsDiscoveryScope::from_filter(filter).is_ok()));
    let provider =
        PrivateMeshProviders::new(vec![], None, vec![], vec![]).with_device_sync_authors([a]);
    let _ordinary = provider.discovery_slots.try_acquire_many(2).unwrap();
    assert!(provider.device_sync_slots.try_acquire().is_ok());
    assert_eq!(provider.device_sync_authors, BTreeSet::from([a]));
}

fn inject_route_need(core: &mut AppCore, author: PublicKey, generation: u64, guarded: bool) {
    core.handle_message(CoreMsg::Internal(Box::new(
        InternalEvent::FipsPeerDiscoveryNeeded {
            generation,
            author,
            requires_pending_work: guarded,
        },
    )));
}

async fn wait_subject_closed(wire: &DiscoveryWire, author: PublicKey, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let connections = subject_queries(wire, author);
        let closed = wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(connection, value)| {
                value[0] == "CLOSE"
                    && connections.contains(&(*connection, value[1].as_str().unwrap().to_owned()))
            })
            .map(|(connection, value)| (*connection, value[1].as_str().unwrap().to_owned()))
            .collect::<HashSet<_>>();
        if closed.len() >= count.saturating_sub(1) && connections.len() >= count {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing completed query"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn mesh_route_retry_core_guard_requires_exact_private_work_on_actual_wire() {
    let directory = tempfile::TempDir::new().unwrap();
    let (sender, _messages) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        sender,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(crate::state::AppState::empty())),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let relay = test_support::relay(vec![]).await;
        let wire = relay.wire.clone();
        let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
        let [a, b] = peers;
        let a_npub = a.to_bech32().unwrap();
        let b_npub = b.to_bech32().unwrap();
        let live = tls_tests::endpoint().await;
        core.install_device_sync_endpoint_for_test(live.clone());
        core.fips_connection_generation = 7;
        let mesh = core.device_sync.as_mut().unwrap();
        let outbox = mesh.nearby_outbox.clone();
        mesh.protocol_subscriptions.discovery.update(
            Arc::new(
                PrivateMeshProviders::new(Vec::new(), None, Vec::new(), vec![relay.url.clone()])
                    .with_relay_pool(test_support::pool()),
            ),
            peers
                .map(|peer| Filter::new().kind(Kind::Custom(37195)).author(peer))
                .to_vec(),
            live.clone(),
            core.core_sender.clone(),
            &core.runtime,
        );
        wait_subject_closed(&wire, a, 1).await;
        wait_subject_closed(&wire, b, 1).await;

        for state in [
            "no work",
            "public",
            "empty targets",
            "other recipient",
            "acknowledged",
        ] {
            {
                let mut outbox = outbox.write().unwrap();
                *outbox = Default::default();
                match state {
                    "public" => outbox.insert_for_targets("public".into(), vec![1], None),
                    "empty targets" => {
                        outbox.insert_for_targets("empty".into(), vec![2], Some(Default::default()))
                    }
                    "other recipient" => outbox.insert_for_targets(
                        "other".into(),
                        vec![3],
                        Some([b_npub.clone()].into()),
                    ),
                    "acknowledged" => {
                        outbox.insert_for_targets(
                            "acked".into(),
                            vec![4],
                            Some([a_npub.clone()].into()),
                        );
                        assert!(outbox.acknowledge(&a_npub, "acked"));
                    }
                    _ => {}
                }
            }
            for _ in 0..128 {
                inject_route_need(&mut core, a, 7, true);
            }
            sleep(Duration::from_millis(100)).await;
            assert_eq!(subject_queries(&wire, a).len(), 1, "idle retry: {state}");
            assert_eq!(
                subject_queries(&wire, b).len(),
                1,
                "unrelated author: {state}"
            );
        }

        {
            let mut outbox = outbox.write().unwrap();
            *outbox = Default::default();
            outbox.insert_for_targets("pending".into(), vec![5], Some([a_npub.clone()].into()));
        }
        inject_route_need(&mut core, a, 6, true);
        inject_route_need(&mut core, a, 6, false);
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_queries(&wire, a).len(),
            1,
            "stale generation woke query"
        );
        inject_route_need(&mut core, a, 7, true);
        wait_subject_closed(&wire, a, 2).await;
        assert!(outbox.write().unwrap().acknowledge(&a_npub, "pending"));
        for _ in 0..128 {
            inject_route_need(&mut core, a, 7, true);
        }
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_queries(&wire, a).len(),
            2,
            "receipt failed to stop retry lookups"
        );
        inject_route_need(&mut core, a, 7, false);
        wait_subject_closed(&wire, a, 3).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            subject_queries(&wire, a).len(),
            3,
            "one link loss must request one lookup"
        );
        assert_eq!(subject_queries(&wire, b).len(), 1);

        let rows = wire.lock().unwrap().clone();
        assert_eq!(
            rows.iter().map(|(id, _)| id).collect::<HashSet<_>>().len(),
            1
        );
        for (_, frame) in &rows {
            assert_ne!(frame[0], "EVENT");
            if frame[0] == "REQ" {
                assert!(frame[2].get("#p").is_none());
                assert_eq!(frame[2]["kinds"], serde_json::json!([37195]));
            }
            if frame[0] == "AUTH" {
                let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
                auth.verify().unwrap();
                assert!(!peers.contains(&auth.pubkey));
                assert_ne!(auth.pubkey.to_bech32().unwrap(), live.npub());
            }
        }
        drop(core.device_sync.take());
        live.shutdown().await.unwrap();
    });
}

#[test]
fn pending_link_history_refreshes_only_the_approved_siblings_advert() {
    use crate::core::tests::{
        configure_test_device_sync_profile, logged_in_test_core_with_updates,
    };

    let (owner, local, sibling) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (mut core, _updates, _directory) =
        logged_in_test_core_with_updates("history-route", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let peer = sibling.public_key();
    let other = Keys::generate().public_key();
    // Deliver the burst in one event-loop turn, then let the real relay worker drain it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let relay = test_support::relay(vec![]).await;
        let live = tls_tests::endpoint().await;
        core.install_device_sync_endpoint_for_test(live.clone());
        core.fips_connection_generation = 7;
        core.device_sync
            .as_mut()
            .unwrap()
            .protocol_subscriptions
            .discovery
            .update(
                Arc::new(
                    PrivateMeshProviders::new(vec![], None, vec![], vec![relay.url.clone()])
                        .with_relay_pool(test_support::pool()),
                ),
                [peer, other]
                    .map(|author| Filter::new().kind(Kind::Custom(37195)).author(author))
                    .to_vec(),
                live.clone(),
                core.core_sender.clone(),
                &runtime,
            );
        wait_subject_closed(&relay.wire, peer, 1).await;
        wait_subject_closed(&relay.wire, other, 1).await;

        // As on a newly signed-in device, the approver is durable but its history policy
        // has not arrived. There are no ordinary envelopes or calls waiting for the peer.
        core.record_device_history_approver(&peer.to_hex(), 1, "pair".into())
            .unwrap();
        assert!(
            core.has_private_mesh_discovery_demand(peer),
            "pending link history must wake its approver"
        );
        assert!(!core.has_private_mesh_discovery_demand(other));
        let stored_advert = tls_tests::advert(
            &sibling,
            unix_now().get(),
            unix_now().get() + 3600,
            "peer.invalid:9000",
        );
        relay.events.lock().unwrap().push(stored_advert.clone());
        inject_route_need(&mut core, peer, 6, true);
        for _ in 0..128 {
            inject_route_need(&mut core, peer, 7, true);
            inject_route_need(&mut core, other, 7, true);
        }
        wait_subject_closed(&relay.wire, peer, 2).await;
        assert_eq!(subject_queries(&relay.wire, other).len(), 1);
        #[cfg(target_os = "linux")]
        assert!(relay
            .responses
            .lock()
            .unwrap()
            .iter()
            .any(|(_, _, id)| *id == Some(stored_advert.id)));
        core.handle_device_history_policy(&peer.to_hex(), 1, 1, "pair".into());
        assert!(
            !core.has_private_mesh_discovery_demand(peer),
            "negotiated no-history grant is complete"
        );

        core.create_device_history_transfer(peer, true, "outbound".into())
            .unwrap();
        assert!(
            core.has_private_mesh_discovery_demand(peer),
            "the approver also needs a route"
        );
        core.handle_device_history_complete(&peer.to_hex(), 100, "outbound".into());
        assert!(
            !core.has_private_mesh_discovery_demand(peer),
            "completed history must stay quiet"
        );

        core.create_device_history_transfer(peer, true, "revoked".into())
            .unwrap();
        core.app_keys
            .get_mut(&owner.public_key().to_hex())
            .unwrap()
            .devices
            .retain(|device| device.identity_pubkey_hex != local.public_key().to_hex());
        assert!(
            !core.has_private_mesh_discovery_demand(peer),
            "a revoked local device cannot request a peer's advert"
        );
        for _ in 0..128 {
            inject_route_need(&mut core, peer, 7, true);
        }
        sleep(Duration::from_millis(100)).await;
        assert_eq!(subject_queries(&relay.wire, peer).len(), 2);
        for (_, frame) in relay.wire.lock().unwrap().iter() {
            if frame[0] == "REQ" && frame[2].get("limit").is_some() {
                assert_eq!(frame[2]["authors"], serde_json::json!([peer]));
                assert_eq!(frame[2]["kinds"], serde_json::json!([37195]));
                assert!(frame[2].get("#p").is_none());
            }
            if frame[0] == "AUTH" {
                let auth: Event = serde_json::from_value(frame[1].clone()).unwrap();
                auth.verify().unwrap();
                assert_ne!(auth.pubkey.to_bech32().unwrap(), live.npub());
                assert!(
                    ![owner.public_key(), local.public_key(), peer, other].contains(&auth.pubkey)
                );
            }
        }
        assert_eq!(relay.connections.load(Ordering::SeqCst), 1);
        drop(core.device_sync.take());
        live.shutdown().await.unwrap();
    });
}
