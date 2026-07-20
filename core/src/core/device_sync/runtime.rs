use super::*;
use crate::core::update_pubsub::{
    run_relay_update_announcement_subscription, run_update_announcement_subscription,
};
use fips_core::config::{
    BleConfig, NostrDiscoveryPolicy, PeerConfig, TransportInstances, UdpConfig, WebSocketConfig,
};
use fips_core::{Config, FipsEndpointOutboundDatagram, WebRtcConfig};
use hashtree_core::BlobRoute;
use std::collections::BTreeMap;
use std::net::SocketAddrV4;

const SAME_HOST_HASHTREE_ENV: &str = "IRIS_CHAT_SAME_HOST_HASHTREE";
const LOCAL_RENDEZVOUS_ADDR_ENV: &str = "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR";
const WEBSOCKET_SEED_URLS_ENV: &str = "IRIS_FIPS_WEBSOCKET_SEED_URLS";
const RECENT_PEERS_FILE_NAME: &str = "fips-recent-peers.json";
const RECENT_PEERS_OBSERVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_WEBSOCKET_SEED_URLS: &[&str] = &[
    "wss://fips2.iris.to/fips", // osiris
    "wss://fips1.iris.to/fips", // lnvps
];

struct SharedFipsOptions {
    same_host_hashtree: bool,
    rendezvous_addr: Option<SocketAddrV4>,
    standalone_route: Option<Arc<dyn BlobRoute>>,
    additional_peers: Vec<PeerConfig>,
    websocket: Option<WebSocketConfig>,
}

impl AppCore {
    pub(in crate::core) fn reconcile_device_sync(&mut self) {
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host_hashtree_enabled(),
            rendezvous_addr: None,
            standalone_route: None,
            additional_peers: Vec::new(),
            websocket: configured_websocket_seeds(),
        });
    }

    #[cfg(test)]
    pub(crate) fn reconcile_device_sync_with_websocket_for_test(
        &mut self,
        websocket: WebSocketConfig,
    ) {
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: false,
            rendezvous_addr: None,
            standalone_route: None,
            additional_peers: Vec::new(),
            websocket: Some(websocket),
        });
    }

    #[cfg(test)]
    pub(crate) fn reconcile_same_host_hashtree_for_test(
        &mut self,
        rendezvous_addr: SocketAddrV4,
        standalone_route: Arc<dyn BlobRoute>,
        additional_peers: Vec<PeerConfig>,
    ) {
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: true,
            rendezvous_addr: Some(rendezvous_addr),
            standalone_route: Some(standalone_route),
            additional_peers,
            websocket: None,
        });
    }

    #[cfg(test)]
    pub(in crate::core) fn same_host_runtime_for_test(
        &self,
    ) -> Option<(
        Arc<FipsEndpoint>,
        bool,
        usize,
        Arc<super::super::attachment_upload::AttachmentBlobRuntime>,
    )> {
        let runtime = self.device_sync.as_ref()?;
        Some((
            runtime.endpoint.clone(),
            runtime.tcp.is_some(),
            runtime.siblings.len(),
            runtime._attachment_blobs.as_ref()?.clone(),
        ))
    }

    fn reconcile_shared_fips(&mut self, options: SharedFipsOptions) {
        let host_ble_requested = self.pending_host_ble.is_some() || self.host_ble_attached;
        let (config, device_sync_enabled) = match self.device_sync_config() {
            Some(config) => {
                let device_sync_enabled =
                    !config.siblings.is_empty() && !config.relay_urls.is_empty();
                (config, device_sync_enabled)
            }
            None if options.same_host_hashtree => {
                let Some(config) = self.same_host_endpoint_config() else {
                    self.stop_device_sync();
                    return;
                };
                (config, false)
            }
            None => {
                self.stop_device_sync();
                return;
            }
        };
        let nearby_enabled = host_ble_requested || config.nearby_ip_enabled;
        let discovery_scope = if nearby_enabled {
            super::super::fips_nearby::FIPS_NEARBY_SCOPE.to_string()
        } else {
            format!("{DEVICE_SYNC_SCOPE_PREFIX}{}", config.owner_hex)
        };
        let runtime_key = format!(
            "{}:same-host={}:nearby={}:ble={}",
            config.key, options.same_host_hashtree, nearby_enabled, host_ble_requested
        );
        let refreshed_bootstrap = self
            .host_ble_attached
            .then(|| self.local_fips_nearby_bootstrap_payloads());
        if self.host_ble_attached {
            if let Some(runtime) = self.device_sync.as_mut() {
                runtime.key = runtime_key;
                runtime.siblings = config.siblings.clone();
                if let Ok(mut payloads) = runtime.nearby_bootstrap_payloads.write() {
                    *payloads = refreshed_bootstrap.unwrap_or_default();
                }
                let endpoint = runtime.endpoint.clone();
                let mut peer_config = config
                    .peers
                    .iter()
                    .map(|peer| PeerConfig {
                        npub: peer.npub(),
                        ..PeerConfig::default()
                    })
                    .collect::<Vec<_>>();
                if let Some(recent_peers) = &runtime.recent_peers {
                    if let Ok(recent_peers) = recent_peers.read() {
                        recent_peers.merge_into(&mut peer_config);
                    }
                }
                self.runtime.spawn(async move {
                    let _ = endpoint.update_peers(peer_config).await;
                });
                return;
            }
        }
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.key == runtime_key)
        {
            return;
        }
        if self.pending_host_ble.is_some() {
            self.stop_device_sync_now();
        } else {
            self.stop_device_sync();
        }

        let mut peer_config = config
            .peers
            .iter()
            .map(|peer| PeerConfig {
                npub: peer.npub(),
                ..PeerConfig::default()
            })
            .collect::<Vec<_>>();
        peer_config.extend(options.additional_peers);
        let recent_peers = match DeviceSyncRecentPeers::load(
            self.data_dir.join(RECENT_PEERS_FILE_NAME),
            &config.local_npub,
            &discovery_scope,
            crate::perflog::now_ms(),
        ) {
            Ok((recent_peers, warning)) => {
                if let Some(warning) = warning {
                    self.push_debug_log("fips.recent_peers.load.error", warning);
                }
                Some(Arc::new(RwLock::new(recent_peers)))
            }
            Err(error) => {
                self.push_debug_log("fips.recent_peers.init.error", error);
                None
            }
        };
        if let Some(recent_peers) = &recent_peers {
            if let Ok(recent_peers) = recent_peers.read() {
                recent_peers.merge_into(&mut peer_config);
            }
        }

        let mut fips_config = Config::new();
        fips_config.node.control.enabled = false;
        fips_config.peers = peer_config;
        let nostr_network_enabled = device_sync_enabled || config.nearby_ip_enabled;
        if nostr_network_enabled {
            fips_config.node.discovery.nostr.enabled = true;
            fips_config.node.discovery.nostr.advertise = true;
            fips_config.node.discovery.nostr.advert_relays = config.relay_urls.clone();
            fips_config.node.discovery.nostr.app = discovery_scope.clone();
            // Nearby is an app-scoped, open service: compatible Iris peers are not
            // necessarily in our device roster yet. FIPS still bounds open discovery
            // and authenticates every connection with Noise.
            fips_config.node.discovery.nostr.policy = if config.nearby_ip_enabled {
                NostrDiscoveryPolicy::Open
            } else {
                NostrDiscoveryPolicy::ConfiguredOnly
            };
            fips_config.transports.webrtc = TransportInstances::Single(WebRtcConfig {
                advertise_on_nostr: Some(true),
                auto_connect: Some(true),
                accept_connections: Some(true),
                ..WebRtcConfig::default()
            });
        } else {
            fips_config.node.discovery.nostr.enabled = false;
            fips_config.node.discovery.nostr.advertise = false;
        }
        configure_fips_lan(&mut fips_config, config.nearby_ip_enabled);
        let rendezvous_addr = match (options.same_host_hashtree, options.rendezvous_addr) {
            (_, Some(address)) => Some(address),
            (true, None) => match configured_local_rendezvous_addr() {
                Ok(address) => address,
                Err(error) => {
                    self.push_debug_log("attachment.same_host.endpoint.error", error);
                    return;
                }
            },
            (false, None) => None,
        };
        if let Some(rendezvous_addr) = rendezvous_addr {
            fips_config.node.discovery.local.rendezvous_addr = rendezvous_addr;
        }
        if let Some(websocket) = options.websocket {
            fips_config.transports.websocket = TransportInstances::Single(websocket);
        }

        let device_sync_packets = if device_sync_enabled {
            let request = serde_json::to_vec(&DeviceSyncPacket::Request {
                v: DEVICE_SYNC_VERSION,
                roster_at: config.roster_at,
                page: None,
            });
            let resync_required = serde_json::to_vec(&DeviceSyncPacket::ResyncRequired {
                v: DEVICE_SYNC_VERSION,
            });
            match (request, resync_required) {
                (Ok(request), Ok(resync_required)) => Some((request, resync_required)),
                _ => return,
            }
        } else {
            None
        };

        let mut builder = FipsEndpoint::builder()
            .config(fips_config)
            .identity_nsec(config.secret_hex)
            .discovery_scope(discovery_scope)
            .without_system_tun();
        if options.same_host_hashtree {
            builder = builder.local_rendezvous();
        }
        let attaching_ble = self.pending_host_ble.is_some();
        if let Some(mut attachment) = self.pending_host_ble.take() {
            let Some(io) = attachment.take() else {
                self.push_debug_log(
                    "fips_ble.start.error",
                    "BLE attachment was empty".to_string(),
                );
                return;
            };
            builder = builder.host_ble(
                io,
                BleConfig {
                    adapter: Some("mobile".to_string()),
                    auto_connect: Some(true),
                    ..BleConfig::default()
                },
            );
        }
        let endpoint = match self.runtime.block_on(builder.bind()) {
            Ok(endpoint) => Arc::new(endpoint),
            Err(error) => {
                let event = if device_sync_enabled {
                    "device_sync.start.error"
                } else {
                    "attachment.same_host.endpoint.error"
                };
                self.push_debug_log(event, error.to_string());
                return;
            }
        };
        self.host_ble_attached = attaching_ble;
        let nearby_receiver = if nearby_enabled {
            match self.runtime.block_on(
                endpoint.register_service_receiver(super::super::fips_nearby::FIPS_NEARBY_PORT),
            ) {
                Ok(receiver) => Some(receiver),
                Err(error) => {
                    self.push_debug_log("fips_nearby.start.error", error.to_string());
                    let _ = self.runtime.block_on(endpoint.shutdown());
                    self.host_ble_attached = false;
                    return;
                }
            }
        } else {
            None
        };
        let (tcp, update_pubsub, update_relay_pubsub, mut tasks) = if device_sync_enabled {
            let Some((request, resync_required)) = device_sync_packets else {
                let _ = self.runtime.block_on(endpoint.shutdown());
                return;
            };
            let (tcp, tcp_task) = match self.runtime.block_on(start_device_sync_tcp(
                endpoint.clone(),
                config.siblings.iter().map(|peer| peer.npub()).collect(),
                DEVICE_SYNC_PORT,
                DEVICE_SYNC_MAX_PACKET_BYTES,
                request,
                resync_required,
                self.core_sender.clone(),
            )) {
                Ok(value) => value,
                Err(error) => {
                    self.push_debug_log("device_sync.tcp.start.error", error);
                    let _ = self.runtime.block_on(endpoint.shutdown());
                    return;
                }
            };
            let update_pubsub = match self.runtime.block_on(FipsPubsubClient::start(
                endpoint.clone(),
                FipsPubsubClientOptions::default(),
            )) {
                Ok(client) => Some(Arc::new(client)),
                Err(error) => {
                    self.push_debug_log("update.pubsub.start.error", error.to_string());
                    None
                }
            };
            let update_relay_pubsub = config.relay_client.clone().and_then(|client| {
                match self.runtime.block_on(RelayEventBus::with_client(
                    client,
                    config.relay_urls.clone(),
                    Duration::from_secs(8),
                )) {
                    Ok(pubsub) => Some(Arc::new(pubsub)),
                    Err(error) => {
                        self.push_debug_log("update.pubsub.relay.start.error", error.to_string());
                        None
                    }
                }
            });
            let update_filter = match crate::update_announcements::update_announcement_filter() {
                Ok(filter) => Some(filter),
                Err(error) => {
                    self.push_debug_log("update.pubsub.filter.error", error.to_string());
                    None
                }
            };
            let mut tasks = vec![tcp_task];
            if let (Some(pubsub), Some(filter)) = (&update_pubsub, &update_filter) {
                let pubsub = pubsub.clone();
                let endpoint = endpoint.clone();
                let filter = filter.clone();
                tasks.push(self.runtime.spawn(async move {
                    run_update_announcement_subscription(endpoint, pubsub, filter).await;
                }));
            }
            if let (Some(pubsub), Some(filter)) = (&update_relay_pubsub, &update_filter) {
                let pubsub = pubsub.clone();
                let filter = filter.clone();
                tasks.push(self.runtime.spawn(async move {
                    run_relay_update_announcement_subscription(pubsub, filter).await;
                }));
            }
            (Some(tcp), update_pubsub, update_relay_pubsub, tasks)
        } else {
            (None, None, None, Vec::new())
        };

        let attachment_store = if options.same_host_hashtree {
            let result = match options.standalone_route {
                Some(route) => self.runtime.block_on(
                    super::super::attachment_upload::bind_same_host_attachment_store(
                        endpoint.clone(),
                        route,
                    ),
                ),
                None => self.runtime.block_on(
                    super::super::attachment_upload::start_same_host_attachment_reuse(
                        endpoint.clone(),
                    ),
                ),
            };
            match result {
                Ok(store) => Some(store),
                Err(error) => {
                    self.push_debug_log("attachment.same_host.start.error", error.to_string());
                    if !device_sync_enabled {
                        let _ = self.runtime.block_on(endpoint.shutdown());
                        return;
                    }
                    None
                }
            }
        } else {
            None
        };

        let nearby_bootstrap_payloads = Arc::new(RwLock::new(if nearby_enabled {
            self.local_fips_nearby_bootstrap_payloads()
        } else {
            Vec::new()
        }));
        let mut initial_nearby_outbox = super::super::fips_nearby::FipsNearbyOutbox::default();
        if nearby_enabled {
            for event in self
                .pending_relay_publishes
                .values()
                .rev()
                .take(super::super::fips_nearby::FIPS_NEARBY_OUTBOX_MAX_EVENTS)
                .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
            {
                if let Some(payload) = super::super::fips_nearby::encode_fips_nearby_event(&event) {
                    initial_nearby_outbox.insert(event.id.to_string(), payload);
                }
            }
        }
        let nearby_outbox = Arc::new(RwLock::new(initial_nearby_outbox));
        if let Some(receiver) = nearby_receiver {
            let nearby_tx = self.core_sender.clone();
            tasks.push(self.runtime.spawn(async move {
                let mut datagrams = Vec::with_capacity(32);
                while receiver.recv_batch_into(&mut datagrams, 32).await.is_some() {
                    for datagram in datagrams.drain(..) {
                        let _ = nearby_tx.send(CoreMsg::Internal(Box::new(
                            InternalEvent::FipsNearbyPacket {
                                source_pubkey_hex: datagram.source_peer.pubkey().to_string(),
                                source_port: datagram.source_port,
                                data: datagram.data.into_vec(),
                            },
                        )));
                    }
                }
            }));
        }
        if nearby_enabled {
            tasks.push(self.runtime.spawn(run_fips_nearby_link_monitor(
                endpoint.clone(),
                nearby_bootstrap_payloads.clone(),
                nearby_outbox.clone(),
                self.core_sender.clone(),
            )));
        }
        if let Some(recent_peers) = &recent_peers {
            tasks.push(self.runtime.spawn(run_recent_peer_observer(
                endpoint.clone(),
                recent_peers.clone(),
            )));
        }

        let sibling_count = config.siblings.len();
        self.device_sync = Some(DeviceSyncRuntime {
            key: runtime_key,
            endpoint,
            tcp,
            siblings: config.siblings,
            nearby_bootstrap_payloads,
            nearby_outbox,
            _attachment_blobs: attachment_store,
            _update_pubsub: update_pubsub,
            _update_relay_pubsub: update_relay_pubsub,
            recent_peers,
            tasks,
        });
        if device_sync_enabled {
            self.push_debug_log("device_sync.start", format!("peers={sibling_count}"));
        } else if nearby_enabled {
            self.push_debug_log("fips_nearby.start", "nearby-only");
        } else {
            self.push_debug_log("attachment.same_host.start", "local-only");
        }
    }

    pub(in crate::core) fn stop_device_sync(&mut self) {
        self.host_ble_attached = false;
        let Some(runtime) = self.device_sync.take() else {
            return;
        };
        let DeviceSyncRuntime {
            endpoint,
            recent_peers,
            tasks,
            ..
        } = runtime;
        for task in tasks {
            task.abort();
        }
        self.runtime.spawn(async move {
            shutdown_shared_fips(endpoint, recent_peers).await;
        });
    }

    pub(in crate::core) fn stop_device_sync_now(&mut self) {
        self.host_ble_attached = false;
        let Some(runtime) = self.device_sync.take() else {
            return;
        };
        let DeviceSyncRuntime {
            endpoint,
            recent_peers,
            tasks,
            ..
        } = runtime;
        for task in tasks {
            task.abort();
        }
        self.runtime
            .block_on(shutdown_shared_fips(endpoint, recent_peers));
    }

    #[cfg(test)]
    pub(in crate::core) fn device_sync_endpoint_for_test(&self) -> Option<Arc<FipsEndpoint>> {
        self.device_sync
            .as_ref()
            .map(|runtime| runtime.endpoint.clone())
    }

    fn same_host_endpoint_config(&self) -> Option<DeviceSyncConfig> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_hex = logged_in.owner_pubkey.to_hex();
        let device_hex = logged_in.device_keys.public_key().to_hex();
        let local_npub = fips_peer_from_hex(&device_hex)?.npub();
        Some(DeviceSyncConfig {
            key: format!("{owner_hex}:{device_hex}:local-only"),
            owner_hex,
            local_npub,
            roster_at: 0,
            secret_hex: logged_in.device_keys.secret_key().to_secret_hex(),
            relay_urls: Vec::new(),
            relay_client: None,
            siblings: Vec::new(),
            peers: Vec::new(),
            nearby_ip_enabled: false,
        })
    }

    fn device_sync_config(&self) -> Option<DeviceSyncConfig> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_hex = logged_in.owner_pubkey.to_hex();
        let local_hex = logged_in.device_keys.public_key().to_hex();
        let local_npub = fips_peer_from_hex(&local_hex)?.npub();
        let roster = self.app_keys.get(&owner_hex);
        let roster_at = roster
            .filter(|roster| {
                roster.created_at_secs > 0
                    && roster
                        .devices
                        .iter()
                        .any(|device| device.identity_pubkey_hex == local_hex)
            })
            .map(|roster| roster.created_at_secs)
            .unwrap_or_default();
        let siblings = roster
            .into_iter()
            .flat_map(|roster| roster.devices.iter())
            .filter(|device| device.identity_pubkey_hex != local_hex)
            .filter_map(|device| fips_peer_from_hex(&device.identity_pubkey_hex))
            .collect::<Vec<_>>();
        let mut peer_by_npub = BTreeMap::new();
        for peer in self
            .app_keys
            .values()
            .flat_map(|known| known.devices.iter())
            .filter(|device| device.identity_pubkey_hex != local_hex)
            .filter_map(|device| fips_peer_from_hex(&device.identity_pubkey_hex))
        {
            peer_by_npub.insert(peer.npub(), peer);
        }
        for peer in &siblings {
            peer_by_npub.insert(peer.npub(), *peer);
        }
        let peers = peer_by_npub.into_values().collect::<Vec<_>>();
        let relay_urls = logged_in
            .relay_urls
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let ble_requested = self.pending_host_ble.is_some() || self.host_ble_attached;
        let nearby_ip_enabled =
            self.preferences.nearby_enabled && self.preferences.nearby_lan_enabled;
        if siblings.is_empty() && !ble_requested && !nearby_ip_enabled {
            return None;
        }
        if relay_urls.is_empty() && !ble_requested && !nearby_ip_enabled {
            return None;
        }
        let key = format!(
            "{}:{}:{}:{}:{}:{}",
            owner_hex,
            roster_at,
            peers
                .iter()
                .map(FipsPeerIdentity::npub)
                .collect::<Vec<_>>()
                .join(","),
            relay_urls.join(","),
            ble_requested,
            nearby_ip_enabled
        );
        Some(DeviceSyncConfig {
            key,
            owner_hex,
            local_npub,
            roster_at,
            secret_hex: logged_in.device_keys.secret_key().to_secret_hex(),
            relay_urls,
            relay_client: Some(logged_in.client.clone()),
            siblings,
            peers,
            nearby_ip_enabled,
        })
    }
}

fn configure_fips_lan(config: &mut Config, enabled: bool) {
    config.node.discovery.lan.enabled = enabled;
    config.transports.udp = if enabled {
        TransportInstances::Single(UdpConfig {
            bind_addr: Some("0.0.0.0:0".to_string()),
            advertise_on_nostr: Some(false),
            public: Some(false),
            outbound_only: Some(false),
            accept_connections: Some(true),
            ..UdpConfig::default()
        })
    } else {
        TransportInstances::default()
    };
}

async fn run_fips_nearby_link_monitor(
    endpoint: Arc<FipsEndpoint>,
    bootstrap_payloads: Arc<RwLock<Vec<Vec<u8>>>>,
    outbox: Arc<RwLock<super::super::fips_nearby::FipsNearbyOutbox>>,
    core_sender: Sender<CoreMsg>,
) {
    let mut initialized_links = BTreeMap::<String, u64>::new();
    let mut reported_links = Vec::new();
    loop {
        let peers = match endpoint.peers().await {
            Ok(peers) => peers,
            Err(_) => return,
        };
        let mut current_links = peers
            .iter()
            .filter(|peer| peer.connected)
            .filter_map(|peer| {
                let identity = FipsPeerIdentity::from_npub(&peer.npub).ok()?;
                Some(crate::updates::FipsNearbyLinkSnapshot {
                    device_pubkey_hex: identity.pubkey().to_string(),
                    transport_type: peer.transport_type.clone().unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        current_links.sort_by(|left, right| {
            left.device_pubkey_hex
                .cmp(&right.device_pubkey_hex)
                .then_with(|| left.transport_type.cmp(&right.transport_type))
        });
        if current_links != reported_links {
            reported_links = current_links.clone();
            let _ = core_sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::FipsNearbyPeersChanged(current_links),
            )));
        }
        for peer in peers.into_iter().filter(|peer| peer.connected) {
            let Ok(identity) = FipsPeerIdentity::from_npub(&peer.npub) else {
                continue;
            };
            let initialized = if initialized_links.get(&peer.npub) == Some(&peer.link_id) {
                true
            } else {
                let payloads = bootstrap_payloads
                    .read()
                    .map(|payloads| payloads.clone())
                    .unwrap_or_default();
                let sent = if payloads.is_empty() {
                    true
                } else {
                    let datagrams = payloads
                        .into_iter()
                        .map(|data| {
                            FipsEndpointOutboundDatagram::new(
                                super::super::fips_nearby::FIPS_NEARBY_PORT,
                                super::super::fips_nearby::FIPS_NEARBY_PORT,
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
                    initialized_links.insert(peer.npub.clone(), peer.link_id);
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
                    FipsEndpointOutboundDatagram::new(
                        super::super::fips_nearby::FIPS_NEARBY_PORT,
                        super::super::fips_nearby::FIPS_NEARBY_PORT,
                        data,
                    )
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

async fn run_recent_peer_observer(
    endpoint: Arc<FipsEndpoint>,
    recent_peers: Arc<RwLock<DeviceSyncRecentPeers>>,
) {
    loop {
        let peers = match endpoint.peers().await {
            Ok(peers) => peers,
            Err(_) => return,
        };
        if let Ok(mut recent_peers) = recent_peers.write() {
            if let Err(error) =
                recent_peers.observe_and_flush_if_due(&peers, crate::perflog::now_ms())
            {
                crate::perflog!("fips_recent_peers.observe error={error}");
            }
        }
        sleep(RECENT_PEERS_OBSERVE_INTERVAL).await;
    }
}

async fn shutdown_shared_fips(
    endpoint: Arc<FipsEndpoint>,
    recent_peers: Option<Arc<RwLock<DeviceSyncRecentPeers>>>,
) {
    if let (Some(recent_peers), Ok(peers)) = (recent_peers, endpoint.peers().await) {
        if let Ok(mut recent_peers) = recent_peers.write() {
            if let Err(error) = recent_peers.observe_and_flush(&peers, crate::perflog::now_ms()) {
                crate::perflog!("fips_recent_peers.shutdown error={error}");
            }
        }
    }
    let _ = endpoint.shutdown().await;
}

fn same_host_hashtree_enabled() -> bool {
    std::env::var(SAME_HOST_HASHTREE_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn configured_websocket_seeds() -> Option<WebSocketConfig> {
    let configured = std::env::var(WEBSOCKET_SEED_URLS_ENV).ok();
    let seed_urls = websocket_seed_urls(configured.as_deref());
    (!seed_urls.is_empty()).then_some(WebSocketConfig {
        seed_urls,
        ..WebSocketConfig::default()
    })
}

fn websocket_seed_urls(configured: Option<&str>) -> Vec<String> {
    configured
        .map(|value| value.split(',').collect::<Vec<_>>())
        .unwrap_or_else(|| DEFAULT_WEBSOCKET_SEED_URLS.to_vec())
        .into_iter()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn configured_local_rendezvous_addr() -> Result<Option<SocketAddrV4>, String> {
    let Some(value) = std::env::var(LOCAL_RENDEZVOUS_ADDR_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    parse_local_rendezvous_addr(&value).map(Some)
}

fn parse_local_rendezvous_addr(value: &str) -> Result<SocketAddrV4, String> {
    let address = value.trim().parse::<SocketAddrV4>().map_err(|error| {
        format!("{LOCAL_RENDEZVOUS_ADDR_ENV} must be an IPv4 loopback address: {error}")
    })?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(format!(
            "{LOCAL_RENDEZVOUS_ADDR_ENV} must be a non-zero IPv4 loopback address"
        ));
    }
    Ok(address)
}

#[cfg(test)]
mod local_rendezvous_tests {
    use super::*;

    #[test]
    fn websocket_seeds_default_to_osiris_then_lnvps() {
        assert_eq!(
            websocket_seed_urls(None),
            vec![
                "wss://fips2.iris.to/fips".to_string(),
                "wss://fips1.iris.to/fips".to_string(),
            ]
        );
    }

    #[test]
    fn websocket_seed_override_can_replace_or_disable_defaults() {
        assert_eq!(
            websocket_seed_urls(Some(" wss://one.example/fips, wss://two.example/fips ")),
            vec![
                "wss://one.example/fips".to_string(),
                "wss://two.example/fips".to_string(),
            ]
        );
        assert!(websocket_seed_urls(Some("  ")).is_empty());
    }

    #[test]
    fn local_rendezvous_override_requires_nonzero_ipv4_loopback() {
        assert_eq!(
            parse_local_rendezvous_addr("127.0.0.1:32112").unwrap(),
            "127.0.0.1:32112".parse::<SocketAddrV4>().unwrap()
        );
        assert!(parse_local_rendezvous_addr("0.0.0.0:32112").is_err());
        assert!(parse_local_rendezvous_addr("127.0.0.1:0").is_err());
        assert!(parse_local_rendezvous_addr("[::1]:32112").is_err());
    }

    #[test]
    fn fips_lan_uses_scoped_bidirectional_ephemeral_udp() {
        let mut config = Config::new();
        configure_fips_lan(&mut config, true);

        assert!(config.node.discovery.lan.enabled);
        let TransportInstances::Single(udp) = config.transports.udp else {
            panic!("expected one UDP transport");
        };
        assert_eq!(udp.bind_addr.as_deref(), Some("0.0.0.0:0"));
        assert_eq!(udp.advertise_on_nostr, Some(false));
        assert_eq!(udp.public, Some(false));
        assert_eq!(udp.outbound_only, Some(false));
        assert_eq!(udp.accept_connections, Some(true));
    }

    #[test]
    fn disabling_fips_lan_removes_udp_transport() {
        let mut config = Config::new();
        configure_fips_lan(&mut config, true);
        configure_fips_lan(&mut config, false);

        assert!(!config.node.discovery.lan.enabled);
        assert!(config.transports.udp.is_empty());
    }

    #[test]
    fn fips_lan_can_start_before_any_remote_peer_is_known() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            temp_dir.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.create_account("LAN discovery");
        core.preferences.nearby_enabled = true;
        core.preferences.nearby_lan_enabled = true;

        let config = core.device_sync_config().expect("nearby-only config");

        assert!(config.peers.is_empty());
        assert!(config.nearby_ip_enabled);
    }
}
