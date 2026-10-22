use super::*;
use nostr_pubsub::{EventBus, EventSource, VerifiedEvent};

const ADVERT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ADVERT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct OwnAdvertPublication {
    previous: Option<(Event, Instant)>,
}

fn same_advert(left: &Event, right: &Event) -> bool {
    // Signing each poll changes the timestamp, ID, signature and expiration.
    // Keep every other field, including unknown tags, in the change detector.
    fn semantic_tags(event: &Event) -> impl Iterator<Item = &nostr::Tag> {
        event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) != Some("expiration"))
    }
    left.pubkey == right.pubkey
        && left.kind == right.kind
        && left.content == right.content
        && semantic_tags(left).eq(semantic_tags(right))
}

impl OwnAdvertPublication {
    async fn publish_if_due<F, Fut>(
        &mut self,
        event: &Event,
        now: Instant,
        unix_now: u64,
        publish: F,
    ) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        if self.previous.as_ref().is_some_and(|(previous, at)| {
            let expires_before_next_poll = previous.tags.expiration().is_some_and(|expiration| {
                expiration.as_secs() <= unix_now.saturating_add(ADVERT_POLL_INTERVAL.as_secs())
            });
            same_advert(previous, event)
                && now.saturating_duration_since(*at) < ADVERT_REFRESH_INTERVAL
                && !expires_before_next_poll
        }) {
            return false;
        }
        publish().await;
        // Count from the start of publication: network waits must not extend
        // the previous advert's signed lifetime or the refresh interval.
        self.previous = Some((event.clone(), now));
        true
    }
}

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
    let mut published = OwnAdvertPublication::default();
    loop {
        if let Ok(Some(event)) = endpoint.local_nostr_discovery_advert_event().await {
            published
                .publish_if_due(
                    &event,
                    Instant::now(),
                    Timestamp::now().as_secs(),
                    || async {
                        if let Ok(verified) = VerifiedEvent::try_from(event.clone()) {
                            let _ = pubsub
                                .publish(
                                    verified,
                                    EventSource::local_index("iris-chat-live-advert"),
                                )
                                .await;
                        }
                        let _ = super::super::super::publish_helpers::publish_event_once(
                            &public_client,
                            &relays,
                            &event,
                        )
                        .await;
                    },
                )
                .await;
        }
        // Endpoint candidates can arrive after bind/STUN, or change on mobile
        // network handoff. Publish the first usable advert promptly.
        sleep(ADVERT_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
#[path = "private_provider/advert_tests.rs"]
mod advert_tests;

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
