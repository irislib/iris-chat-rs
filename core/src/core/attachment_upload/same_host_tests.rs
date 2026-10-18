use super::*;
use fips_core::config::{PeerConfig, RoutingMode, TransportInstances, UdpConfig};
use fips_core::FipsEndpoint;
use hashtree_core::{BlobRoute, MemoryStore, StoreBlobRoute};
use hashtree_fips_transport::{TcpBlobTransport, TcpBlobTransportConfig, TCP_BLOB_CAPABILITY};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Instant;

fn reserve_udp_addr() -> SocketAddr {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve UDP address");
    socket.local_addr().expect("reserved UDP address")
}

async fn endpoint(
    rendezvous: SocketAddrV4,
    udp_addr: SocketAddr,
    scope: &str,
    peers: Vec<PeerConfig>,
) -> Arc<FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.local.rendezvous_addr = rendezvous;
    config.node.routing.mode = RoutingMode::ReplyLearned;
    config.peers = peers;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some(udp_addr.to_string()),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..UdpConfig::default()
    });
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .discovery_scope(scope)
            .local_rendezvous()
            .without_system_tun()
            .bind()
            .await
            .expect("bind test endpoint"),
    )
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}

async fn seed_file(store: Arc<MemoryStore>, bytes: &[u8]) -> String {
    let tree = HashTree::new(HashTreeConfig::new(store));
    let (cid, _) = tree.put(bytes).await.expect("seed Hashtree file");
    nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .expect("encode seeded file")
}

async fn download_bytes(nhash: &str, store: Arc<RoutedStore<AttachmentChunkStore>>) -> Vec<u8> {
    // Other AppCore fixtures run concurrently and replace the process-wide active
    // store. Exercise this profile's real routes without borrowing another test's.
    let data = nhash_decode(nhash).expect("decode attachment hash");
    read_hashtree_attachment(
        &Cid {
            hash: data.hash,
            key: data.decrypt_key,
        },
        store,
        MAX_ATTACHMENT_BYTES,
    )
    .await
    .expect("download attachment")
}

#[test]
fn single_device_without_relays_reuses_provider_and_preserves_fallback_and_outbound_link() {
    let provider_runtime = tokio::runtime::Runtime::new().expect("provider runtime");
    let rendezvous = match reserve_udp_addr() {
        SocketAddr::V4(addr) => addr,
        SocketAddr::V6(_) => unreachable!("IPv4 reservation"),
    };
    let sibling_rendezvous = match reserve_udp_addr() {
        SocketAddr::V4(addr) => addr,
        SocketAddr::V6(_) => unreachable!("IPv4 reservation"),
    };
    let sibling_addr = reserve_udp_addr();
    let sibling = provider_runtime.block_on(endpoint(
        sibling_rendezvous,
        sibling_addr,
        "iris-chat-sibling-test",
        Vec::new(),
    ));
    let provider_endpoint = provider_runtime.block_on(endpoint(
        rendezvous,
        reserve_udp_addr(),
        "iris-drive-provider-test",
        Vec::new(),
    ));
    let provider_local = Arc::new(MemoryStore::new());
    let provider_data = b"attachment supplied by the same-host provider".to_vec();
    let provider_nhash =
        provider_runtime.block_on(seed_file(provider_local.clone(), &provider_data));
    let provider = Arc::new(
        provider_runtime
            .block_on(TcpBlobTransport::bind_advertised_with_config(
                provider_endpoint.clone(),
                provider_local,
                TcpBlobTransportConfig::default(),
                100,
            ))
            .expect("bind provider"),
    );
    let fallback = Arc::new(MemoryStore::new());
    let before = b"standalone fallback after provider miss".to_vec();
    let after = b"standalone fallback after provider death".to_vec();
    let before_nhash = provider_runtime.block_on(seed_file(fallback.clone(), &before));
    let after_nhash = provider_runtime.block_on(seed_file(fallback.clone(), &after));
    let fallback: Arc<dyn BlobRoute> = Arc::new(StoreBlobRoute::new(fallback));

    let owner = nostr::Keys::generate();
    let device = nostr::Keys::generate();
    let temp_dir = tempfile::tempdir().expect("Chat data directory");
    let (update_tx, _update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: nostr_sdk::Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    assert!(
        core.app_keys.is_empty(),
        "test profile has no roster sibling"
    );
    core.reconcile_same_host_hashtree_for_test(
        rendezvous,
        fallback,
        vec![PeerConfig::new(
            sibling.npub(),
            "udp",
            sibling_addr.to_string(),
        )],
    );
    let (consumer, has_device_sync, sibling_count, store) = core
        .same_host_runtime_for_test()
        .expect("shared FIPS runtime and attachment store");
    assert!(!has_device_sync, "device-sync service stays disabled");
    assert_eq!(sibling_count, 0, "no device-sync peer is synthesized");

    wait_until(|| {
        consumer
            .local_instance_advertisements()
            .is_ok_and(|adverts| {
                adverts.iter().any(|advert| {
                    advert.npub == provider_endpoint.npub()
                        && advert.capability(TCP_BLOB_CAPABILITY).is_some()
                })
            })
    });
    wait_until(|| {
        core.runtime.block_on(consumer.peers()).is_ok_and(|peers| {
            peers.iter().any(|peer| {
                peer.npub == sibling.npub()
                    && peer.connected
                    && peer.transport_type.as_deref() == Some("udp")
            })
        })
    });

    assert_eq!(
        core.runtime
            .block_on(download_bytes(&provider_nhash, store.store.clone())),
        provider_data
    );
    assert_eq!(
        core.runtime
            .block_on(download_bytes(&before_nhash, store.store.clone())),
        before
    );
    wait_until(|| {
        provider_endpoint
            .local_instance_advertisements()
            .is_ok_and(|adverts| {
                adverts.iter().any(|advert| {
                    advert.npub == consumer.npub()
                        && advert.capability(TCP_BLOB_CAPABILITY).is_some()
                })
            })
    });
    let consumer_identity = fips_core::PeerIdentity::from_npub(consumer.npub()).unwrap();
    let downloaded = nhash_decode(&provider_nhash).unwrap();
    let cached_ciphertext = provider_runtime
        .block_on(provider.fetch_from_peer(&downloaded.hash, consumer_identity))
        .unwrap()
        .expect("Chat serves downloaded ciphertext");
    assert_eq!(
        hashtree_core::decrypt_chk(&cached_ciphertext, &downloaded.decrypt_key.unwrap()).unwrap(),
        provider_data
    );

    // Exercise the real encrypted upload path, then fetch its ciphertext from Chat.
    let upload_path = temp_dir.path().join("peer-attachment.txt");
    let plaintext = b"private file shared from the Chat attachment cache";
    std::fs::write(&upload_path, plaintext).unwrap();
    let (upload_url, upload_server) = super::upload_tests::serve_one_blossom_upload("200 OK");
    let uploading = Arc::new(
        UploadingBlossomStore::new(nostr::Keys::generate(), Vec::new(), vec![upload_url], None)
            .unwrap()
            .with_local(Some(
                core.attachment_service.as_ref().unwrap().store.clone(),
            )),
    );
    let uploaded = core
        .runtime
        .block_on(upload_file_to_store(&upload_path, uploading))
        .unwrap();
    upload_server.join().unwrap();
    let uploaded = nhash_decode(&uploaded).unwrap();
    let ciphertext = provider_runtime
        .block_on(provider.fetch_from_peer(&uploaded.hash, consumer_identity))
        .unwrap()
        .expect("Chat serves its uploaded chunk");
    assert_ne!(ciphertext, plaintext);
    assert_eq!(
        hashtree_core::decrypt_chk(&ciphertext, &uploaded.decrypt_key.unwrap()).unwrap(),
        plaintext
    );
    drop(provider);
    provider_runtime
        .block_on(provider_endpoint.shutdown())
        .unwrap();
    wait_until(|| {
        consumer
            .local_instance_advertisements()
            .is_ok_and(|adverts| {
                adverts.iter().all(|advert| {
                    advert.npub != provider_endpoint.npub()
                        || advert.capability(TCP_BLOB_CAPABILITY).is_none()
                })
            })
    });

    assert_eq!(
        core.runtime
            .block_on(download_bytes(&after_nhash, store.store.clone())),
        after
    );
    assert!(core
        .runtime
        .block_on(consumer.peers())
        .unwrap()
        .iter()
        .any(|peer| {
            peer.npub == sibling.npub()
                && peer.connected
                && peer.transport_type.as_deref() == Some("udp")
        }));

    drop(store);
    core.stop_device_sync();
    core.runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    provider_runtime.block_on(sibling.shutdown()).unwrap();
}

#[test]
fn peer_retrieves_pending_ciphertext_after_store_reopen_without_http_servers() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("ciphertext");
    let original = AttachmentChunkStore::open(&storage, local_store::DEFAULT_BUDGET).unwrap();
    let path = dir.path().join("pending.bin");
    let plaintext = b"durable peer attachment while every HTTP server is offline";
    std::fs::write(&path, plaintext).unwrap();
    let session = original.begin(None).unwrap();
    let nhash = runtime
        .block_on(upload_file_to_store(&path, session.clone()))
        .unwrap();
    original
        .finish_batch(&[(session, nhash.clone())], &[])
        .unwrap();
    drop(original);
    let reopened = AttachmentChunkStore::open(&storage, local_store::DEFAULT_BUDGET).unwrap();
    let rendezvous = match reserve_udp_addr() {
        SocketAddr::V4(addr) => addr,
        _ => unreachable!(),
    };
    let provider = runtime.block_on(endpoint(
        rendezvous,
        reserve_udp_addr(),
        "durable-provider",
        Vec::new(),
    ));
    let consumer = runtime.block_on(endpoint(
        rendezvous,
        reserve_udp_addr(),
        "durable-consumer",
        Vec::new(),
    ));
    let serving = runtime
        .block_on(TcpBlobTransport::bind_advertised_with_config(
            provider.clone(),
            reopened,
            TcpBlobTransportConfig::default(),
            100,
        ))
        .unwrap();
    let fetching = runtime
        .block_on(TcpBlobTransport::bind(
            consumer.clone(),
            Arc::new(MemoryStore::new()),
        ))
        .unwrap();
    wait_until(|| {
        consumer
            .local_instance_advertisements()
            .is_ok_and(|adverts| {
                adverts.iter().any(|advert| {
                    advert.npub == provider.npub()
                        && advert.capability(TCP_BLOB_CAPABILITY).is_some()
                })
            })
    });
    let decoded = nhash_decode(&nhash).unwrap();
    let bytes = runtime
        .block_on(fetching.fetch_from_peer(
            &decoded.hash,
            fips_core::PeerIdentity::from_npub(provider.npub()).unwrap(),
        ))
        .unwrap()
        .unwrap();
    assert_ne!(bytes, plaintext);
    assert_eq!(
        hashtree_core::decrypt_chk(&bytes, &decoded.decrypt_key.unwrap()).unwrap(),
        plaintext
    );
    drop(serving);
    drop(fetching);
    runtime.block_on(consumer.shutdown()).unwrap();
    runtime.block_on(provider.shutdown()).unwrap();
}

#[test]
fn software_update_blocks_use_attachment_peers_and_are_reshared() {
    let executor = tokio::runtime::Runtime::new().unwrap();
    executor.block_on(async {
        let SocketAddr::V4(rendezvous) = reserve_udp_addr() else {
            unreachable!()
        };
        let source_endpoint =
            endpoint(rendezvous, reserve_udp_addr(), "update-source", Vec::new()).await;
        let app_endpoint =
            endpoint(rendezvous, reserve_udp_addr(), "update-chat", Vec::new()).await;
        let origin = Arc::new(MemoryStore::new());
        let bytes = b"verified public update block".to_vec();
        let hash = hashtree_core::sha256(&bytes);
        origin.put(hash, bytes.clone()).await.unwrap();
        let source = TcpBlobTransport::bind_advertised_with_config(
            source_endpoint.clone(),
            origin,
            TcpBlobTransportConfig::default(),
            10,
        )
        .await
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let runtime = bind_same_host_attachment_store(
            app_endpoint.clone(),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
            Vec::new(),
            primary.clone(),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            while !runtime
                .provider
                .discovered_provider_ids()
                .unwrap()
                .iter()
                .any(|id| id == source_endpoint.npub())
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        let updates = hashtree_updater::build_update_store(
            runtime.update_store(),
            None,
            hashtree_updater::SecurePubsubBlossomConfig::default(),
        )
        .unwrap();
        assert_eq!(updates.get(&hash).await.unwrap(), Some(bytes.clone()));
        assert_eq!(primary.get(&hash).await.unwrap(), Some(bytes.clone()));
        source.shutdown().await.unwrap();
        source_endpoint.shutdown().await.unwrap();
        let receiver_endpoint = endpoint(
            rendezvous,
            reserve_udp_addr(),
            "update-third-peer",
            Vec::new(),
        )
        .await;
        let receiver = TcpBlobTransport::in_memory(receiver_endpoint.clone())
            .await
            .unwrap();
        let peer = fips_core::PeerIdentity::from_npub(app_endpoint.npub()).unwrap();
        assert_eq!(receiver.get(&hash, peer).await.unwrap(), Some(bytes));
        receiver.shutdown().await.unwrap();
        receiver_endpoint.shutdown().await.unwrap();
        drop(updates);
        drop(runtime);
        app_endpoint.shutdown().await.unwrap();
    });
}
