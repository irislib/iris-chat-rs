use super::*;
use fips_core::config::{PeerConfig, TransportInstances, UdpConfig, WebSocketConfig};
use fips_core::{Config, FipsEndpoint};
use nostr_pubsub::QueryOptions;
use nostr_pubsub_fips::{FipsAnonymousPublisher, FipsDiscoveryScope, FipsPubsubClientOptions};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(12);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);

#[cfg(test)]
mod tests;

/// Public infrastructure only. Never inherit contacts, account discovery,
/// listeners, local rendezvous, host BLE, or an identified endpoint's key.
#[derive(Clone)]
pub(in crate::core) struct PrivateMeshProviders {
    config: Config,
    options: FipsPubsubClientOptions,
    slots: Arc<tokio::sync::Semaphore>,
    discovery_slots: Arc<tokio::sync::Semaphore>,
    relays: Vec<RelayUrl>,
    key: String,
}

impl PrivateMeshProviders {
    pub(in crate::core) fn new(
        peers: Vec<PeerConfig>,
        websocket: Option<WebSocketConfig>,
        providers: Vec<String>,
        relays: Vec<RelayUrl>,
    ) -> Self {
        let key = format!("{peers:?}:{websocket:?}:{providers:?}:{relays:?}");
        let mut config = Config::new();
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.local.enabled = false;
        config.node.discovery.lan.enabled = false;
        config.peers = peers;
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some("0.0.0.0:0".into()),
            advertise_on_nostr: Some(false),
            accept_connections: Some(true),
            public: Some(false),
            ..Default::default()
        });
        if let Some(websocket) = websocket {
            config.transports.websocket = TransportInstances::Single(WebSocketConfig {
                seed_urls: websocket.seed_urls,
                ..Default::default()
            });
        }
        Self {
            config,
            options: FipsPubsubClientOptions {
                routed_peers: providers,
                query_timeout: OPERATION_TIMEOUT,
                max_replay_events: MESH_REPLAY_EVENTS,
                ..Default::default()
            },
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
            discovery_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            relays,
            key,
        }
    }

    pub(in crate::core) fn available(&self) -> bool {
        !self.options.routed_peers.is_empty()
    }

    async fn endpoint(&self) -> Result<PrivateEndpoint, String> {
        let endpoint = FipsEndpoint::builder()
            .config(self.config.clone())
            .identity_nsec(Keys::generate().secret_key().to_secret_hex())
            .without_system_tun()
            .bind()
            .await
            .map_err(|error| error.to_string())?;
        Ok(PrivateEndpoint(Arc::new(endpoint)))
    }

    pub(in crate::core) async fn publish(
        &self,
        event: VerifiedEvent,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), String> {
        if !self.available() {
            return Err("no approved private message provider".into());
        }
        let _slot = self
            .slots
            .acquire()
            .await
            .map_err(|error| error.to_string())?;
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("private publication cancelled".into());
        }
        let endpoint = self.endpoint().await?;
        let publisher = FipsAnonymousPublisher::start(endpoint.0.clone(), self.options.clone())
            .await
            .map_err(|error| error.to_string())?;
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("private publication cancelled".into());
        }
        // Only a provider-bound remote OK confirms admission. The application's
        // durable queue still owns retries and recipient receipt tracking.
        publisher
            .publish_once(event, OPERATION_TIMEOUT)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn discover(
        &self,
        filters: Vec<Filter>,
        live: Arc<FipsEndpoint>,
        sender: Sender<CoreMsg>,
    ) {
        let Some(filter) = filters.first() else {
            return;
        };
        let Ok(scope) = FipsDiscoveryScope::from_filter(filter) else {
            return;
        };
        let Ok(_slot) = self.discovery_slots.acquire().await else {
            return;
        };
        // FIPS's External mode never opens its own relay client. Cold live-peer
        // adverts are fetched here on a one-author anonymous context instead.
        let adverts = filters
            .iter()
            .filter(|filter| {
                filter
                    .kinds
                    .as_ref()
                    .is_some_and(|kinds| kinds.iter().all(|kind| kind.as_u16() == 37195))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !adverts.is_empty() && !self.relays.is_empty() {
            let reader = PrivateRelayReader(Client::new(Keys::generate()));
            ensure_session_relays_configured(&reader.0, &self.relays).await;
            connect_client_with_timeout(&reader.0, Duration::from_secs(3)).await;
            for filter in adverts {
                if let Ok(events) = reader.0.fetch_events(filter, Duration::from_secs(3)).await {
                    for event in events.iter() {
                        let _ = live.ingest_nostr_discovery_event(event.clone()).await;
                    }
                }
            }
            let _ = reader.0.shutdown().await;
        }
        if !self.available() {
            return;
        }
        let Ok(endpoint) = self.endpoint().await else {
            return;
        };
        let Ok(client) = FipsPubsubClient::start_anonymous_discovery(
            endpoint.0.clone(),
            self.options.clone(),
            scope,
        )
        .await
        else {
            return;
        };
        if let Ok(result) = EventBus::query(
            &client,
            filters,
            QueryOptions {
                limit: Some(MESH_REPLAY_EVENTS),
            },
        )
        .await
        {
            for delivery in result.events {
                let event = delivery.event.into_event();
                if event.kind.as_u16() == 37195 {
                    let _ = live.ingest_nostr_discovery_event(event).await;
                } else {
                    let _ =
                        sender.send(CoreMsg::Internal(Box::new(InternalEvent::MeshEvent(event))));
                }
            }
        }
        client.shutdown_shared().await;
    }
}

/// Also shuts down after cancellation, including a blocked query/publication.
struct PrivateEndpoint(Arc<FipsEndpoint>);
impl Drop for PrivateEndpoint {
    fn drop(&mut self) {
        let endpoint = self.0.clone();
        tokio::spawn(async move {
            let _ = endpoint.shutdown().await;
        });
    }
}

struct PrivateRelayReader(Client);
impl Drop for PrivateRelayReader {
    fn drop(&mut self) {
        let client = self.0.clone();
        tokio::spawn(async move {
            let _ = client.shutdown().await;
        });
    }
}

#[derive(Default)]
pub(super) struct PrivateMeshDiscovery {
    filters: Vec<Filter>,
    provider_key: String,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for PrivateMeshDiscovery {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl PrivateMeshDiscovery {
    pub(super) fn update(
        &mut self,
        providers: Arc<PrivateMeshProviders>,
        filters: Vec<Filter>,
        live: Arc<FipsEndpoint>,
        sender: Sender<CoreMsg>,
        runtime: &tokio::runtime::Runtime,
    ) {
        if self.filters == filters && self.provider_key == providers.key {
            return;
        }
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.filters = filters.clone();
        self.provider_key.clone_from(&providers.key);
        if !providers.available() && providers.relays.is_empty() {
            return;
        }
        let mut scopes = std::collections::BTreeMap::<String, Vec<Filter>>::new();
        for filter in filters {
            let Ok(scope) = FipsDiscoveryScope::from_filter(&filter) else {
                continue;
            };
            scopes.entry(format!("{scope:?}")).or_default().push(filter);
        }
        // Refuse excess scopes rather than combining unrelated lookup subjects.
        if scopes.len() > 512 {
            return;
        }
        for filters in scopes.into_values() {
            let providers = providers.clone();
            let live = live.clone();
            let sender = sender.clone();
            self.tasks.push(runtime.spawn(async move {
                loop {
                    providers
                        .discover(filters.clone(), live.clone(), sender.clone())
                        .await;
                    sleep(DISCOVERY_INTERVAL).await;
                }
            }));
        }
    }
}
