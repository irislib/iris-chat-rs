//! Signed update-root announcements shared by the desktop and CLI updaters.
//!
//! Transport ownership stays with the application. A FIPS or other
//! `nostr-pubsub` provider can refresh this small process-local cache, while
//! update resolution and authenticated downloads remain in `hashtree-updater`.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use hashtree_resolver::Event;
use hashtree_updater::{
    build_secure_nostr_blossom_updater_with_events, SecureNostrBlossomConfig,
    SecureNostrBlossomUpdater, UpdateError, UpdateEventCache, UpdateRef,
};
use nostr_pubsub::EventBus;

pub const HTREE_UPDATE_REF: &str =
    "htree://npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/releases%2Firis-chat-rs/latest";

const DEFAULT_UPDATE_RELAYS: &[&str] = &[
    "wss://temp.iris.to",
    "wss://relay.damus.io",
    "wss://relay.snort.social",
    "wss://relay.primal.net",
    "wss://upload.iris.to/nostr",
];
const DEFAULT_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];
const UPDATE_MANIFEST_TIMEOUT: Duration = Duration::from_secs(8);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
struct CachedUpdateEvents {
    reference: UpdateRef,
    cache: UpdateEventCache,
}

static UPDATE_EVENTS: OnceLock<Mutex<Option<CachedUpdateEvents>>> = OnceLock::new();

fn update_events() -> &'static Mutex<Option<CachedUpdateEvents>> {
    UPDATE_EVENTS.get_or_init(|| Mutex::new(None))
}

pub fn secure_update_ref() -> Result<UpdateRef, UpdateError> {
    let raw = std::env::var("IRIS_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| HTREE_UPDATE_REF.to_string());
    UpdateRef::parse(&raw)
}

pub fn update_announcement_filter() -> Result<nostr_pubsub::Filter, UpdateError> {
    Ok(UpdateEventCache::new(&secure_update_ref()?)?
        .filter()
        .clone())
}

fn secure_update_config() -> SecureNostrBlossomConfig {
    SecureNostrBlossomConfig {
        relays: env_csv("IRIS_UPDATE_RELAYS").unwrap_or_else(|| strings(DEFAULT_UPDATE_RELAYS)),
        blossom_read_servers: env_csv("IRIS_UPDATE_BLOSSOM_SERVERS")
            .unwrap_or_else(|| strings(DEFAULT_BLOSSOM_READ_SERVERS)),
        manifest_timeout: UPDATE_MANIFEST_TIMEOUT,
        download_timeout: UPDATE_DOWNLOAD_TIMEOUT,
    }
}

/// Query a selected pubsub provider using the exact trusted release filter.
///
/// The provider may be backed by FIPS, a relay, or a deterministic test bus.
/// `UpdateEventCache` verifies both the signature and publisher/tree filter
/// before any event becomes visible to an update check.
pub async fn refresh_update_announcements<P>(provider: &P) -> Result<bool, UpdateError>
where
    P: EventBus + ?Sized,
{
    let reference = secure_update_ref()?;
    let mut refreshed = UpdateEventCache::new(&reference)?;
    refreshed.refresh(provider).await?;
    merge_update_events(&reference, refreshed.resolver_events())
}

/// Ingest one signed event delivered by an app-owned transport subscription.
pub fn ingest_update_announcement(event: Event) -> Result<bool, UpdateError> {
    let reference = secure_update_ref()?;
    merge_update_events(&reference, [event])
}

/// Construct the one secure updater used by both the native desktop bridge and
/// the CLI. Cached peer announcements are seeded before the resolver consults
/// its relay fallback.
pub async fn build_secure_update_updater(
) -> Result<(UpdateRef, SecureNostrBlossomUpdater), UpdateError> {
    let reference = secure_update_ref()?;
    let events = cached_resolver_events(&reference)?;
    let updater =
        build_secure_nostr_blossom_updater_with_events(secure_update_config(), events).await?;
    Ok((reference, updater))
}

fn merge_update_events(
    reference: &UpdateRef,
    events: impl IntoIterator<Item = Event>,
) -> Result<bool, UpdateError> {
    let mut guard = update_events()
        .lock()
        .map_err(|_| UpdateError::Announcement("update event cache lock poisoned".to_string()))?;
    if guard
        .as_ref()
        .is_none_or(|cached| cached.reference != *reference)
    {
        *guard = Some(CachedUpdateEvents {
            reference: reference.clone(),
            cache: UpdateEventCache::new(reference)?,
        });
    }
    let cache = &mut guard
        .as_mut()
        .ok_or_else(|| UpdateError::Announcement("update cache not initialized".to_string()))?
        .cache;
    let mut advanced = false;
    for event in events {
        advanced |= cache.ingest_event(event)?;
    }
    Ok(advanced)
}

fn cached_resolver_events(reference: &UpdateRef) -> Result<Vec<Event>, UpdateError> {
    let guard = update_events()
        .lock()
        .map_err(|_| UpdateError::Announcement("update event cache lock poisoned".to_string()))?;
    Ok(guard
        .as_ref()
        .filter(|cached| cached.reference == *reference)
        .map(|cached| cached.cache.resolver_events())
        .unwrap_or_default())
}

fn env_csv(name: &str) -> Option<Vec<String>> {
    let values = std::env::var(name)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use hashtree_core::Cid;
    use hashtree_resolver::{nostr::HASHTREE_KIND, RootResolver};
    use nostr::{
        Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag, TagKind, Timestamp, ToBech32,
    };
    use nostr_pubsub::{EventSource, InMemoryEventBus, VerifiedEvent};

    use super::*;

    fn root_event(keys: &Keys, tree_name: &str, created_at: u64, cid: &Cid) -> Event {
        EventBuilder::new(Kind::Custom(HASHTREE_KIND), "")
            .tags([
                Tag::identifier(tree_name),
                Tag::custom(
                    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
                    ["hashtree"],
                ),
                Tag::custom(
                    TagKind::Custom("hash".into()),
                    [cid.hash
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()],
                ),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("signed update root")
    }

    #[tokio::test]
    async fn provider_event_is_filtered_and_seeds_the_secure_resolver_without_relays() {
        let release_keys = Keys::generate();
        let other_keys = Keys::generate();
        let tree_name = "releases/iris-chat-rs";
        let reference = UpdateRef {
            npub: release_keys.public_key().to_bech32().expect("release npub"),
            tree_name: tree_name.to_string(),
            path: Some("latest".to_string()),
        };
        let expected = Cid {
            hash: [0x42; 32],
            key: None,
        };
        let provider = InMemoryEventBus::new();
        for event in [
            root_event(
                &other_keys,
                tree_name,
                2,
                &Cid {
                    hash: [1; 32],
                    key: None,
                },
            ),
            root_event(
                &release_keys,
                "releases/other",
                3,
                &Cid {
                    hash: [2; 32],
                    key: None,
                },
            ),
            root_event(&release_keys, tree_name, 4, &expected),
        ] {
            provider
                .publish(
                    VerifiedEvent::try_from(event).expect("verified provider event"),
                    EventSource::peer("connected-update-peer"),
                )
                .await
                .expect("publish provider event");
        }

        let mut cache = UpdateEventCache::new(&reference).expect("update event cache");
        assert!(cache.refresh(&provider).await.expect("refresh provider"));
        assert_eq!(
            cache.resolver_events().len(),
            1,
            "wrong roots were filtered"
        );

        let updater = build_secure_nostr_blossom_updater_with_events(
            SecureNostrBlossomConfig {
                relays: Vec::new(),
                blossom_read_servers: Vec::new(),
                manifest_timeout: Duration::from_millis(50),
                download_timeout: Duration::from_millis(50),
            },
            cache.resolver_events(),
        )
        .await
        .expect("relayless secure updater");

        assert_eq!(
            updater
                .resolver()
                .resolve(&reference.resolver_key())
                .await
                .expect("relayless root resolution"),
            Some(expected)
        );
    }
}
