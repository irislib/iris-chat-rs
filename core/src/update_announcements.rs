//! Update discovery shares the application's pubsub transport. Standalone
//! update commands bootstrap FIPS without a login or VPN tunnel.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use fips_core::config::{TransportInstances, WebSocketConfig};
use hashtree_updater::{
    build_secure_pubsub_blossom_updater, NostrEventSubscriber, SecurePubsubBlossomConfig,
    SecurePubsubBlossomUpdater, UpdateCheck, UpdateCheckOptions, UpdateError, UpdateEventCache,
    UpdateRef, UpdateTarget,
};
use nostr::{Keys, ToBech32};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};

pub const HTREE_UPDATE_REF: &str =
    "htree://npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/releases%2Firis-chat-rs/latest";
const DEFAULT_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];
const UPDATE_MANIFEST_TIMEOUT: Duration = Duration::from_secs(8);
static UPDATE_PROVIDER: OnceLock<Mutex<Option<Weak<dyn NostrEventSubscriber>>>> = OnceLock::new();
static UPDATE_EVENTS: OnceLock<Mutex<Option<(UpdateRef, UpdateEventCache)>>> = OnceLock::new();

pub(crate) fn register_update_provider(provider: &Arc<dyn NostrEventSubscriber>) {
    if let Ok(mut registered) = UPDATE_PROVIDER.get_or_init(|| Mutex::new(None)).lock() {
        *registered = Some(Arc::downgrade(provider));
    }
}

pub fn secure_update_ref() -> Result<UpdateRef, UpdateError> {
    let raw = std::env::var("IRIS_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_UPDATE_REF.to_string());
    UpdateRef::parse(&raw)
}

fn secure_update_config() -> SecurePubsubBlossomConfig {
    SecurePubsubBlossomConfig {
        blossom_read_servers: env_csv("IRIS_UPDATE_BLOSSOM_SERVERS").unwrap_or_else(|| {
            DEFAULT_BLOSSOM_READ_SERVERS
                .iter()
                .map(|s| (*s).into())
                .collect()
        }),
        manifest_timeout: UPDATE_MANIFEST_TIMEOUT,
        download_timeout: Duration::from_secs(180),
    }
}

pub async fn build_secure_update_updater(
) -> Result<(UpdateRef, SecurePubsubBlossomUpdater), UpdateError> {
    let reference = secure_update_ref()?;
    let shared = UPDATE_PROVIDER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| UpdateError::Announcement("update provider lock poisoned".into()))?
        .as_ref()
        .and_then(Weak::upgrade);
    let provider = match shared {
        Some(provider) => provider,
        None => tokio::time::timeout(
            Duration::from_secs(4),
            standalone_update_provider(&reference),
        )
        .await
        .map_err(|_| UpdateError::Announcement("timed out starting update pubsub".into()))??,
    };
    let updater = build_secure_pubsub_blossom_updater(provider, secure_update_config()).await?;
    let events = UPDATE_EVENTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| UpdateError::Announcement("update event cache lock poisoned".into()))?
        .as_ref()
        .filter(|(cached_ref, _)| cached_ref == &reference)
        .map(|(_, cache)| cache.resolver_events())
        .unwrap_or_default();
    for event in events {
        updater.resolver().ingest_event(event).await?;
    }
    Ok((reference, updater))
}

/// Keep the observed signed root between checks, without treating it as fresh.
pub async fn check_secure_update(
    current_version: String,
    target: UpdateTarget,
) -> Result<(SecurePubsubBlossomUpdater, UpdateCheck), UpdateError> {
    let (reference, updater) = build_secure_update_updater().await?;
    let result = updater
        .check(UpdateCheckOptions {
            reference: reference.clone(),
            current_version,
            target,
            ..Default::default()
        })
        .await;
    if let Some(event) = updater
        .resolver()
        .latest_event(&reference.resolver_key())
        .await?
    {
        remember_update_event(&reference, event)?;
    }
    Ok((updater, result?))
}

fn remember_update_event(
    reference: &UpdateRef,
    event: hashtree_resolver::Event,
) -> Result<(), UpdateError> {
    let id = event.id;
    let mut guard = UPDATE_EVENTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| UpdateError::Announcement("update event cache lock poisoned".into()))?;
    if guard
        .as_ref()
        .is_none_or(|(cached_ref, _)| cached_ref != reference)
    {
        *guard = Some((reference.clone(), UpdateEventCache::new(reference)?));
    }
    let (_, cache) = guard.as_mut().ok_or_else(|| {
        UpdateError::Announcement("update event cache was not initialized".into())
    })?;
    cache.ingest_event(event)?;
    if cache
        .latest()
        .is_none_or(|latest| latest.as_event().id != id)
    {
        return Err(UpdateError::Announcement(
            "a newer signed release was already observed".into(),
        ));
    }
    Ok(())
}

pub(crate) fn trusted_update_publisher(reference: &UpdateRef) -> Result<String, UpdateError> {
    fips_core::PeerIdentity::from_npub(&reference.npub)
        .map(|peer| peer.npub())
        .map_err(|error| {
            UpdateError::InvalidReference(format!("invalid update publisher: {error}"))
        })
}

fn update_pubsub_options(reference: &UpdateRef) -> Result<FipsPubsubClientOptions, UpdateError> {
    Ok(FipsPubsubClientOptions {
        // Only the configured release authority is a routing target. Event
        // authors observed through pubsub never expand this trusted list.
        routed_peers: vec![trusted_update_publisher(reference)?],
        ..FipsPubsubClientOptions::default()
    })
}

async fn standalone_update_provider(
    reference: &UpdateRef,
) -> Result<Arc<dyn NostrEventSubscriber>, UpdateError> {
    // Relays are an explicit standalone override; no private updater relay list.
    if let Some(relays) = env_csv("IRIS_UPDATE_RELAYS").filter(|relays| !relays.is_empty()) {
        return nostr_pubsub_relay::RelayEventBus::new(relays, UPDATE_MANIFEST_TIMEOUT)
            .await
            .map(|provider| Arc::new(provider) as Arc<dyn NostrEventSubscriber>)
            .map_err(|error| UpdateError::Announcement(error.to_string()));
    }
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.nostr.advertise = false;
    config.transports.websocket = TransportInstances::Single(WebSocketConfig {
        seed_urls: websocket_seed_urls(
            std::env::var("IRIS_FIPS_WEBSOCKET_SEED_URLS")
                .ok()
                .as_deref(),
        ),
        ..WebSocketConfig::default()
    });
    standalone_fips_update_provider(reference, config).await
}

async fn standalone_fips_update_provider(
    reference: &UpdateRef,
    config: fips_core::Config,
) -> Result<Arc<dyn NostrEventSubscriber>, UpdateError> {
    let pubsub_options = update_pubsub_options(reference)?;
    let endpoint = fips_core::FipsEndpoint::builder()
        .config(config)
        .identity_nsec(
            Keys::generate()
                .secret_key()
                .to_bech32()
                .map_err(|error| UpdateError::Announcement(error.to_string()))?,
        )
        .discovery_scope("iris-chat-updates")
        .local_rendezvous()
        .without_system_tun()
        .bind()
        .await
        .map_err(|error| UpdateError::Announcement(error.to_string()))?;
    let client = Arc::new(
        FipsPubsubClient::start(Arc::new(endpoint), pubsub_options)
            .await
            .map_err(|error| UpdateError::Announcement(error.to_string()))?,
    );
    Ok(Arc::new(client.fresh_subscriber()))
}

pub(crate) fn websocket_seed_urls(configured: Option<&str>) -> Vec<String> {
    configured
        .unwrap_or("wss://fips2.iris.to/fips,wss://fips1.iris.to/fips")
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn env_csv(name: &str) -> Option<Vec<String>> {
    std::env::var(name).ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_resolver::{nostr::HASHTREE_KIND, RootResolver};
    use nostr::{EventBuilder, Kind, Tag, TagKind};
    use nostr_pubsub::{EventBus, EventSource, InMemoryEventBus, VerifiedEvent};

    #[test]
    fn standalone_routes_only_to_the_configured_update_publisher() {
        let default = UpdateRef::parse(HTREE_UPDATE_REF).unwrap();
        assert_eq!(
            update_pubsub_options(&default).unwrap().routed_peers,
            [default.npub]
        );

        let reference = UpdateRef {
            npub: Keys::generate().public_key().to_bech32().unwrap(),
            tree_name: "releases/test".into(),
            path: Some("latest".into()),
        };
        assert_eq!(
            update_pubsub_options(&reference).unwrap().routed_peers,
            [reference.npub]
        );
        let malformed = UpdateRef::parse("htree://npub1invalid/releases/test").unwrap();
        assert!(update_pubsub_options(&malformed).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn standalone_websocket_updater_discovers_local_provider_across_scopes() {
        let rendezvous = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(rendezvous_addr) = rendezvous.local_addr().unwrap() else {
            panic!("expected loopback IPv4 address");
        };
        drop(rendezvous);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let seed_addr = listener.local_addr().unwrap();
        drop(listener);

        // The WebSocket seed cannot reach the local-only publisher. Joining
        // local rendezvous is the only path to its retained signed event.
        let mut seed_config = fips_core::Config::new();
        seed_config.node.control.enabled = false;
        seed_config.node.discovery.nostr.enabled = false;
        seed_config.node.discovery.lan.enabled = false;
        seed_config.transports.websocket = TransportInstances::Single(WebSocketConfig {
            bind_addr: Some(seed_addr.to_string()),
            ..WebSocketConfig::default()
        });
        let seed = fips_core::FipsEndpoint::builder()
            .config(seed_config)
            .without_system_tun()
            .bind()
            .await
            .unwrap();

        let keys = Keys::generate();
        let reference = UpdateRef {
            npub: keys.public_key().to_bech32().unwrap(),
            tree_name: "releases/local-regression".into(),
            path: Some("latest".into()),
        };
        let mut config = fips_core::Config::new();
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.lan.enabled = false;
        config.node.discovery.local.rendezvous_addr = rendezvous_addr;
        config.transports.websocket = TransportInstances::Single(WebSocketConfig {
            seed_urls: vec![format!("ws://{seed_addr}/fips")],
            ..WebSocketConfig::default()
        });
        let mut publisher_config = config.clone();
        publisher_config.transports.websocket =
            TransportInstances::Single(WebSocketConfig::default());
        let publisher_endpoint = Arc::new(
            fips_core::FipsEndpoint::builder()
                .config(publisher_config)
                .identity_nsec(keys.secret_key().to_bech32().unwrap())
                .discovery_scope("hashtree-provider")
                .local_rendezvous()
                .without_system_tun()
                .bind()
                .await
                .unwrap(),
        );
        let publisher = FipsPubsubClient::start(
            publisher_endpoint.clone(),
            FipsPubsubClientOptions::default(),
        )
        .await
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(HASHTREE_KIND), "")
            .tags([
                Tag::identifier(&reference.tree_name),
                Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        publisher
            .publish(
                VerifiedEvent::try_from(event).unwrap(),
                EventSource::local_index("release-test"),
            )
            .await
            .unwrap();

        assert_eq!(
            update_pubsub_options(&reference).unwrap().routed_peers,
            [reference.npub.clone()],
            "same-machine discovery must retain the trusted publisher route"
        );
        let provider = standalone_fips_update_provider(&reference, config)
            .await
            .unwrap();
        let updater = build_secure_pubsub_blossom_updater(
            provider,
            SecurePubsubBlossomConfig {
                blossom_read_servers: Vec::new(),
                ..secure_update_config()
            },
        )
        .await
        .unwrap();
        // Subscribe immediately: production startup must replay the request
        // when the authenticated local provider becomes reachable.
        let resolved = updater.resolver().resolve(&reference.resolver_key()).await;
        drop(updater);
        publisher.shutdown().await;
        publisher_endpoint.shutdown().await.unwrap();
        seed.shutdown().await.unwrap();
        assert_eq!(resolved.unwrap().unwrap().hash, [0x42; 32]);
    }

    #[tokio::test]
    async fn shared_provider_resolves_live_signed_release_without_relays() {
        let keys = Keys::generate();
        let reference = UpdateRef {
            npub: keys.public_key().to_bech32().unwrap(),
            tree_name: "releases/iris-chat-rs".into(),
            path: Some("latest".into()),
        };
        let event = EventBuilder::new(Kind::Custom(HASHTREE_KIND), "")
            .tags([
                Tag::identifier(&reference.tree_name),
                Tag::custom(TagKind::Custom("l".into()), ["hashtree"]),
                Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let provider = Arc::new(InMemoryEventBus::new());
        let shared: Arc<dyn NostrEventSubscriber> = provider.clone();
        register_update_provider(&shared);
        let (_, updater) = build_secure_update_updater().await.unwrap();
        let key = reference.resolver_key();
        let (resolved, _) = tokio::join!(updater.resolver().resolve(&key), async {
            provider
                .publish(
                    VerifiedEvent::try_from(event).unwrap(),
                    EventSource::peer("release-peer"),
                )
                .await
                .unwrap();
        });
        assert_eq!(resolved.unwrap().unwrap().hash, [0x42; 32]);
    }

    #[tokio::test]
    async fn quiet_provider_reports_inconclusive_check() {
        let provider = Arc::new(InMemoryEventBus::new());
        let updater = build_secure_pubsub_blossom_updater(
            provider,
            SecurePubsubBlossomConfig {
                blossom_read_servers: Vec::new(),
                manifest_timeout: Duration::from_millis(1),
                download_timeout: Duration::from_millis(1),
            },
        )
        .await
        .unwrap();
        assert!(updater
            .resolver()
            .resolve(&secure_update_ref().unwrap().resolver_key())
            .await
            .is_err());
    }

    #[test]
    fn release_cache_never_rolls_back_between_checks() {
        let keys = Keys::generate();
        let reference = UpdateRef {
            npub: keys.public_key().to_bech32().unwrap(),
            tree_name: "releases/cache-regression".into(),
            path: None,
        };
        let root = |at| {
            EventBuilder::new(Kind::Custom(HASHTREE_KIND), "")
                .tags([
                    Tag::identifier(&reference.tree_name),
                    Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
                ])
                .custom_created_at(nostr::Timestamp::from(at))
                .sign_with_keys(&keys)
                .unwrap()
        };
        let latest = root(2);
        remember_update_event(&reference, latest.clone()).unwrap();
        remember_update_event(&reference, latest.clone()).unwrap();
        assert!(remember_update_event(&reference, root(1)).is_err());
        let cache = UPDATE_EVENTS.get().unwrap().lock().unwrap();
        assert_eq!(
            cache.as_ref().unwrap().1.latest().unwrap().as_event().id,
            latest.id
        );
    }
}
