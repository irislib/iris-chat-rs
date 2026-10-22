use super::*;
use crate::core::fips_nearby::{FipsNearbyOutbox, FIPS_NEARBY_PORT};
use crate::updates::FipsNearbyLinkSnapshot;
use fips_core::{FipsEndpointOutboundDatagram, FipsEndpointPeer};

#[derive(Clone, Copy)]
struct DiscoveryPeerState {
    connected: bool,
    retries: u32,
    failures: u32,
}

#[derive(Default)]
struct DiscoveryRouteEdges {
    previous: BTreeMap<PublicKey, DiscoveryPeerState>,
    last_retry_poke: BTreeMap<PublicKey, std::time::Instant>,
}

impl DiscoveryRouteEdges {
    fn observe(&mut self, peers: &[FipsEndpointPeer]) -> Vec<(PublicKey, bool)> {
        self.update(
            peers
                .iter()
                .filter_map(|peer| {
                    let author = PublicKey::from_hex(&peer_key_hex(&peer.npub)?).ok()?;
                    Some((
                        author,
                        DiscoveryPeerState {
                            connected: peer.connected,
                            retries: peer.direct_probe_retry_count,
                            failures: peer.nostr_traversal_consecutive_failures,
                        },
                    ))
                })
                .collect(),
            std::time::Instant::now(),
        )
    }

    fn update(
        &mut self,
        current: BTreeMap<PublicKey, DiscoveryPeerState>,
        now: std::time::Instant,
    ) -> Vec<(PublicKey, bool)> {
        let mut needed = Vec::new();
        for (author, previous) in &self.previous {
            match current.get(author) {
                Some(next) if previous.connected && !next.connected => {
                    needed.push((*author, false));
                    self.last_retry_poke.insert(*author, now);
                }
                None if previous.connected => {
                    needed.push((*author, false));
                }
                Some(next)
                    if (next.retries > previous.retries || next.failures > previous.failures)
                        && self.last_retry_poke.get(author).is_none_or(|last| {
                            now.saturating_duration_since(*last) >= Duration::from_secs(30)
                        }) =>
                {
                    // Configured peers retry autonomously. Core must additionally
                    // require exact private outbound/call demand before querying.
                    needed.push((*author, true));
                    self.last_retry_poke.insert(*author, now);
                }
                _ => {}
            }
        }
        self.last_retry_poke
            .retain(|author, _| current.contains_key(author));
        // The first snapshot is a baseline, not a reason to wake every author.
        // Link loss is immediate; automatic retries cannot bypass quiet cadence.
        self.previous = current;
        needed
    }
}

pub(super) async fn run(
    endpoint: Arc<FipsEndpoint>,
    bootstrap_payloads: Arc<RwLock<Vec<Vec<u8>>>>,
    outbox: Arc<RwLock<FipsNearbyOutbox>>,
    core_sender: Sender<CoreMsg>,
    generation: u64,
    nearby_enabled: bool,
) {
    let mut initialized_links = BTreeMap::<String, (u64, u64)>::new();
    let mut current_bootstrap = Vec::new();
    let mut bootstrap_revision = 0_u64;
    let mut reported_links = Vec::new();
    let mut discovery_edges = DiscoveryRouteEdges::default();
    let carrier = debug_snapshot_file_writes_enabled()
        .then(|| {
            endpoint
                .enable_service_carrier_diagnostics(FIPS_NEARBY_PORT)
                .ok()
        })
        .flatten();
    let mut previous_carrier = None;
    loop {
        if let Some(handle) = &carrier {
            let snapshot = handle.snapshot();
            if previous_carrier.as_ref() != Some(&snapshot) {
                let transports = snapshot
                    .transports
                    .iter()
                    .filter(|value| value.submitted_packets > 0)
                    .map(|value| {
                        format!(
                            "{}:{}/{}",
                            value.transport, value.submitted_packets, value.fips_payload_bytes
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let detail = format!(
                    "generation={generation} discarded={} transports={transports}",
                    snapshot.discarded_outputs
                );
                crate::perflog!("fips_nearby.tx.carrier {detail}");
                let _ = core_sender.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                    category: "fips_nearby.tx.carrier".to_string(),
                    detail,
                })));
                previous_carrier = Some(snapshot);
            }
        }
        if let Ok(payloads) = bootstrap_payloads.read() {
            if *payloads != current_bootstrap {
                current_bootstrap = payloads.clone();
                bootstrap_revision = bootstrap_revision.wrapping_add(1);
            }
        }
        let peers = match peer_snapshot::query_with_timeout_handler(
            || endpoint.peers(),
            || report_links(&core_sender, generation, &mut reported_links, Vec::new()),
        )
        .await
        {
            Ok(peers) => peers,
            Err(_) => {
                report_links(&core_sender, generation, &mut reported_links, Vec::new());
                return;
            }
        };
        for (author, requires_pending_work) in discovery_edges.observe(&peers) {
            let _ = core_sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::FipsPeerDiscoveryNeeded {
                    generation,
                    author,
                    requires_pending_work,
                },
            )));
        }
        let mut current_links = peers
            .iter()
            .filter(|peer| peer.connected)
            .filter_map(|peer| {
                Some(FipsNearbyLinkSnapshot {
                    device_pubkey_hex: peer_key_hex(&peer.npub)?,
                    transport_type: peer.transport_type.clone().unwrap_or_default(),
                    transport_addr: peer.transport_addr.clone(),
                })
            })
            .collect::<Vec<_>>();
        current_links.sort_by(|left, right| {
            left.device_pubkey_hex
                .cmp(&right.device_pubkey_hex)
                .then_with(|| left.transport_type.cmp(&right.transport_type))
        });
        report_links(&core_sender, generation, &mut reported_links, current_links);
        for peer in peers.into_iter().filter(|peer| peer.connected) {
            let Ok(identity) = FipsPeerIdentity::from_npub(&peer.npub) else {
                continue;
            };
            let initialization = (peer.link_id, bootstrap_revision);
            let initialized = if initialized_links.get(&peer.npub) == Some(&initialization) {
                true
            } else {
                let payloads = if nearby_enabled {
                    current_bootstrap.clone()
                } else {
                    Vec::new()
                };
                let sent = if payloads.is_empty() {
                    true
                } else {
                    let datagrams = payloads
                        .into_iter()
                        .map(|data| {
                            FipsEndpointOutboundDatagram::new(
                                FIPS_NEARBY_PORT,
                                FIPS_NEARBY_PORT,
                                data,
                            )
                        })
                        .collect();
                    endpoint
                        .send_datagram_batch_to_peer(identity, datagrams)
                        .await
                        .is_ok()
                };
                if sent {
                    initialized_links.insert(peer.npub.clone(), initialization);
                }
                sent
            };
            if !initialized {
                continue;
            }

            let pending = outbox
                .read()
                .map(|outbox| {
                    outbox.pending_for_link_with_public(&peer.npub, peer.link_id, nearby_enabled)
                })
                .unwrap_or_default();
            if pending.is_empty() {
                continue;
            }
            let event_ids = pending
                .iter()
                .map(|(event_id, _)| event_id.clone())
                .collect::<Vec<_>>();
            let datagrams = pending
                .into_iter()
                .map(|(_, data)| {
                    FipsEndpointOutboundDatagram::new(FIPS_NEARBY_PORT, FIPS_NEARBY_PORT, data)
                })
                .collect();
            if endpoint
                .send_datagram_batch_to_peer(identity, datagrams)
                .await
                .is_ok()
            {
                if let Ok(mut outbox) = outbox.write() {
                    outbox.mark_sent_on_link(&peer.npub, peer.link_id, &event_ids);
                }
            }
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn peer_key_hex(npub: &str) -> Option<String> {
    // Reporting needs only the validated public key, not routing or ECDH state.
    Some(fips_core::decode_npub(npub).ok()?.to_string())
}

fn report_links(
    core_sender: &Sender<CoreMsg>,
    generation: u64,
    reported_links: &mut Vec<FipsNearbyLinkSnapshot>,
    peers: Vec<FipsNearbyLinkSnapshot>,
) {
    if *reported_links == peers {
        return;
    }
    *reported_links = peers.clone();
    let _ = core_sender.send(CoreMsg::Internal(Box::new(
        InternalEvent::FipsNearbyPeersChanged { generation, peers },
    )));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

    fn discovery_state(connected: bool, retries: u32, failures: u32) -> DiscoveryPeerState {
        DiscoveryPeerState {
            connected,
            retries,
            failures,
        }
    }

    #[test]
    fn discovery_route_demand_is_exact_author_and_edge_triggered() {
        let a = Keys::generate().public_key();
        let b = Keys::generate().public_key();
        let mut edges = DiscoveryRouteEdges::default();
        let now = std::time::Instant::now();
        let initial = BTreeMap::from([
            (a, discovery_state(true, 0, 0)),
            (b, discovery_state(false, 5, 2)),
        ]);
        assert!(
            edges.update(initial.clone(), now).is_empty(),
            "initial scope sweep must not force every author"
        );
        assert!(edges.update(initial, now).is_empty());
        let retry = BTreeMap::from([
            (a, discovery_state(true, 0, 0)),
            (b, discovery_state(false, 6, 2)),
        ]);
        assert_eq!(edges.update(retry.clone(), now), vec![(b, true)]);
        assert!(
            edges.update(retry, now).is_empty(),
            "stable failed peer must not wake each1s tick"
        );
        let failure = BTreeMap::from([
            (a, discovery_state(true, 0, 0)),
            (b, discovery_state(false, 6, 3)),
        ]);
        assert!(edges
            .update(failure, now + Duration::from_secs(1))
            .is_empty());
        let disconnected = BTreeMap::from([
            (a, discovery_state(false, 0, 0)),
            (b, discovery_state(false, 6, 3)),
        ]);
        assert_eq!(
            edges.update(disconnected.clone(), now + Duration::from_secs(2)),
            vec![(a, false)]
        );
        assert!(edges
            .update(disconnected, now + Duration::from_secs(3))
            .is_empty());
    }

    #[test]
    fn discovery_autonomous_retry_edges_require_work_and_are_throttled() {
        let author = Keys::generate().public_key();
        let mut edges = DiscoveryRouteEdges::default();
        let now = std::time::Instant::now();
        assert!(edges
            .update(
                BTreeMap::from([(author, discovery_state(false, 0, 0))]),
                now
            )
            .is_empty());
        let mut observations = Vec::new();
        for second in 1..=90_u32 {
            for (peer, requires_pending_work) in edges.update(
                BTreeMap::from([(author, discovery_state(false, second, second))]),
                now + Duration::from_secs(u64::from(second)),
            ) {
                assert_eq!(peer, author);
                assert!(
                    requires_pending_work,
                    "idle retries must never force a query"
                );
                observations.push(second);
            }
        }
        assert_eq!(observations, vec![1, 31, 61]);
    }

    #[test]
    fn discovery_link_loss_bypasses_retry_throttle_once() {
        let author = Keys::generate().public_key();
        let mut edges = DiscoveryRouteEdges::default();
        let now = std::time::Instant::now();
        edges.update(BTreeMap::from([(author, discovery_state(true, 0, 0))]), now);
        assert_eq!(
            edges.update(BTreeMap::from([(author, discovery_state(true, 1, 0))]), now),
            vec![(author, true)]
        );
        let disconnected = BTreeMap::from([(author, discovery_state(false, 2, 1))]);
        assert_eq!(
            edges.update(disconnected.clone(), now + Duration::from_secs(1)),
            vec![(author, false)]
        );
        assert!(edges
            .update(disconnected, now + Duration::from_secs(2))
            .is_empty());
        assert!(edges
            .update(
                BTreeMap::from([(author, discovery_state(false, 3, 2))]),
                now + Duration::from_secs(3),
            )
            .is_empty());
    }

    #[test]
    fn discovery_route_disappearance_pokes_once_and_new_scope_starts_normally() {
        let author = Keys::generate().public_key();
        let mut edges = DiscoveryRouteEdges::default();
        let now = std::time::Instant::now();
        assert!(edges
            .update(BTreeMap::from([(author, discovery_state(true, 0, 0))]), now)
            .is_empty());
        assert_eq!(edges.update(BTreeMap::new(), now), vec![(author, false)]);
        assert!(edges.update(BTreeMap::new(), now).is_empty());
        assert!(edges
            .update(
                BTreeMap::from([(author, discovery_state(false, 1, 1))]),
                now
            )
            .is_empty());
        assert!(edges
            .update(BTreeMap::from([(author, discovery_state(true, 0, 0))]), now)
            .is_empty());
        assert!(edges
            .update(BTreeMap::from([(author, discovery_state(true, 0, 0))]), now)
            .is_empty());
    }

    #[test]
    fn displayed_peer_keys_preserve_identity_validation() {
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().unwrap();
        let expected = FipsPeerIdentity::from_npub(&npub)
            .unwrap()
            .pubkey()
            .to_string();
        assert_eq!(peer_key_hex(&npub), Some(expected.clone()));
        assert_eq!(peer_key_hex(&npub.to_uppercase()), Some(expected));
        let invalid_curve_key = PublicKey::from_byte_array([0xff; 32]).to_bech32().unwrap();
        for invalid in [
            "",
            "npub1invalid",
            "nsec1invalid",
            invalid_curve_key.as_str(),
        ] {
            assert!(peer_key_hex(invalid).is_none(), "invalid peer key accepted");
        }
    }

    async fn local_endpoint(rendezvous: SocketAddrV4) -> Arc<FipsEndpoint> {
        let mut config = Config::new();
        config.node.control.enabled = false;
        config.node.discovery.local.rendezvous_addr = rendezvous;
        config.node.discovery.local.retry_interval_ms = 20;
        config.node.discovery.lan.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
        Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .local_rendezvous()
                .without_system_tun()
                .bind()
                .await
                .expect("bind local FIPS endpoint"),
        )
    }

    async fn next_snapshot(
        receiver: &flume::Receiver<CoreMsg>,
    ) -> (u64, Vec<FipsNearbyLinkSnapshot>) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let message = tokio::time::timeout_at(deadline, receiver.recv_async())
                .await
                .expect("connection monitor update timed out")
                .expect("connection monitor channel closed");
            let CoreMsg::Internal(event) = message else {
                panic!("expected internal connection update");
            };
            match *event {
                InternalEvent::FipsNearbyPeersChanged { generation, peers } => {
                    return (generation, peers);
                }
                InternalEvent::DebugLog { .. } => {}
                _ => panic!("expected FIPS connection snapshot"),
            }
        }
    }

    #[tokio::test]
    async fn unacknowledged_nearby_event_retries_without_reconnecting() {
        let SocketAddr::V4(rendezvous) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
        else {
            panic!("expected IPv4 rendezvous");
        };
        let endpoint = local_endpoint(rendezvous).await;
        let sibling = local_endpoint(rendezvous).await;
        let receiver = sibling
            .register_service_receiver(FIPS_NEARBY_PORT)
            .await
            .unwrap();
        let event_id = "ab".repeat(32);
        let payload = b"receipt that was lost during peer restart".to_vec();
        let outbox = Arc::new(RwLock::new(FipsNearbyOutbox::default()));
        outbox
            .write()
            .unwrap()
            .insert(event_id.clone(), payload.clone());
        let (sender, _updates) = flume::unbounded();
        let monitor = tokio::spawn(run(
            endpoint.clone(),
            Arc::new(RwLock::new(Vec::new())),
            outbox.clone(),
            sender,
            1,
            true,
        ));
        let mut datagrams = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            receiver.recv_batch_into(&mut datagrams, 8),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(datagrams
            .iter()
            .any(|datagram| datagram.data.as_ref() == payload.as_slice()));
        let first_link = endpoint
            .peers()
            .await
            .unwrap()
            .into_iter()
            .find(|peer| peer.npub == sibling.npub())
            .unwrap()
            .link_id;
        // The send was accepted, but the receiver deliberately drops it before
        // acknowledgement. Recovery must not depend on identity changes or a new link.
        datagrams.clear();
        tokio::time::timeout(
            Duration::from_secs(6),
            receiver.recv_batch_into(&mut datagrams, 8),
        )
        .await
        .expect("lost nearby event was never retried")
        .unwrap();
        assert!(datagrams
            .iter()
            .any(|datagram| datagram.data.as_ref() == payload.as_slice()));
        let retry_link = endpoint
            .peers()
            .await
            .unwrap()
            .into_iter()
            .find(|peer| peer.npub == sibling.npub())
            .unwrap()
            .link_id;
        assert_eq!(retry_link, first_link);
        outbox
            .write()
            .unwrap()
            .acknowledge(sibling.npub(), &event_id);
        datagrams.clear();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                receiver.recv_batch_into(&mut datagrams, 8)
            )
            .await
            .is_err(),
            "acknowledged events must stop retrying"
        );
        monitor.abort();
        endpoint.shutdown().await.unwrap();
        sibling.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn authenticated_links_report_with_nearby_disabled_and_clear_on_shutdown() {
        let SocketAddr::V4(rendezvous) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .expect("reserve local rendezvous address")
            .local_addr()
            .expect("reserved rendezvous address")
        else {
            panic!("IPv4 loopback bind returned IPv6");
        };
        let endpoint = local_endpoint(rendezvous).await;
        let sibling = local_endpoint(rendezvous).await;
        let sibling_hex = FipsPeerIdentity::from_npub(sibling.npub())
            .expect("sibling identity")
            .pubkey()
            .to_string();
        let nearby_receiver = sibling
            .register_service_receiver(FIPS_NEARBY_PORT)
            .await
            .expect("register sibling Nearby receiver");
        let bootstrap = Arc::new(RwLock::new(vec![b"nearby bootstrap".to_vec()]));
        let mut outbox = FipsNearbyOutbox::default();
        outbox.insert("queued".to_string(), b"nearby outbox".to_vec());
        let private = EventBuilder::new(Kind::Custom(1060), "intended private delivery")
            .tags([
                nostr::Tag::parse(["v", "2"]).unwrap(),
                nostr::Tag::public_key(PublicKey::from_hex(&sibling_hex).unwrap()),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let private_payload = crate::core::fips_nearby::encode_fips_nearby_event(&private).unwrap();
        outbox.insert(private.id.to_hex(), private_payload.clone());
        let elsewhere = EventBuilder::new(Kind::Custom(1060), "another recipient")
            .tags([
                nostr::Tag::parse(["v", "2"]).unwrap(),
                nostr::Tag::public_key(Keys::generate().public_key()),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        outbox.insert(
            elsewhere.id.to_hex(),
            crate::core::fips_nearby::encode_fips_nearby_event(&elsewhere).unwrap(),
        );
        let outbox = Arc::new(RwLock::new(outbox));
        let (sender, receiver) = flume::unbounded();
        let monitor = tokio::spawn(run(
            endpoint.clone(),
            bootstrap,
            outbox.clone(),
            sender,
            17,
            false,
        ));

        let (generation, connected) = next_snapshot(&receiver).await;
        assert_eq!(generation, 17);
        assert!(connected
            .iter()
            .any(|peer| peer.device_pubkey_hex == sibling_hex));
        let mut nearby_datagrams = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            nearby_receiver.recv_batch_into(&mut nearby_datagrams, 8),
        )
        .await
        .expect("exact-recipient private delivery must remain available")
        .unwrap();
        assert!(!nearby_datagrams.is_empty());
        assert!(
            nearby_datagrams
                .iter()
                .all(|packet| packet.data.as_ref() == private_payload.as_slice()),
            "Nearby disabled must suppress public bootstrap/backlog and other recipients"
        );
        outbox
            .write()
            .unwrap()
            .acknowledge(sibling.npub(), &private.id.to_hex());
        nearby_datagrams.clear();
        assert!(tokio::time::timeout(
            Duration::from_millis(250),
            nearby_receiver.recv_batch_into(&mut nearby_datagrams, 8)
        )
        .await
        .is_err());

        endpoint
            .shutdown()
            .await
            .expect("shutdown observed endpoint");
        let (generation, disconnected) = next_snapshot(&receiver).await;
        assert_eq!(generation, 17);
        assert!(
            disconnected.is_empty(),
            "shutdown must clear connected devices"
        );
        tokio::time::timeout(Duration::from_secs(10), monitor)
            .await
            .expect("connection monitor did not stop")
            .expect("connection monitor panicked");
        sibling.shutdown().await.expect("shutdown sibling endpoint");
    }
}
