use super::anonymous_reads::diagnostics;
use super::*;

const MAX_PRIVATE_DISCOVERY_SCOPES: usize = 512;

/// Quiet public discovery waits at most two minutes between polling attempts;
/// bounded admission and network time still apply. Live ratchet and
/// invite-recipient subscriptions do not use this scheduler.
#[derive(Debug)]
pub(in crate::core) struct DiscoveryCadence {
    next: Duration,
}

impl Default for DiscoveryCadence {
    fn default() -> Self {
        Self {
            next: Duration::from_secs(30),
        }
    }
}

impl DiscoveryCadence {
    pub(in crate::core) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(in crate::core) fn after_poll(&mut self, changed: bool) -> Duration {
        if changed {
            self.reset();
        }
        let delay = self.next;
        self.next = (delay * 2).min(Duration::from_secs(120));
        delay
    }
}

#[derive(Default)]
struct DiscoverySnapshot(Option<[u8; 32]>);

impl DiscoverySnapshot {
    fn observe(&mut self, events: &[Event]) -> bool {
        use sha2::Digest;
        // A poll is already bounded by anonymous_reads' result limits. Retain
        // only its digest between rounds; a tiny delivery-ID cache must not
        // classify a stable response larger than that cache as new forever.
        let mut ids = events.iter().map(|event| event.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let mut digest = sha2::Sha256::new();
        for id in &ids {
            digest.update(id.as_bytes());
        }
        let digest = digest.finalize().into();
        let changed = self
            .0
            .map_or(!ids.is_empty(), |previous| previous != digest);
        self.0 = Some(digest);
        changed
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DiscoveryConfig {
    relays: Vec<RelayUrl>,
    filters: Vec<Filter>,
    #[cfg(test)]
    tls_test_roots: Vec<(RelayUrl, Vec<Vec<u8>>)>,
}

impl DiscoveryConfig {
    fn tls_connector(
        &self,
        relay: &RelayUrl,
    ) -> Option<super::anonymous_reads::ScopedTlsConnector> {
        #[cfg(test)]
        if let Some((_, certificates)) = self
            .tls_test_roots
            .iter()
            .find(|(test_relay, _)| test_relay == relay)
        {
            return super::anonymous_reads::ScopedTlsConnector::new_with_roots(
                relay,
                &self.filters,
                test_tls_roots(certificates).expect("validated discovery test roots"),
            );
        }
        super::anonymous_reads::ScopedTlsConnector::new(relay, &self.filters)
    }
}

#[cfg(test)]
fn test_tls_roots(certificates: &[Vec<u8>]) -> Result<rustls::RootCertStore, String> {
    if certificates.is_empty() {
        return Err("discovery test roots are empty".into());
    }
    let mut roots = rustls::RootCertStore::empty();
    for certificate in certificates {
        roots
            .add(rustls::pki_types::CertificateDer::from(certificate.clone()))
            .map_err(|error| error.to_string())?;
    }
    Ok(roots)
}

/// Retained state is configuration, bounded author/relay TLS tickets, positive
/// signature tuples and read-only sockets, and a cancellable task, never an SDK client.
/// Dropping the last descriptor cancels even a queued read and releases its state.
#[derive(Debug)]
pub(in crate::core) struct PrivateDiscoveryClient {
    config: tokio::sync::watch::Sender<Option<DiscoveryConfig>>,
    refresh: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
    cancelled: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    signature_calls: Arc<std::sync::atomic::AtomicUsize>,
}

pub(in crate::core) type PrivateDiscoverySubscription =
    (std::sync::Weak<PrivateDiscoveryClient>, Vec<Filter>);

impl Drop for PrivateDiscoveryClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl PrivateDiscoveryClient {
    fn new(runtime: &tokio::runtime::Handle, sender: Sender<CoreMsg>) -> Self {
        let (config, mut updates) = tokio::sync::watch::channel(None::<DiscoveryConfig>);
        let refresh = Arc::new(tokio::sync::Notify::new());
        let demanded = refresh.clone();
        let mut verification = super::anonymous_reads::VerificationCache::default();
        #[cfg(test)]
        let signature_calls = verification.signature_calls.clone();
        // Construct before spawning so dropping an unpolled task still balances it.
        let mut diagnostic_scope = diagnostics::Scope::new();
        let task = runtime.spawn(async move {
            let mut seen = HashSet::new();
            let mut order = std::collections::VecDeque::new();
            let mut cadence = DiscoveryCadence::default();
            let mut snapshots = Vec::<DiscoverySnapshot>::new();
            let mut tls_connectors = Vec::new();
            let mut connections = Vec::new();
            let mut tls_config = None;
            let mut redeliver = false;
            loop {
                let current = updates.borrow_and_update().clone();
                let Some(current) = current else {
                    if updates.changed().await.is_err() {
                        break;
                    }
                    continue;
                };
                snapshots.resize_with(current.relays.len(), DiscoverySnapshot::default);
                // A watch update can arrive between a completed sleep and
                // borrow_and_update, without selecting the changed branch.
                // Bind retained tickets and signature results to the actual
                // configuration, including the ordering of relay indices.
                if tls_config.as_ref() != Some(&current) {
                    diagnostic_scope.configured();
                    verification.clear();
                    tls_connectors = current
                        .relays
                        .iter()
                        .map(|relay| current.tls_connector(relay))
                        .collect();
                    connections = current
                        .relays
                        .iter()
                        .map(|relay| {
                            super::anonymous_reads::ScopedReadConnection::new(
                                relay,
                                &current.filters,
                            )
                        })
                        .collect();
                    tls_config = Some(current.clone());
                }
                let mut changed = false;
                let mut read_succeeded = false;
                let cycle = async {
                    for (relay_index, ((relay, snapshot), tls)) in current
                        .relays
                        .iter()
                        .zip(snapshots.iter_mut())
                        .zip(&tls_connectors)
                        .enumerate()
                    {
                        let (before, remaining) = connections.split_at_mut(relay_index);
                        let Some((connection, after)) = remaining.split_first_mut() else {
                            break;
                        };
                        // Other exact scopes may already have healthy sockets.
                        // Keep their bounded control handling alive while this
                        // read waits for admission or a relay response. Cancelling
                        // idle handling retains healthy sockets and opens no REQ.
                        let result = tokio::select! {
                            result = super::anonymous_reads::poll_retained(
                                relay,
                                current.filters.clone(),
                                tls.as_ref(),
                                verification.relay(relay_index),
                                Some(connection),
                            ) => result,
                            _ = async {
                                tokio::join!(
                                    super::anonymous_reads::idle_connections(before),
                                    super::anonymous_reads::idle_connections(after),
                                );
                            } => unreachable!("idle handling never starts another poll"),
                        };
                        if let Ok(events) = result {
                            read_succeeded = true;
                            let response_changed = snapshot.observe(&events);
                            changed |= response_changed;
                            if !response_changed && !redeliver {
                                continue;
                            }
                            for event in events {
                                let unseen = seen.insert(event.id);
                                if !unseen && !redeliver {
                                    continue;
                                }
                                if unseen {
                                    order.push_back(event.id);
                                    if order.len() > 128 {
                                        if let Some(old) = order.pop_front() {
                                            seen.remove(&old);
                                        }
                                    }
                                }
                                if sender
                                    .send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
                                        event,
                                    ))))
                                    .is_err()
                                {
                                    return true;
                                }
                            }
                        }
                    }
                    // Re-read the original filters, rather than advancing a
                    // timestamp past late-published metadata. Core deduplicates
                    // IDs. EOSE gaps, reconnects and restarts cannot lose an
                    // older AppKeys/invite update through a moving watermark.
                    false
                };
                tokio::select! {
                    stopped = cycle => {
                        if stopped { break; }
                        diagnostic_scope.sweep_completed();
                    },
                    result = updates.changed() => {
                        if result.is_err() { break; }
                        cadence.reset();
                        snapshots.clear();
                        tls_connectors.clear();
                        connections.clear();
                        tls_config = None;
                        verification.clear();
                        continue;
                    },
                }
                // A failed attempt does not satisfy explicit recovery demand.
                if read_succeeded {
                    redeliver = false;
                }
                // A demand coalesces while a query is running. Consume it
                // after completion instead of cancelling a useful catch-up.
                tokio::select! {
                    _ = super::anonymous_reads::idle_connections(&mut connections) => {},
                    _ = sleep(cadence.after_poll(changed)) => {},
                    _ = demanded.notified() => {
                        cadence.reset();
                        redeliver = true;
                    },
                    result = updates.changed() => {
                        if result.is_err() { break; }
                        cadence.reset();
                        snapshots.clear();
                        tls_connectors.clear();
                        connections.clear();
                        tls_config = None;
                        verification.clear();
                    },
                }
            }
        });
        Self {
            config,
            refresh,
            task,
            cancelled: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            signature_calls,
        }
    }

    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.task.abort();
    }

    fn refresh(&self) {
        if !self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            diagnostics::refresh_requested();
            self.refresh.notify_one();
        }
    }

    fn configure(&self, relays: &[RelayUrl], filters: Vec<Filter>) -> Result<(), String> {
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("anonymous discovery scope was removed".into());
        }
        let next = DiscoveryConfig {
            relays: relays.to_vec(),
            filters,
            #[cfg(test)]
            tls_test_roots: self
                .config
                .borrow()
                .as_ref()
                .map(|config| config.tls_test_roots.clone())
                .unwrap_or_default(),
        };
        if self.config.borrow().as_ref() == Some(&next) {
            return Ok(());
        }
        self.config
            .send(Some(next))
            .map_err(|_| "anonymous discovery worker stopped".to_string())
    }

    /// Test-only trust for one exact configured relay, without altering the
    /// normal verifier or any other worker. A watch change discards old tickets.
    #[cfg(test)]
    pub(in crate::core) fn set_test_tls_roots(
        &self,
        relay: &RelayUrl,
        certificates: Vec<Vec<u8>>,
    ) -> Result<(), String> {
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("anonymous discovery scope was removed".into());
        }
        test_tls_roots(&certificates)?;
        let mut next = self
            .config
            .borrow()
            .clone()
            .ok_or_else(|| "anonymous discovery scope is not configured".to_string())?;
        if !next.relays.contains(relay) {
            return Err("test roots require an exact configured discovery relay".into());
        }
        next.tls_test_roots.retain(|(url, _)| url != relay);
        next.tls_test_roots.push((relay.clone(), certificates));
        self.config
            .send(Some(next))
            .map_err(|_| "anonymous discovery worker stopped".to_string())
    }
}

impl AppCore {
    /// Keep foreground recovery on the existing one-author discovery workers.
    /// Their unbounded filters also recover late metadata; unrelated/private
    /// selectors retain the ordinary cold-fetch path.
    pub(super) fn delegate_private_discovery_catch_up(
        &mut self,
        requested: Vec<Filter>,
        relays: &[RelayUrl],
    ) -> Vec<Filter> {
        if requested.is_empty() {
            return requested;
        }
        let Some(plan) = self
            .protocol_subscription_runtime
            .desired_plan
            .clone()
            .or_else(|| self.compute_protocol_subscription_plan())
        else {
            return requested;
        };
        let (_, stable) =
            self.split_private_discovery_filters(build_protocol_subscription_filters(&plan));
        let scopes = match self.private_discovery_subscriptions(stable) {
            Ok(scopes) => scopes,
            Err(error) => {
                self.push_debug_log("protocol.discovery.catch_up.error", error);
                return requested;
            }
        };
        let mut configured = std::collections::BTreeMap::new();
        for (scope, filters) in scopes {
            let Some(scope) = scope.upgrade() else {
                continue;
            };
            let Some(author) = filters
                .first()
                .and_then(|filter| filter.authors.as_ref())
                .and_then(|authors| authors.first())
                .copied()
            else {
                continue;
            };
            let already_configured = scope.config.borrow().is_some();
            match scope.configure(relays, filters.clone()) {
                Ok(()) => {
                    configured.insert(author, (scope, filters, already_configured));
                }
                Err(error) => self.push_debug_log("protocol.discovery.catch_up.error", error),
            }
        }
        let mut demanded = HashSet::new();
        requested
            .into_iter()
            .filter(|filter| {
                let Some(author) = filter
                    .authors
                    .as_ref()
                    .filter(|authors| authors.len() == 1)
                    .and_then(|authors| authors.first())
                else {
                    return true;
                };
                let Some((scope, filters, already_configured)) = configured.get(author) else {
                    return true;
                };
                if !discovery_filter_is_covered(filter, filters) {
                    return true;
                }
                // A new worker immediately delivers its initial poll. Existing
                // workers must redeliver unchanged events for explicit recovery.
                if *already_configured && demanded.insert(*author) {
                    scope.refresh();
                }
                false
            })
            .collect()
    }

    pub(in crate::core) fn refresh_private_discovery_for_author(&self, author: PublicKey) {
        if let Some(scope) = self
            .protocol_subscription_runtime
            .discovery_clients
            .get(&author)
        {
            scope.refresh();
        }
    }

    pub(in crate::core) fn refresh_private_discovery(&self) {
        for scope in self
            .protocol_subscription_runtime
            .discovery_clients
            .values()
        {
            scope.refresh();
        }
    }

    fn private_discovery_client(&mut self, author: PublicKey) -> Arc<PrivateDiscoveryClient> {
        self.protocol_subscription_runtime
            .discovery_clients
            .entry(author)
            .or_insert_with(|| {
                Arc::new(PrivateDiscoveryClient::new(
                    self.runtime.handle(),
                    self.core_sender.clone(),
                ))
            })
            .clone()
    }

    pub(in crate::core) fn private_discovery_subscriptions(
        &mut self,
        filters: Vec<Filter>,
    ) -> Result<Vec<PrivateDiscoverySubscription>, String> {
        let mut scopes: std::collections::BTreeMap<PublicKey, Vec<Filter>> =
            std::collections::BTreeMap::new();
        for filter in filters.into_iter().flat_map(split_public_discovery_authors) {
            let author = filter
                .authors
                .as_ref()
                .and_then(|authors| authors.first().filter(|_| authors.len() == 1))
                .copied()
                .ok_or_else(|| {
                    "private discovery requires exactly one public author".to_string()
                })?;
            scopes.entry(author).or_default().push(filter);
        }
        if scopes.len() > MAX_PRIVATE_DISCOVERY_SCOPES {
            return Err("too many private discovery scopes".to_string());
        }
        self.protocol_subscription_runtime
            .discovery_clients
            .retain(|author, scope| {
                let keep = scopes.contains_key(author);
                if !keep {
                    scope.cancel();
                }
                keep
            });
        Ok(scopes
            .into_iter()
            .map(|(author, filters)| {
                (
                    Arc::downgrade(&self.private_discovery_client(author)),
                    filters,
                )
            })
            .collect())
    }

    /// Public peer discovery must not share the own-account receive context.
    /// Each public author gets its own anonymous discovery context. These
    /// contexts never receive own-identity filters or publish private events.
    pub(in crate::core) fn split_private_discovery_filters(
        &self,
        filters: Vec<Filter>,
    ) -> (Vec<Filter>, Vec<Filter>) {
        let mut own = HashSet::new();
        if let Some(local) = &self.logged_in {
            own.insert(local.owner_pubkey);
            own.insert(local.device_keys.public_key());
            if let Some(keys) = self.app_keys.get(&local.owner_pubkey.to_hex()) {
                own.extend(
                    keys.devices
                        .iter()
                        .filter_map(|device| PublicKey::from_hex(&device.identity_pubkey_hex).ok()),
                );
            }
            if let Some(engine) = &self.protocol_engine {
                own.extend(
                    engine.known_device_identity_pubkeys_for_owner_with_snapshot(
                        &engine.session_manager_snapshot(),
                        local.owner_pubkey,
                    ),
                );
            }
        }
        let mut receiver = Vec::new();
        let mut discovery = Vec::new();
        for filter in filters {
            if !is_public_discovery_filter(&filter) {
                receiver.push(filter);
                continue;
            }
            let Some(authors) = filter.authors.as_ref() else {
                receiver.push(filter);
                continue;
            };
            let own_authors = authors
                .iter()
                .filter(|key| own.contains(key))
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            let peer_authors = authors
                .iter()
                .filter(|key| !own.contains(key))
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            if !own_authors.is_empty() {
                let mut own_filter = filter.clone();
                own_filter.authors = Some(own_authors);
                receiver.push(own_filter);
            }
            if !peer_authors.is_empty() {
                let mut peer_filter = filter;
                peer_filter.authors = Some(peer_authors);
                discovery.extend(split_public_discovery_authors(peer_filter));
            }
        }
        (receiver, discovery)
    }
}

fn discovery_filter_is_covered(requested: &Filter, configured: &[Filter]) -> bool {
    let mut unbounded = requested.clone();
    unbounded.since = None;
    unbounded.until = None;
    unbounded.limit = None;
    configured.iter().any(|filter| filter == &unbounded)
}

fn is_public_discovery_filter(filter: &Filter) -> bool {
    (filter.authors.is_some() && filter.kinds.is_none())
        || filter.kinds.as_ref().is_some_and(|kinds| {
            kinds.iter().any(|kind| {
                matches!(
                    kind.as_u16() as u32,
                    0 | 10002
                        | 10050
                        | APP_KEYS_EVENT_KIND
                        | INVITE_EVENT_KIND
                        | INVITE_RESPONSE_KIND
                )
            })
        })
}

pub(super) fn split_public_discovery_authors(filter: Filter) -> Vec<Filter> {
    let Some(authors) = filter.authors.as_ref().filter(|authors| authors.len() > 1) else {
        return vec![filter];
    };
    authors
        .iter()
        .map(|author| {
            let mut scoped = filter.clone();
            scoped.authors = Some(std::collections::BTreeSet::from([*author]));
            scoped
        })
        .collect()
}

pub(in crate::core) async fn apply_private_discovery_subscriptions(
    scopes: Vec<PrivateDiscoverySubscription>,
    relay_urls: &[RelayUrl],
) -> Result<(), String> {
    // Applying config is immediate; the bounded scheduler performs catch-up.
    // Do not wait for a whole populated account inside the 8s apply deadline.
    for (scope, filters) in scopes {
        let scope = scope
            .upgrade()
            .ok_or_else(|| "anonymous discovery scope was removed".to_string())?;
        scope.configure(relay_urls, filters)?;
    }
    Ok(())
}

#[cfg(test)]
mod tls_tests;
#[cfg(test)]
mod verification_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_up_requires_the_exact_unbounded_discovery_selectors() {
        let author = Keys::generate().public_key();
        let stable = Filter::new().author(author).kind(Kind::Metadata);
        let bounded = stable
            .clone()
            .since(Timestamp::from(10))
            .until(Timestamp::from(20))
            .limit(1);
        assert!(discovery_filter_is_covered(
            &bounded,
            std::slice::from_ref(&stable)
        ));
        assert!(!discovery_filter_is_covered(&stable, &[bounded]));
        for changed in [
            Filter::new()
                .author(Keys::generate().public_key())
                .kind(Kind::Metadata),
            Filter::new()
                .author(author)
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16)),
            stable.clone().pubkey(Keys::generate().public_key()),
            stable.clone().search("uncovered selector"),
        ] {
            assert!(!discovery_filter_is_covered(
                &changed,
                std::slice::from_ref(&stable)
            ));
        }
    }

    #[test]
    fn quiet_discovery_has_bounded_freshness_and_demand_resets_it() {
        let mut cadence = DiscoveryCadence::default();
        assert_eq!(cadence.after_poll(false), Duration::from_secs(30));
        assert_eq!(cadence.after_poll(false), Duration::from_secs(60));
        for _ in 0..100 {
            assert_eq!(cadence.after_poll(false), Duration::from_secs(120));
        }
        assert_eq!(cadence.after_poll(true), Duration::from_secs(30));
        assert_eq!(cadence.after_poll(false), Duration::from_secs(60));
        cadence.reset();
        assert_eq!(cadence.after_poll(false), Duration::from_secs(30));
    }

    #[test]
    fn unchanged_large_responses_do_not_keep_discovery_at_active_cadence() {
        let author = Keys::generate();
        let mut events = (0..256)
            .map(|index| {
                EventBuilder::new(Kind::Custom(1059), index.to_string())
                    .sign_with_keys(&author)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut snapshot = DiscoverySnapshot::default();
        let mut cadence = DiscoveryCadence::default();
        assert_eq!(
            cadence.after_poll(snapshot.observe(&events)),
            Duration::from_secs(30)
        );
        events.reverse();
        events.push(events[0].clone());
        assert_eq!(
            cadence.after_poll(snapshot.observe(&events)),
            Duration::from_secs(60)
        );
        assert_eq!(
            cadence.after_poll(snapshot.observe(&events)),
            Duration::from_secs(120)
        );
        events.push(
            EventBuilder::new(Kind::Custom(1059), "late update")
                .sign_with_keys(&author)
                .unwrap(),
        );
        assert_eq!(
            cadence.after_poll(snapshot.observe(&events)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn unchanged_large_wire_response_is_not_redelivered_until_explicit_demand() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let relay = crate::local_relay::TestRelay::start();
        let url = RelayUrl::parse(relay.url()).unwrap();
        runtime.block_on(async {
            let peer = Keys::generate();
            let publisher = Client::new(peer.clone());
            publisher.add_relay(url.clone()).await.unwrap();
            connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
            for index in 0..256 {
                let event = EventBuilder::new(Kind::Custom(1059), index.to_string())
                    .sign_with_keys(&peer)
                    .unwrap();
                publisher.send_event(&event).await.unwrap();
            }
            let (sender, incoming) = flume::unbounded();
            let reader = PrivateDiscoveryClient::new(runtime.handle(), sender);
            reader
                .configure(
                    &[url],
                    vec![Filter::new()
                        .author(peer.public_key())
                        .kind(Kind::Custom(1059))],
                )
                .unwrap();
            let mut delivered = HashSet::new();
            tokio::time::timeout(Duration::from_secs(10), async {
                while delivered.len() < 256 {
                    if let CoreMsg::Internal(event) = incoming.recv_async().await.unwrap() {
                        if let InternalEvent::RelayEvent(event) = *event {
                            delivered.insert(event.id);
                        }
                    }
                }
            })
            .await
            .unwrap();
            // Use the actual production interval: the old 128-ID delivery
            // cache churned on the next unchanged 256-event response.
            sleep(Duration::from_secs(32)).await;
            assert!(
                incoming.is_empty(),
                "unchanged large response was forwarded again"
            );
            reader.refresh();
            let mut retried = HashSet::new();
            tokio::time::timeout(Duration::from_secs(10), async {
                while retried.len() < 256 {
                    if let CoreMsg::Internal(event) = incoming.recv_async().await.unwrap() {
                        if let InternalEvent::RelayEvent(event) = *event {
                            retried.insert(event.id);
                        }
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                retried, delivered,
                "explicit recovery must allow redelivery"
            );
            reader.cancel();
            publisher.shutdown().await;
        });
    }

    #[test]
    fn targeted_discovery_demand_and_restart_keep_fresh_anonymous_wire_scopes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let owner = Keys::generate();
            let peers = [Keys::generate().public_key(), Keys::generate().public_key()];
            let (relay, wire) =
                crate::core::publish_helpers::authenticated_relay(owner.public_key(), false).await;
            let receiver =
                crate::core::publish_helpers::identity_receiver(&owner, &relay, &wire).await;
            let (sender, _events) = flume::unbounded();
            let filters = peers.map(|peer| {
                vec![Filter::new()
                    .author(peer)
                    .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))]
            });
            let first = PrivateDiscoveryClient::new(runtime.handle(), sender.clone());
            let other = PrivateDiscoveryClient::new(runtime.handle(), sender.clone());
            let completed_queries = |peer: PublicKey| {
                let rows = wire.lock().unwrap();
                rows.iter()
                    .filter(|(connection, value)| {
                        value[0] == "REQ"
                            && value.to_string().contains(&peer.to_hex())
                            && rows.iter().any(|(id, closed)| {
                                id == connection && closed[0] == "CLOSE" && closed[1] == value[1]
                            })
                    })
                    .map(|(connection, request)| {
                        (*connection, request[1].as_str().unwrap().to_owned())
                    })
                    .collect::<HashSet<_>>()
                    .len()
            };
            first
                .configure(std::slice::from_ref(&relay), filters[0].clone())
                .unwrap();
            other
                .configure(std::slice::from_ref(&relay), filters[1].clone())
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while completed_queries(peers[0]) < 1 || completed_queries(peers[1]) < 1 {
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            for expected in 2..=3 {
                first.refresh();
                tokio::time::timeout(Duration::from_secs(3), async {
                    while completed_queries(peers[0]) < expected {
                        sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
            }
            let before_restart = wire
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, value)| {
                    value[0] == "REQ" && value.to_string().contains(&peers[0].to_hex())
                })
                .map(|(connection, _)| *connection)
                .collect::<HashSet<_>>();
            first.cancel();
            tokio::time::timeout(Duration::from_secs(1), async {
                while !first.task.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let restarted = PrivateDiscoveryClient::new(runtime.handle(), sender);
            restarted
                .configure(std::slice::from_ref(&relay), filters[0].clone())
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while completed_queries(peers[0]) < 4 {
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                completed_queries(peers[1]),
                1,
                "another author's idle scope was woken"
            );
            restarted.cancel();
            other.cancel();

            let rows = wire.lock().unwrap().clone();
            let restarted_connections = rows
                .iter()
                .filter(|(connection, value)| {
                    value[0] == "REQ"
                        && value.to_string().contains(&peers[0].to_hex())
                        && !before_restart.contains(connection)
                })
                .map(|(connection, _)| *connection)
                .collect::<HashSet<_>>();
            assert_eq!(
                restarted_connections.len(),
                1,
                "restart must discard the previous socket"
            );
            let identified = rows
                .iter()
                .filter(|(_, value)| {
                    value[0] == "AUTH" && value[1]["pubkey"] == owner.public_key().to_hex()
                })
                .map(|(connection, _)| *connection)
                .collect::<HashSet<_>>();
            let mut anonymous_authors = HashSet::new();
            let mut read_connections = HashSet::new();
            let mut subjects = std::collections::BTreeMap::<usize, HashSet<String>>::new();
            for (connection, value) in &rows {
                if value[0] != "REQ"
                    || !peers
                        .iter()
                        .any(|peer| value.to_string().contains(&peer.to_hex()))
                {
                    continue;
                }
                assert!(!identified.contains(connection));
                assert!(!value.to_string().contains(&owner.public_key().to_hex()));
                assert_eq!(
                    peers
                        .iter()
                        .filter(|peer| value.to_string().contains(&peer.to_hex()))
                        .count(),
                    1
                );
                let author = peers
                    .iter()
                    .find(|peer| value.to_string().contains(&peer.to_hex()))
                    .unwrap();
                subjects
                    .entry(*connection)
                    .or_default()
                    .insert(author.to_hex());
                read_connections.insert(*connection);
            }
            for connection in &read_connections {
                let auth = rows
                    .iter()
                    .find(|(id, value)| id == connection && value[0] == "AUTH")
                    .expect("challenged read authenticated anonymously");
                let key = auth.1[1]["pubkey"].as_str().unwrap();
                assert_ne!(key, owner.public_key().to_hex());
                assert!(peers.iter().all(|peer| key != peer.to_hex()));
                assert!(
                    anonymous_authors.insert(key.to_string()),
                    "a reconnect/restart reused an authentication identity"
                );
            }
            assert!(
                subjects.values().all(|authors| authors.len() == 1),
                "one socket linked different public authors"
            );
            // Repeated demand can reuse its exact anonymous scope; restart
            // must create a new connection and a new anonymous signer. A full
            // retention budget may still use disposable reads for each poll.
            assert!((3..=5).contains(&read_connections.len()));
            assert_eq!(completed_queries(peers[0]), 4);
            assert_eq!(completed_queries(peers[1]), 1);
            receiver.shutdown().await;
        });
    }

    #[test]
    fn private_discovery_stops_when_core_event_receiver_closes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let relay = crate::local_relay::TestRelay::start();
        let url = RelayUrl::parse(relay.url()).unwrap();
        let peer = Keys::generate();
        let event = EventBuilder::new(Kind::Metadata, "{}")
            .sign_with_keys(&peer)
            .unwrap();
        let (sender, receiver) = flume::unbounded();
        drop(receiver);
        let reader = PrivateDiscoveryClient::new(runtime.handle(), sender);
        runtime.block_on(async {
            let client = Client::new(peer.clone());
            client.add_relay(url.clone()).await.unwrap();
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            client.send_event(&event).await.unwrap();
            reader
                .configure(
                    &[url],
                    vec![Filter::new().author(peer.public_key()).kind(Kind::Metadata)],
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !reader.task.is_finished() {
                assert!(
                    Instant::now() < deadline,
                    "closed Core receiver left discovery worker running"
                );
                sleep(Duration::from_millis(5)).await;
            }
            client.shutdown().await;
        });
    }
}
