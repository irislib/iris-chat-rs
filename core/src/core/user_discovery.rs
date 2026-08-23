use super::user_discovery_social::{
    apply_follow_order, rank_followed_owners, MAX_SOCIAL_OPINION_AUTHORS,
};
use super::*;
use futures_util::{stream, StreamExt};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

const DISCOVERY_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_RECONNECT_FLOOR: Duration = Duration::from_secs(60);
const DISCOVERY_AUTHOR_CHUNK: usize = 100;
const DISCOVERY_CONCURRENT_REQUESTS: usize = 4;
const MAX_DISCOVERY_FOLLOWS: usize = 5_000;
const MAX_PEER_FOLLOW_EVENTS_PER_CHUNK: usize = 512;
const DISCOVERY_EVENT_FUTURE_TOLERANCE_SECS: u64 = 600;

pub(super) fn discovery_event_time_is_acceptable(created_at_secs: u64, now_secs: u64) -> bool {
    created_at_secs <= now_secs.saturating_add(DISCOVERY_EVENT_FUTURE_TOLERANCE_SECS)
}

#[derive(Clone, Debug)]
struct FollowSeed {
    owner: PublicKey,
    position: u32,
    petname: Option<String>,
}

impl AppCore {
    pub(super) fn request_user_discovery_refresh(&mut self, force: bool) {
        let Some((client, relay_urls, local_owner)) = self
            .logged_in
            .as_ref()
            .filter(|session| !session.relay_urls.is_empty())
            .map(|session| {
                (
                    session.client.clone(),
                    session.relay_urls.clone(),
                    session.owner_pubkey,
                )
            })
        else {
            return;
        };

        if self.user_discovery_runtime.in_flight {
            self.user_discovery_runtime.refresh_pending = true;
            return;
        }
        if !force
            && self
                .user_discovery_runtime
                .last_started_at
                .is_some_and(|started| started.elapsed() < DISCOVERY_RECONNECT_FLOOR)
        {
            return;
        }

        self.user_discovery_runtime.token =
            self.user_discovery_runtime.token.wrapping_add(1).max(1);
        let token = self.user_discovery_runtime.token;
        self.user_discovery_runtime.in_flight = true;
        self.user_discovery_runtime.last_started_at = Some(Instant::now());
        self.refresh_people_syncing();
        self.bump_user_discovery_revision();
        self.rebuild_state();
        self.emit_state();

        let previous = self.user_discovery.clone();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let result = fetch_user_discovery(client, relay_urls, local_owner, previous).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::UserDiscoveryFetchFinished { token, result },
            )));
        });
    }

    pub(super) fn handle_user_discovery_fetch_finished(
        &mut self,
        token: u64,
        result: UserDiscoveryFetchResult,
    ) {
        if token != self.user_discovery_runtime.token || !self.user_discovery_runtime.in_flight {
            return;
        }
        self.user_discovery_runtime.in_flight = false;
        self.refresh_people_syncing();

        let mut metadata_changed = false;
        for event in newest_verified_events_by_author(result.metadata_events, Kind::Metadata) {
            metadata_changed |= self.apply_profile_metadata_event(&event);
        }

        let cache_changed = result.cache != self.user_discovery;
        if cache_changed {
            match self.app_store.replace_user_discovery(&result.cache) {
                Ok(()) => self.user_discovery = result.cache,
                Err(error) => {
                    self.push_debug_log("user.discovery.persist.error", format!("error={error}"))
                }
            }
        }
        if metadata_changed {
            self.persist_best_effort();
        }
        self.bump_user_discovery_revision();
        self.push_debug_log("user.discovery.complete", result.detail);
        self.rebuild_state();
        self.emit_state();

        if std::mem::take(&mut self.user_discovery_runtime.refresh_pending) {
            self.request_user_discovery_refresh(true);
        }
    }

    pub(super) fn restore_user_discovery_cache(&mut self) {
        match self.app_store.load_user_discovery() {
            Ok(cache) => self.user_discovery = cache,
            Err(error) => {
                self.user_discovery = UserDiscoveryCache::default();
                self.push_debug_log("user.discovery.restore.error", error.to_string());
            }
        }
    }

    pub(super) fn reset_user_discovery_runtime(&mut self) {
        let invalidated_discovery_token = self.user_discovery_runtime.token.wrapping_add(1).max(1);
        let invalidated_search_token = self.profile_search_runtime.token.wrapping_add(1).max(1);
        self.user_discovery = UserDiscoveryCache::default();
        self.user_discovery_runtime = UserDiscoveryRuntime::default();
        self.user_discovery_runtime.token = invalidated_discovery_token;
        self.profile_search_runtime = ProfileSearchRuntime::default();
        self.profile_search_runtime.token = invalidated_search_token;
        self.refresh_people_syncing();
        self.bump_user_discovery_revision();
    }

    pub(super) fn bump_user_discovery_revision(&mut self) {
        self.user_discovery_revision = self.user_discovery_revision.wrapping_add(1).max(1);
    }
}

async fn fetch_user_discovery(
    client: Client,
    relay_urls: Vec<RelayUrl>,
    local_owner: PublicKey,
    previous: UserDiscoveryCache,
) -> UserDiscoveryFetchResult {
    ensure_session_relays_configured(&client, &relay_urls).await;
    connect_client_with_timeout(&client, DISCOVERY_REQUEST_TIMEOUT).await;

    let follow_events = match client
        .fetch_events(
            Filter::new().kind(Kind::ContactList).author(local_owner),
            DISCOVERY_REQUEST_TIMEOUT,
        )
        .await
    {
        Ok(events) => events.iter().cloned().collect::<Vec<_>>(),
        Err(error) => {
            return UserDiscoveryFetchResult {
                cache: previous,
                metadata_events: Vec::new(),
                detail: format!("follow_list_error={error}"),
            };
        }
    };
    let root_now_secs = unix_now().get();
    let follow_events = follow_events
        .into_iter()
        .filter(|event| {
            discovery_event_time_is_acceptable(event.created_at.as_secs(), root_now_secs)
        })
        .collect();
    let Some(follow_event) = newest_verified_event(follow_events, Kind::ContactList, local_owner)
    else {
        return UserDiscoveryFetchResult {
            cache: previous,
            metadata_events: Vec::new(),
            detail: "follow_list_missing=true".to_string(),
        };
    };
    if follow_head_is_older(&follow_event, &previous, root_now_secs) {
        return UserDiscoveryFetchResult {
            cache: previous,
            metadata_events: Vec::new(),
            detail: "follow_list_stale=true".to_string(),
        };
    }

    let follows = parse_follow_seeds(&follow_event, local_owner);
    let mut next_users = follows
        .iter()
        .map(|follow| {
            let owner_pubkey_hex = follow.owner.to_hex();
            let follow_position = previous
                .users
                .get(&owner_pubkey_hex)
                .map(|user| user.follow_position)
                .unwrap_or(follow.position);
            (
                owner_pubkey_hex.clone(),
                DiscoveredUserRecord {
                    owner_pubkey_hex,
                    follow_position,
                    petname: follow.petname.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let followed_owners = follows
        .iter()
        .map(|follow| follow.owner)
        .collect::<Vec<_>>();
    let opinion_authors = followed_owners
        .iter()
        .take(MAX_SOCIAL_OPINION_AUTHORS)
        .copied()
        .collect::<Vec<_>>();
    let peer_follow_chunks = fetch_author_chunks(
        &client,
        Kind::ContactList,
        opinion_authors.clone(),
        Some(MAX_PEER_FOLLOW_EVENTS_PER_CHUNK),
    )
    .await;
    let now_secs = unix_now().get();

    let mut social_failed_chunks = 0usize;
    let mut peer_follow_events = Vec::new();
    for (_, result) in peer_follow_chunks {
        match result {
            Ok(events) => peer_follow_events.extend(events),
            Err(_) => social_failed_chunks += 1,
        }
    }
    let social_order = if social_failed_chunks == 0 {
        let ranked = rank_followed_owners(
            local_owner,
            &follow_event,
            &followed_owners,
            &opinion_authors,
            peer_follow_events,
            now_secs,
        );
        if ranked.is_none() {
            social_failed_chunks = 1;
        }
        ranked
    } else {
        None
    };
    let follow_event_id = follow_event.id.to_hex();
    if social_order.is_some()
        || previous.follow_event_id.as_deref() != Some(follow_event_id.as_str())
        || !previous.users.keys().eq(next_users.keys())
    {
        apply_follow_order(&mut next_users, &followed_owners, social_order.as_deref());
    }

    let metadata_chunks = fetch_author_chunks(
        &client,
        Kind::Metadata,
        next_users
            .keys()
            .filter_map(|owner| PublicKey::from_hex(owner).ok())
            .collect(),
        None,
    )
    .await;
    let mut metadata_events = Vec::new();
    let mut metadata_failed_chunks = 0usize;
    for (owners, result) in metadata_chunks {
        match result {
            Ok(events) => {
                let requested = owners.into_iter().collect::<HashSet<_>>();
                metadata_events.extend(
                    events
                        .into_iter()
                        .filter(|event| requested.contains(&event.pubkey)),
                );
            }
            Err(_) => metadata_failed_chunks += 1,
        }
    }

    UserDiscoveryFetchResult {
        cache: UserDiscoveryCache {
            follow_event_id: Some(follow_event_id),
            follow_created_at_secs: follow_event.created_at.as_secs(),
            users: next_users,
        },
        metadata_events,
        detail: format!(
            "follows={} metadata_failed_chunks={} social_failed_chunks={}",
            follows.len(),
            metadata_failed_chunks,
            social_failed_chunks
        ),
    }
}

async fn fetch_author_chunks(
    client: &Client,
    kind: Kind,
    authors: Vec<PublicKey>,
    max_events_per_chunk: Option<usize>,
) -> Vec<(Vec<PublicKey>, Result<Vec<Event>, String>)> {
    let chunks = authors
        .chunks(DISCOVERY_AUTHOR_CHUNK)
        .map(<[PublicKey]>::to_vec)
        .collect::<Vec<_>>();
    stream::iter(chunks)
        .map(|owners| {
            let client = client.clone();
            async move {
                let mut filter = Filter::new().kind(kind).authors(owners.clone());
                if let Some(max_events) = max_events_per_chunk {
                    filter = filter.limit(max_events.saturating_add(1));
                }
                let result = client
                    .fetch_events(filter, DISCOVERY_REQUEST_TIMEOUT)
                    .await
                    .map(|events| events.iter().cloned().collect::<Vec<_>>())
                    .map_err(|error| error.to_string())
                    .and_then(|events| match max_events_per_chunk {
                        Some(max_events) if events.len() > max_events => {
                            Err("event limit exceeded".to_string())
                        }
                        _ => Ok(events),
                    });
                (owners, result)
            }
        })
        .buffer_unordered(DISCOVERY_CONCURRENT_REQUESTS)
        .collect()
        .await
}

fn newest_verified_event(events: Vec<Event>, kind: Kind, author: PublicKey) -> Option<Event> {
    events
        .into_iter()
        .filter(|event| event.kind == kind && event.pubkey == author && event.verify().is_ok())
        .min_by(compare_replaceable_heads)
}

pub(super) fn newest_verified_events_by_author(events: Vec<Event>, kind: Kind) -> Vec<Event> {
    let mut grouped = HashMap::<PublicKey, Vec<Event>>::new();
    for event in events {
        grouped.entry(event.pubkey).or_default().push(event);
    }
    grouped
        .into_iter()
        .filter_map(|(owner, events)| newest_verified_event(events, kind, owner))
        .collect()
}

fn compare_replaceable_heads(left: &Event, right: &Event) -> Ordering {
    right
        .created_at
        .cmp(&left.created_at)
        .then_with(|| left.id.cmp(&right.id))
}

fn follow_head_is_older(event: &Event, previous: &UserDiscoveryCache, now_secs: u64) -> bool {
    if !discovery_event_time_is_acceptable(previous.follow_created_at_secs, now_secs) {
        return false;
    }
    let timestamp = event.created_at.as_secs();
    if timestamp != previous.follow_created_at_secs {
        return timestamp < previous.follow_created_at_secs;
    }
    previous
        .follow_event_id
        .as_ref()
        .is_some_and(|current| event.id.to_hex() > *current)
}

fn parse_follow_seeds(event: &Event, local_owner: PublicKey) -> Vec<FollowSeed> {
    let mut seen = HashSet::new();
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            if values.first().map(String::as_str) != Some("p") {
                return None;
            }
            let owner = values
                .get(1)
                .and_then(|value| PublicKey::from_hex(value).ok())?;
            if owner == local_owner || !seen.insert(owner) || seen.len() > MAX_DISCOVERY_FOLLOWS {
                return None;
            }
            let petname = values
                .get(3)
                .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|value| !value.is_empty());
            Some(FollowSeed {
                owner,
                position: (seen.len() - 1) as u32,
                petname,
            })
        })
        .take(MAX_DISCOVERY_FOLLOWS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    fn follow_event(keys: &Keys, created_at: u64, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::ContactList, "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn newest_follow_list_uses_lower_event_id_for_timestamp_ties() {
        let keys = Keys::generate();
        let first = follow_event(
            &keys,
            10,
            vec![Tag::parse(["p", Keys::generate().public_key().to_hex().as_str()]).unwrap()],
        );
        let second = follow_event(
            &keys,
            10,
            vec![Tag::parse(["p", Keys::generate().public_key().to_hex().as_str()]).unwrap()],
        );
        let selected = newest_verified_event(
            vec![first.clone(), second.clone()],
            Kind::ContactList,
            keys.public_key(),
        )
        .unwrap();
        assert_eq!(selected.id, std::cmp::min(first.id, second.id));
    }

    #[test]
    fn follow_tags_preserve_order_deduplicate_and_exclude_self() {
        let keys = Keys::generate();
        let alice = Keys::generate().public_key();
        let bob = Keys::generate().public_key();
        let event = follow_event(
            &keys,
            10,
            vec![
                Tag::parse(["p", alice.to_hex().as_str(), "", "Alice P."]).unwrap(),
                Tag::parse(["p", "malformed"]).unwrap(),
                Tag::parse(["p", keys.public_key().to_hex().as_str()]).unwrap(),
                Tag::parse(["p", alice.to_hex().as_str()]).unwrap(),
                Tag::parse(["p", bob.to_hex().as_str()]).unwrap(),
            ],
        );
        let follows = parse_follow_seeds(&event, keys.public_key());
        assert_eq!(follows.len(), 2);
        assert_eq!(follows[0].owner, alice);
        assert_eq!(follows[0].position, 0);
        assert_eq!(follows[0].petname.as_deref(), Some("Alice P."));
        assert_eq!(follows[1].owner, bob);
        assert_eq!(follows[1].position, 1);
    }

    #[test]
    fn invalid_newer_follow_list_is_ignored() {
        let keys = Keys::generate();
        let valid = follow_event(&keys, 10, Vec::new());
        let mut invalid = follow_event(&keys, 20, Vec::new());
        invalid.content = "tampered".to_string();
        let selected = newest_verified_event(
            vec![invalid, valid.clone()],
            Kind::ContactList,
            keys.public_key(),
        )
        .unwrap();
        assert_eq!(selected.id, valid.id);
    }

    #[test]
    fn follow_list_is_bounded() {
        let keys = Keys::generate();
        let tags = (0..(MAX_DISCOVERY_FOLLOWS + 20))
            .map(|_| Tag::parse(["p", Keys::generate().public_key().to_hex().as_str()]).unwrap())
            .collect();
        let event = follow_event(&keys, 10, tags);
        let follows = parse_follow_seeds(&event, keys.public_key());
        assert_eq!(follows.len(), MAX_DISCOVERY_FOLLOWS);
        assert_eq!(follows.last().unwrap().position, 4_999);
    }
}

#[cfg(test)]
#[path = "user_discovery_refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "user_discovery_fetch_tests.rs"]
mod fetch_tests;
