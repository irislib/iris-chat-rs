use super::*;

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
    pub(in crate::core) fn private_discovery_client(&mut self) -> Client {
        if let Some(discovery) = &self.protocol_subscription_runtime.discovery_client {
            return discovery.client.clone();
        }
        let client = Client::new(Keys::generate());
        self.start_notifications_loop(client.clone());
        self.protocol_subscription_runtime.discovery_client =
            Some(Arc::new(PrivateDiscoveryClient {
                client: client.clone(),
                runtime: self.runtime.handle().clone(),
            }));
        client
    }

    /// Public peer discovery must not share the own-account receive context.
    /// The anonymous discovery pool never receives own-identity filters and is
    /// never used to publish private events.
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
            let public_authors = filter.kinds.as_ref().is_some_and(|kinds| {
                kinds.iter().any(|kind| {
                    matches!(
                        kind.as_u16() as u32,
                        0 | APP_KEYS_EVENT_KIND | INVITE_EVENT_KIND | INVITE_RESPONSE_KIND
                    )
                })
            });
            if !public_authors {
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
                discovery.push(peer_filter);
            }
        }
        (receiver, discovery)
    }
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
