use super::public_discovery::PublicDiscovery;
use super::*;
use std::collections::BTreeSet;

pub(in crate::core) type PrivateDiscoverySubscription =
    (std::sync::Weak<PublicDiscovery>, Vec<Filter>);

impl AppCore {
    pub(in crate::core) fn reset_public_discovery(&mut self) {
        self.discovery_relays.stop(self.runtime.handle());
        self.discovery_relays = Arc::default();
        if let Some(mesh) = &mut self.device_sync {
            mesh.protocol_subscriptions.discovery = Default::default();
            if let Some(providers) = &mut mesh.private_providers {
                *providers = Arc::new(
                    (**providers)
                        .clone()
                        .with_relay_pool(self.discovery_relays.clone()),
                );
            }
        }
    }

    /// Foreground recovery uses the shared live discovery connection. Existing
    /// subscriptions retain their original window so late metadata is recoverable.
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
        let subscriptions = match self.private_discovery_subscriptions(stable.clone()) {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                self.push_debug_log("protocol.discovery.catch_up.error", error);
                return requested;
            }
        };
        let Some((worker, filters)) = subscriptions.into_iter().next() else {
            return requested;
        };
        let Some(worker) = worker.upgrade() else {
            return requested;
        };
        let configured = worker.is_configured();
        if worker.configure(relays, filters).is_err() {
            return requested;
        }
        requested
            .into_iter()
            .filter(|filter| {
                if !discovery_filter_is_covered(filter, &stable) {
                    return true;
                }
                if configured {
                    if let Some(authors) = &filter.authors {
                        for author in authors {
                            worker.refresh(Some(*author));
                        }
                    }
                }
                false
            })
            .collect()
    }

    pub(in crate::core) fn refresh_private_discovery_for_author(&self, author: PublicKey) {
        if let Some(worker) = &self.protocol_subscription_runtime.discovery {
            worker.refresh(Some(author));
        }
    }

    pub(in crate::core) fn refresh_private_discovery(&self) {
        if let Some(worker) = &self.protocol_subscription_runtime.discovery {
            worker.refresh(None);
        }
    }

    pub(in crate::core) fn private_discovery_subscriptions(
        &mut self,
        filters: Vec<Filter>,
    ) -> Result<Vec<PrivateDiscoverySubscription>, String> {
        if filters.is_empty() {
            if let Some(worker) = self.protocol_subscription_runtime.discovery.take() {
                worker.cancel();
            }
            return Ok(Vec::new());
        }
        if filters
            .iter()
            .any(|filter| !is_public_discovery_filter(filter))
        {
            return Err("public discovery requires public author filters".into());
        }
        let authors = filters
            .iter()
            .filter_map(|filter| filter.authors.as_ref())
            .flatten()
            .collect::<BTreeSet<_>>();
        if authors.len() > 512 {
            return Err("too many public discovery authors".into());
        }
        let worker = self
            .protocol_subscription_runtime
            .discovery
            .get_or_insert_with(|| {
                Arc::new(PublicDiscovery::new(
                    self.runtime.handle(),
                    self.core_sender.clone(),
                    self.discovery_relays.clone(),
                ))
            });
        Ok(vec![(Arc::downgrade(worker), filters)])
    }

    /// Public peer discovery must not share the own-account receive context.
    /// Shared public discovery never authenticates with account/device keys
    /// or publishes private events.
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
                discovery.push(peer_filter);
            }
        }
        (receiver, discovery)
    }
}

fn discovery_filter_is_covered(requested: &Filter, configured: &[Filter]) -> bool {
    let Some(authors) = &requested.authors else {
        return false;
    };
    let mut unbounded = requested.clone();
    unbounded.since = None;
    unbounded.until = None;
    unbounded.limit = None;
    configured.iter().any(|filter| {
        if !filter
            .authors
            .as_ref()
            .is_some_and(|available| authors.is_subset(available))
        {
            return false;
        }
        let mut available = filter.clone();
        available.authors = unbounded.authors.clone();
        available == unbounded
    })
}

pub(super) fn is_public_discovery_filter(filter: &Filter) -> bool {
    filter
        .authors
        .as_ref()
        .is_some_and(|authors| !authors.is_empty())
        && filter.kinds.as_ref().is_some_and(|kinds| {
            !kinds.is_empty()
                && kinds.iter().all(|kind| {
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
        && !filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::P))
}

pub(in crate::core) async fn apply_private_discovery_subscriptions(
    subscriptions: Vec<PrivateDiscoverySubscription>,
    relays: &[RelayUrl],
) -> Result<(), String> {
    for (worker, filters) in subscriptions {
        worker
            .upgrade()
            .ok_or_else(|| "public discovery subscription was removed".to_string())?
            .configure(relays, filters)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_up_requires_covered_authors_and_exact_unbounded_selectors() {
        let author = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let stable = Filter::new().authors([author, other]).kind(Kind::Metadata);
        let bounded = Filter::new()
            .author(author)
            .kind(Kind::Metadata)
            .since(Timestamp::from(10))
            .until(Timestamp::from(20))
            .limit(1);
        assert!(discovery_filter_is_covered(
            &bounded,
            std::slice::from_ref(&stable)
        ));
        for changed in [
            Filter::new()
                .author(Keys::generate().public_key())
                .kind(Kind::Metadata),
            Filter::new()
                .author(author)
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16)),
            stable.clone().pubkey(other),
            stable.clone().search("uncovered"),
        ] {
            assert!(!discovery_filter_is_covered(
                &changed,
                std::slice::from_ref(&stable)
            ));
        }
    }

    #[test]
    fn private_recipients_and_mixed_private_kinds_cannot_use_public_discovery() {
        let author = Keys::generate().public_key();
        let public = Filter::new().author(author).kind(Kind::Metadata);
        assert!(is_public_discovery_filter(&public));
        assert!(!is_public_discovery_filter(&public.clone().pubkey(author)));
        assert!(!is_public_discovery_filter(
            &public.kind(Kind::from(MESSAGE_EVENT_KIND as u16))
        ));
        assert!(!is_public_discovery_filter(&Filter::new().author(author)));
    }
}

// Private cold reads retain separate subject scopes; public lookups bypass
// this reader and share DiscoveryRelays with live public subscriptions.
pub(super) fn split_public_discovery_authors(filter: Filter) -> Vec<Filter> {
    let Some(authors) = filter.authors.as_ref().filter(|authors| authors.len() > 1) else {
        return vec![filter];
    };
    authors
        .iter()
        .map(|author| {
            let mut scoped = filter.clone();
            scoped.authors = Some(BTreeSet::from([*author]));
            scoped
        })
        .collect()
}
