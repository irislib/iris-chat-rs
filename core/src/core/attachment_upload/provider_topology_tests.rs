use super::*;

pub(super) enum StartsFirst {
    App,
    Provider,
    Other,
}

pub(super) async fn check_local_topology(first: StartsFirst) {
    tokio::time::timeout(Duration::from_secs(30), async {
        let reservation = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(rendezvous) = reservation.local_addr().unwrap() else { unreachable!() };
        drop(reservation);
        let local = || {
            let mut config = network(vec![]);
            config.node.discovery.local.enabled = true;
            config.node.discovery.local.rendezvous_addr = rendezvous;
            config
        };
        let other = if matches!(first, StartsFirst::Other) {
            Some(identified_endpoint(&Keys::generate(), local()).await)
        } else { None };
        let (live, provider) = if matches!(first, StartsFirst::Provider) {
            let provider = identified_endpoint(&Keys::generate(), local()).await;
            (identified_endpoint(&Keys::generate(), local()).await, provider)
        } else {
            let live = identified_endpoint(&Keys::generate(), local()).await;
            (live, identified_endpoint(&Keys::generate(), local()).await)
        };
        let provider_id = PeerIdentity::from_npub(provider.npub()).unwrap();
        let (requests, capture) = capture_requests(provider.clone(), true).await;
        tokio::time::timeout(Duration::from_secs(8), async {
            while !live.local_instance_advertisements().unwrap().iter().any(|advert| {
                advert.npub == provider.npub() && advert.capability(TCP_BLOB_CAPABILITY).is_some()
            }) { tokio::time::sleep(Duration::from_millis(20)).await; }
        }).await.expect("existing endpoint discovered the local provider");
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let attachments = bind_same_host_attachment_store(
            live.clone(), ProviderReadOptions::new(&local(), vec![]),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))), vec![], vec![], primary,
        ).await.unwrap();
        let hash = hashtree_core::sha256(b"provider knows the upload identity for this hash");
        for _ in 0..2 {
            assert_eq!(attachments.store.get(&hash).await.unwrap(), None);
            let requester = observed(&requests, hash).await;
            assert_eq!(requester.npub(), attachments.reads.endpoint().npub());
            assert_ne!(requester.npub(), live.npub());
        }
        let provider_adjacent = provider.peers().await.unwrap().into_iter()
            .filter(|peer| peer.connected).map(|peer| peer.npub).collect::<Vec<_>>();
        let carrier = attachments.reads.endpoint().source_route_quality(provider_id, Duration::from_secs(5))
            .await.unwrap().next_hop;
        let used_identified_anchor = carrier == Some(*live.node_addr());
        assert!(carrier.is_some(), "recorded actual request carrier");
        assert!(attachments.reads.endpoint().local_instance_advertisements().unwrap().is_empty());
        assert!(attachments.reads.endpoint().peers().await.unwrap().iter()
            .all(|peer| !peer.connected || peer.npub != live.npub()), "reader must never join its identified app");
        eprintln!("provider_wire.anchor_route anonymous_request=true identified_carrier={used_identified_anchor} provider_only_identified_neighbor={}", provider_adjacent == vec![live.npub().to_string()]);
        attachments.shutdown_reads().await;
        drop(attachments);
        capture.abort(); let _ = capture.await;
        provider.shutdown().await.unwrap(); live.shutdown().await.unwrap();
        if let Some(other) = other { other.shutdown().await.unwrap(); }
        assert!(!used_identified_anchor, "fresh application keys do not hide an identified carrier from the file provider");
    }).await.expect("bounded identified-anchor privacy test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_first_uses_an_independent_anonymous_connection() {
    check_local_topology(StartsFirst::Provider).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_anchor_preserves_anonymous_offline_provider_access() {
    check_local_topology(StartsFirst::Other).await;
}

// Keep the tap open until the final admission checks so a reader response
// cannot disappear when the forwarding socket closes. Any response fails the
// test: an outbound-only reader must reject these unsolicited handshakes.
async fn forward_reader_join_attempts(
    endpoint: &FipsEndpoint,
    reader_npub: &str,
    reader_addr: std::net::SocketAddr,
) -> (tokio::net::UdpSocket, std::net::SocketAddr) {
    let tap = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source = endpoint.bound_udp_listen_addrs().await.unwrap()[0];
    endpoint
        .update_peers(vec![PeerConfig::new(
            reader_npub,
            "udp",
            tap.local_addr().unwrap().to_string(),
        )])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut packet = [0u8; 2048];
        // Observe both the actual initial Noise IK msg1 and its retransmit.
        // direct_probe_retry_count only changes after a failed connection;
        // it does not count these packets during the 30-second handshake.
        for _ in 0..2 {
            let (len, from) = tap.recv_from(&mut packet).await.unwrap();
            assert_eq!(from, source, "reader answered an unsolicited handshake");
            assert_eq!(len, 8 + fips_core::noise::HANDSHAKE_MSG1_SIZE);
            assert_eq!(&packet[..2], &[0x01, 0x00], "FMP Noise IK msg1");
            assert_eq!(u16::from_le_bytes([packet[2], packet[3]]) as usize, len - 4);
            assert_eq!(tap.send_to(&packet[..len], reader_addr).await.unwrap(), len);
        }
    })
    .await
    .expect("captured and forwarded initial and retransmitted UDP handshakes");
    (tap, source)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsolicited_identified_peers_cannot_join_anonymous_reader() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let live = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let sibling = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let unrelated = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let provider = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let provider_id = PeerIdentity::from_npub(provider.npub()).unwrap();
        let address = provider.bound_udp_listen_addrs().await.unwrap()[0];
        let public = vec![PeerConfig::new(provider.npub(), "udp", address.to_string())];
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let mut poisoned = public.clone();
        // Even mistaken explicit hints must not admit our own identities.
        for endpoint in [&live, &sibling] {
            poisoned.push(PeerConfig::new(
                endpoint.npub(),
                "udp",
                endpoint.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
            ));
        }
        let attachments = bind_same_host_attachment_store(
            live.clone(),
            ProviderReadOptions::new(&network(vec![]), poisoned),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
            vec![provider_id],
            vec![PeerIdentity::from_npub(sibling.npub()).unwrap()],
            primary,
        )
        .await
        .unwrap();
        let reader = attachments.reads.endpoint();
        let (requests, capture) = capture_requests(provider.clone(), false).await;
        let hash = hashtree_core::sha256(b"provider request while identified peers try to join");
        assert!(matches!(
            attachments
                .provider
                .route(hashtree_core::BlobRequest { hash, htl: 1 })
                .await
                .unwrap(),
            BlobReply::NoResult,
        ));
        assert_eq!(observed(&requests, hash).await.npub(), reader.npub());
        assert!(reader.bound_udp_listen_addrs().await.unwrap().is_empty());
        let port = provider
            .peers()
            .await
            .unwrap()
            .into_iter()
            .find(|peer| peer.connected && peer.npub == reader.npub())
            .unwrap()
            .transport_addr
            .unwrap()
            .parse::<std::net::SocketAddr>()
            .unwrap()
            .port();
        let reader_addr = format!("127.0.0.1:{port}").parse().unwrap();
        let (live_tap, sibling_tap, unrelated_tap) = tokio::join!(
            forward_reader_join_attempts(&live, reader.npub(), reader_addr),
            forward_reader_join_attempts(&sibling, reader.npub(), reader_addr),
            forward_reader_join_attempts(&unrelated, reader.npub(), reader_addr),
        );
        // After all three real attempts reached the reader's socket, require a
        // new provider request to prove the anonymous reader remains usable.
        let after_hash = hashtree_core::sha256(b"provider request after unsolicited handshakes");
        assert!(matches!(
            attachments
                .provider
                .route(hashtree_core::BlobRequest {
                    hash: after_hash,
                    htl: 1,
                })
                .await
                .unwrap(),
            BlobReply::NoResult,
        ));
        assert_eq!(observed(&requests, after_hash).await.npub(), reader.npub());
        assert!(
            reader
                .peers()
                .await
                .unwrap()
                .iter()
                .filter(|peer| peer.connected)
                .all(|peer| peer.npub == provider.npub()),
            "only approved public peers may authenticate to the reader"
        );
        for endpoint in [&live, &sibling, &unrelated] {
            assert!(endpoint
                .peers()
                .await
                .unwrap()
                .iter()
                .all(|peer| !peer.connected || peer.npub != reader.npub()));
        }
        for (tap, source) in [live_tap, sibling_tap, unrelated_tap] {
            let mut packet = [0u8; 2048];
            loop {
                match tap.try_recv_from(&mut packet) {
                    Ok((_, from)) => {
                        assert_eq!(from, source, "reader answered an unsolicited handshake");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("UDP admission tap failed: {error}"),
                }
            }
        }
        attachments.shutdown_reads().await;
        capture.abort();
        let _ = capture.await;
        for endpoint in [live, sibling, unrelated, provider] {
            endpoint.shutdown().await.unwrap();
        }
    })
    .await
    .expect("bounded reader admission wire test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn newly_linked_peer_revokes_its_route_and_preserves_other_provider_reads() {
    tokio::time::timeout(Duration::from_secs(40), async {
        let live = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let becoming_sibling = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let provider = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let sibling_id = PeerIdentity::from_npub(becoming_sibling.npub()).unwrap();
        let provider_id = PeerIdentity::from_npub(provider.npub()).unwrap();
        let public = vec![
            PeerConfig::new(
                becoming_sibling.npub(),
                "udp",
                becoming_sibling.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
            ),
            PeerConfig::new(
                provider.npub(),
                "udp",
                provider.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
            ),
        ];
        let (sibling_requests, sibling_capture) =
            capture_requests(becoming_sibling.clone(), false).await;
        let (provider_requests, provider_capture) = capture_requests(provider.clone(), false).await;
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let fallback = Arc::new(MemoryStore::new());
        let fallback_bytes = b"ordinary HTTP route remains available".to_vec();
        let fallback_hash = hashtree_core::sha256(&fallback_bytes);
        fallback
            .put(fallback_hash, fallback_bytes.clone())
            .await
            .unwrap();
        let attachments = bind_same_host_attachment_store(
            live.clone(),
            ProviderReadOptions::new(&network(vec![]), public.clone()),
            Arc::new(StoreBlobRoute::new(fallback.clone())),
            vec![sibling_id],
            vec![],
            primary.clone(),
        )
        .await
        .unwrap();
        let hash = hashtree_core::sha256(b"probe before linking this public peer");
        assert_eq!(attachments.store.get(&hash).await.unwrap(), None);
        let old_reader = observed(&sibling_requests, hash).await;
        assert!(attachments
            .reads
            .endpoint()
            .peers()
            .await
            .unwrap()
            .iter()
            .any(|peer| peer.connected && peer.npub == becoming_sibling.npub()));
        attachments
            .set_peers(vec![provider_id], vec![sibling_id])
            .await
            .unwrap();
        assert!(
            !attachments.reads.endpoint().is_stopped().await,
            "safe reader remains operational"
        );
        assert!(!attachments
            .reads
            .endpoint()
            .peers()
            .await
            .unwrap()
            .iter()
            .any(|peer| peer.connected && peer.npub == becoming_sibling.npub()));
        let request = hashtree_core::BlobRequest { hash, htl: 1 };
        assert!(matches!(
            attachments.provider.route(request).await.unwrap(),
            BlobReply::NoResult
        ));
        let continuing_reader = observed(&provider_requests, hash).await;
        assert_eq!(continuing_reader, old_reader);
        assert_eq!(
            continuing_reader.npub(),
            attachments.reads.endpoint().npub()
        );
        assert!(
            sibling_requests.try_recv().is_err(),
            "revoked provider received another GET"
        );
        assert_eq!(
            attachments.store.get(&fallback_hash).await.unwrap(),
            Some(fallback_bytes)
        );
        attachments.shutdown_reads().await;
        sibling_capture.abort();
        provider_capture.abort();
        let _ = sibling_capture.await;
        let _ = provider_capture.await;
        for endpoint in [live, becoming_sibling, provider] {
            endpoint.shutdown().await.unwrap();
        }
    })
    .await
    .expect("bounded reader revocation/recovery wire test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linking_cancels_already_prepared_hedged_provider_gets() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let live = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let stalled = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let linking = identified_endpoint(&Keys::generate(), network(vec![])).await;
        let stalled_id = PeerIdentity::from_npub(stalled.npub()).unwrap();
        let linking_id = PeerIdentity::from_npub(linking.npub()).unwrap();
        let public = vec![
            PeerConfig::new(
                stalled.npub(),
                "udp",
                stalled.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
            ),
            PeerConfig::new(
                linking.npub(),
                "udp",
                linking.bound_udp_listen_addrs().await.unwrap()[0].to_string(),
            ),
        ];
        let (started, stalled_capture) =
            capture_requests_with_reply(stalled.clone(), false, false).await;
        let (forbidden, linking_capture) = capture_requests(linking.clone(), false).await;
        let directory = tempfile::tempdir().unwrap();
        let primary = AttachmentChunkStore::open(directory.path(), 8 * 1024 * 1024).unwrap();
        let attachments = bind_same_host_attachment_store(
            live.clone(),
            ProviderReadOptions::new(&network(vec![]), public),
            Arc::new(StoreBlobRoute::new(Arc::new(MemoryStore::new()))),
            vec![stalled_id, linking_id],
            vec![],
            primary,
        )
        .await
        .unwrap();
        let hash = hashtree_core::sha256(b"already prepared hedged GET");
        let route = attachments.provider.clone();
        let pending = tokio::spawn(async move {
            route
                .route(hashtree_core::BlobRequest { hash, htl: 1 })
                .await
        });
        assert_eq!(
            observed(&started, hash).await.npub(),
            attachments.reads.endpoint().npub()
        );
        attachments
            .set_peers(vec![stalled_id], vec![linking_id])
            .await
            .unwrap();
        assert!(
            !attachments.reads.endpoint().is_stopped().await,
            "admission barrier preserves unrelated paths"
        );
        assert!(!attachments
            .reads
            .endpoint()
            .peers()
            .await
            .unwrap()
            .iter()
            .any(|peer| peer.connected && peer.npub == linking.npub()));
        assert!(pending.await.unwrap().is_err());
        assert!(
            forbidden.try_recv().is_err(),
            "queued hedge transmitted a GET after device linking"
        );
        stalled_capture.abort();
        linking_capture.abort();
        let _ = stalled_capture.await;
        let _ = linking_capture.await;
        for endpoint in [live, stalled, linking] {
            endpoint.shutdown().await.unwrap();
        }
    })
    .await
    .expect("bounded in-flight policy transition test");
}
