use super::*;
use nostr_pubsub::{EventBus, EventSource, VerifiedEvent};

pub(super) fn approved_providers(
    direct: &[PeerConfig],
    routed: &[FipsPeerIdentity],
    update: Option<&str>,
    limit: usize,
) -> Vec<String> {
    let mut providers = routed
        .iter()
        .map(FipsPeerIdentity::npub)
        .chain(direct.iter().map(|peer| peer.npub.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(update) = update {
        providers.remove(update);
        let mut selected = providers
            .into_iter()
            .take(limit.saturating_sub(1))
            .collect::<Vec<_>>();
        if limit != 0 {
            selected.push(update.to_string());
        }
        selected
    } else {
        providers.into_iter().take(limit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_provider_pool_requires_explicit_hints_and_public_updates_keep_their_route() {
        let update = Keys::generate().public_key().to_bech32().unwrap();
        assert!(
            approved_providers(&[], &[], None, 64).is_empty(),
            "contacts, siblings and release services must not become implicit message providers"
        );
        let direct = (0..6)
            .map(|_| {
                PeerConfig::new(
                    Keys::generate().public_key().to_bech32().unwrap(),
                    "udp",
                    "127.0.0.1:7000",
                )
            })
            .collect::<Vec<_>>();
        let routed = FipsPeerIdentity::from_npub(&direct[0].npub).unwrap();
        let private = approved_providers(&direct, &[routed], None, 64);
        assert_eq!(private.len(), direct.len());
        assert!(!private.contains(&update));
        for limit in 0..5 {
            let public = approved_providers(&direct, &[routed], Some(&update), limit);
            assert!(public.len() <= limit);
            assert_eq!(public.contains(&update), limit != 0);
        }
    }
}

impl AppCore {
    pub(in crate::core) fn mesh_public_service_authors(&self) -> Vec<PublicKey> {
        let mut authors = Vec::new();
        if let Some(login) = &self.logged_in {
            authors.extend([login.owner_pubkey, login.device_keys.public_key()]);
        }
        if let Ok(reference) = secure_update_ref() {
            if let Ok(publisher) = trusted_update_publisher(&reference) {
                if let Ok(key) = PublicKey::parse(&publisher) {
                    authors.push(key);
                }
            }
        }
        authors.extend(
            super::super::settings::pubsub_policy_options()
                .reputation
                .trusted_raters
                .iter()
                .filter_map(|key| PublicKey::parse(key).ok()),
        );
        authors.sort_unstable();
        authors.dedup();
        authors
    }
}

pub(super) async fn publish_own_adverts(
    endpoint: Arc<FipsEndpoint>,
    pubsub: Arc<FipsPubsubClient>,
    public_client: Client,
    relays: Vec<RelayUrl>,
) {
    let mut published: Option<(String, Instant)> = None;
    loop {
        if let Ok(Some(event)) = endpoint.local_nostr_discovery_advert_event().await {
            let payload = serde_json::to_string(&(&event.tags, &event.content)).unwrap_or_default();
            if published.as_ref().is_some_and(|(previous, at)| {
                previous == &payload && at.elapsed() < Duration::from_secs(60)
            }) {
                sleep(Duration::from_secs(2)).await;
                continue;
            }
            if let Ok(verified) = VerifiedEvent::try_from(event.clone()) {
                let _ = pubsub
                    .publish(verified, EventSource::local_index("iris-chat-live-advert"))
                    .await;
            }
            let _ = super::super::super::publish_helpers::publish_event_once(
                &public_client,
                &relays,
                &event,
            )
            .await;
            published = Some((payload, Instant::now()));
        }
        // Endpoint candidates can arrive after bind/STUN, or change on mobile
        // network handoff. Publish the first usable advert promptly.
        sleep(Duration::from_secs(2)).await;
    }
}
