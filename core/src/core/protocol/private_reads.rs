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
}

/// Retained scope state is just configuration and a cancellable task, never an
/// SDK client/cache. Dropping the last descriptor cancels even a queued read.
#[derive(Debug)]
pub(in crate::core) struct PrivateDiscoveryClient {
    config: tokio::sync::watch::Sender<Option<DiscoveryConfig>>,
    refresh: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
    cancelled: std::sync::atomic::AtomicBool,
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
        let task = runtime.spawn(async move {
            let mut seen = HashSet::new();
            let mut order = std::collections::VecDeque::new();
            let mut cadence = DiscoveryCadence::default();
            let mut snapshots = Vec::<DiscoverySnapshot>::new();
            loop {
                let current = updates.borrow_and_update().clone();
                let Some(current) = current else {
                    if updates.changed().await.is_err() {
                        break;
                    }
                    continue;
                };
                snapshots.resize_with(current.relays.len(), DiscoverySnapshot::default);
                let mut changed = false;
                let cycle = async {
                    for (relay, snapshot) in current.relays.iter().zip(snapshots.iter_mut()) {
                        if let Ok(events) =
                            super::anonymous_reads::poll(relay, current.filters.clone()).await
                        {
                            changed |= snapshot.observe(&events);
                            for event in events {
                                if !seen.insert(event.id) {
                                    continue;
                                }
                                order.push_back(event.id);
                                if order.len() > 128 {
                                    if let Some(old) = order.pop_front() {
                                        seen.remove(&old);
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
                    stopped = cycle => if stopped { break; },
                    result = updates.changed() => {
                        if result.is_err() { break; }
                        cadence.reset();
                        snapshots.clear();
                        continue;
                    },
                }
                // A demand coalesces while a query is running. Consume it
                // after completion instead of cancelling a useful catch-up.
                tokio::select! {
                    _ = sleep(cadence.after_poll(changed)) => {},
                    _ = demanded.notified() => cadence.reset(),
                    result = updates.changed() => {
                        if result.is_err() { break; }
                        cadence.reset();
                        snapshots.clear();
                    },
                }
            }
        });
        Self {
            config,
            refresh,
            task,
            cancelled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.task.abort();
    }

    fn refresh(&self) {
        if !self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
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
        };
        if self.config.borrow().as_ref() == Some(&next) {
            return Ok(());
        }
        self.config
            .send(Some(next))
            .map_err(|_| "anonymous discovery worker stopped".to_string())
    }
}

impl AppCore {
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
mod tests {
    use super::*;

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
            let connections = |peer: PublicKey| {
                let rows = wire.lock().unwrap();
                rows.iter()
                    .filter(|(connection, value)| {
                        value[0] == "REQ"
                            && value.to_string().contains(&peer.to_hex())
                            && rows
                                .iter()
                                .any(|(id, value)| id == connection && value[0] == "CLOSE")
                    })
                    .map(|(connection, _)| *connection)
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
                while connections(peers[0]) < 1 || connections(peers[1]) < 1 {
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            for expected in 2..=3 {
                first.refresh();
                tokio::time::timeout(Duration::from_secs(3), async {
                    while connections(peers[0]) < expected {
                        sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
            }
            first.cancel();
            let restarted = PrivateDiscoveryClient::new(runtime.handle(), sender);
            restarted
                .configure(std::slice::from_ref(&relay), filters[0].clone())
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while connections(peers[0]) < 4 {
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                connections(peers[1]),
                1,
                "another author's idle scope was woken"
            );
            restarted.cancel();
            other.cancel();

            let rows = wire.lock().unwrap().clone();
            let identified = rows
                .iter()
                .filter(|(_, value)| {
                    value[0] == "AUTH" && value[1]["pubkey"] == owner.public_key().to_hex()
                })
                .map(|(connection, _)| *connection)
                .collect::<HashSet<_>>();
            let mut anonymous_authors = HashSet::new();
            let mut read_connections = HashSet::new();
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
                    "demand/restart reused an authentication identity"
                );
            }
            assert_eq!(read_connections.len(), 5);
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
