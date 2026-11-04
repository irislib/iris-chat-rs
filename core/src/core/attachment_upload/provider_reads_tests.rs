use super::*;
use fips_core::PeerIdentity;
use fips_tcp_endpoint::FipsTcpEndpoint;
use hashtree_core::{
    decode_blob_request, encode_blob_reply_header, BlobReply, MemoryStore, BLOB_REQUEST_BYTES,
};
use hashtree_fips_transport::{TCP_BLOB_CAPABILITY, TCP_BLOB_SERVICE_PORT};

fn network(peers: Vec<PeerConfig>) -> fips_core::Config {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.peers = peers;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..Default::default()
    });
    config
}

async fn identified_endpoint(keys: &Keys, config: fips_core::Config) -> Arc<FipsEndpoint> {
    let local_rendezvous = config.node.discovery.local.enabled;
    let mut builder = FipsEndpoint::builder()
        .config(config)
        .identity_nsec(keys.secret_key().to_secret_hex())
        .without_system_tun();
    if local_rendezvous {
        builder = builder.local_rendezvous();
    }
    Arc::new(builder.bind().await.unwrap())
}

/// Observe the authenticated remote peer and bytes actually delivered through
/// FIPS/TCP. A config check or a ciphertext roundtrip would miss this regression.
async fn capture_requests(
    endpoint: Arc<FipsEndpoint>,
    advertise: bool,
) -> (
    flume::Receiver<(PeerIdentity, Vec<u8>)>,
    tokio::task::JoinHandle<()>,
) {
    let mut tcp = if advertise {
        FipsTcpEndpoint::bind_with_capability(
            endpoint,
            fips_core::discovery::local::LocalInstanceCapability::service(
                TCP_BLOB_CAPABILITY,
                TCP_BLOB_SERVICE_PORT,
            ),
            fips_tcp::Config::default(),
            718,
        )
        .await
        .unwrap()
    } else {
        FipsTcpEndpoint::bind(
            endpoint,
            TCP_BLOB_SERVICE_PORT,
            fips_tcp::Config::default(),
            718,
        )
        .await
        .unwrap()
    };
    let (tx, rx) = flume::unbounded();
    let task = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut pending = HashMap::new();
        loop {
            let now = started.elapsed().as_millis() as u64;
            tcp.poll(now).await.unwrap();
            while let Some(connection) = tcp.accept() {
                pending.insert(connection, Vec::new());
            }
            for connection in pending.keys().copied().collect::<Vec<_>>() {
                let bytes = pending.get_mut(&connection).unwrap();
                bytes.extend(
                    tcp.read(connection, BLOB_REQUEST_BYTES - bytes.len(), now)
                        .await
                        .unwrap(),
                );
                if bytes.len() != BLOB_REQUEST_BYTES {
                    continue;
                }
                decode_blob_request(bytes).expect("serialized blob GET");
                tx.send((
                    tcp.peer(connection).expect("authenticated requester"),
                    bytes.clone(),
                ))
                .unwrap();
                let response = encode_blob_reply_header(&BlobReply::NoResult).unwrap();
                assert_eq!(
                    tcp.write(connection, &response, now).await.unwrap(),
                    response.len()
                );
                tcp.close(connection, now).await.unwrap();
                pending.remove(&connection);
            }
            tokio::select! {
                result = tcp.wait_for_input() => {
                    result.unwrap();
                    tcp.receive(started.elapsed().as_millis() as u64).await.unwrap();
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    });
    (rx, task)
}

async fn observed(rx: &flume::Receiver<(PeerIdentity, Vec<u8>)>, hash: Hash) -> PeerIdentity {
    let (peer, bytes) = tokio::time::timeout(Duration::from_secs(10), rx.recv_async())
        .await
        .expect("provider received a request")
        .unwrap();
    assert_eq!(decode_blob_request(&bytes).unwrap().hash, hash);
    peer
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_wire_hides_device_identity_on_first_read_retry_and_restart() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let owner = Keys::generate();
        let device = Keys::generate();
        let provider = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let provider_id = PeerIdentity::from_npub(provider.npub()).unwrap();
        let address = provider.bound_udp_listen_addrs().await.unwrap()[0];
        let config = network(vec![PeerConfig::new(
            provider.npub(),
            "udp",
            address.to_string(),
        )]);
        let (requests, capture) = capture_requests(provider.clone(), false).await;
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let hash = hashtree_core::sha256(b"private attachment ciphertext");
        let owner_id = PeerIdentity::from_npub(&owner.public_key().to_bech32().unwrap()).unwrap();
        let device_id = PeerIdentity::from_npub(&device.public_key().to_bech32().unwrap()).unwrap();

        // Positive leak control: the former wiring exposes the real device key
        // alongside exactly the same serialized hash, despite FIPS encryption.
        let old_endpoint = identified_endpoint(&device, config.clone()).await;
        let old_transport = TcpBlobTransport::bind_client_with_config(
            old_endpoint.clone(),
            primary.clone(),
            TcpBlobTransportConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            old_transport
                .fetch_from_peer(&hash, provider_id)
                .await
                .unwrap(),
            None
        );
        assert_eq!(observed(&requests, hash).await, device_id);
        old_transport.shutdown().await.unwrap();
        old_endpoint.shutdown().await.unwrap();

        let mut previous = None;
        for _ in 0..2 {
            let live = identified_endpoint(&device, config.clone()).await;
            let attachments = bind_same_host_attachment_store(
                live.clone(),
                ProviderReadOptions::new(&config, false),
                Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
                vec![provider_id],
                Vec::new(),
                primary.clone(),
            )
            .await
            .unwrap();
            let mut first = None;
            for _ in 0..2 {
                assert_eq!(attachments.store.get(&hash).await.unwrap(), None);
                let source = observed(&requests, hash).await;
                assert_ne!(
                    source, device_id,
                    "provider must not learn the requesting device"
                );
                assert_ne!(
                    source, owner_id,
                    "provider must not learn the requesting account"
                );
                assert_ne!(
                    Some(source),
                    previous,
                    "restart must rotate the anonymous key"
                );
                assert_eq!(*first.get_or_insert(source), source);
            }
            previous = first;
            // With the anonymous endpoint gone, a read may fail or miss, but
            // must never retry over the still-running identified endpoint.
            attachments.shutdown_reads().await;
            let _ = attachments.store.get(&hash).await;
            assert!(
                requests.try_recv().is_err(),
                "identified fallback emitted a request"
            );
            drop(attachments);
            live.shutdown().await.unwrap();
        }
        capture.abort();
        let _ = capture.await;
        provider.shutdown().await.unwrap();
    })
    .await
    .expect("bounded provider wire test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identified_cache_wire_rejects_contacts_and_revoked_siblings() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let live = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let live_id = PeerIdentity::from_npub(live.npub()).unwrap();
        let address = live.bound_udp_listen_addrs().await.unwrap()[0];
        let to_live = || {
            network(vec![PeerConfig::new(
                live.npub(),
                "udp",
                address.to_string(),
            )])
        };
        let contact = identified_endpoint(&Keys::generate(), to_live()).await;
        let sibling = identified_endpoint(&Keys::generate(), to_live()).await;
        let rotated = identified_endpoint(&Keys::generate(), to_live()).await;
        let contact_id = PeerIdentity::from_npub(contact.npub()).unwrap();
        let sibling_id = PeerIdentity::from_npub(sibling.npub()).unwrap();
        let rotated_id = PeerIdentity::from_npub(rotated.npub()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let bytes = b"cached private ciphertext known to the upload provider".to_vec();
        let hash = hashtree_core::sha256(&bytes);
        primary.put(hash, bytes.clone()).await.unwrap();
        let attachments = bind_same_host_attachment_store(
            live.clone(),
            ProviderReadOptions::new(&network(vec![]), false),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
            vec![contact_id, sibling_id],
            vec![sibling_id],
            primary,
        )
        .await
        .unwrap();
        let contact_tcp = TcpBlobTransport::in_memory(contact.clone()).await.unwrap();
        let sibling_cache = Arc::new(MemoryStore::new());
        let sibling_bytes = b"trusted linked-device copy".to_vec();
        let sibling_hash = hashtree_core::sha256(&sibling_bytes);
        sibling_cache
            .put(sibling_hash, sibling_bytes.clone())
            .await
            .unwrap();
        let sibling_tcp = TcpBlobTransport::bind(sibling.clone(), sibling_cache)
            .await
            .unwrap();
        let rotated_tcp = TcpBlobTransport::in_memory(rotated.clone()).await.unwrap();

        // Authentication succeeds, but a known ordinary contact cannot probe H.
        assert!(contact_tcp.fetch_from_peer(&hash, live_id).await.is_err());
        assert!(live
            .peers()
            .await
            .unwrap()
            .iter()
            .any(|peer| peer.npub == contact.npub() && peer.connected));
        assert_eq!(
            sibling_tcp.fetch_from_peer(&hash, live_id).await.unwrap(),
            Some(bytes.clone())
        );
        assert_eq!(
            attachments.store.get(&sibling_hash).await.unwrap(),
            Some(sibling_bytes)
        );

        // The anonymous outbound endpoint shares the cache only as a client;
        // probing it must not become a second way to extract that cache.
        let anonymous_id = PeerIdentity::from_npub(attachments.reads.endpoint.npub()).unwrap();
        let anonymous_port = attachments
            .reads
            .endpoint
            .bound_udp_listen_addrs()
            .await
            .unwrap()[0]
            .port();
        contact
            .update_peers(vec![PeerConfig::new(
                attachments.reads.endpoint.npub(),
                "udp",
                format!("127.0.0.1:{anonymous_port}"),
            )])
            .await
            .unwrap();
        assert!(contact_tcp
            .fetch_from_peer(&hash, anonymous_id)
            .await
            .is_err());

        // Updating the verified sibling set revokes old device keys without
        // making every known contact eligible for identified cache access.
        attachments.set_peers(vec![contact_id, sibling_id, rotated_id], vec![rotated_id]);
        assert!(sibling_tcp.fetch_from_peer(&hash, live_id).await.is_err());
        assert_eq!(
            rotated_tcp.fetch_from_peer(&hash, live_id).await.unwrap(),
            Some(bytes)
        );

        attachments.shutdown_reads().await;
        drop(attachments);
        contact_tcp.shutdown().await.unwrap();
        sibling_tcp.shutdown().await.unwrap();
        rotated_tcp.shutdown().await.unwrap();
        for endpoint in [contact, sibling, rotated, live] {
            endpoint.shutdown().await.unwrap();
        }
    })
    .await
    .expect("bounded cache authorization wire test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_read_uses_existing_directory_but_anonymous_wire_identity() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let reservation = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(rendezvous) = reservation.local_addr().unwrap() else {
            unreachable!("IPv4 reservation")
        };
        drop(reservation);
        let local = || {
            let mut config = network(vec![]);
            config.node.discovery.local.enabled = true;
            config.node.discovery.local.rendezvous_addr = rendezvous;
            config
        };
        let provider = identified_endpoint(&Keys::generate(), local()).await;
        let device = Keys::generate();
        let live = identified_endpoint(&device, local()).await;
        let (requests, capture) = capture_requests(provider.clone(), true).await;
        tokio::time::timeout(Duration::from_secs(8), async {
            while !live
                .local_instance_advertisements()
                .unwrap()
                .iter()
                .any(|advert| {
                    advert.npub == provider.npub()
                        && advert.capability(TCP_BLOB_CAPABILITY).is_some()
                })
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("existing endpoint knows the local provider");

        // The new anonymous endpoint has no capability directory at all. A
        // first fetch must use the existing public snapshot, not wait for a
        // second discovery cycle or fall back to identified transport.
        let address = provider.bound_udp_listen_addrs().await.unwrap()[0];
        let read_config = network(vec![PeerConfig::new(
            provider.npub(),
            "udp",
            address.to_string(),
        )]);
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let attachments = bind_same_host_attachment_store(
            live.clone(),
            ProviderReadOptions::new(&read_config, false),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
            vec![],
            vec![],
            primary,
        )
        .await
        .unwrap();
        assert!(attachments
            .reads
            .endpoint
            .local_instance_advertisements()
            .unwrap()
            .is_empty());
        assert!(
            !attachments
                .provider
                .discovered_provider_ids()
                .unwrap()
                .iter()
                .any(|provider| provider == live.npub()),
            "never probe our identified cache"
        );
        let hash = hashtree_core::sha256(b"first read before anonymous discovery");
        assert_eq!(attachments.store.get(&hash).await.unwrap(), None);
        let requester = observed(&requests, hash).await;
        assert_eq!(requester.npub(), attachments.reads.endpoint.npub());
        assert_ne!(requester.npub(), device.public_key().to_bech32().unwrap());

        attachments.shutdown_reads().await;
        drop(attachments);
        capture.abort();
        let _ = capture.await;
        live.shutdown().await.unwrap();
        provider.shutdown().await.unwrap();
    })
    .await
    .expect("bounded first-read directory wire test");
}
