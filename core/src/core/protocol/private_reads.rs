use super::*;

const MAX_PRIVATE_DISCOVERY_SCOPES: usize = 512;

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
    task: tokio::task::JoinHandle<()>,
    cancelled: std::sync::atomic::AtomicBool,
}

impl Drop for PrivateDiscoveryClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl PrivateDiscoveryClient {
    fn new(runtime: &tokio::runtime::Handle, sender: Sender<CoreMsg>) -> Self {
        let (config, mut updates) = tokio::sync::watch::channel(None::<DiscoveryConfig>);
        let task = runtime.spawn(async move {
            let mut seen = HashSet::new();
            let mut order = std::collections::VecDeque::new();
            loop {
                let current = updates.borrow_and_update().clone();
                let Some(current) = current else {
                    if updates.changed().await.is_err() {
                        break;
                    }
                    continue;
                };
                let cycle = async {
                    for relay in &current.relays {
                        if let Ok(events) =
                            super::anonymous_reads::poll(relay, current.filters.clone()).await
                        {
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
                    sleep(Duration::from_secs(30)).await;
                    false
                };
                tokio::select! {
                    stopped = cycle => if stopped { break; },
                    result = updates.changed() => if result.is_err() { break; },
                }
            }
        });
        Self {
            config,
            task,
            cancelled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.task.abort();
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
    ) -> Result<Vec<(std::sync::Weak<PrivateDiscoveryClient>, Vec<Filter>)>, String> {
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
    scopes: Vec<(std::sync::Weak<PrivateDiscoveryClient>, Vec<Filter>)>,
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
