//! Public discovery shares one anonymous SDK connection per relay. Account
//! receiving and private publication never obtain this client.
use super::*;
use nostr_pubsub::NostrEventSubscription;
use nostr_pubsub_relay::RelayEventBus;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

mod query;
use query::{client_options, QueryFailure};

#[derive(Debug)]
pub(in crate::core) struct DiscoveryRelays {
    client: Client,
    users: tokio::sync::Mutex<BTreeMap<RelayUrl, usize>>,
    stopped: AtomicBool,
    queries: tokio::sync::Semaphore,
}

impl Default for DiscoveryRelays {
    fn default() -> Self {
        Self {
            // This generated key is only for public-discovery AUTH. It is never
            // an account, device, session or publication key.
            client: Client::builder()
                .signer(Keys::generate())
                .opts(client_options())
                .build(),
            users: Default::default(),
            stopped: AtomicBool::new(false),
            queries: tokio::sync::Semaphore::new(1),
        }
    }
}

impl DiscoveryRelays {
    async fn open(self: &Arc<Self>, urls: &[RelayUrl]) -> Result<RelayLease, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("public discovery stopped".into());
        }
        let mut lease = RelayLease {
            pool: self.clone(),
            urls: Vec::new(),
        };
        let mut users = self.users.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Err("public discovery stopped".into());
        }
        for url in urls.iter().collect::<BTreeSet<_>>() {
            let count = users.entry(url.clone()).or_default();
            *count += 1;
            lease.urls.push(url.clone());
            if *count == 1 {
                self.client
                    .add_relay(url.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                self.client
                    .connect_relay(url.clone())
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(lease)
    }

    pub(in crate::core) async fn fetch(
        self: &Arc<Self>,
        urls: &[RelayUrl],
        filters: Vec<Filter>,
        timeout: Duration,
    ) -> Result<Vec<Event>, String> {
        let lease = self.open(urls).await?;
        lease
            .fetch(filters, timeout)
            .await
            .map_err(|e| e.to_string())
    }

    // Share the bounded query collector and its admission budget, never the
    // anonymous transport. Account reads keep the caller's existing client.
    pub(super) async fn fetch_account(
        &self,
        client: &Client,
        urls: &[RelayUrl],
        filters: Vec<Filter>,
        timeout: Duration,
    ) -> Result<Vec<Event>, String> {
        query::fetch(self, client, urls, filters, timeout)
            .await
            .map_err(|error| error.to_string())
    }

    pub(in crate::core) fn stop(&self, runtime: &tokio::runtime::Handle) {
        self.stopped.store(true, Ordering::Release);
        let client = self.client.clone();
        runtime.spawn(async move {
            client.shutdown().await;
        });
    }
}

// RelayEventBus aborts its forwarder on Drop; CLOSE also needs to remove its
// retained SDK subscription when the other discovery worker keeps the pool alive.
struct LiveSubscription(Option<Box<dyn NostrEventSubscription>>);
impl Drop for LiveSubscription {
    fn drop(&mut self) {
        if let Some(subscription) = self.0.take() {
            tokio::spawn(async move {
                let _ = subscription.close().await;
            });
        }
    }
}

struct RelayLease {
    pool: Arc<DiscoveryRelays>,
    urls: Vec<RelayUrl>,
}

impl RelayLease {
    async fn bus(&self, timeout: Duration) -> Result<RelayEventBus, String> {
        RelayEventBus::with_client(
            self.pool.client.clone(),
            self.urls.iter().map(ToString::to_string),
            timeout,
        )
        .await
        .map_err(|e| e.to_string())
    }
}

impl Drop for RelayLease {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let urls = std::mem::take(&mut self.urls);
        tokio::spawn(async move {
            let mut users = pool.users.lock().await;
            for url in urls {
                if let Some(count) = users.get_mut(&url) {
                    *count -= 1;
                    if *count == 0 {
                        users.remove(&url);
                        let _ = pool.client.remove_relay(url).await;
                    }
                }
            }
        });
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Config {
    relays: Vec<RelayUrl>,
    filters: Vec<Filter>,
}

#[derive(Debug, Default)]
struct Demand {
    all: bool,
    authors: BTreeSet<PublicKey>,
}

enum Destination {
    Protocol(Sender<CoreMsg>),
    Mesh(Arc<fips_core::FipsEndpoint>),
}

impl Destination {
    async fn deliver(&self, event: Event) {
        match self {
            Self::Protocol(sender) => {
                let _ = sender.send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                    event,
                ))));
            }
            Self::Mesh(endpoint) => {
                let _ = endpoint.ingest_nostr_discovery_event(event).await;
            }
        }
    }
}

/// One live subscription worker for protocol metadata, and (when enabled) one
/// for mesh endpoint adverts. Both borrow the same relay pool. Contact count
/// changes filters, never socket, task, timer or TLS-cache counts.
#[derive(Debug)]
pub(in crate::core) struct PublicDiscovery {
    config: tokio::sync::watch::Sender<Option<Config>>,
    demand: Arc<std::sync::Mutex<Demand>>,
    wake: Arc<tokio::sync::Notify>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for PublicDiscovery {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl PublicDiscovery {
    pub(in crate::core) fn new(
        runtime: &tokio::runtime::Handle,
        sender: Sender<CoreMsg>,
        pool: Arc<DiscoveryRelays>,
    ) -> Self {
        Self::start(runtime, pool, Destination::Protocol(sender))
    }

    pub(in crate::core) fn mesh(
        runtime: &tokio::runtime::Handle,
        pool: Arc<DiscoveryRelays>,
        endpoint: Arc<fips_core::FipsEndpoint>,
    ) -> Self {
        Self::start(runtime, pool, Destination::Mesh(endpoint))
    }

    fn start(
        runtime: &tokio::runtime::Handle,
        pool: Arc<DiscoveryRelays>,
        destination: Destination,
    ) -> Self {
        let (config, updates) = tokio::sync::watch::channel(None);
        let demand = Arc::new(std::sync::Mutex::new(Demand::default()));
        let wake = Arc::new(tokio::sync::Notify::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        runtime.spawn(run(
            pool,
            updates,
            demand.clone(),
            wake.clone(),
            destination,
            cancelled.clone(),
        ));
        Self {
            config,
            demand,
            wake,
            cancelled,
        }
    }

    pub(in crate::core) fn is_configured(&self) -> bool {
        self.config.borrow().is_some()
    }

    pub(in crate::core) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // Finish subscription creation before closing it. Aborting midway can
        // strand SDK subscriptions on the pool kept alive by the other worker.
        let _ = self.config.send(None);
    }

    pub(in crate::core) fn configure(
        &self,
        relays: &[RelayUrl],
        filters: Vec<Filter>,
    ) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("public discovery stopped".into());
        }
        let next = Config {
            relays: relays
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            filters: merge_filters(filters),
        };
        if self.config.borrow().as_ref() != Some(&next) {
            self.config
                .send(Some(next))
                .map_err(|_| "public discovery stopped".to_string())?;
        }
        Ok(())
    }

    pub(in crate::core) fn refresh(&self, author: Option<PublicKey>) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        if author.is_some_and(|author| {
            !self.config.borrow().as_ref().is_some_and(|config| {
                config.filters.iter().any(|filter| {
                    filter
                        .authors
                        .as_ref()
                        .is_some_and(|authors| authors.contains(&author))
                })
            })
        }) {
            return;
        }
        if let Ok(mut demand) = self.demand.lock() {
            match author {
                Some(author) => {
                    demand.authors.insert(author);
                }
                None => demand.all = true,
            }
            self.wake.notify_one();
        }
    }
}

/// Combine author sets only when every other selector is identical. Keep tag,
/// kind and time constraints intact; never turn a private recipient filter into
/// public discovery or broaden an unrequested author.
fn merge_filters(filters: Vec<Filter>) -> Vec<Filter> {
    let mut merged: Vec<Filter> = Vec::new();
    for filter in filters {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            let mut left = (*existing).clone();
            let mut right = filter.clone();
            left.authors = None;
            right.authors = None;
            left == right
                && filter.limit.is_none()
                && existing.authors.is_some()
                && filter.authors.is_some()
        }) {
            if let (Some(existing), Some(authors)) = (&mut existing.authors, filter.authors) {
                existing.extend(authors);
            }
        } else if !merged.contains(&filter) {
            merged.push(filter);
        }
    }
    merged
}

async fn run(
    pool: Arc<DiscoveryRelays>,
    mut updates: tokio::sync::watch::Receiver<Option<Config>>,
    demand: Arc<std::sync::Mutex<Demand>>,
    wake: Arc<tokio::sync::Notify>,
    destination: Destination,
    cancelled: Arc<AtomicBool>,
) {
    let (send, mut events) = tokio::sync::mpsc::channel::<Event>(256);
    let handler = Arc::new(move |delivery: nostr_pubsub::QueryEvent| {
        match send.try_send(delivery.event.into_event()) {
            Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => false,
        }
    });
    let mut lease: Option<RelayLease> = None;
    let mut bus: Option<RelayEventBus> = None;
    let mut subscription = LiveSubscription(None);
    let mut applied: Option<Config> = None;
    let mut retry = Duration::from_secs(1);
    let mut retry_at = None;
    let mut query = None::<nostr::util::BoxedFuture<'static, Result<Vec<Event>, QueryFailure>>>;
    let mut seen = HashSet::new();
    let mut order = VecDeque::new();
    loop {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let current = updates.borrow_and_update().clone();
        if current != applied || retry_at.is_some_and(|at| Instant::now() >= at) {
            if let Some(old) = subscription.0.take() {
                let _ = old.close().await;
            }
            query = None;
            if let Some(config) = &current {
                if applied.as_ref().map(|old| &old.relays) != Some(&config.relays) {
                    bus = None;
                    lease = None;
                }
                let result = async {
                    if config.relays.is_empty() || config.filters.is_empty() {
                        return Ok(());
                    }
                    if lease.is_none() {
                        lease = Some(pool.open(&config.relays).await?);
                    }
                    if bus.is_none() {
                        if let Some(lease) = &lease {
                            bus = Some(lease.bus(Duration::from_secs(12)).await?);
                        }
                    }
                    if let Some(bus) = &bus {
                        subscription.0 = Some(
                            bus.subscribe_with_admission(config.filters.clone(), handler.clone())
                                .await
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    Ok::<_, String>(())
                }
                .await;
                if result.is_err() {
                    retry_at = Some(Instant::now() + retry);
                    retry = (retry * 2).min(Duration::from_secs(60));
                } else {
                    retry_at = None;
                    retry = Duration::from_secs(1);
                }
                if config.filters.is_empty() {
                    bus = None;
                    lease = None;
                }
            }
            applied = current;
        }
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            biased;
            changed = updates.changed() => { if changed.is_err() { break; } },
            _ = tokio::time::sleep_until(retry_at.unwrap_or_else(Instant::now)), if retry_at.is_some() => {},
            Some(event) = events.recv() => {
                // Drop bodies from a removed subscription that were queued
                // before its CLOSE; reconfiguration cannot resurrect a peer.
                if applied.as_ref().is_some_and(|config| config.filters.iter().any(|filter|
                    filter.match_event(&event, nostr::filter::MatchEventOptions::new()))) && seen.insert(event.id) {
                    order.push_back(event.id);
                    if order.len() > 4096 { if let Some(id) = order.pop_front() { seen.remove(&id); } }
                    destination.deliver(event).await;
                }
            },
            _ = wake.notified(), if query.is_none() && bus.is_some() => {
                let requested = demand.lock().map(|mut value| std::mem::take(&mut *value)).unwrap_or_default();
                let filters = applied.as_ref().map(|config| config.filters.iter().filter_map(|filter| {
                    if requested.all { return Some(filter.clone()); }
                    let authors = filter.authors.as_ref()?.intersection(&requested.authors).copied().collect::<BTreeSet<_>>();
                    if authors.is_empty() { return None; }
                    let mut filter = filter.clone();
                    filter.authors = Some(authors);
                    Some(filter)
                }).collect::<Vec<_>>()).unwrap_or_default();
                if !filters.is_empty() {
                    if let Some(lease) = &lease {
                        let pool = lease.pool.clone();
                        let urls = lease.urls.clone();
                        query = Some(Box::pin(async move {
                            let mut retry = Duration::from_secs(1);
                            loop {
                                match query::fetch(&pool, &pool.client, &urls, filters.clone(), Duration::from_secs(12)).await {
                                    Ok(events) => return Ok(events),
                                    Err(QueryFailure::Limit) => return Err(QueryFailure::Limit),
                                    Err(QueryFailure::Transport(_)) => {}
                                }
                                sleep(retry).await;
                                retry = (retry * 2).min(Duration::from_secs(60));
                            }
                        }));
                    }
                }
            },
            report = async {
                match &mut query {
                    Some(query) => query.await,
                    None => std::future::pending().await,
                }
            }, if query.is_some() => {
                query = None;
                // Explicit recovery must redeliver unchanged events.
                if let Ok(events) = report {
                    for event in events { destination.deliver(event).await; }
                }
            },
        }
    }
    drop(query);
    if let Some(subscription) = subscription.0.take() {
        let _ = subscription.close().await;
    }
}

#[cfg(test)]
pub(in crate::core) mod test_support;
#[cfg(test)]
mod tests;
