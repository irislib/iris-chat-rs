use super::*;
use crate::core::protocol::anonymous_reads::{
    idle_connections, ScopedReadConnection, ScopedTlsConnector,
};
use crate::core::protocol::private_reads::DiscoveryCadence;
use fips_core::config::{PeerConfig, TransportInstances, UdpConfig, WebSocketConfig};
use fips_core::{Config, FipsEndpoint};
use nostr_pubsub::QueryOptions;
use nostr_pubsub_fips::{FipsAnonymousPublisher, FipsDiscoveryScope, FipsPubsubClientOptions};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(12);

fn is_peer_advert_filter(filter: &Filter) -> bool {
    filter
        .kinds
        .as_ref()
        .is_some_and(|kinds| !kinds.is_empty() && kinds.iter().all(|kind| kind.as_u16() == 37195))
}

#[derive(Default)]
struct DiscoveryObservations {
    seen: HashSet<nostr::EventId>,
    order: std::collections::VecDeque<nostr::EventId>,
    advert_valid_until: Option<u64>,
    advert: Option<Event>,
}

impl DiscoveryObservations {
    fn remember(&mut self, event: &Event, advert_ttl_secs: u64) -> bool {
        let unseen = self.seen.insert(event.id);
        if unseen {
            self.order.push_back(event.id);
            if self.order.len() > 128 {
                if let Some(old) = self.order.pop_front() {
                    self.seen.remove(&old);
                }
            }
        }
        if event.kind.as_u16() == 37195 {
            // FIPS also limits the signed expiration by its local maximum
            // advert age. Never let quiet polling sleep past either bound.
            let max_age_until = event
                .created_at
                .as_secs()
                .saturating_add(advert_ttl_secs.saturating_mul(2));
            self.advert_valid_until =
                Some(event.tags.expiration().map_or(max_age_until, |expiration| {
                    expiration.as_secs().min(max_age_until)
                }));
            // Renewed signatures/timestamps/expiration are liveness, not new
            // endpoint semantics. Every accepted event still reaches FIPS and
            // updates this signed deadline, including an identical replay.
            // Multiple different adverts from one signed timestamp can all
            // remain acceptable to FIPS. Replaying A then B must not restart
            // the fast cadence on every poll just because their fields differ.
            let changed = unseen
                && self.advert.as_ref().is_none_or(|previous| {
                    previous.pubkey != event.pubkey
                        || previous.kind != event.kind
                        || previous.content != event.content
                        || !advert_semantic_tags(previous).eq(advert_semantic_tags(event))
                });
            self.advert = Some(event.clone());
            return changed;
        }
        unseen
    }

    fn delay(&mut self, quiet_delay: Duration, adverts: bool, now: u64) -> Duration {
        if !adverts {
            return quiet_delay;
        }
        match self
            .advert_valid_until
            .and_then(|until| until.checked_sub(now))
        {
            Some(remaining) if remaining > 0 => quiet_delay.min(Duration::from_secs(remaining)),
            // With no future deadline there is nothing to refresh before.
            // Quiet missing/expired adverts back off too; real route demand
            // wakes the exact peer. Admission/network time remains additional.
            _ => {
                self.advert_valid_until = None;
                quiet_delay
            }
        }
    }
}

fn advert_semantic_tags(event: &Event) -> impl Iterator<Item = &nostr::Tag> {
    event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) != Some("expiration"))
}

#[derive(Default)]
struct DiscoveryPoll {
    attempted: bool,
    changed: bool,
    closed: bool,
}

impl DiscoveryPoll {
    fn next_delay(&self, cadence: &mut DiscoveryCadence) -> Option<Duration> {
        self.attempted.then(|| cadence.after_poll(self.changed))
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::core) mod tls_tests;

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
    #[cfg(test)]
    tls_test_roots: Vec<(RelayUrl, rustls::RootCertStore)>,
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
        // Disposable clients must not volunteer to route unrelated traffic.
        config.node.leaf_only = true;
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
            #[cfg(test)]
            tls_test_roots: Vec::new(),
        }
    }

    fn advert_tls(&self, filters: &[Filter]) -> Vec<Option<ScopedTlsConnector>> {
        self.relays
            .iter()
            .map(|relay| {
                #[cfg(test)]
                if let Some((_, roots)) = self.tls_test_roots.iter().find(|(url, _)| url == relay) {
                    return ScopedTlsConnector::new_with_roots(relay, filters, roots.clone());
                }
                ScopedTlsConnector::new(relay, filters)
            })
            .collect()
    }

    #[cfg(test)]
    pub(in crate::core) fn set_test_tls_roots(
        &mut self,
        relay: &RelayUrl,
        certificates: Vec<Vec<u8>>,
    ) -> Result<(), String> {
        use sha2::Digest;
        if !self.relays.contains(relay) || certificates.is_empty() {
            return Err("test roots require an exact configured mesh relay".into());
        }
        let mut roots = rustls::RootCertStore::empty();
        let mut digest = sha2::Sha256::new();
        for certificate in certificates {
            digest.update(&certificate);
            roots
                .add(rustls::pki_types::CertificateDer::from(certificate))
                .map_err(|error| error.to_string())?;
        }
        self.tls_test_roots.retain(|(url, _)| url != relay);
        self.tls_test_roots.push((relay.clone(), roots));
        self.key
            .push_str(&format!(":test-roots:{relay}:{:x}", digest.finalize()));
        Ok(())
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
        observations: &mut DiscoveryObservations,
        advert_tls: &[Option<ScopedTlsConnector>],
        connections: &mut [ScopedReadConnection],
    ) -> DiscoveryPoll {
        let mut poll = DiscoveryPoll::default();
        let Some(filter) = filters.first() else {
            return poll;
        };
        let Ok(scope) = FipsDiscoveryScope::from_filter(filter) else {
            return poll;
        };
        let Ok(_slot) = self.discovery_slots.acquire().await else {
            return poll;
        };
        // FIPS's External mode never opens its own relay client. Cold live-peer
        // adverts are fetched here on a one-author anonymous context instead.
        let adverts = filters
            .iter()
            .filter(|filter| is_peer_advert_filter(filter))
            .cloned()
            .collect::<Vec<_>>();
        if !adverts.is_empty() && !self.relays.is_empty() {
            poll.attempted = true;
            if let Ok(events) = crate::core::protocol::anonymous_reads::poll_relays_reusing(
                &self.relays,
                &adverts,
                advert_tls,
                connections,
            )
            .await
            {
                for event in events {
                    self.deliver_discovery(event, &live, &sender, observations, &mut poll)
                        .await;
                }
            }
        }
        if !self.available() {
            return poll;
        }
        poll.attempted = true;
        let Ok(endpoint) = self.endpoint().await else {
            return poll;
        };
        let Ok(client) = FipsPubsubClient::start_anonymous_discovery(
            endpoint.0.clone(),
            self.options.clone(),
            scope,
        )
        .await
        else {
            return poll;
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
                self.deliver_discovery(
                    delivery.event.into_event(),
                    &live,
                    &sender,
                    observations,
                    &mut poll,
                )
                .await;
            }
        }
        client.shutdown_shared().await;
        poll
    }

    async fn deliver_discovery(
        &self,
        event: Event,
        live: &FipsEndpoint,
        sender: &Sender<CoreMsg>,
        observations: &mut DiscoveryObservations,
        poll: &mut DiscoveryPoll,
    ) {
        if event.kind.as_u16() != 37195 && observations.seen.contains(&event.id) {
            return;
        }
        if event.kind.as_u16() == 37195 {
            // An event is new discovery data only after the endpoint accepts
            // its app/schema/freshness. Re-ingesting an equal advert reports
            // Replaced too, so that boolean alone is not a change detector.
            // Still offer it to FIPS: an exact-peer demand may be repairing an
            // evicted cache entry. Semantic equality suppresses only cadence
            // resets, never this cache repair or signed lifetime update.
            if !live
                .ingest_nostr_discovery_event(event.clone())
                .await
                .unwrap_or(false)
            {
                return;
            }
        } else if sender
            .send(CoreMsg::Internal(Box::new(InternalEvent::MeshEvent(
                event.clone(),
            ))))
            .is_err()
        {
            poll.closed = true;
            return;
        }
        poll.changed |=
            observations.remember(&event, self.config.node.discovery.nostr.advert_ttl_secs);
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

#[derive(Default)]
pub(in crate::core) struct PrivateMeshDiscovery {
    provider_key: String,
    workers: BTreeMap<String, DiscoveryWorker>,
}

struct DiscoveryWorker {
    filters: Vec<Filter>,
    author: PublicKey,
    wake: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for DiscoveryWorker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PrivateMeshDiscovery {
    pub(in crate::core) fn refresh_author(&self, author: PublicKey) {
        for worker in self
            .workers
            .values()
            .filter(|worker| worker.author == author)
        {
            worker.wake.notify_one();
        }
    }

    pub(in crate::core) fn refresh_all(&self) {
        for worker in self.workers.values() {
            worker.wake.notify_one();
        }
    }

    pub(in crate::core) fn update(
        &mut self,
        providers: Arc<PrivateMeshProviders>,
        filters: Vec<Filter>,
        live: Arc<FipsEndpoint>,
        sender: Sender<CoreMsg>,
        runtime: &tokio::runtime::Runtime,
    ) {
        if self.provider_key != providers.key {
            self.workers.clear();
            self.provider_key.clone_from(&providers.key);
        }
        if !providers.available() && providers.relays.is_empty() {
            self.workers.clear();
            return;
        }
        let mut scopes = std::collections::BTreeMap::<String, Vec<Filter>>::new();
        for filter in filters {
            // Without an explicit FIPS provider, only adverts have a usable
            // source (the anonymous relay reader). Do not retain no-I/O timers.
            if !providers.available() && !is_peer_advert_filter(&filter) {
                continue;
            }
            let Ok(scope) = FipsDiscoveryScope::from_filter(&filter) else {
                continue;
            };
            scopes.entry(format!("{scope:?}")).or_default().push(filter);
        }
        // Refuse excess scopes rather than combining unrelated lookup subjects.
        if scopes.len() > 512 {
            self.workers.clear();
            return;
        }
        self.workers
            .retain(|key, worker| scopes.get(key) == Some(&worker.filters));
        for (key, filters) in scopes {
            if self.workers.contains_key(&key) {
                continue;
            }
            let Some(scope) = filters
                .first()
                .and_then(|filter| FipsDiscoveryScope::from_filter(filter).ok())
            else {
                continue;
            };
            let author = match scope {
                FipsDiscoveryScope::Author(author) | FipsDiscoveryScope::Device(author) => author,
            };
            let adverts = filters.iter().any(|filter| {
                filter
                    .kinds
                    .as_ref()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_u16() == 37195))
            });
            let providers = providers.clone();
            let live = live.clone();
            let sender = sender.clone();
            let wake = Arc::new(tokio::sync::Notify::new());
            let requested = wake.clone();
            let worker_filters = filters.clone();
            let task = runtime.spawn(async move {
                // TLS tickets and key-exchange hints belong only to this exact
                // worker configuration. Ordinary demand retains them; dropping
                // or replacing the worker drops them even during a blocked read.
                let advert_filters = worker_filters
                    .iter()
                    .filter(|filter| is_peer_advert_filter(filter))
                    .cloned()
                    .collect::<Vec<_>>();
                let advert_tls = providers.advert_tls(&advert_filters);
                let mut connections = providers
                    .relays
                    .iter()
                    .map(|relay| ScopedReadConnection::new(relay, &advert_filters))
                    .collect::<Vec<_>>();
                let mut observations = DiscoveryObservations::default();
                let mut cadence = DiscoveryCadence::default();
                loop {
                    let poll = providers
                        .discover(
                            worker_filters.clone(),
                            live.clone(),
                            sender.clone(),
                            &mut observations,
                            &advert_tls,
                            &mut connections,
                        )
                        .await;
                    if poll.closed {
                        break;
                    }
                    // Unchanged and failed reads both back off. Only accepted
                    // changed semantics or explicit demand resets the quiet schedule.
                    let Some(delay) = poll.next_delay(&mut cadence) else {
                        break;
                    };
                    let delay = observations.delay(delay, adverts, unix_now().get());
                    // A real demand is coalesced during a busy query. It never
                    // interrupts the in-flight anonymous authentication/read.
                    tokio::select! {
                        _ = idle_connections(&mut connections) => {},
                        _ = sleep(delay) => {},
                        _ = requested.notified() => cadence.reset(),
                    }
                }
            });
            self.workers.insert(
                key,
                DiscoveryWorker {
                    filters,
                    author,
                    wake,
                    task,
                },
            );
        }
    }
}
