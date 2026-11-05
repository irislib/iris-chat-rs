use super::*;
use fips_core::config::{
    PeerAdmissionConfig, PeerConfig, TransportInstances, UdpConfig, WebSocketConfig,
};

/// Only public route hints cross this boundary. Never borrow the live device's
/// authenticated endpoint, identity, advertisements or service registrations.
#[derive(Clone)]
pub(in crate::core) struct ProviderReadOptions {
    config: fips_core::Config,
}

impl ProviderReadOptions {
    pub(in crate::core) fn new(source: &fips_core::Config, public_peers: Vec<PeerConfig>) -> Self {
        let mut config = fips_core::Config::new();
        config.node.strict_client_only = true;
        config.node.routing.mode = fips_core::config::RoutingMode::ReplyLearned;
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
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
            // A client-only reader never needs unsolicited inbound handshakes.
            accept_connections: Some(false),
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

    async fn bind(self) -> anyhow::Result<Arc<FipsEndpoint>> {
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

fn admission_policy(local_npub: &str, siblings: &[fips_core::PeerIdentity]) -> PeerAdmissionConfig {
    let mut denied_npubs = vec![local_npub.to_owned()];
    denied_npubs.extend(siblings.iter().map(|peer| peer.npub()));
    denied_npubs.sort();
    denied_npubs.dedup();
    PeerAdmissionConfig {
        // Public WebSocket seeds are configured by URL before their identity is
        // known. Explicit denials still apply to them, without carrier exceptions.
        configured_only: false,
        denied_npubs,
    }
}

struct ReaderState {
    endpoint: Arc<FipsEndpoint>,
    transport: Option<Arc<TcpBlobTransport<AttachmentChunkStore>>>,
}

impl ReaderState {
    async fn bind(
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
                transport: Some(Arc::new(transport)),
            }),
            Err(error) => {
                let _ = endpoint.shutdown().await;
                Err(error.into())
            }
        }
    }
}

pub(super) struct ProviderReads {
    state: RwLock<ReaderState>,
    options: ProviderReadOptions,
    primary: Arc<AttachmentChunkStore>,
    shutdown: std::sync::atomic::AtomicBool,
    public_peers: RwLock<Vec<PeerConfig>>,
    applied_peers: tokio::sync::Mutex<Option<Vec<u8>>>,
    local_npub: String,
    // Shared with the identified cache's admission policy.
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
            options.config.node.peer_admission = admission_policy(&local_npub, &excluded);
            options.config.peers.retain(|peer| {
                peer.npub != local_npub
                    && !excluded.iter().any(|sibling| sibling.npub() == peer.npub)
            });
        }
        let public_peers = options.config.peers.clone();
        let state = ReaderState::bind(options.clone(), primary.clone()).await?;
        Ok(Self {
            state: RwLock::new(state),
            options,
            primary,
            shutdown: std::sync::atomic::AtomicBool::new(false),
            public_peers: RwLock::new(public_peers),
            applied_peers: tokio::sync::Mutex::new(None),
            local_npub,
            siblings,
        })
    }

    pub(super) fn endpoint(&self) -> Arc<FipsEndpoint> {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .endpoint
            .clone()
    }

    pub(super) fn transport(&self) -> anyhow::Result<Arc<TcpBlobTransport<AttachmentChunkStore>>> {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .transport
            .clone()
            .ok_or_else(|| anyhow::anyhow!("private attachment reads are closed"))
    }

    // The caller holds applied_peers, serializing replacement with admission
    // and path updates. Retry only once in an ordinary reconciliation/request;
    // a bind failure stays closed until a later lifecycle operation retries.
    async fn ready_reader(
        &self,
        applied: &mut Option<Vec<u8>>,
    ) -> anyhow::Result<Arc<FipsEndpoint>> {
        anyhow::ensure!(
            !self.shutdown.load(Ordering::Acquire),
            "private attachment reads are closed"
        );
        let endpoint = self.endpoint();
        if self.transport().is_ok() && !endpoint.is_stopped().await {
            return Ok(endpoint);
        }
        self.close_reader().await;
        *applied = None;
        let mut options = self.options.clone();
        options.config.peers = self
            .public_peers
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|peer| self.permits(&peer.npub))
            .cloned()
            .collect();
        options.config.node.peer_admission = admission_policy(
            &self.local_npub,
            &self
                .siblings
                .read()
                .unwrap_or_else(|error| error.into_inner()),
        );
        let replacement = ReaderState::bind(options, self.primary.clone()).await?;
        // Shutdown may have been requested while binding. Never revive an
        // intentionally retired runtime, even for an already prepared read.
        if self.shutdown.load(Ordering::Acquire) {
            let _ = replacement.endpoint.shutdown().await;
            if let Some(transport) = replacement.transport {
                if let Ok(transport) = Arc::try_unwrap(transport) {
                    let _ = transport.shutdown().await;
                }
            }
            anyhow::bail!("private attachment reads are closed");
        }
        let endpoint = replacement.endpoint.clone();
        *self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner()) = replacement;
        Ok(endpoint)
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

    pub(super) async fn set_siblings(
        &self,
        siblings: Vec<fips_core::PeerIdentity>,
    ) -> anyhow::Result<()> {
        // The same lock prevents a concurrent path refresh from racing this
        // policy change. The acknowledged FIPS barrier retires denied links,
        // sessions and queued work while preserving unrelated provider paths.
        let mut applied = self.applied_peers.lock().await;
        if *self
            .siblings
            .read()
            .unwrap_or_else(|error| error.into_inner())
            == siblings
        {
            self.ready_reader(&mut applied).await?;
            return Ok(());
        }
        let result = self
            .endpoint()
            .update_peer_admission(admission_policy(&self.local_npub, &siblings))
            .await;
        if result.is_err() {
            // Without the barrier acknowledgement, no further reader traffic
            // is safe. Retire only this reader; the live endpoint stays intact.
            self.close_reader().await;
        }
        *self
            .siblings
            .write()
            .unwrap_or_else(|error| error.into_inner()) = siblings;
        *applied = None;
        if let Err(error) = result {
            crate::perflog!("attachment.provider.admission.rebind={error}");
        }
        self.ready_reader(&mut applied).await?;
        Ok(())
    }

    pub(super) async fn prepare_paths(
        &self,
        discovery: &FipsEndpoint,
        local_providers: &[String],
        providers: &[String],
    ) -> Result<Arc<TcpBlobTransport<AttachmentChunkStore>>, StoreError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(StoreError::Other(
                "private attachment reads are closed".into(),
            ));
        }
        // Each local lookup may wait for metadata. Keep one bounded wait for
        // all providers, without holding up a new sibling admission barrier.
        let local_hints = futures_util::future::try_join_all(
            local_providers
                .iter()
                .filter(|npub| self.permits(npub))
                .map(|npub| async move {
                    let identity = fips_core::PeerIdentity::from_npub(npub)
                        .map_err(|error| StoreError::Other(error.to_string()))?;
                    let addresses = discovery
                        .authenticated_local_udp_listen_addrs(identity)
                        .await
                        .map_err(|error| StoreError::Other(error.to_string()))?;
                    Ok::<_, StoreError>((npub, addresses))
                }),
        )
        .await?;
        // Serialize only application of the resulting paths. Recheck current
        // admission after waiting, because device linking may have changed it.
        let mut applied = self.applied_peers.lock().await;
        let endpoint = self
            .ready_reader(&mut applied)
            .await
            .map_err(|error| StoreError::Other(error.to_string()))?;
        let mut peers = self
            .public_peers
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        peers.retain(|peer| self.permits(&peer.npub));
        for (npub, addresses) in local_hints {
            if !self.permits(npub) {
                continue;
            }
            // The authenticated directory also covers providers behind a local
            // anchor. Dial their accepting listener directly, never the anchor
            // or an observed source port that may reject new connections.
            for address in addresses {
                let hint =
                    fips_core::config::PeerAddress::new("udp", address.to_string()).authenticated();
                if let Some(existing) = peers.iter_mut().find(|candidate| &candidate.npub == npub) {
                    if !existing.addresses.contains(&hint) {
                        existing.addresses.push(hint);
                    }
                } else {
                    peers.push(PeerConfig {
                        npub: npub.clone(),
                        addresses: vec![hint],
                        ..Default::default()
                    });
                }
            }
        }
        // Keep provider identities available for routed handshakes when a
        // public transit peer supplies their reachability.
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
            endpoint
                .update_peers(peers)
                .await
                .map_err(|error| StoreError::Other(error.to_string()))?;
            *applied = Some(fingerprint);
        }
        self.transport()
            .map_err(|error| StoreError::Other(error.to_string()))
    }

    pub(super) async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _applied = self.applied_peers.lock().await;
        self.close_reader().await;
    }

    async fn close_reader(&self) {
        let (endpoint, transport) = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|error| error.into_inner());
            (state.endpoint.clone(), state.transport.take())
        };
        // Closing the endpoint closes the TCP worker's receive channel and
        // cancels active requests. Removing our strong reference also prevents
        // the provider route (which retains only a Weak) from starting new work.
        let _ = endpoint.shutdown().await;
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
