#[test]
fn linked_device_reconciliation_preserves_anonymous_provider_reads_with_live_host_bridge() {
    check_provider_reader_reconciliation(false);
}

#[test]
fn linked_device_reconciliation_recovers_closed_anonymous_reader_without_restarting_live_endpoint()
{
    check_provider_reader_reconciliation(true);
}

fn check_provider_reader_reconciliation(close_reader: bool) {
    use fips_core::config::{TransportInstances, UdpConfig};
    use hashtree_core::{MemoryStore, Store, StoreBlobRoute};
    use hashtree_fips_transport::{TcpBlobTransport, TcpBlobTransportConfig};

    let owner = Keys::generate();
    let local = Keys::generate();
    let old_sibling = Keys::generate();
    let new_sibling = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("provider-link-refresh", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &old_sibling, None);
    let reservation = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(rendezvous) = reservation.local_addr().unwrap() else {
        unreachable!("IPv4 fixture")
    };
    drop(reservation);
    core.test_fips_rendezvous_addr = Some(rendezvous);
    let mut config = fips_core::Config::new();
    config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
    config.node.control.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.local.rendezvous_addr = rendezvous;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..Default::default()
    });
    let provider = Arc::new(
        core.runtime
            .block_on(
                fips_core::FipsEndpoint::builder()
                    .config(config)
                    .local_rendezvous()
                    .without_system_tun()
                    .bind(),
            )
            .unwrap(),
    );
    let provider_cache = Arc::new(MemoryStore::new());
    let before = b"provider block before linking".to_vec();
    let after = b"uncached provider block after linking".to_vec();
    let recovered = b"uncached provider block after reader-only failure".to_vec();
    let before_hash = hashtree_core::sha256(&before);
    let after_hash = hashtree_core::sha256(&after);
    let recovered_hash = hashtree_core::sha256(&recovered);
    core.runtime
        .block_on(provider_cache.put(before_hash, before.clone()))
        .unwrap();
    core.runtime
        .block_on(provider_cache.put(after_hash, after.clone()))
        .unwrap();
    core.runtime
        .block_on(provider_cache.put(recovered_hash, recovered.clone()))
        .unwrap();
    let requesters = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = requesters.clone();
    let serving = core
        .runtime
        .block_on(
            TcpBlobTransport::bind_advertised_route_with_config_and_policy(
                provider.clone(),
                provider_cache.clone(),
                Arc::new(StoreBlobRoute::new(provider_cache)),
                TcpBlobTransportConfig::default(),
                10,
                Arc::new(move |peer| {
                    captured.lock().unwrap().push(peer);
                    true
                }),
            ),
        )
        .unwrap();
    let fallback: Arc<dyn hashtree_core::BlobRoute> =
        Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new())));
    // These lifecycle cases discover the provider through the real local
    // directory, including its accepting listener hints. Explicit public-hint
    // reads have separate coverage; mixed-path roster readiness is not proven
    // by this fixture.
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback.clone(), vec![]);
    let (live, _, _, blobs) = core.same_host_runtime_for_test().unwrap();
    core.runtime.block_on(async {
        let readiness = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            while !live
                .local_instance_advertisements()
                .unwrap()
                .iter()
                .any(|advert| {
                    advert.npub == provider.npub()
                        && advert
                            .capability(hashtree_fips_transport::TCP_BLOB_CAPABILITY)
                            .is_some()
                })
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        if let Err(error) = readiness {
            let paths = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::join!(live.peers(), provider.peers())
            })
            .await;
            panic!(
                "local provider readiness failed: {error}; live/provider peer paths={paths:?}; live directory={:?}; provider directory={:?}",
                live.local_instance_advertisements(),
                provider.local_instance_advertisements(),
            );
        }
        assert!(live.routing_status().await.unwrap().client_only);
        assert_eq!(
            blobs.update_store().get(&before_hash).await.unwrap(),
            Some(before)
        );
    });
    let initial_reader = blobs.provider_read_endpoint_for_test();
    assert_eq!(
        requesters.lock().unwrap().last().unwrap().npub(),
        initial_reader.npub()
    );
    if close_reader {
        // Exercise a real failed admission barrier, not a mock recovery flag.
        // The live app endpoint, host bridge and cache remain operational.
        core.runtime.block_on(initial_reader.shutdown()).unwrap();
    }

    // Exercise the production in-place branch used by a single-use host BLE
    // bridge. This fixture needs no physical radio or bridge I/O to prove its
    // endpoint/reader lifecycle; the live FIPS endpoint must survive the update.
    core.host_ble_attached = true;
    configure_test_device_sync_profile(&mut core, &owner, &local, &new_sibling, None);
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback.clone(), vec![]);
    // Repeat ordinary reconciliation: matching keys must not leave a closed
    // reader permanently installed after the first policy transition.
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback, vec![]);
    let (same_live, _, sibling_count, refreshed) = core.same_host_runtime_for_test().unwrap();
    assert!(
        Arc::ptr_eq(&live, &same_live),
        "keep the live bridge and sync endpoint"
    );
    assert_eq!(sibling_count, 1);
    let current_reader = refreshed.provider_read_endpoint_for_test();
    if close_reader {
        assert!(!Arc::ptr_eq(&initial_reader, &current_reader));
        assert_ne!(initial_reader.npub(), current_reader.npub());
    } else {
        assert!(
            Arc::ptr_eq(&initial_reader, &current_reader),
            "the admission barrier preserves the healthy reader"
        );
    }
    core.runtime.block_on(async {
        assert!(!same_live.is_stopped().await);
        assert!(same_live.routing_status().await.unwrap().client_only);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            refreshed.update_store().get(&after_hash),
        )
        .await
        .unwrap();
        assert_eq!(
            result.unwrap(),
            Some(after),
            "production reconciliation must restore unrelated provider reads"
        );
    });
    let source = requesters.lock().unwrap().last().copied().unwrap();
    assert_eq!(source.npub(), current_reader.npub());
    assert_ne!(
        source.npub(),
        same_live.npub(),
        "recovery must stay anonymous"
    );
    if close_reader {
        // A second real endpoint failure has no roster change to trigger a
        // reconciliation. An ordinary request must repair only its reader.
        core.runtime.block_on(current_reader.shutdown()).unwrap();
        core.runtime.block_on(async {
            assert_eq!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(12),
                    refreshed.update_store().get(&recovered_hash),
                )
                .await
                .unwrap()
                .unwrap(),
                Some(recovered),
            );
        });
        let request_reader = refreshed.provider_read_endpoint_for_test();
        assert_ne!(current_reader.npub(), request_reader.npub());
        assert_eq!(
            requesters.lock().unwrap().last().unwrap().npub(),
            request_reader.npub(),
        );
        let (retained_live, _, _, _) = core.same_host_runtime_for_test().unwrap();
        assert!(Arc::ptr_eq(&same_live, &retained_live));
        core.runtime.block_on(async {
            assert!(!retained_live.is_stopped().await);
            // The provider knows the hashes it served. Neither the identified
            // cache nor the replacement client may answer its active probes.
            for target in [retained_live.npub(), request_reader.npub()] {
                assert!(tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    serving.fetch_from_peer(
                        &recovered_hash,
                        fips_core::PeerIdentity::from_npub(target).unwrap(),
                    ),
                )
                .await
                .expect("bounded cache probe")
                .is_err());
            }
        });
    }
    core.host_ble_attached = false;
    core.stop_device_sync_now();
    core.runtime.block_on(serving.shutdown()).unwrap();
    core.runtime.block_on(provider.shutdown()).unwrap();
}
