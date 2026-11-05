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
    route: RwLock<ReaderRoute>,
}

struct ReaderRoute {
    transport: Weak<TcpBlobTransport<AttachmentChunkStore>>,
    route: Arc<FipsBlobRoute<AttachmentChunkStore>>,
}

impl ProviderRoute {
    pub(super) fn new(
        discovery: Arc<FipsEndpoint>,
        reads: Arc<provider_reads::ProviderReads>,
        explicit: Vec<PeerIdentity>,
    ) -> anyhow::Result<Self> {
        let transport = reads.transport()?;
        Ok(Self {
            discovery,
            route: RwLock::new(ReaderRoute {
                transport: Arc::downgrade(&transport),
                route: Arc::new(FipsBlobRoute::explicit(transport, Vec::new(), 4)?),
            }),
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

    async fn prepare(&self) -> Result<Arc<FipsBlobRoute<AttachmentChunkStore>>, StoreError> {
        let local_providers = self.discovered_provider_ids()?;
        let discovered = local_providers
            .iter()
            .filter_map(|npub| PeerIdentity::from_npub(npub).ok())
            .collect();
        let explicit = self
            .explicit
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .copied()
            .filter(|peer| self.reads.permits(&peer.npub()))
            .collect();
        let mut peers = ordered_provider_peers(discovered, explicit);
        let transport = self
            .reads
            .prepare_paths(
                &self.discovery,
                &local_providers,
                &peers.iter().map(|peer| peer.npub()).collect::<Vec<_>>(),
            )
            .await?;
        peers.retain(|peer| self.reads.permits(&peer.npub()));
        let mut current = self
            .route
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if !current
            .transport
            .upgrade()
            .is_some_and(|previous| Arc::ptr_eq(&previous, &transport))
        {
            // In-flight requests retain only the retired reader. A replacement
            // can never redirect their private GETs onto the identified endpoint.
            *current = ReaderRoute {
                transport: Arc::downgrade(&transport),
                route: Arc::new(
                    FipsBlobRoute::explicit(transport, Vec::new(), 4)
                        .map_err(|error| StoreError::Other(error.to_string()))?,
                ),
            };
        }
        current.route.set_explicit_peers(peers);
        Ok(current.route.clone())
    }
}

// Keep the shared route's candidate order: deduplicate each input and remove
// overlaps before interleaving. Filtering only afterward changes which peers
// fit into its four-attempt window. Attempts and retry rotation remain there.
fn ordered_provider_peers(
    mut discovered: Vec<PeerIdentity>,
    mut explicit: Vec<PeerIdentity>,
) -> Vec<PeerIdentity> {
    let mut discovered_ids = HashSet::new();
    discovered.retain(|peer| discovered_ids.insert(peer.npub()));
    let mut explicit_ids = HashSet::new();
    explicit.retain(|peer| {
        let npub = peer.npub();
        !discovered_ids.contains(&npub) && explicit_ids.insert(npub)
    });
    let mut peers = Vec::with_capacity(discovered.len() + explicit.len());
    for index in 0..discovered.len().max(explicit.len()) {
        peers.extend(discovered.get(index));
        peers.extend(explicit.get(index));
    }
    peers
}

#[async_trait]
impl BlobRoute for ProviderRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        self.prepare().await?.route(request).await
    }

    async fn route_with_context(
        &self,
        request: BlobRequest,
        context: BlobRouteContext,
    ) -> Result<BlobReply, StoreError> {
        self.prepare()
            .await?
            .route_with_context(request, context)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_overlap_and_duplicates_preserve_the_provider_attempt_window() {
        let [a, b, c, d, e] = std::array::from_fn(|_| {
            PeerIdentity::from_npub(&Keys::generate().public_key().to_bech32().unwrap()).unwrap()
        });
        let expected = vec![a, d, b, e, c];
        assert_eq!(
            ordered_provider_peers(vec![a, b, c], vec![a, d, e]),
            expected
        );
        assert_eq!(
            ordered_provider_peers(vec![a, a, b, c], vec![d, d, e]),
            expected
        );
        assert_eq!(ordered_provider_peers(vec![], vec![d, d, e]), vec![d, e]);
    }
}
