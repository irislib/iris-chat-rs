use super::*;
use crate::core::fips_nearby::{FipsNearbyOutbox, FIPS_NEARBY_PORT};
use crate::updates::FipsNearbyLinkSnapshot;
use fips_core::FipsEndpointOutboundDatagram;

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
    loop {
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
        let mut current_links = peers
            .iter()
            .filter(|peer| peer.connected)
            .filter_map(|peer| {
                let identity = FipsPeerIdentity::from_npub(&peer.npub).ok()?;
                Some(FipsNearbyLinkSnapshot {
                    device_pubkey_hex: identity.pubkey().to_string(),
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
        for peer in peers
            .into_iter()
            .filter(|peer| nearby_enabled && peer.connected)
        {
            let Ok(identity) = FipsPeerIdentity::from_npub(&peer.npub) else {
                continue;
            };
            let initialization = (peer.link_id, bootstrap_revision);
            let initialized = if initialized_links.get(&peer.npub) == Some(&initialization) {
                true
            } else {
                let payloads = current_bootstrap.clone();
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
                .map(|outbox| outbox.pending_for_link(&peer.npub, peer.link_id))
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
