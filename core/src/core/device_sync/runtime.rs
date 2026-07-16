use super::*;
use crate::core::update_pubsub::run_update_announcement_subscription;
use fips_core::config::{
    NostrDiscoveryPolicy, NostrRelayConfig, PeerAddress, PeerConfig, TransportInstances,
};
use fips_core::{Config, WebRtcConfig};
use hashtree_core::BlobRoute;
use std::net::SocketAddrV4;

const SAME_HOST_HASHTREE_ENV: &str = "IRIS_CHAT_SAME_HOST_HASHTREE";
const LOCAL_RENDEZVOUS_ADDR_ENV: &str = "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR";

struct SharedFipsOptions {
    same_host_hashtree: bool,
    rendezvous_addr: Option<SocketAddrV4>,
    standalone_route: Option<Arc<dyn BlobRoute>>,
    additional_peers: Vec<PeerConfig>,
}

impl AppCore {
    pub(in crate::core) fn reconcile_device_sync(&mut self) {
        self.reconcile_shared_fips(SharedFipsOptions {
            same_host_hashtree: same_host_hashtree_enabled(),
            rendezvous_addr: None,
            standalone_route: None,
            additional_peers: Vec::new(),
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
        });
    }

    #[cfg(test)]
    pub(in crate::core) fn same_host_runtime_for_test(
        &self,
    ) -> Option<(
        Arc<FipsEndpoint>,
        bool,
        usize,
        Arc<super::super::attachment_upload::SameHostAttachmentStore>,
    )> {
        let runtime = self.device_sync.as_ref()?;
        Some((
            runtime.endpoint.clone(),
            runtime.tcp.is_some(),
            runtime.siblings.len(),
            runtime._attachment_store.as_ref()?.clone(),
        ))
    }

    fn reconcile_shared_fips(&mut self, options: SharedFipsOptions) {
        let (config, device_sync_enabled) = match self.device_sync_config() {
            Some(config) => (config, true),
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
        let runtime_key = format!("{}:same-host={}", config.key, options.same_host_hashtree);
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.key == runtime_key)
        {
            return;
        }
        self.stop_device_sync();

        let mut fips_config = Config::new();
        if device_sync_enabled {
            fips_config.node.discovery.nostr.enabled = true;
            fips_config.node.discovery.nostr.advertise = true;
            fips_config.node.discovery.nostr.advert_relays = config.relay_urls.clone();
            fips_config.node.discovery.nostr.app =
                format!("{DEVICE_SYNC_SCOPE_PREFIX}{}", config.owner_hex);
            fips_config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
            fips_config.peers = config
                .siblings
                .iter()
                .map(|peer| {
                    let npub = peer.npub();
                    PeerConfig {
                        npub: npub.clone(),
                        addresses: vec![PeerAddress::with_priority("nostr_relay", npub, 250)],
                        ..PeerConfig::default()
                    }
                })
                .collect();
            fips_config.transports.nostr_relay = TransportInstances::Single(NostrRelayConfig {
                auto_connect: Some(false),
                accept_connections: Some(true),
                ..NostrRelayConfig::default()
            });
            fips_config.transports.webrtc = TransportInstances::Single(WebRtcConfig {
                advertise_on_nostr: Some(true),
                auto_connect: Some(false),
                accept_connections: Some(true),
                ..WebRtcConfig::default()
            });
        } else {
            fips_config.node.discovery.nostr.enabled = false;
            fips_config.node.discovery.nostr.advertise = false;
            fips_config.node.discovery.lan.enabled = false;
        }
        fips_config.peers.extend(options.additional_peers);
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
            .discovery_scope(format!("{DEVICE_SYNC_SCOPE_PREFIX}{}", config.owner_hex))
            .without_system_tun();
        if options.same_host_hashtree {
            builder = builder.local_rendezvous();
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
        let relay_adapter = if device_sync_enabled {
            match self.runtime.block_on(NostrRelayAdapter::start(
                endpoint.clone(),
                &config.relay_urls,
            )) {
                Ok(Some(adapter)) => Some(adapter),
                Ok(None) => {
                    self.push_debug_log(
                        "device_sync.relay.start.error",
                        "configured message servers produced no FIPS relay adapter",
                    );
                    let _ = self.runtime.block_on(endpoint.shutdown());
                    return;
                }
                Err(error) => {
                    self.push_debug_log("device_sync.relay.start.error", error);
                    let _ = self.runtime.block_on(endpoint.shutdown());
                    return;
                }
            }
        } else {
            None
        };

        let (tcp, update_pubsub, tasks) = if device_sync_enabled {
            let Some((request, resync_required)) = device_sync_packets else {
                self.runtime
                    .block_on(shutdown_shared_fips(endpoint, relay_adapter));
                return;
            };
            let (tcp, tcp_task) = match self.runtime.block_on(start_device_sync_tcp(
                endpoint.clone(),
                DEVICE_SYNC_PORT,
                DEVICE_SYNC_MAX_PACKET_BYTES,
                request,
                resync_required,
                self.core_sender.clone(),
            )) {
                Ok(value) => value,
                Err(error) => {
                    self.push_debug_log("device_sync.tcp.start.error", error);
                    self.runtime
                        .block_on(shutdown_shared_fips(endpoint, relay_adapter));
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
            let update_subscription_task = update_pubsub.as_ref().and_then(|pubsub| {
                let filter = match crate::update_announcements::update_announcement_filter() {
                    Ok(filter) => filter,
                    Err(error) => {
                        self.push_debug_log("update.pubsub.filter.error", error.to_string());
                        return None;
                    }
                };
                let pubsub = pubsub.clone();
                let endpoint = endpoint.clone();
                Some(self.runtime.spawn(async move {
                    run_update_announcement_subscription(endpoint, pubsub, filter).await;
                }))
            });
            let mut tasks = vec![tcp_task];
            tasks.extend(update_subscription_task);
            (Some(tcp), update_pubsub, tasks)
        } else {
            (None, None, Vec::new())
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
                        self.runtime
                            .block_on(shutdown_shared_fips(endpoint, relay_adapter));
                        return;
                    }
                    None
                }
            }
        } else {
            None
        };

        let sibling_count = config.siblings.len();
        self.device_sync = Some(DeviceSyncRuntime {
            key: runtime_key,
            endpoint,
            tcp,
            siblings: config.siblings,
            _attachment_store: attachment_store,
            _update_pubsub: update_pubsub,
            relay_adapter,
            tasks,
        });
        if device_sync_enabled {
            self.push_debug_log("device_sync.start", format!("peers={sibling_count}"));
        } else {
            self.push_debug_log("attachment.same_host.start", "local-only");
        }
    }

    pub(in crate::core) fn stop_device_sync(&mut self) {
        let Some(runtime) = self.device_sync.take() else {
            return;
        };
        let DeviceSyncRuntime {
            endpoint,
            relay_adapter,
            tasks,
            ..
        } = runtime;
        for task in tasks {
            task.abort();
        }
        self.runtime.spawn(async move {
            shutdown_shared_fips(endpoint, relay_adapter).await;
        });
    }

    #[cfg(test)]
    pub(in crate::core) fn device_sync_relay_adapter_running_for_test(&self) -> bool {
        self.device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.relay_adapter.is_some())
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
        Some(DeviceSyncConfig {
            key: format!("{owner_hex}:{device_hex}:local-only"),
            owner_hex,
            roster_at: 0,
            secret_hex: logged_in.device_keys.secret_key().to_secret_hex(),
            relay_urls: Vec::new(),
            siblings: Vec::new(),
        })
    }

    fn device_sync_config(&self) -> Option<DeviceSyncConfig> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_hex = logged_in.owner_pubkey.to_hex();
        let roster = self.app_keys.get(&owner_hex)?;
        let local_hex = logged_in.device_keys.public_key().to_hex();
        if roster.created_at_secs == 0
            || !roster
                .devices
                .iter()
                .any(|device| device.identity_pubkey_hex == local_hex)
        {
            return None;
        }
        let siblings = roster
            .devices
            .iter()
            .filter(|device| device.identity_pubkey_hex != local_hex)
            .filter_map(|device| fips_peer_from_hex(&device.identity_pubkey_hex))
            .collect::<Vec<_>>();
        if siblings.is_empty() {
            return None;
        }
        let relay_urls = logged_in
            .relay_urls
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if relay_urls.is_empty() {
            return None;
        }
        let key = format!(
            "{}:{}:{}:{}",
            owner_hex,
            roster.created_at_secs,
            roster
                .devices
                .iter()
                .map(|device| device.identity_pubkey_hex.as_str())
                .collect::<Vec<_>>()
                .join(","),
            relay_urls.join(",")
        );
        Some(DeviceSyncConfig {
            key,
            owner_hex,
            roster_at: roster.created_at_secs,
            secret_hex: logged_in.device_keys.secret_key().to_secret_hex(),
            relay_urls,
            siblings,
        })
    }
}

async fn shutdown_shared_fips(
    endpoint: Arc<FipsEndpoint>,
    relay_adapter: Option<NostrRelayAdapter>,
) {
    if let Some(adapter) = relay_adapter {
        adapter.stop().await;
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
    fn local_rendezvous_override_requires_nonzero_ipv4_loopback() {
        assert_eq!(
            parse_local_rendezvous_addr("127.0.0.1:32112").unwrap(),
            "127.0.0.1:32112".parse::<SocketAddrV4>().unwrap()
        );
        assert!(parse_local_rendezvous_addr("0.0.0.0:32112").is_err());
        assert!(parse_local_rendezvous_addr("127.0.0.1:0").is_err());
        assert!(parse_local_rendezvous_addr("[::1]:32112").is_err());
    }
}
