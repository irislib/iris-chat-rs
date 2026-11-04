use super::*;
use fips_core::config::{PeerConfig, TransportInstances, UdpConfig, WebSocketConfig};

/// Only public route hints cross this boundary. Never borrow the live device's
/// authenticated endpoint, identity, advertisements or service registrations.
pub(in crate::core) struct ProviderReadOptions {
    config: fips_core::Config,
    local_rendezvous: bool,
}

impl ProviderReadOptions {
    pub(in crate::core) fn new(source: &fips_core::Config, local_rendezvous: bool) -> Self {
        let mut config = fips_core::Config::new();
        config.node.leaf_only = true;
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.local.rendezvous_addr = source.node.discovery.local.rendezvous_addr;
        config.node.discovery.local.retry_interval_ms =
            source.node.discovery.local.retry_interval_ms;
        config.node.discovery.local.enabled = local_rendezvous;
        config.node.discovery.lan.enabled = source.node.discovery.lan.enabled;
        // Do not copy an account-derived scope or a user-specific service name.
        config.node.discovery.lan.scope = super::super::support::offline_lan::requested_scope()
            .ok()
            .flatten()
            .or_else(|| Some(super::super::fips_nearby::FIPS_NEARBY_SCOPE.into()));
        config.peers = dialing_peers(source.peers.clone());
        // A separate ephemeral socket must not rebind the live device's port.
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some("0.0.0.0:0".into()),
            advertise_on_nostr: Some(false),
            public: Some(false),
            ..Default::default()
        });
        let seed_urls = source
            .transports
            .websocket
            .iter()
            .flat_map(|(_, config)| config.seed_urls.clone())
            .collect::<Vec<_>>();
        if !seed_urls.is_empty() {
            config.transports.websocket = TransportInstances::Single(WebSocketConfig {
                seed_urls,
                ..Default::default()
            });
        }
        Self {
            config,
            local_rendezvous,
        }
    }

    pub(super) async fn bind(self) -> anyhow::Result<Arc<FipsEndpoint>> {
        let mut builder = FipsEndpoint::builder()
            .config(self.config)
            .identity_nsec(Keys::generate().secret_key().to_secret_hex())
            // The live endpoint may use an account-derived discovery scope.
            .discovery_scope(super::super::fips_nearby::FIPS_NEARBY_SCOPE)
            .without_system_tun();
        if self.local_rendezvous {
            builder = builder.local_rendezvous();
        }
        Ok(Arc::new(builder.bind().await?))
    }
}

fn dialing_peers(peers: Vec<PeerConfig>) -> Vec<PeerConfig> {
    peers
        .into_iter()
        .map(|mut peer| {
            peer.alias = None;
            peer
        })
        .collect()
}

pub(super) struct ProviderReads {
    pub(super) endpoint: Arc<FipsEndpoint>,
    transport: std::sync::Mutex<Option<Arc<TcpBlobTransport<AttachmentChunkStore>>>>,
}

impl ProviderReads {
    pub(super) async fn bind(
        options: ProviderReadOptions,
        primary: Arc<AttachmentChunkStore>,
    ) -> anyhow::Result<Self> {
        let endpoint = options.bind().await?;
        match TcpBlobTransport::bind_client_with_config(
            endpoint.clone(),
            primary,
            TcpBlobTransportConfig::default(),
        )
        .await
        {
            Ok(transport) => Ok(Self {
                endpoint,
                transport: std::sync::Mutex::new(Some(Arc::new(transport))),
            }),
            Err(error) => {
                let _ = endpoint.shutdown().await;
                Err(error.into())
            }
        }
    }

    pub(super) fn transport(&self) -> anyhow::Result<Arc<TcpBlobTransport<AttachmentChunkStore>>> {
        self.transport
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| anyhow::anyhow!("private attachment reads are closed"))
    }

    pub(super) async fn update_peers(&self, peers: Vec<PeerConfig>) {
        let _ = self.endpoint.update_peers(dialing_peers(peers)).await;
    }

    pub(super) async fn shutdown(&self) {
        let transport = self
            .transport
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        // Closing the endpoint closes the TCP worker's receive channel and
        // cancels active requests. Removing our strong reference also prevents
        // the provider route (which retains only a Weak) from starting new work.
        let _ = self.endpoint.shutdown().await;
        if let Some(transport) = transport {
            if let Ok(transport) = Arc::try_unwrap(transport) {
                let _ = transport.shutdown().await;
            }
        }
    }
}

#[cfg(test)]
#[path = "provider_reads_tests.rs"]
mod tests;
