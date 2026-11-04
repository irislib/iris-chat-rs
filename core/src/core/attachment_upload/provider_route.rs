use super::*;
use fips_core::discovery::local::rank_capability_providers;
use fips_core::PeerIdentity;
use hashtree_core::{BlobReply, BlobRequest, BlobRouteContext};
use hashtree_fips_transport::{TCP_BLOB_CAPABILITY, TCP_BLOB_SERVICE_PORT};

/// Reuse public discovery in memory, then establish independent anonymous
/// paths. FipsBlobRoute remains the sole owner of provider attempts/retries.
pub(super) struct ProviderRoute {
    discovery: Arc<FipsEndpoint>,
    reads: Arc<provider_reads::ProviderReads>,
    explicit: RwLock<Vec<PeerIdentity>>,
    route: FipsBlobRoute<AttachmentChunkStore>,
}

impl ProviderRoute {
    pub(super) fn new(
        discovery: Arc<FipsEndpoint>,
        reads: Arc<provider_reads::ProviderReads>,
        explicit: Vec<PeerIdentity>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            discovery,
            route: FipsBlobRoute::explicit(reads.transport()?, Vec::new(), 4)?,
            reads,
            explicit: RwLock::new(explicit),
        })
    }

    pub(super) fn set_explicit_peers(&self, peers: Vec<PeerIdentity>) {
        *self
            .explicit
            .write()
            .unwrap_or_else(|error| error.into_inner()) = peers;
    }

    pub(super) fn discovered_provider_ids(&self) -> Result<Vec<String>, StoreError> {
        let adverts = self
            .discovery
            .local_instance_advertisements()
            .map_err(|error| StoreError::Other(error.to_string()))?;
        Ok(rank_capability_providers(&adverts, TCP_BLOB_CAPABILITY)
            .into_iter()
            .filter(|advert| self.reads.permits(&advert.npub))
            .filter(|advert| {
                advert
                    .capability(TCP_BLOB_CAPABILITY)
                    .and_then(|capability| capability.fsp_port)
                    == Some(TCP_BLOB_SERVICE_PORT)
            })
            .map(|advert| advert.npub.clone())
            .collect())
    }

    async fn prepare(&self) -> Result<(), StoreError> {
        let discovered = self.discovered_provider_ids()?;
        let explicit = self
            .explicit
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .copied()
            .filter(|peer| !discovered.contains(&peer.npub()))
            .collect::<Vec<_>>();
        // Interleave the discovery ranking with explicit candidates, then let
        // the shared route apply its bounded per-hash attempt window.
        let mut peers = Vec::new();
        for index in 0..discovered.len().max(explicit.len()) {
            for peer in [
                discovered
                    .get(index)
                    .and_then(|npub| PeerIdentity::from_npub(npub).ok()),
                explicit.get(index).copied(),
            ]
            .into_iter()
            .flatten()
            {
                if self.reads.permits(&peer.npub()) && !peers.contains(&peer) {
                    peers.push(peer);
                }
            }
        }
        self.reads
            .prepare_paths(
                &self.discovery,
                &peers.iter().map(|peer| peer.npub()).collect::<Vec<_>>(),
            )
            .await?;
        self.route.set_explicit_peers(peers);
        Ok(())
    }
}

#[async_trait]
impl BlobRoute for ProviderRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        self.prepare().await?;
        self.route.route(request).await
    }

    async fn route_with_context(
        &self,
        request: BlobRequest,
        context: BlobRouteContext,
    ) -> Result<BlobReply, StoreError> {
        self.prepare().await?;
        self.route.route_with_context(request, context).await
    }
}
