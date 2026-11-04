use super::*;
use fips_core::config::{PeerConfig, TransportInstances, UdpConfig, WebSocketConfig};

/// Only public route hints cross this boundary. Never borrow the live device's
/// authenticated endpoint, identity, advertisements or service registrations.
pub(in crate::core) struct ProviderReadOptions {
    config: fips_core::Config,
}

impl ProviderReadOptions {
    pub(in crate::core) fn new(source: &fips_core::Config, public_peers: Vec<PeerConfig>) -> Self {
        let mut config = fips_core::Config::new();
        config.node.leaf_only = true;
        config.node.control.enabled = false;
        // FIPS currently couples configured-only peer admission to this flag.
        // External peerfinding performs no Nostr queries or relay connections.
        config.node.discovery.nostr.enabled = true;
        config.node.discovery.nostr.policy =
            fips_core::config::NostrDiscoveryPolicy::ConfiguredOnly;
        config.node.discovery.nostr.peerfinding_source =
            fips_core::config::NostrPeerfindingSource::External;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.nostr.advert_relays.clear();
        config.node.discovery.nostr.stun_servers.clear();
        // Discovery can attach this reader to our identified app or a linked
        // device and carry anonymous GETs over that identity's public link.
        // Learn public addresses in memory instead; dial them independently.
        config.node.discovery.local.enabled = false;
        config.node.discovery.lan.enabled = false;
        config.peers = dialing_peers(public_peers);
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
        Self { config }
    }

    pub(super) async fn bind(self) -> anyhow::Result<Arc<FipsEndpoint>> {
        let builder = FipsEndpoint::builder()
            .config(self.config)
            .identity_nsec(Keys::generate().secret_key().to_secret_hex())
            .without_system_tun();
        Ok(Arc::new(builder.bind().await?))
    }
}

fn dialing_peers(peers: Vec<PeerConfig>) -> Vec<PeerConfig> {
    peers
        .into_iter()
        .filter_map(|mut peer| {
            peer.npub = nostr::PublicKey::parse(&peer.npub).ok()?.to_bech32().ok()?;
            peer.alias = None;
            Some(peer)
        })
        .collect()
}

pub(super) struct ProviderReads {
    pub(super) endpoint: Arc<FipsEndpoint>,
    transport: std::sync::Mutex<Option<Arc<TcpBlobTransport<AttachmentChunkStore>>>>,
    public_peers: RwLock<Vec<PeerConfig>>,
    applied_peers: tokio::sync::Mutex<Option<Vec<u8>>>,
    local_npub: String,
    siblings: Arc<RwLock<Vec<fips_core::PeerIdentity>>>,
}

impl ProviderReads {
    pub(super) async fn bind(
        mut options: ProviderReadOptions,
        primary: Arc<AttachmentChunkStore>,
        local_npub: String,
        siblings: Arc<RwLock<Vec<fips_core::PeerIdentity>>>,
    ) -> anyhow::Result<Self> {
        {
            let excluded = siblings.read().unwrap_or_else(|error| error.into_inner());
            options.config.peers.retain(|peer| {
                peer.npub != local_npub
                    && !excluded.iter().any(|sibling| sibling.npub() == peer.npub)
            });
        }
        let public_peers = options.config.peers.clone();
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
                public_peers: RwLock::new(public_peers),
                applied_peers: tokio::sync::Mutex::new(None),
                local_npub,
                siblings,
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

    pub(super) fn permits(&self, npub: &str) -> bool {
        npub != self.local_npub
            && !self
                .siblings
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .any(|sibling| sibling.npub() == npub)
    }

    pub(super) fn update_peers(&self, peers: Vec<PeerConfig>) {
        *self
            .public_peers
            .write()
            .unwrap_or_else(|error| error.into_inner()) = dialing_peers(peers);
    }

    pub(super) async fn prepare_paths(
        &self,
        discovery: &FipsEndpoint,
        providers: &[String],
    ) -> Result<(), StoreError> {
        // Serialize updates, but do not hold this lock during blob transfers.
        let mut applied = self.applied_peers.lock().await;
        let connected = self
            .endpoint
            .peers()
            .await
            .map_err(|error| StoreError::Other(error.to_string()))?;
        if connected
            .iter()
            .any(|peer| peer.connected && !self.permits(&peer.npub))
        {
            // Roster updates can classify an existing public peer as a linked
            // device. FIPS update_peers leaves live links connected, so close
            // this reader before another GET. Runtime recreation rotates it.
            self.shutdown().await;
            return Err(StoreError::Other(
                "anonymous file reader needs recreation after device linking".into(),
            ));
        }
        self.transport()
            .map_err(|error| StoreError::Other(error.to_string()))?;
        let mut peers = self
            .public_peers
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        peers.retain(|peer| self.permits(&peer.npub));
        for peer in discovery
            .peers()
            .await
            .map_err(|error| StoreError::Other(error.to_string()))?
        {
            let Some(address) = peer.authenticated_udp_restart_addr() else {
                continue;
            };
            // A third-party local anchor can route to another local provider.
            // Never reuse the app's or a linked device's authenticated socket.
            if self.permits(&peer.npub)
                && (providers.contains(&peer.npub) || address.ip().is_loopback())
            {
                if let Some(existing) = peers
                    .iter_mut()
                    .find(|candidate| candidate.npub == peer.npub)
                {
                    let hint = fips_core::config::PeerAddress::new("udp", address.to_string())
                        .authenticated();
                    if !existing.addresses.contains(&hint) {
                        existing.addresses.push(hint);
                    }
                } else {
                    let mut hint = PeerConfig::new(peer.npub, "udp", address.to_string());
                    hint.addresses[0] = hint.addresses[0].clone().authenticated();
                    peers.push(hint);
                }
            }
        }
        // End-to-end provider handshakes must also satisfy configured-only
        // admission when a public transit peer supplies their reachability.
        for npub in providers {
            if self.permits(npub) && !peers.iter().any(|peer| &peer.npub == npub) {
                peers.push(PeerConfig {
                    npub: npub.clone(),
                    ..Default::default()
                });
            }
        }
        for peer in &mut peers {
            peer.alias = None;
        }
        peers.sort_by(|left, right| left.npub.cmp(&right.npub));
        let fingerprint =
            serde_json::to_vec(&peers).map_err(|error| StoreError::Other(error.to_string()))?;
        if applied.as_ref() != Some(&fingerprint) {
            self.endpoint
                .update_peers(peers)
                .await
                .map_err(|error| StoreError::Other(error.to_string()))?;
            *applied = Some(fingerprint);
        }
        Ok(())
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
