use super::profile::fallback_profile_name_for_identity;
use super::*;
use crate::state::FollowedUserSearchResult;
use futures_util::{stream, StreamExt};
use nostr_double_ratchet::VerifiedAppKeysIndex;
use rusqlite::Connection;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

const DISCOVERY_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_RECONNECT_FLOOR: Duration = Duration::from_secs(60);
const DISCOVERY_AUTHOR_CHUNK: usize = 100;
const DISCOVERY_CONCURRENT_REQUESTS: usize = 4;
const MAX_DISCOVERY_FOLLOWS: usize = 5_000;

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
        self.user_discovery_syncing = true;
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
        self.user_discovery_syncing = false;

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
            self.request_user_discovery_refresh(false);
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
        let invalidated_token = self.user_discovery_runtime.token.wrapping_add(1).max(1);
        self.user_discovery = UserDiscoveryCache::default();
        self.user_discovery_runtime = UserDiscoveryRuntime::default();
        self.user_discovery_runtime.token = invalidated_token;
        self.user_discovery_syncing = false;
        self.bump_user_discovery_revision();
    }

    pub(super) fn promote_discovered_user_for_peer_input(&mut self, peer_input: &str) {
        let Ok((owner_hex, _)) = parse_peer_input(peer_input) else {
            return;
        };
        if self.app_keys.contains_key(&owner_hex) {
            return;
        }
        let Some(raw_event) = self
            .user_discovery
            .users
            .get(&owner_hex)
            .map(|user| user.app_keys_event_json.clone())
        else {
            return;
        };
        let result = serde_json::from_str::<Event>(&raw_event)
            .map_err(anyhow::Error::from)
            .and_then(|event| self.apply_app_keys_event(&event));
        match result {
            Ok(true) => {
                self.push_debug_log("user.discovery.promote.ok", format!("owner={owner_hex}"))
            }
            Ok(false) => {
                self.push_debug_log("user.discovery.promote.skip", format!("owner={owner_hex}"))
            }
            Err(error) => self.push_debug_log(
                "user.discovery.promote.error",
                format!("owner={owner_hex} error={error}"),
            ),
        }
    }

    fn bump_user_discovery_revision(&mut self) {
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
    let Some(follow_event) = newest_verified_event(follow_events, Kind::ContactList, local_owner)
    else {
        return UserDiscoveryFetchResult {
            cache: previous,
            metadata_events: Vec::new(),
            detail: "follow_list_missing=true".to_string(),
        };
    };
    if follow_head_is_older(&follow_event, &previous) {
        return UserDiscoveryFetchResult {
            cache: previous,
            metadata_events: Vec::new(),
            detail: "follow_list_stale=true".to_string(),
        };
    }

    let follows = parse_follow_seeds(&follow_event, local_owner);
    let followed_owner_hexes = follows
        .iter()
        .map(|follow| follow.owner.to_hex())
        .collect::<HashSet<_>>();
    let mut next_users = previous
        .users
        .iter()
        .filter(|(owner, _)| followed_owner_hexes.contains(*owner))
        .map(|(owner, user)| (owner.clone(), user.clone()))
        .collect::<BTreeMap<_, _>>();
    for follow in &follows {
        if let Some(user) = next_users.get_mut(&follow.owner.to_hex()) {
            user.follow_position = follow.position;
            user.petname = follow.petname.clone();
        }
    }

    let app_keys_chunks = fetch_author_chunks(
        &client,
        Kind::from(APP_KEYS_EVENT_KIND as u16),
        follows.iter().map(|follow| follow.owner).collect(),
    )
    .await;
    let now_secs = unix_now().get();
    let follow_by_owner = follows
        .iter()
        .map(|follow| (follow.owner, follow.clone()))
        .collect::<HashMap<_, _>>();
    let mut failed_chunks = 0usize;
    for (owners, result) in app_keys_chunks {
        let Ok(events) = result else {
            failed_chunks += 1;
            continue;
        };
        let mut events_by_owner = events.into_iter().fold(
            HashMap::<PublicKey, Vec<Event>>::new(),
            |mut grouped, event| {
                grouped.entry(event.pubkey).or_default().push(event);
                grouped
            },
        );
        for owner in owners {
            let owner_hex = owner.to_hex();
            let mut candidates = events_by_owner.remove(&owner).unwrap_or_default();
            if let Some(previous_user) = previous.users.get(&owner_hex) {
                if let Ok(event) = serde_json::from_str::<Event>(&previous_user.app_keys_event_json)
                {
                    candidates.push(event);
                }
            }
            match select_eligible_app_keys_event(owner, candidates, now_secs) {
                Some(event) => {
                    let Some(follow) = follow_by_owner.get(&owner) else {
                        continue;
                    };
                    let Ok(app_keys_event_json) = serde_json::to_string(&event) else {
                        continue;
                    };
                    next_users.insert(
                        owner_hex.clone(),
                        DiscoveredUserRecord {
                            owner_pubkey_hex: owner_hex,
                            follow_position: follow.position,
                            petname: follow.petname.clone(),
                            app_keys_created_at_secs: event.created_at.as_secs(),
                            app_keys_event_id: event.id.to_hex(),
                            app_keys_event_json,
                        },
                    );
                }
                None => {
                    next_users.remove(&owner_hex);
                }
            }
        }
    }

    let metadata_chunks = fetch_author_chunks(
        &client,
        Kind::Metadata,
        next_users
            .keys()
            .filter_map(|owner| PublicKey::from_hex(owner).ok())
            .collect(),
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

    let eligible_count = next_users.len();
    UserDiscoveryFetchResult {
        cache: UserDiscoveryCache {
            follow_event_id: Some(follow_event.id.to_hex()),
            follow_created_at_secs: follow_event.created_at.as_secs(),
            users: next_users,
        },
        metadata_events,
        detail: format!(
            "follows={} eligible={} failed_chunks={} metadata_failed_chunks={}",
            follows.len(),
            eligible_count,
            failed_chunks,
            metadata_failed_chunks
        ),
    }
}

async fn fetch_author_chunks(
    client: &Client,
    kind: Kind,
    authors: Vec<PublicKey>,
) -> Vec<(Vec<PublicKey>, Result<Vec<Event>, String>)> {
    let chunks = authors
        .chunks(DISCOVERY_AUTHOR_CHUNK)
        .map(<[PublicKey]>::to_vec)
        .collect::<Vec<_>>();
    stream::iter(chunks)
        .map(|owners| {
            let client = client.clone();
            async move {
                let result = client
                    .fetch_events(
                        Filter::new().kind(kind).authors(owners.clone()),
                        DISCOVERY_REQUEST_TIMEOUT,
                    )
                    .await
                    .map(|events| events.iter().cloned().collect())
                    .map_err(|error| error.to_string());
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

fn newest_verified_events_by_author(events: Vec<Event>, kind: Kind) -> Vec<Event> {
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

fn follow_head_is_older(event: &Event, previous: &UserDiscoveryCache) -> bool {
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

fn select_eligible_app_keys_event(
    owner: PublicKey,
    mut events: Vec<Event>,
    now_secs: u64,
) -> Option<Event> {
    events.retain(|event| event.pubkey == owner);
    events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut index = VerifiedAppKeysIndex::default();
    for event in events {
        let _ = index.ingest(event, now_secs);
    }
    let mut heads = index.events_for_owner(owner);
    if heads.len() != 1 {
        return None;
    }
    let event = heads.pop()?;
    let app_keys = AppKeys::from_event(&event).ok()?;
    (!app_keys.get_all_devices().is_empty()).then_some(event)
}

pub(crate) fn search_followed_users(
    conn: &Connection,
    query: &str,
    excluded_owner_hexes: &HashSet<String>,
) -> anyhow::Result<Vec<FollowedUserSearchResult>> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }
    let terms = normalized_query.split_whitespace().collect::<Vec<_>>();
    let mut stmt = conn.prepare(
        "SELECT d.owner_pubkey_hex, d.follow_position, d.petname,
                p.name, p.display_name, p.picture, p.about
         FROM user_discovery_users d
         LEFT JOIN owner_profiles p ON p.owner_pubkey_hex = d.owner_pubkey_hex",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut matches = Vec::new();
    for row in rows {
        let (owner_hex, position, petname, name, display_name, picture, about) = row?;
        if excluded_owner_hexes.contains(&owner_hex) {
            continue;
        }
        let pubkey = PublicKey::from_hex(&owner_hex)?;
        let npub = pubkey.to_bech32().unwrap_or_else(|_| owner_hex.clone());
        let profile_name = normalize_profile_field(name);
        let profile_display_name = normalize_profile_field(display_name);
        let profile_label = profile_display_name
            .clone()
            .or_else(|| profile_name.clone());
        let petname = normalize_profile_field(petname);
        let about = normalize_profile_field(about);
        let display_label = petname
            .clone()
            .or_else(|| profile_label.clone())
            .unwrap_or_else(|| fallback_profile_name_for_identity(&owner_hex));
        let fields = [
            petname.as_deref().unwrap_or_default(),
            profile_name.as_deref().unwrap_or_default(),
            profile_display_name.as_deref().unwrap_or_default(),
            about.as_deref().unwrap_or_default(),
            owner_hex.as_str(),
            npub.as_str(),
        ];
        if !terms.iter().all(|term| {
            fields
                .iter()
                .any(|field| field.to_lowercase().contains(term))
        }) {
            continue;
        }
        let rank = if [
            petname.as_deref(),
            profile_name.as_deref(),
            profile_display_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|label| label.to_lowercase().starts_with(&normalized_query))
        {
            0u8
        } else {
            1u8
        };
        matches.push((
            rank,
            position,
            owner_hex.clone(),
            FollowedUserSearchResult {
                owner_pubkey_hex: owner_hex,
                display_label,
                profile_label,
                picture_url: normalize_profile_url(picture),
                about,
                user_id: compact_user_id(&npub),
            },
        ));
    }
    matches.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(matches.into_iter().map(|(_, _, _, row)| row).collect())
}

fn compact_user_id(user_id: &str) -> String {
    if user_id.len() > 16 {
        format!("{}…{}", &user_id[..10], &user_id[user_id.len() - 4..])
    } else {
        user_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};
    use nostr_double_ratchet::DeviceEntry;
    use tempfile::TempDir;

    fn follow_event(keys: &Keys, created_at: u64, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::ContactList, "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn app_keys_event(keys: &Keys, devices: &[PublicKey], created_at: u64) -> Event {
        AppKeys::new(
            devices
                .iter()
                .copied()
                .map(|device| DeviceEntry::new(device, created_at))
                .collect(),
        )
        .get_event_at(keys.public_key(), created_at)
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

    #[test]
    fn app_keys_requires_one_unambiguous_nonempty_current_head() {
        let owner = Keys::generate();
        let device_a = Keys::generate().public_key();
        let device_b = Keys::generate().public_key();
        let valid = app_keys_event(&owner, &[device_a], 100);
        let stale = app_keys_event(&owner, &[device_b], 90);
        assert_eq!(
            select_eligible_app_keys_event(owner.public_key(), vec![valid.clone(), stale], 200,)
                .unwrap()
                .id,
            valid.id
        );

        let equivalent = app_keys_event(&owner, &[device_a], 100);
        let equivalent_selected = select_eligible_app_keys_event(
            owner.public_key(),
            vec![valid.clone(), equivalent.clone()],
            200,
        )
        .unwrap();
        assert_eq!(
            equivalent_selected.id,
            std::cmp::min(valid.id, equivalent.id)
        );

        let mut invalid = app_keys_event(&owner, &[device_b], 120);
        invalid.content = "tampered".to_string();
        assert_eq!(
            select_eligible_app_keys_event(owner.public_key(), vec![valid.clone(), invalid], 200,)
                .unwrap()
                .id,
            valid.id
        );

        let empty = app_keys_event(&owner, &[], 110);
        assert!(select_eligible_app_keys_event(
            owner.public_key(),
            vec![valid.clone(), empty],
            200,
        )
        .is_none());

        let conflict = app_keys_event(&owner, &[device_b], 100);
        assert!(
            select_eligible_app_keys_event(owner.public_key(), vec![valid, conflict], 200,)
                .is_none()
        );
    }

    #[test]
    fn followed_search_matches_all_profile_fields_and_preserves_order() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE user_discovery_users (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 follow_position INTEGER NOT NULL,
                 petname TEXT,
                 app_keys_created_at_secs INTEGER NOT NULL,
                 app_keys_event_id TEXT NOT NULL,
                 app_keys_event_json TEXT NOT NULL
             );
             CREATE TABLE owner_profiles (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 name TEXT,
                 display_name TEXT,
                 picture TEXT,
                 about TEXT
             );",
        )
        .unwrap();
        let first = Keys::generate().public_key().to_hex();
        let second = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO user_discovery_users VALUES (?1, 0, NULL, 1, 'a', '{}')",
            [&first],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_discovery_users VALUES (?1, 1, 'Alfred', 1, 'b', '{}')",
            [&second],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO owner_profiles VALUES (?1, 'Wonderland', 'Alice', NULL, 'Rust builder')",
            [&first],
        )
        .unwrap();

        let rows = search_followed_users(&conn, "al", &HashSet::new()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].owner_pubkey_hex, first);
        assert_eq!(rows[1].owner_pubkey_hex, second);
        assert_eq!(
            search_followed_users(&conn, "wonder", &HashSet::new())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search_followed_users(&conn, "builder", &HashSet::from([first]))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn local_relay_discovers_app_keys_metadata_and_removes_unfollowed_user() {
        let relay = crate::local_relay::TestRelay::start();
        let local_owner = Keys::generate();
        let followed_owner = Keys::generate();
        let followed_device = Keys::generate().public_key();
        let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
        let follow = follow_event(
            &local_owner,
            100,
            vec![Tag::parse([
                "p",
                followed_owner.public_key().to_hex().as_str(),
                "",
                "Relay friend",
            ])
            .unwrap()],
        );
        let app_keys = app_keys_event(&followed_owner, &[followed_device], 100);
        let metadata = EventBuilder::new(
            Kind::Metadata,
            r#"{"name":"relay-alice","display_name":"Relay Alice","about":"Local test"}"#,
        )
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&followed_owner)
        .unwrap();
        let publisher = Client::new(Keys::generate());
        let discovery_client = Client::new(Keys::generate());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let first = runtime.block_on(async {
            ensure_session_relays_configured(&publisher, &relay_urls).await;
            connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
            publisher.send_event(&follow).await.unwrap();
            publisher.send_event(&app_keys).await.unwrap();
            publisher.send_event(&metadata).await.unwrap();
            fetch_user_discovery(
                discovery_client.clone(),
                relay_urls.clone(),
                local_owner.public_key(),
                UserDiscoveryCache::default(),
            )
            .await
        });
        assert_eq!(first.cache.users.len(), 1);
        assert_eq!(
            first
                .cache
                .users
                .get(&followed_owner.public_key().to_hex())
                .and_then(|user| user.petname.as_deref()),
            Some("Relay friend")
        );
        assert_eq!(first.metadata_events.len(), 1);

        let unfollow = follow_event(&local_owner, 101, Vec::new());
        let second = runtime.block_on(async {
            publisher.send_event(&unfollow).await.unwrap();
            let result = fetch_user_discovery(
                discovery_client.clone(),
                relay_urls,
                local_owner.public_key(),
                first.cache,
            )
            .await;
            let _ = publisher.shutdown().await;
            let _ = discovery_client.shutdown().await;
            result
        });
        assert!(second.cache.users.is_empty());
        assert_eq!(second.cache.follow_event_id, Some(unfollow.id.to_hex()));
    }

    #[test]
    fn cached_app_keys_are_promoted_before_chat_creation() {
        let temp = TempDir::new().unwrap();
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        let remote_owner = Keys::generate();
        let remote_device = Keys::generate().public_key();
        let event = app_keys_event(&remote_owner, &[remote_device], unix_now().get());
        let remote_hex = remote_owner.public_key().to_hex();
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            temp.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.logged_in = Some(LoggedInState {
            owner_pubkey: local_owner.public_key(),
            owner_keys: Some(local_owner),
            device_keys: local_device.clone(),
            client: Client::new(local_device),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });
        core.user_discovery.users.insert(
            remote_hex.clone(),
            DiscoveredUserRecord {
                owner_pubkey_hex: remote_hex.clone(),
                follow_position: 0,
                petname: None,
                app_keys_created_at_secs: event.created_at.as_secs(),
                app_keys_event_id: event.id.to_hex(),
                app_keys_event_json: serde_json::to_string(&event).unwrap(),
            },
        );

        assert!(!core.app_keys.contains_key(&remote_hex));
        core.create_chat(&remote_hex);

        assert!(core.app_keys.contains_key(&remote_hex));
        assert!(core.threads.contains_key(&remote_hex));
        assert_eq!(
            core.state.router.screen_stack,
            vec![Screen::Chat {
                chat_id: remote_hex
            }]
        );
    }
}
