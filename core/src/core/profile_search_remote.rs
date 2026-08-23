use super::{ProfileSearchCandidate, ProfileSearchFetchResult};
use async_trait::async_trait;
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{nhash_decode, Cid, Hash, MemoryStore, Store, StoreError};
use hashtree_index::{SearchIndex, SearchIndexOptions, SearchOptions, SearchResult};
use hashtree_resolver::{
    nostr::{NostrResolverConfig, NostrRootResolver},
    RootResolver,
};
use nostr::{Keys, PublicKey, RelayUrl};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROFILE_SEARCH_REF: &str =
    "npub1dhuna75xx06lj4v4gkf9klgklrem9ez82h9u9zpxd77usm73pcdqctllwf/profile-search";
const PROFILE_SEARCH_SNAPSHOT: &str =
    "nhash1qqsdspyk9j47vfde5w6lgjqftp2uuzw6wqptkwyuvlg8w7lh7dn370c9yr8hastd4k5cf49de7nfvtqu0t3v8mqn339fywyz4hafp66pspfx78z5lgs";
const PROFILE_SEARCH_RELAY: &str = "wss://hashtree.iris.to/ws";
const PROFILE_SEARCH_BLOSSOM_SERVERS: &[&str] =
    &["https://hashtree.iris.to", "https://cdn.iris.to"];

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const BLOSSOM_TIMEOUT: Duration = Duration::from_secs(4);
const OVERALL_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESOLVER_RELAYS: usize = 12;
const MAX_QUERY_BYTES: usize = 256;
const MAX_QUERY_CHARS: usize = 128;
const MAX_QUERY_TERMS: usize = 8;
const MAX_INDEX_RESULTS: usize = 64;
const MAX_INDEX_VALUE_BYTES: usize = 16 * 1024;
const MAX_PROFILE_TEXT_BYTES: usize = 400;
const MAX_PROFILE_TEXT_CHARS: usize = 100;
const MAX_ALIASES: usize = 16;
const MAX_NIP05_BYTES: usize = 256;
const MAX_PICTURE_BYTES: usize = 2 * 1024;
const MAX_STORE_READS: usize = 256;
const MAX_STORE_BLOB_BYTES: usize = 1024 * 1024;
const MAX_STORE_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const PROFILE_EVENT_FUTURE_TOLERANCE_SECS: u64 = 600;

#[derive(Debug, Deserialize)]
struct StoredProfileSearchEntry {
    pubkey: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    nip05: Option<String>,
    picture: Option<String>,
    #[serde(default)]
    created_at: u64,
}

struct CandidateBatch {
    candidates: Vec<ProfileSearchCandidate>,
    dropped: usize,
}

struct ReadBudgetStore<S> {
    inner: Arc<S>,
    reads_left: AtomicUsize,
    bytes_left: AtomicUsize,
}

impl<S> ReadBudgetStore<S> {
    fn new(inner: Arc<S>) -> Self {
        Self::with_limits(inner, MAX_STORE_READS, MAX_STORE_TOTAL_BYTES)
    }

    fn with_limits(inner: Arc<S>, reads: usize, bytes: usize) -> Self {
        Self {
            inner,
            reads_left: AtomicUsize::new(reads),
            bytes_left: AtomicUsize::new(bytes),
        }
    }

    fn consume(counter: &AtomicUsize, amount: usize) -> Result<(), StoreError> {
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(amount)
            })
            .map(|_| ())
            .map_err(|_| StoreError::Other("profile search read budget exceeded".to_string()))
    }
}

#[async_trait]
impl<S: Store> Store for ReadBudgetStore<S> {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Err(StoreError::Other(
            "profile search store is read-only".to_string(),
        ))
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        Self::consume(&self.reads_left, 1)?;
        let data = self.inner.get(hash).await?;
        if let Some(data) = data.as_ref() {
            if data.len() > MAX_STORE_BLOB_BYTES {
                return Err(StoreError::Other(
                    "profile search blob is too large".to_string(),
                ));
            }
            Self::consume(&self.bytes_left, data.len())?;
        }
        Ok(data)
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Self::consume(&self.reads_left, 1)?;
        self.inner.has(hash).await
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Err(StoreError::Other(
            "profile search store is read-only".to_string(),
        ))
    }
}

/// Fetch globally indexed Nostr profiles without making the index an authority
/// for messaging eligibility. AppKeys/NDR resolution remains at the existing
/// user-selection and chat boundary.
pub(super) async fn fetch_profile_candidates(
    query: &str,
    relay_urls: &[String],
) -> Result<ProfileSearchFetchResult, String> {
    let deadline = Instant::now() + OVERALL_TIMEOUT;
    tokio::time::timeout(
        OVERALL_TIMEOUT,
        fetch_profile_candidates_within_deadline(query, relay_urls, deadline),
    )
    .await
    .map_err(|_| "profile search timed out".to_string())?
}

async fn fetch_profile_candidates_within_deadline(
    query: &str,
    relay_urls: &[String],
    deadline: Instant,
) -> Result<ProfileSearchFetchResult, String> {
    let Some(query) = normalize_profile_search_query(query)? else {
        return Ok(ProfileSearchFetchResult {
            candidates: Vec::new(),
            detail: "source=none reason=query-too-short".to_string(),
        });
    };
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let snapshot_root = decode_nhash(PROFILE_SEARCH_SNAPSHOT)
        .map_err(|error| format!("invalid built-in profile snapshot: {error}"))?;
    let store = Arc::new(ReadBudgetStore::new(Arc::new(BlossomStore::new(
        BlossomClient::new_empty(Keys::generate())
            .with_read_servers(
                PROFILE_SEARCH_BLOSSOM_SERVERS
                    .iter()
                    .map(|server| (*server).to_string())
                    .collect(),
            )
            .with_timeout(BLOSSOM_TIMEOUT),
    ))));

    let live_root = resolve_live_root(relay_urls).await;
    let (batch, source, live_detail) = match live_root {
        Ok(Some(root)) if root != snapshot_root => {
            match search_root(store.clone(), &root, &query, now_secs, deadline).await {
                Ok(batch) => (batch, "live", "resolved".to_string()),
                Err(live_error) => {
                    match search_root(store.clone(), &snapshot_root, &query, now_secs, deadline)
                        .await
                    {
                        Ok(batch) => (
                            batch,
                            "snapshot",
                            format!("live-search-error={}", compact_detail(&live_error)),
                        ),
                        Err(snapshot_error) => {
                            return Err(format!(
                                "live profile search failed: {}; snapshot fallback failed: {}",
                                compact_detail(&live_error),
                                compact_detail(&snapshot_error)
                            ));
                        }
                    }
                }
            }
        }
        Ok(Some(_)) => (
            search_root(store.clone(), &snapshot_root, &query, now_secs, deadline)
                .await
                .map_err(|error| {
                    format!("resolved profile snapshot could not be searched: {error}")
                })?,
            "live",
            "resolved-snapshot".to_string(),
        ),
        Ok(None) => (
            search_root(store.clone(), &snapshot_root, &query, now_secs, deadline)
                .await
                .map_err(|error| format!("profile snapshot search failed: {error}"))?,
            "snapshot",
            "not-found".to_string(),
        ),
        Err(error) => (
            search_root(store, &snapshot_root, &query, now_secs, deadline)
                .await
                .map_err(|snapshot_error| {
                    format!(
                        "profile root resolution failed: {}; snapshot fallback failed: {}",
                        compact_detail(&error),
                        compact_detail(&snapshot_error)
                    )
                })?,
            "snapshot",
            format!("resolve-error={}", compact_detail(&error)),
        ),
    };

    let detail = format!(
        "source={source} candidates={} dropped={} live={live_detail}",
        batch.candidates.len(),
        batch.dropped,
    );

    Ok(ProfileSearchFetchResult {
        candidates: batch.candidates,
        detail,
    })
}

async fn resolve_live_root(relay_urls: &[String]) -> Result<Option<Cid>, String> {
    let relays = resolver_relays(relay_urls);
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays,
        resolve_timeout: RESOLVE_TIMEOUT,
        secret_key: None,
    })
    .await
    .map_err(|error| error.to_string())?;
    let result = resolver
        .resolve(PROFILE_SEARCH_REF)
        .await
        .map_err(|error| error.to_string());
    let _ = resolver.stop().await;
    result
}

fn resolver_relays(relay_urls: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    std::iter::once(PROFILE_SEARCH_RELAY)
        .chain(relay_urls.iter().map(String::as_str))
        .filter_map(|raw| RelayUrl::parse(raw.trim()).ok())
        .map(|relay| relay.to_string())
        .filter(|relay| seen.insert(relay.clone()))
        .take(MAX_RESOLVER_RELAYS)
        .collect()
}

async fn search_root<S: Store + 'static>(
    store: Arc<S>,
    root: &Cid,
    query: &str,
    now_secs: u64,
    deadline: Instant,
) -> Result<CandidateBatch, String> {
    // hashtree-index 0.2.82's recursive B-tree traversal future is not Send.
    // Keep that implementation detail off the app's multithreaded runtime.
    let root = root.clone();
    let query = query.to_string();
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start profile index reader: {error}"))?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        runtime.block_on(async {
            tokio::time::timeout(remaining, search_root_local(store, &root, &query, now_secs))
                .await
                .map_err(|_| "profile search timed out".to_string())?
        })
    })
    .await
    .map_err(|error| format!("profile index reader stopped: {error}"))?
}

async fn search_root_local<S: Store + 'static>(
    store: Arc<S>,
    root: &Cid,
    query: &str,
    now_secs: u64,
) -> Result<CandidateBatch, String> {
    let index = SearchIndex::new(store, SearchIndexOptions::default());
    let results = index
        .search(
            Some(root),
            "p:",
            query,
            SearchOptions {
                limit: Some(MAX_INDEX_RESULTS),
                full_match: false,
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    let mut owners = HashSet::new();
    let mut candidates = Vec::with_capacity(results.len());
    let mut dropped = 0;
    for result in results.into_iter().take(MAX_INDEX_RESULTS) {
        match parse_search_result(result, now_secs) {
            Ok(candidate) if owners.insert(candidate.owner_pubkey_hex.clone()) => {
                candidates.push(candidate)
            }
            Ok(_) | Err(_) => dropped += 1,
        }
    }
    Ok(CandidateBatch {
        candidates,
        dropped,
    })
}

fn parse_search_result(
    result: SearchResult,
    now_secs: u64,
) -> Result<ProfileSearchCandidate, String> {
    if result.value.len() > MAX_INDEX_VALUE_BYTES {
        return Err("profile index value is too large".to_string());
    }
    let stored = serde_json::from_str::<StoredProfileSearchEntry>(&result.value)
        .map_err(|error| format!("invalid profile index value: {error}"))?;
    let result_owner = PublicKey::from_hex(result.id.trim())
        .map_err(|_| "profile index result id is invalid".to_string())?;
    let owner = match stored
        .pubkey
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(pubkey) => {
            let owner = PublicKey::from_hex(pubkey)
                .map_err(|_| "profile index pubkey is invalid".to_string())?;
            if owner != result_owner {
                return Err("profile index id does not match its pubkey".to_string());
            }
            owner
        }
        None => result_owner,
    };
    if stored.created_at > i64::MAX as u64
        || stored.created_at > now_secs.saturating_add(PROFILE_EVENT_FUTURE_TOLERANCE_SECS)
    {
        return Err("profile timestamp is out of range".to_string());
    }

    let owner_pubkey_hex = owner.to_hex();
    let name = bounded_text(&stored.name, MAX_PROFILE_TEXT_BYTES)
        .map_err(|_| "profile name is invalid".to_string())?
        .unwrap_or_else(|| owner_pubkey_hex.clone());
    let aliases = bounded_aliases(stored.aliases)?;
    let nip05 = stored
        .nip05
        .as_deref()
        .and_then(|value| bounded_single_line(value, MAX_NIP05_BYTES));
    let picture = stored.picture.as_deref().and_then(bounded_picture_url);
    Ok(ProfileSearchCandidate {
        owner_pubkey_hex,
        name,
        aliases,
        nip05,
        picture,
        created_at_secs: stored.created_at,
    })
}

fn bounded_aliases(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > MAX_ALIASES {
        return Err("profile has too many aliases".to_string());
    }
    let mut seen = HashSet::new();
    let mut aliases = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = bounded_text(&value, MAX_PROFILE_TEXT_BYTES)
            .map_err(|_| "profile alias is invalid".to_string())?
        else {
            continue;
        };
        if seen.insert(value.to_lowercase()) {
            aliases.push(value);
        }
    }
    Ok(aliases)
}

pub(super) fn normalize_profile_search_query(query: &str) -> Result<Option<String>, String> {
    if query.len() > MAX_QUERY_BYTES || query.chars().count() > MAX_QUERY_CHARS {
        return Err("profile search query is too long".to_string());
    }
    if query
        .chars()
        .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err("profile search query contains control characters".to_string());
    }
    let parser = SearchIndex::new(Arc::new(MemoryStore::new()), SearchIndexOptions::default());
    let keywords = parser.parse_keywords(query);
    if keywords.len() > MAX_QUERY_TERMS {
        return Err("profile search query has too many terms".to_string());
    }
    let searchable_chars = query
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    Ok((searchable_chars >= 2 && !keywords.is_empty()).then(|| keywords.join(" ")))
}

/// `Ok(None)` means empty text. `Err(())` means malformed or over the limit.
fn bounded_text(value: &str, max_bytes: usize) -> Result<Option<String>, ()> {
    if value.len() > max_bytes
        || value.chars().count() > MAX_PROFILE_TEXT_CHARS
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(());
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok((!normalized.is_empty()).then_some(normalized))
}

fn bounded_single_line(value: &str, max_bytes: usize) -> Option<String> {
    if value.len() > max_bytes || value.chars().any(char::is_whitespace) {
        return None;
    }
    bounded_text(value, max_bytes).ok().flatten()
}

fn bounded_picture_url(value: &str) -> Option<String> {
    let value = bounded_single_line(value, MAX_PICTURE_BYTES)?;
    let parsed = url::Url::parse(&value).ok()?;
    matches!(parsed.scheme(), "https" | "http" | "htree").then_some(value)
}

fn decode_nhash(value: &str) -> Result<Cid, String> {
    let decoded = nhash_decode(value).map_err(|error| error.to_string())?;
    Ok(Cid {
        hash: decoded.hash,
        key: decoded.decrypt_key,
    })
}

fn compact_detail(value: &str) -> String {
    const MAX_DETAIL_CHARS: usize = 240;
    let mut compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > MAX_DETAIL_CHARS {
        compact = compact.chars().take(MAX_DETAIL_CHARS).collect();
        compact.push('…');
    }
    compact
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct HangingStore;

    #[async_trait]
    impl Store for HangingStore {
        async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
            Err(StoreError::Other("unexpected write".to_string()))
        }

        async fn get(&self, _hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
            std::future::pending().await
        }

        async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
            Ok(true)
        }

        async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
            Err(StoreError::Other("unexpected delete".to_string()))
        }
    }

    const SIRIUS_HEX: &str = "336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b";
    const GIGI_HEX: &str = "6e468422dfb74a5738702a8823b9b28168abab8655faacb6853cd0ee15deee93";

    fn index_value(pubkey: &str, name: &str) -> String {
        json!({
            "pubkey": pubkey,
            "name": name,
            "aliases": [],
            "nip05": null,
            "picture": null,
            "created_at": 42,
            "event_nhash": null
        })
        .to_string()
    }

    #[test]
    fn parser_accepts_bounded_profile_and_rejects_mismatched_id() {
        let value = json!({
            "pubkey": SIRIUS_HEX,
            "name": "  Sirius   Business ",
            "aliases": ["Sirius", "sirius"],
            "nip05": "sirius@iris.to",
            "picture": "https://cdn.iris.to/sirius.jpg",
            "created_at": 42,
            "event_nhash": PROFILE_SEARCH_SNAPSHOT
        })
        .to_string();
        let parsed = parse_search_result(
            SearchResult {
                id: SIRIUS_HEX.to_string(),
                value: value.clone(),
                score: 1,
            },
            100,
        )
        .unwrap();
        assert_eq!(parsed.name, "Sirius Business");
        assert_eq!(parsed.aliases, vec!["Sirius"]);

        let error = parse_search_result(
            SearchResult {
                id: GIGI_HEX.to_string(),
                value,
                score: 1,
            },
            100,
        )
        .unwrap_err();
        assert!(error.contains("does not match"));
    }

    #[test]
    fn parser_rejects_future_dated_profiles() {
        let value = json!({
            "pubkey": SIRIUS_HEX,
            "name": "Future Sirius",
            "aliases": [],
            "created_at": 701,
        })
        .to_string();
        let error = parse_search_result(
            SearchResult {
                id: SIRIUS_HEX.to_string(),
                value,
                score: 1,
            },
            100,
        )
        .unwrap_err();
        assert!(error.contains("timestamp"));
    }

    #[test]
    fn query_limit_uses_hashtree_keywords_not_only_whitespace() {
        assert!(normalize_profile_search_query("aa-bb-cc-dd-ee-ff-gg-hh-ii").is_err());
        assert_eq!(
            normalize_profile_search_query("JohnDoe")
                .unwrap()
                .as_deref(),
            Some("johndoe john doe")
        );
    }

    #[tokio::test]
    async fn read_budget_rejects_oversized_store_blobs() {
        let inner = Arc::new(MemoryStore::new());
        let hash = [7; 32];
        inner
            .put(hash, vec![0; MAX_STORE_BLOB_BYTES + 1])
            .await
            .unwrap();
        let store = ReadBudgetStore::new(inner);

        let error = store.get(&hash).await.unwrap_err();
        assert!(error.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn read_budget_enforces_access_and_aggregate_byte_limits() {
        let inner = Arc::new(MemoryStore::new());
        let first = [1; 32];
        let second = [2; 32];
        inner.put(first, vec![1, 2]).await.unwrap();
        inner.put(second, vec![3, 4]).await.unwrap();

        let reads = ReadBudgetStore::with_limits(inner.clone(), 2, 10);
        assert!(reads.has(&first).await.unwrap());
        assert!(reads.has(&second).await.unwrap());
        assert!(reads.has(&first).await.is_err());

        let bytes = ReadBudgetStore::with_limits(inner, 3, 3);
        assert!(bytes.get(&first).await.unwrap().is_some());
        assert!(bytes.get(&second).await.is_err());
    }

    #[tokio::test]
    async fn blocking_traversal_stops_at_its_internal_deadline() {
        let started = Instant::now();
        let error = search_root(
            Arc::new(HangingStore),
            &Cid::public([0; 32]),
            "sirius",
            100,
            Instant::now() + Duration::from_millis(20),
        )
        .await
        .err()
        .expect("hanging traversal should time out");

        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn memory_index_prefix_search_returns_the_expected_profile() {
        let store = Arc::new(MemoryStore::new());
        let index = SearchIndex::new(store.clone(), SearchIndexOptions::default());
        let root = index
            .index(
                None,
                "p:",
                &["sirius".to_string(), "business".to_string()],
                SIRIUS_HEX,
                &index_value(SIRIUS_HEX, "Sirius Business"),
            )
            .await
            .unwrap();
        let root = index
            .index(
                Some(&root),
                "p:",
                &["gigi".to_string()],
                GIGI_HEX,
                &index_value(GIGI_HEX, "Gigi"),
            )
            .await
            .unwrap();

        let store = Arc::new(ReadBudgetStore::new(store));
        let batch = search_root(
            store,
            &root,
            "sir",
            100,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(batch.dropped, 0);
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(batch.candidates[0].owner_pubkey_hex, SIRIUS_HEX);
    }

    #[tokio::test]
    #[ignore = "live production profile index smoke test"]
    async fn production_index_finds_default_graph_profiles() {
        let relays = vec![
            "wss://nos.lol".to_string(),
            "wss://relay.damus.io".to_string(),
            "wss://relay.primal.net".to_string(),
        ];
        for query in ["sirius", "gigi"] {
            let result = fetch_profile_candidates(query, &relays)
                .await
                .unwrap_or_else(|error| panic!("{query}: {error}"));
            assert!(
                !result.candidates.is_empty(),
                "production index returned no candidates for {query}: {}",
                result.detail
            );
        }
    }
}
