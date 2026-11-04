#[test]
fn linked_device_reconciliation_preserves_anonymous_provider_reads_with_live_host_bridge() {
    use fips_core::config::{PeerConfig, TransportInstances, UdpConfig};
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
    let mut config = fips_core::Config::new();
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
    let provider = Arc::new(core.runtime.block_on(
        fips_core::FipsEndpoint::builder().config(config).local_rendezvous()
            .without_system_tun().bind(),
    ).unwrap());
    let provider_cache = Arc::new(MemoryStore::new());
    let before = b"provider block before linking".to_vec();
    let after = b"uncached provider block after linking".to_vec();
    let before_hash = hashtree_core::sha256(&before);
    let after_hash = hashtree_core::sha256(&after);
    core.runtime.block_on(provider_cache.put(before_hash, before.clone())).unwrap();
    core.runtime.block_on(provider_cache.put(after_hash, after.clone())).unwrap();
    let requesters = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = requesters.clone();
    let serving = core.runtime.block_on(TcpBlobTransport::bind_advertised_route_with_config_and_policy(
        provider.clone(), provider_cache.clone(), Arc::new(StoreBlobRoute::new(provider_cache)),
        TcpBlobTransportConfig::default(), 10, Arc::new(move |peer| {
            captured.lock().unwrap().push(peer);
            true
        }),
    )).unwrap();
    let public_peers = vec![PeerConfig::new(provider.npub(), "udp",
        core.runtime.block_on(provider.bound_udp_listen_addrs()).unwrap()[0].to_string())];
    let fallback: Arc<dyn hashtree_core::BlobRoute> = Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new())));
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback.clone(), public_peers.clone());
    let (live, _, _, blobs) = core.same_host_runtime_for_test().unwrap();
    core.runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            while !live.local_instance_advertisements().unwrap().iter().any(|advert| {
                advert.npub == provider.npub()
                    && advert.capability(hashtree_fips_transport::TCP_BLOB_CAPABILITY).is_some()
            }) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        assert_eq!(blobs.update_store().get(&before_hash).await.unwrap(), Some(before));
    });
    let initial_reader = blobs.provider_read_endpoint_for_test();
    assert_eq!(requesters.lock().unwrap().last().unwrap().npub(), initial_reader.npub());

    // Exercise the production in-place branch used by a single-use host BLE
    // bridge. This fixture needs no physical radio or bridge I/O to prove its
    // endpoint/reader lifecycle; the live FIPS endpoint must survive the update.
    core.host_ble_attached = true;
    configure_test_device_sync_profile(&mut core, &owner, &local, &new_sibling, None);
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback.clone(), public_peers.clone());
    // Repeat ordinary reconciliation: matching keys must not leave a closed
    // reader permanently installed after the first policy transition.
    core.reconcile_same_host_hashtree_for_test(rendezvous, fallback, public_peers);
    let (same_live, _, sibling_count, refreshed) = core.same_host_runtime_for_test().unwrap();
    assert!(Arc::ptr_eq(&live, &same_live), "keep the live bridge and sync endpoint");
    assert_eq!(sibling_count, 1);
    let current_reader = refreshed.provider_read_endpoint_for_test();
    core.runtime.block_on(async {
        assert!(!same_live.is_stopped().await);
        let result = tokio::time::timeout(std::time::Duration::from_secs(12),
            refreshed.update_store().get(&after_hash)).await.unwrap();
        assert_eq!(result.unwrap(), Some(after), "production reconciliation must restore unrelated provider reads");
    });
    let source = requesters.lock().unwrap().last().copied().unwrap();
    assert_eq!(source.npub(), current_reader.npub());
    assert_ne!(source.npub(), same_live.npub(), "recovery must stay anonymous");
    core.host_ble_attached = false;
    core.stop_device_sync_now();
    core.runtime.block_on(serving.shutdown()).unwrap();
    core.runtime.block_on(provider.shutdown()).unwrap();
}
