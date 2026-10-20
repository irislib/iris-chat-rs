use super::*;

const MAX_PRIVATE_DISCOVERY_SCOPES: usize = 512;

#[derive(Debug)]
pub(in crate::core) struct PrivateDiscoveryClient {
    client: Client,
    runtime: tokio::runtime::Handle,
}

impl Drop for PrivateDiscoveryClient {
    fn drop(&mut self) {
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let _ = client.shutdown().await;
        });
    }
}

impl AppCore {
    pub(in crate::core) fn private_discovery_client(&mut self, author: PublicKey) -> Client {
        if let Some(discovery) = self
            .protocol_subscription_runtime
            .discovery_clients
            .get(&author)
        {
            return discovery.client.clone();
        }
        let client = Client::new(Keys::generate());
        self.start_notifications_loop(client.clone());
        self.protocol_subscription_runtime.discovery_clients.insert(
            author,
            Arc::new(PrivateDiscoveryClient {
                client: client.clone(),
                runtime: self.runtime.handle().clone(),
            }),
        );
        client
    }

    pub(in crate::core) fn private_discovery_subscriptions(
        &mut self,
        filters: Vec<Filter>,
    ) -> Result<Vec<(Client, Vec<Filter>)>, String> {
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
        self.protocol_subscription_runtime
            .discovery_clients
            .retain(|author, _| scopes.contains_key(author));
        // Match the shared SDK's bound; never collapse excess scopes onto one
        // connection, where the batch would disclose a set of private contacts.
        if scopes.len() > MAX_PRIVATE_DISCOVERY_SCOPES {
            return Err("too many private discovery scopes".to_string());
        }
        Ok(scopes
            .into_iter()
            .map(|(author, filters)| (self.private_discovery_client(author), filters))
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
    if !is_public_discovery_filter(&filter) {
        return vec![filter];
    }
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

pub(super) async fn apply_private_discovery_subscriptions(
    scopes: Vec<(Client, Vec<Filter>)>,
    relay_urls: &[RelayUrl],
) -> Result<(), String> {
    let mut tasks = tokio::task::JoinSet::new();
    for (client, filters) in scopes {
        let relay_urls = relay_urls.to_vec();
        tasks.spawn(async move {
            apply_private_discovery_subscription(&client, &relay_urls, filters).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.map_err(|error| error.to_string())??;
    }
    Ok(())
}

pub(super) async fn apply_private_discovery_subscription(
    client: &Client,
    relay_urls: &[RelayUrl],
    filters: Vec<Filter>,
) -> Result<(), String> {
    let id = SubscriptionId::new("ndr-private-discovery");
    let _ = client.unsubscribe(&id).await;
    if filters.is_empty() {
        return Ok(());
    }
    ensure_session_relays_configured(client, relay_urls).await;
    connect_client_with_timeout(client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS)).await;
    subscribe_protocol_filters_with_id(client, id, filters).await
}
