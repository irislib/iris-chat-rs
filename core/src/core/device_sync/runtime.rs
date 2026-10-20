use super::*;
use crate::update_announcements::{
    secure_update_ref, trusted_update_publisher, websocket_seed_urls,
};
use fips_core::config::{
    BleConfig, NostrDiscoveryPolicy, NostrPeerfindingSource, PeerConfig, TransportInstances,
    UdpConfig, WebSocketConfig,
};
use fips_core::{Config, WebRtcConfig};
use hashtree_core::BlobRoute;
use std::collections::BTreeMap;
use std::net::SocketAddrV4;

mod connection_monitor;
mod peer_snapshot;
mod private_provider;
const SAME_HOST_HASHTREE_ENV: &str = "IRIS_CHAT_SAME_HOST_HASHTREE";
const LOCAL_RENDEZVOUS_ADDR_ENV: &str = "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR";
const WEBSOCKET_SEED_URLS_ENV: &str = "IRIS_FIPS_WEBSOCKET_SEED_URLS";
const RECENT_PEERS_FILE_NAME: &str = "fips-recent-peers.json";
const RECENT_PEERS_OBSERVE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct SharedFipsOptions {
    same_host_hashtree: bool,
    rendezvous_addr: Option<SocketAddrV4>,
    standalone_route: Option<Arc<dyn BlobRoute>>,
    additional_peers: Vec<PeerConfig>,
    websocket: Option<WebSocketConfig>,
    routed_peers: Vec<FipsPeerIdentity>,
    udp_bind_addr: Option<String>,
}

fn configured_direct_peer_ids(peers: &[PeerConfig]) -> std::collections::BTreeSet<String> {
    peers.iter().map(|peer| peer.npub.clone()).collect()
}

impl AppCore {
    #[cfg(test)]
    pub(super) fn reconcile_update_mesh_for_test(
        &mut self,
        same_host: bool,
        rendezvous: SocketAddrV4,
    ) {
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host,
            rendezvous_addr: Some(rendezvous),
            websocket: Some(WebSocketConfig::default()),
            ..SharedFipsOptions::default()
        });
    }

    pub(in crate::core) fn reconcile_device_sync(&mut self) {
        if self.apply_current_device_labels_to_local_app_keys(false) {
            self.persist_best_effort();
        }
        #[cfg(test)]
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.key == "test")
        {
            // The in-memory wire harness owns this transport. A peer roster
            // import must not replace it with real networking halfway through sync.
            return;
        }
        #[cfg(test)]
        if let Some((local, peer, identity)) = self.test_fips_udp.clone() {
            // History can now reach a sibling through the caller. Reconciliation
            // must retain this fixture's transport, just like configured production peers.
            self.reconcile_calls_udp_for_test(local, peer, &identity);
            return;
        }
        let (additional_peers, routed_peers) = match super::settings::configured_peer_hints() {
            Ok(peers) => peers,
            Err(error) => {
                self.push_debug_log("fips.config.error", error);
                return;
            }
        };
        let websocket = configured_websocket_seeds();
        #[cfg(test)]
        let websocket = self
            .test_fips_rendezvous_addr
            .is_none()
            .then_some(websocket)
            .flatten();
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host_hashtree_enabled(),
            #[cfg(test)]
            rendezvous_addr: self.test_fips_rendezvous_addr,
            additional_peers,
            routed_peers,
            udp_bind_addr: std::env::var("IRIS_CHAT_FIPS_UDP_BIND_ADDR")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            websocket,
            ..SharedFipsOptions::default()
        });
    }

    #[cfg(test)]
    pub(crate) fn reconcile_device_sync_at_rendezvous_for_test(
        &mut self,
        rendezvous_addr: SocketAddrV4,
    ) {
        self.test_fips_rendezvous_addr = Some(rendezvous_addr);
        // Preserve the production runtime key while keeping fixture discovery
        // separate from other tests and any applications running on this host.
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host_hashtree_enabled(),
            rendezvous_addr: Some(rendezvous_addr),
            ..SharedFipsOptions::default()
        });
    }

    #[cfg(test)]
    pub(crate) fn reconcile_calls_udp_for_test(
        &mut self,
        local: std::net::SocketAddr,
        peer: std::net::SocketAddr,
        identity: &str,
    ) {
        self.test_fips_udp = Some((local, peer, identity.to_owned()));
        self.reconcile_shared_fips(SharedFipsOptions {
            udp_bind_addr: Some(local.to_string()),
            additional_peers: vec![PeerConfig::new(
                identity.to_string(),
                "udp",
                peer.to_string(),
            )],
            ..SharedFipsOptions::default()
        });
    }

    #[cfg(test)]
    pub(crate) fn reconcile_device_sync_with_websocket_for_test(
        &mut self,
        websocket: WebSocketConfig,
    ) {
        self.reconcile_device_sync_with_websocket_options_for_test(websocket, None);
    }

    #[cfg(test)]
    pub(crate) fn reconcile_device_sync_with_isolated_websocket_for_test(
        &mut self,
        websocket: WebSocketConfig,
    ) {
        // Model separate machines so local discovery cannot bypass the tested route.
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(rendezvous_addr) = socket.local_addr().unwrap() else {
            unreachable!("IPv4 reservation");
        };
        drop(socket);
        self.reconcile_device_sync_with_websocket_options_for_test(
            websocket,
            Some(rendezvous_addr),
        );
    }

    #[cfg(test)]
    fn reconcile_device_sync_with_websocket_options_for_test(
        &mut self,
        websocket: WebSocketConfig,
        rendezvous_addr: Option<SocketAddrV4>,
    ) {
        // Message handling also runs normal reconciliation; keep the same service set.
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host_hashtree_enabled(),
            rendezvous_addr,
            standalone_route: None,
            additional_peers: Vec::new(),
            websocket: Some(websocket),
            ..SharedFipsOptions::default()
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
            ..SharedFipsOptions::default()
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
        let fixture_scope = match super::super::support::offline_lan::requested_scope() {
            Ok(scope) => scope,
            Err(error) => {
                self.stop_device_sync();
                self.push_debug_log("fips.config.error", error);
                return;
            }
        };
        // Scoped DEBUG fixtures keep the LAN blob service without local rendezvous.
        let attachment_blob_service_enabled = options.same_host_hashtree || fixture_scope.is_some();
        self.reconcile_update_sources();
        let host_ble_requested = self.pending_host_ble.is_some() || self.host_ble_attached;
        let (mut config, device_sync_enabled) = match self.device_sync_config() {
            Some(config) => {
                let device_sync_enabled = !config.siblings.is_empty();
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
        for peer in &options.routed_peers {
            if !config.peers.contains(peer) {
                config.peers.push(*peer);
            }
        }
        let update_publisher =
            match secure_update_ref().and_then(|reference| trusted_update_publisher(&reference)) {
                Ok(publisher) => Some(publisher),
                Err(error) => {
                    self.push_debug_log("update.pubsub.config.error", error.to_string());
                    None
                }
            };
        let mut pubsub_options = FipsPubsubClientOptions {
            #[cfg(feature = "stack-fixture")]
            max_connected_peers: crate::stack_mesh_fixture::max_connected_peers(),
            #[cfg(feature = "stack-fixture")]
            fanout: nostr_pubsub::DEFAULT_INV_WANT_FANOUT
                .min(crate::stack_mesh_fixture::max_connected_peers()),
            max_replay_events: super::super::mesh_pubsub::MESH_REPLAY_EVENTS,
            ..FipsPubsubClientOptions::default()
        };
        pubsub_options.routed_peers = private_provider::approved_providers(
            &options.additional_peers,
            &options.routed_peers,
            update_publisher.as_deref(),
            pubsub_options.max_connected_peers,
        );
        let private_providers = Arc::new(super::super::mesh_pubsub::PrivateMeshProviders::new(
            options.additional_peers.clone(),
            options.websocket.clone(),
            private_provider::approved_providers(
                &options.additional_peers,
                &options.routed_peers,
                None,
                pubsub_options.max_connected_peers,
            ),
            config
                .relay_urls
                .iter()
                .filter_map(|url| RelayUrl::parse(url).ok())
                .collect(),
        ));
        let peer_advert_authors = config
            .peers
            .iter()
            .filter_map(|peer| PublicKey::from_hex(&peer.pubkey().to_string()).ok())
            .collect::<Vec<_>>();
        let nearby_enabled = host_ble_requested || config.nearby_ip_enabled;
        let discovery_scope = if nearby_enabled || !config.peers.is_empty() {
            super::super::fips_nearby::FIPS_NEARBY_SCOPE.to_string()
        } else {
            format!("{DEVICE_SYNC_SCOPE_PREFIX}{}", config.owner_hex)
        };
        let runtime_key = format!(
            "{}:same-host={}:nearby={}:ble={}:routed={:?}:static={:?}:udp={:?}:update={:?}:fixture-scope={:?}",
            config.key,
            options.same_host_hashtree,
            nearby_enabled,
            host_ble_requested,
            options.routed_peers,
            options.additional_peers,
            options.udp_bind_addr,
            update_publisher,
            fixture_scope,
        );
        // Learning another contact changes routing, not our transport or Noise
        // identity. Keep established sessions (including an in-progress call).
        let peer_refresh_key = format!(
            "{}:{}:{}:{:?}:{:?}:{}:{}:{}:{}:{:?}:{:?}:{}:{:?}",
            config.owner_hex,
            config.local_npub,
            config.roster_at,
            config.siblings,
            config.relay_urls,
            options.same_host_hashtree,
            nearby_enabled,
            config.nearby_ip_enabled,
            host_ble_requested,
            options.udp_bind_addr,
            options.rendezvous_addr,
            discovery_scope,
            fixture_scope,
        );
        let device_sync_packets = if device_sync_enabled {
            let request = serde_json::to_vec(&DeviceSyncPacket::Request {
                v: DEVICE_SYNC_VERSION,
                roster_at: config.roster_at,
                page: None,
                record_reconcile: Some(1),
                private_events: Some(2),
                read_states: Some(2),
                name_approvals: Some(2),
                private_facts: Some(2),
                history_since: None,
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

        // Host BLE I/O is single-use. Native hosts serialize detach/reattach
        // when LAN settings change; keep this bridge alive until that detach.
        let refresh_peers = self.host_ble_attached
            || self.device_sync.as_ref().is_some_and(|runtime| {
                runtime.key != runtime_key && runtime.peer_refresh_key == peer_refresh_key
            });
        let refreshed_bootstrap =
            refresh_peers.then(|| self.local_fips_nearby_bootstrap_payloads());
        if refresh_peers {
            self.restrict_direct_file_devices();
            if let Some(runtime) = self.device_sync.as_mut() {
                let siblings = config.siblings.iter().map(|peer| peer.npub()).collect();
                if let Some(tcp) = &runtime.tcp {
                    tcp.update_peers(siblings);
                } else if let Some((request, resync_required)) = device_sync_packets {
                    // Keep the mobile transport alive while enabling history for
                    // the first linked device on that existing endpoint.
                    match self.runtime.block_on(start_device_sync_tcp(
                        runtime.endpoint.clone(),
                        siblings,
                        DEVICE_SYNC_PORT,
                        DEVICE_SYNC_MAX_PACKET_BYTES,
                        request,
                        resync_required,
                        self.core_sender.clone(),
                    )) {
                        Ok((tcp, task)) => {
                            runtime.tcp = Some(tcp);
                            runtime.tasks.push(task);
                        }
                        Err(error) => {
                            crate::perflog!("device_sync.tcp.refresh.error={error}");
                            return;
                        }
                    }
                }
                runtime.key = runtime_key;
                runtime.peer_refresh_key = peer_refresh_key;
                runtime.siblings = config.siblings.clone();
                runtime.private_providers = Some(private_providers);
                runtime.peer_advert_authors = peer_advert_authors;
                if let Some(pubsub) = &runtime.pubsub {
                    if let Err(error) = pubsub.set_routed_peers(pubsub_options.routed_peers) {
                        crate::perflog!("fips.pubsub.peer_refresh error={error}");
                    }
                }
                if let Some(blobs) = &runtime._attachment_blobs {
                    blobs.set_peers(config.peers.clone());
                }
                if let Ok(mut payloads) = runtime.nearby_bootstrap_payloads.write() {
                    *payloads = refreshed_bootstrap.unwrap_or_default();
                }
                let endpoint = runtime.endpoint.clone();
                runtime.configured_direct_peers =
                    configured_direct_peer_ids(&options.additional_peers);
                let mut peer_config = config
                    .peers
                    .iter()
                    .map(|peer| PeerConfig {
                        npub: peer.npub(),
                        ..PeerConfig::default()
                    })
                    .collect::<Vec<_>>();
                peer_config.extend(options.additional_peers);
                if let Some(recent_peers) = &runtime.recent_peers {
                    if let Ok(recent_peers) = recent_peers.read() {
                        recent_peers.merge_into(&mut peer_config);
                    }
                }
                self.runtime.spawn(async move {
                    let _ = endpoint.update_peers(peer_config).await;
                });
                self.reconcile_mesh_protocol_subscriptions();
                self.replay_mesh_outbox();
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
        // Release listeners before binding the replacement endpoint. An async
        // shutdown races a fixed WebSocket/UDP port when contact keys refresh.
        self.stop_device_sync_now();

        let mut peer_config = config
            .peers
            .iter()
            .map(|peer| PeerConfig {
                npub: peer.npub(),
                ..PeerConfig::default()
            })
            .collect::<Vec<_>>();
        let configured_direct_peers = configured_direct_peer_ids(&options.additional_peers);
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
        let webrtc_enabled = super::settings::webrtc_enabled(
            device_sync_enabled || config.nearby_ip_enabled || !config.peers.is_empty(),
            std::env::var("IRIS_CHAT_FIPS_ENABLE_WEBRTC")
                .ok()
                .as_deref(),
        );
        #[cfg(test)]
        let webrtc_enabled = webrtc_enabled && self.test_fips_rendezvous_addr.is_none();
        if webrtc_enabled {
            // This also enables signed, in-band transport upgrades over an
            // existing FIPS route. An empty relay list stays entirely local.
            fips_config.node.discovery.nostr.enabled = true;
            // External advert ingestion preserves in-band FIPS signaling while
            // preventing a device-signed advert and peer query sharing a relay.
            fips_config.node.discovery.nostr.peerfinding_source = NostrPeerfindingSource::External;
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
                // Inherit FIPS STUN defaults to discover direct routes through NAT.
                ..WebRtcConfig::default()
            });
        } else {
            fips_config.node.discovery.nostr.enabled = false;
            fips_config.node.discovery.nostr.advertise = false;
        }
        configure_fips_lan(&mut fips_config, config.nearby_ip_enabled);
        if let Some(scope) = fixture_scope {
            fips_config.node.discovery.lan.scope = Some(scope);
        }
        if let Some(bind_addr) = options.udp_bind_addr {
            fips_config.transports.udp = TransportInstances::Single(UdpConfig {
                bind_addr: Some(bind_addr),
                advertise_on_nostr: Some(false),
                public: Some(false),
                accept_connections: Some(true),
                ..UdpConfig::default()
            });
        }
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

        super::super::support::offline_lan::capture_network(
            &fips_config,
            &config.relay_urls,
            host_ble_requested,
            options.same_host_hashtree,
        );
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
        // Exact-recipient live delivery remains available independently of
        // public nearby discovery and mailbag forwarding preferences.
        let nearby_receiver = if nearby_enabled || !config.peers.is_empty() {
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
        let (tcp, mut tasks) = if device_sync_enabled {
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
            (Some(tcp), vec![tcp_task])
        } else {
            (None, Vec::new())
        };
        self.fips_connection_generation = self.fips_connection_generation.wrapping_add(1);
        let direct_files = self.start_direct_file_transport(endpoint.clone(), &mut tasks);
        let calls_tx = match self.runtime.block_on(super::super::calls::start_transport(
            endpoint.clone(),
            self.core_sender.clone(),
        )) {
            Ok((tx, call_tasks)) => {
                tasks.extend(call_tasks);
                Some(tx)
            }
            Err(error) => {
                self.push_debug_log("calls.start.error", error);
                None
            }
        };
        let update_pubsub =
            match self
                .runtime
                .block_on(FipsPubsubClient::start_receive_only_with_public_authors(
                    endpoint.clone(),
                    pubsub_options,
                    self.mesh_public_service_authors(),
                )) {
                Ok(client) => Some(Arc::new(client)),
                Err(error) => {
                    self.push_debug_log("update.pubsub.start.error", error.to_string());
                    None
                }
            };
        if webrtc_enabled {
            if let (Some(pubsub), Some(login)) = (&update_pubsub, &self.logged_in) {
                tasks.push(self.runtime.spawn(private_provider::publish_own_adverts(
                    endpoint.clone(),
                    pubsub.clone(),
                    login.client.clone(),
                    login.relay_urls.clone(),
                )));
            }
        }
        self.register_update_sources(update_pubsub.as_ref());
        #[cfg(feature = "stack-fixture")]
        if let (Some(pubsub), Some(logged_in)) = (&update_pubsub, &self.logged_in) {
            crate::stack_mesh_fixture::register(
                &endpoint,
                pubsub,
                logged_in.device_keys.clone(),
                config.nearby_ip_enabled,
                config.relay_urls.len(),
            );
        }

        let local_attachments = self.ensure_attachment_service();
        let attachment_store = if let (true, Ok(local_attachments)) =
            (attachment_blob_service_enabled, local_attachments)
        {
            let result = match options.standalone_route {
                Some(route) => self.runtime.block_on(
                    super::super::attachment_upload::bind_same_host_attachment_store(
                        endpoint.clone(),
                        route,
                        config.peers.clone(),
                        local_attachments.clone(),
                    ),
                ),
                None => self.runtime.block_on(
                    super::super::attachment_upload::start_same_host_attachment_reuse(
                        endpoint.clone(),
                        config.peers.clone(),
                        local_attachments,
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
        {
            for event in self
                .pending_relay_publishes
                .values()
                .rev()
                .take(super::super::fips_nearby::FIPS_NEARBY_OUTBOX_MAX_EVENTS)
                .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
                .filter(|event| self.private_publication_allowed(event))
                .filter(|event| {
                    nearby_enabled || super::super::fips_nearby::is_private_transport_event(event)
                })
            {
                if let Some(payload) = super::super::fips_nearby::encode_fips_nearby_event(&event) {
                    initial_nearby_outbox.insert_for_targets(
                        event.id.to_string(),
                        payload,
                        self.private_nearby_targets(&event),
                    );
                }
            }
        }
        let nearby_outbox = Arc::new(RwLock::new(initial_nearby_outbox));
        if let Some(receiver) = nearby_receiver {
            let nearby_tx = self.core_sender.clone();
            let nearby_generation = self.fips_connection_generation;
            tasks.push(self.runtime.spawn(async move {
                let mut datagrams = Vec::with_capacity(32);
                let mut dequeued = 0_u64;
                let mut dequeued_bytes = 0_u64;
                if debug_snapshot_file_writes_enabled() {
                    let detail = format!("generation={nearby_generation} registered=true snapshot=true perf={}", crate::perflog::__enabled());
                    crate::perflog!("fips_nearby.rx.listener {detail}");
                    let _ = nearby_tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                        category: "fips_nearby.rx.listener".to_string(), detail,
                    })));
                }
                while receiver.recv_batch_into(&mut datagrams, 32).await.is_some() {
                    for datagram in datagrams.drain(..) {
                        if debug_snapshot_file_writes_enabled() {
                            dequeued = dequeued.saturating_add(1);
                            dequeued_bytes = dequeued_bytes.saturating_add(datagram.data.len() as u64);
                            if super::super::fips_nearby::diagnostics::sample_receive_diagnostic(dequeued) {
                                let detail = format!("generation={nearby_generation} packets={dequeued} bytes={dequeued_bytes} source_port={} last_bytes={}", datagram.source_port, datagram.data.len());
                                crate::perflog!("fips_nearby.rx.dequeue {detail}");
                                let _ = nearby_tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
                                    category: "fips_nearby.rx.dequeue".to_string(), detail,
                                })));
                            }
                        }
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
        tasks.push(self.runtime.spawn(connection_monitor::run(
            endpoint.clone(),
            nearby_bootstrap_payloads.clone(),
            nearby_outbox.clone(),
            self.core_sender.clone(),
            self.fips_connection_generation,
            nearby_enabled,
        )));
        if let Some(recent_peers) = &recent_peers {
            tasks.push(self.runtime.spawn(run_recent_peer_observer(
                endpoint.clone(),
                recent_peers.clone(),
            )));
        }

        self.restore_device_history_progress();
        let sibling_count = config.siblings.len();
        crate::update_announcements::register_update_blob_store(
            attachment_store
                .as_ref()
                .map(|runtime| runtime.update_store()),
        );
        self.device_sync = Some(DeviceSyncRuntime {
            key: runtime_key,
            peer_refresh_key,
            endpoint,
            calls_tx,
            configured_direct_peers,
            direct_files,
            tcp,
            siblings: config.siblings,
            snapshot_pending: false,
            history: history::HistoryState::default(),
            nearby_enabled,
            nearby_bootstrap_payloads,
            nearby_outbox,
            _attachment_blobs: attachment_store,
            pubsub: update_pubsub,
            private_providers: Some(private_providers),
            peer_advert_authors,
            protocol_subscriptions: super::super::mesh_pubsub::MeshProtocolSubscriptions::default(),
            recent_peers,
            tasks,
        });
        self.restrict_direct_file_devices();
        self.reconcile_mesh_protocol_subscriptions();
        self.replay_mesh_outbox();
        if device_sync_enabled {
            self.push_debug_log("device_sync.start", format!("peers={sibling_count}"));
        } else if nearby_enabled {
            self.push_debug_log("fips_nearby.start", "nearby-only");
        } else {
            self.push_debug_log("attachment.same_host.start", "local-only");
        }
    }

    pub(in crate::core) fn stop_device_sync(&mut self) {
        if let Some(shutdown) = self.take_device_sync_shutdown() {
            self.runtime.spawn(shutdown);
        }
    }

    pub(in crate::core) fn stop_device_sync_now(&mut self) {
        if let Some(shutdown) = self.take_device_sync_shutdown() {
            self.runtime.block_on(shutdown);
        }
    }

    fn take_device_sync_shutdown(
        &mut self,
    ) -> Option<impl std::future::Future<Output = ()> + Send + 'static> {
        crate::update_announcements::register_update_blob_store(None);
        self.register_update_sources(None);
        self.restore_device_history_progress();
        self.interrupt_direct_files();
        self.host_ble_attached = false;
        self.fips_connection_generation = self.fips_connection_generation.wrapping_add(1);
        self.update_fips_connection_links(Vec::new());
        let DeviceSyncRuntime {
            endpoint,
            pubsub,
            recent_peers,
            tasks,
            ..
        } = self.device_sync.take()?;
        if let Some(client) = &pubsub {
            let _ = client.set_replay_source(None);
        }
        for task in tasks {
            task.abort();
        }
        Some(shutdown_shared_fips(endpoint, pubsub, recent_peers))
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
        let roster_at = self.device_sync_roster_at().unwrap_or_default();
        let siblings = roster
            .into_iter()
            .flat_map(|roster| roster.devices.iter())
            .filter(|device| self.device_sync_peer_is_authorized(&device.identity_pubkey_hex))
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
        if peers.is_empty() && siblings.is_empty() && !ble_requested && !nearby_ip_enabled {
            return None;
        }
        let key = format!(
            "{}:{}:{:?}:{}:{}:{}:{}",
            owner_hex,
            roster_at,
            siblings,
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
            siblings,
            peers,
            nearby_ip_enabled,
        })
    }
}

mod environment;
use environment::*;

#[cfg(test)]
mod local_rendezvous_tests;

#[cfg(test)]
mod offline_blob_tests;
