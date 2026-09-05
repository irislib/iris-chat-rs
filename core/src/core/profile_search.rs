use super::profile::fallback_profile_name_for_identity;
use super::profile_search_capability::{fetch_search_app_keys, MAX_SEARCH_CAPABILITY_CANDIDATES};
use super::*;
use crate::state::FollowedUserSearchResult;
use nostr_social_graph::SocialGraph;
use rusqlite::{Connection, OptionalExtension};
use std::cmp::Reverse;
use std::sync::OnceLock;

const PROFILE_SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);
const PROFILE_SEARCH_RETRY_FLOOR: Duration = Duration::from_secs(30);
const MAX_RECENT_PROFILE_SEARCHES: usize = 32;
// Same reduced fallback graph shipped by nostr-social-graph/Iris Client.
// SHA-256: 6d6ce09dfcc587e4de377a40ced50221568d674705d7e909182390e8edd58d1c.
const DEFAULT_SOCIAL_GRAPH_ROOT: &str =
    "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0";
static DEFAULT_SOCIAL_GRAPH: OnceLock<Option<SocialGraph>> = OnceLock::new();

impl AppCore {
    pub(super) fn request_profile_search(&mut self, query: &str) {
        if self.logged_in.is_none() {
            return;
        }
        let query = match super::profile_search_remote::normalize_profile_search_query(query) {
            Ok(Some(query)) => query,
            Ok(None) | Err(_) => {
                if self.profile_search_runtime.in_flight {
                    self.profile_search_runtime.pending = Some(PendingProfileSearch::Cancel);
                } else {
                    if self.profile_search_runtime.debounce_pending {
                        self.profile_search_runtime.token =
                            self.profile_search_runtime.token.wrapping_add(1).max(1);
                    }
                    self.profile_search_runtime.debounce_pending = false;
                    self.profile_search_runtime.query.clear();
                    self.profile_search_runtime.pending = None;
                }
                self.refresh_people_syncing_and_emit_if_changed();
                return;
            }
        };

        let now = Instant::now();
        self.profile_search_runtime
            .recent_attempts
            .retain(|(_, attempted_at)| {
                now.saturating_duration_since(*attempted_at) < PROFILE_SEARCH_RETRY_FLOOR
            });
        if self.profile_search_runtime.in_flight {
            self.profile_search_runtime.pending = (self.profile_search_runtime.query != query)
                .then_some(PendingProfileSearch::Query(query));
            return;
        }
        if self.profile_search_runtime.debounce_pending {
            if self.profile_search_runtime.query == query {
                return;
            }
            self.profile_search_runtime.token =
                self.profile_search_runtime.token.wrapping_add(1).max(1);
            self.profile_search_runtime.debounce_pending = false;
            self.profile_search_runtime.query.clear();
        }
        if self
            .profile_search_runtime
            .recent_attempts
            .iter()
            .any(|(attempted, _)| attempted == &query)
        {
            self.refresh_people_syncing_and_emit_if_changed();
            return;
        }

        self.profile_search_runtime.token =
            self.profile_search_runtime.token.wrapping_add(1).max(1);
        self.profile_search_runtime.query = query.clone();
        self.profile_search_runtime.debounce_pending = true;
        self.refresh_people_syncing_and_emit_if_changed();
        let token = self.profile_search_runtime.token;
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            sleep(PROFILE_SEARCH_DEBOUNCE).await;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProfileSearchDebounceElapsed { token, query },
            )));
        });
    }

    pub(super) fn handle_profile_search_debounce_elapsed(&mut self, token: u64, query: &str) {
        if token != self.profile_search_runtime.token
            || query != self.profile_search_runtime.query
            || !self.profile_search_runtime.debounce_pending
            || self.profile_search_runtime.in_flight
            || self.logged_in.is_none()
        {
            return;
        }
        let relay_urls = self
            .logged_in
            .as_ref()
            .map(|session| {
                session
                    .relay_urls
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let client = self.logged_in.as_ref().unwrap().client.clone();
        let local_owner = self
            .logged_in
            .as_ref()
            .map(|session| session.owner_pubkey.to_hex());
        let mut excluded = self
            .preferences
            .blocked_owner_pubkeys
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if let Some(owner) = &local_owner {
            excluded.insert(owner.clone());
        }
        let local_candidates = self
            .app_store
            .shared()
            .lock()
            .ok()
            .and_then(|conn| {
                search_people_candidates(&conn, query, &excluded, local_owner.as_deref()).ok()
            })
            .unwrap_or_default()
            .into_iter()
            .take(MAX_SEARCH_CAPABILITY_CANDIDATES)
            .filter_map(|person| PublicKey::parse(&person.owner_pubkey_hex).ok())
            .collect::<Vec<_>>();

        self.profile_search_runtime.debounce_pending = false;
        self.profile_search_runtime.in_flight = true;
        self.refresh_people_syncing();

        let query = query.to_string();
        let tx = self.core_sender.clone();
        self.runtime.spawn(async move {
            let (result, mut app_keys_events) = tokio::join!(
                super::profile_search_remote::fetch_profile_candidates(&query, &relay_urls),
                fetch_search_app_keys(&client, local_candidates.clone()),
            );
            // An unavailable index must not stop verification of locally known people.
            let mut result = result.unwrap_or_else(|error| ProfileSearchFetchResult {
                candidates: Vec::new(),
                app_keys_events: Vec::new(),
                detail: error,
            });
            let remote_owners = result
                .candidates
                .iter()
                .filter(|person| !excluded.contains(&person.owner_pubkey_hex))
                .filter_map(|person| PublicKey::parse(&person.owner_pubkey_hex).ok())
                .filter(|owner| !local_candidates.contains(owner))
                .take(MAX_SEARCH_CAPABILITY_CANDIDATES)
                .collect();
            app_keys_events.extend(fetch_search_app_keys(&client, remote_owners).await);
            result.app_keys_events = app_keys_events;
            let result = Ok(result);
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::ProfileSearchFetchFinished {
                    token,
                    query,
                    result,
                },
            )));
        });
    }

    pub(super) fn handle_profile_search_fetch_finished(
        &mut self,
        token: u64,
        query: &str,
        result: Result<ProfileSearchFetchResult, String>,
    ) {
        if token != self.profile_search_runtime.token
            || query != self.profile_search_runtime.query
            || !self.profile_search_runtime.in_flight
        {
            return;
        }
        self.profile_search_runtime.in_flight = false;
        if let Some(pending) = self.profile_search_runtime.pending.take() {
            match pending {
                PendingProfileSearch::Query(query) => self.request_profile_search(&query),
                PendingProfileSearch::Cancel => self.profile_search_runtime.query.clear(),
            }
            self.refresh_people_syncing();
            self.bump_user_discovery_revision();
            self.rebuild_state();
            self.emit_state();
            return;
        }
        remember_profile_search_attempt(&mut self.profile_search_runtime, query);

        let detail = match result {
            Ok(result) => {
                for event in result.app_keys_events {
                    self.handle_relay_event(event);
                }
                match self
                    .app_store
                    .upsert_profile_search_candidates(&result.candidates)
                {
                    Ok(_) => {}
                    Err(error) => self.push_debug_log(
                        "profile.search.persist.error",
                        format!("query={query} error={error}"),
                    ),
                }
                result.detail
            }
            Err(error) => format!("query={query} error={error}"),
        };

        self.refresh_people_syncing();
        self.bump_user_discovery_revision();
        self.push_debug_log("profile.search.complete", detail);
        self.rebuild_state();
        self.emit_state();
    }

    pub(super) fn refresh_people_syncing(&mut self) {
        self.user_discovery_syncing = self.user_discovery_runtime.in_flight
            || self.profile_search_runtime.in_flight
            || self.profile_search_runtime.debounce_pending;
    }

    fn refresh_people_syncing_and_emit_if_changed(&mut self) {
        let previous = self.user_discovery_syncing;
        self.refresh_people_syncing();
        if self.user_discovery_syncing != previous {
            self.bump_user_discovery_revision();
            self.rebuild_state();
            self.emit_state();
        }
    }

    pub(super) fn cancel_people_fetches_for_suspend(&mut self) {
        let discovery_was_active =
            self.user_discovery_runtime.in_flight || self.user_discovery_runtime.refresh_pending;
        let search_was_active = self.profile_search_runtime.in_flight
            || self.profile_search_runtime.debounce_pending
            || self.profile_search_runtime.pending.is_some();

        self.user_discovery_runtime.token =
            self.user_discovery_runtime.token.wrapping_add(1).max(1);
        self.user_discovery_runtime.in_flight = false;
        self.user_discovery_runtime.refresh_pending = false;
        self.user_discovery_runtime.last_started_at = None;

        self.profile_search_runtime.token =
            self.profile_search_runtime.token.wrapping_add(1).max(1);
        self.profile_search_runtime.query.clear();
        self.profile_search_runtime.debounce_pending = false;
        self.profile_search_runtime.in_flight = false;
        self.profile_search_runtime.pending = None;

        self.refresh_people_syncing();
        if discovery_was_active || search_was_active {
            self.bump_user_discovery_revision();
        }
    }
}

fn remember_profile_search_attempt(runtime: &mut ProfileSearchRuntime, query: &str) {
    runtime
        .recent_attempts
        .retain(|(attempted, _)| attempted != query);
    runtime
        .recent_attempts
        .push_back((query.to_string(), Instant::now()));
    while runtime.recent_attempts.len() > MAX_RECENT_PROFILE_SEARCHES {
        runtime.recent_attempts.pop_front();
    }
}

/// Public People results require a device list verified by the protocol layer.
/// Reading the persisted cache keeps verified results available offline.
pub(crate) fn search_people(
    conn: &Connection,
    query: &str,
    excluded_owner_hexes: &HashSet<String>,
    current_owner_hex: Option<&str>,
) -> anyhow::Result<Vec<FollowedUserSearchResult>> {
    let mut stmt = conn.prepare("SELECT owner_pubkey_hex, devices_json FROM app_keys")?;
    let records = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut supported = HashSet::new();
    for record in records {
        let (owner, json) = record?;
        if serde_json::from_str::<Vec<KnownAppKeyDevice>>(&json)
            .is_ok_and(|devices| !devices.is_empty())
        {
            supported.insert(owner);
        }
    }
    Ok(
        search_people_candidates(conn, query, excluded_owner_hexes, current_owner_hex)?
            .into_iter()
            .filter(|person| supported.contains(&person.owner_pubkey_hex))
            .collect(),
    )
}

fn search_people_candidates(
    conn: &Connection,
    query: &str,
    excluded_owner_hexes: &HashSet<String>,
    current_owner_hex: Option<&str>,
) -> anyhow::Result<Vec<FollowedUserSearchResult>> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }
    let compact_query = compact_search_text(&normalized_query);
    let terms = normalized_query
        .split_whitespace()
        .map(|term| (term.to_string(), compact_search_text(term)))
        .collect::<Vec<_>>();
    let personalized_social = match current_owner_hex {
        Some(owner) => conn
            .query_row(
                "SELECT social_rank_ready FROM user_discovery_state
                 WHERE id = 1 AND owner_pubkey_hex = ?1",
                [owner],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false),
        None => false,
    };
    let mut stmt = conn.prepare(
        "WITH current_discovery AS (
             SELECT u.* FROM user_discovery_users u
             JOIN user_discovery_state d ON d.id = 1
             WHERE COALESCE(d.owner_pubkey_hex, '') = ?1
         ), candidate_owners AS (
             SELECT owner_pubkey_hex FROM current_discovery
             UNION SELECT owner_pubkey_hex FROM profile_search_candidates
             UNION SELECT owner_pubkey_hex FROM owner_profiles
         )
         SELECT c.owner_pubkey_hex, d.follow_position, d.petname,
                p.name, p.display_name, p.picture, p.about,
                p.owner_pubkey_hex IS NOT NULL,
                s.name, s.aliases_json, s.nip05, s.picture, r.friend_support
         FROM candidate_owners c
         LEFT JOIN current_discovery d
           ON d.owner_pubkey_hex = c.owner_pubkey_hex
         LEFT JOIN owner_profiles p
           ON p.owner_pubkey_hex = c.owner_pubkey_hex
         LEFT JOIN profile_search_candidates s
           ON s.owner_pubkey_hex = c.owner_pubkey_hex
         LEFT JOIN user_discovery_social r
           ON r.account_owner_pubkey_hex = ?1
          AND r.target_owner_pubkey_hex = c.owner_pubkey_hex
          AND EXISTS (
              SELECT 1 FROM user_discovery_state sr
              WHERE sr.id = 1 AND sr.owner_pubkey_hex = ?1
                AND sr.social_rank_ready = 1
          )",
    )?;
    let rows = stmt.query_map([current_owner_hex.unwrap_or_default()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?.map(|value| value as u32),
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, bool>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<u16>>(12)?.map(usize::from),
        ))
    })?;

    let mut matches = Vec::new();
    for row in rows {
        let (
            owner_hex,
            follow_position,
            petname,
            profile_name,
            profile_display_name,
            profile_picture,
            about,
            has_canonical_profile,
            indexed_name,
            aliases_json,
            nip05,
            indexed_picture,
            personalized_friend_support,
        ) = row?;
        if excluded_owner_hexes.contains(&owner_hex) {
            continue;
        }
        let Ok(pubkey) = PublicKey::from_hex(&owner_hex) else {
            continue;
        };
        let npub = pubkey.to_bech32().unwrap_or_else(|_| owner_hex.clone());
        let petname = normalize_profile_field(petname);
        let profile_name = normalize_profile_field(profile_name);
        let profile_display_name = normalize_profile_field(profile_display_name);
        // A verified kind-0 event is authoritative once we have one. The
        // global index remains a discovery hint, but must not resurrect fields
        // that the owner subsequently cleared.
        let indexed_name = (!has_canonical_profile)
            .then(|| normalize_profile_field(indexed_name))
            .flatten();
        let nip05 = (!has_canonical_profile)
            .then(|| normalize_profile_field(nip05))
            .flatten();
        let about = normalize_profile_field(about);
        let aliases = if has_canonical_profile {
            Vec::new()
        } else {
            aliases_json
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| normalize_profile_field(Some(value)))
                .collect::<Vec<_>>()
        };
        let profile_label = profile_display_name
            .clone()
            .or_else(|| profile_name.clone())
            .or_else(|| indexed_name.clone());
        let display_label = petname
            .clone()
            .or_else(|| profile_label.clone())
            .unwrap_or_else(|| fallback_profile_name_for_identity(&owner_hex));
        let picture_url = normalize_profile_url(if has_canonical_profile {
            profile_picture
        } else {
            profile_picture.or(indexed_picture)
        });
        let mut fields = vec![
            petname.as_deref().unwrap_or_default(),
            profile_name.as_deref().unwrap_or_default(),
            profile_display_name.as_deref().unwrap_or_default(),
            indexed_name.as_deref().unwrap_or_default(),
            nip05.as_deref().unwrap_or_default(),
            about.as_deref().unwrap_or_default(),
            owner_hex.as_str(),
            npub.as_str(),
        ];
        fields.extend(aliases.iter().map(String::as_str));
        let searchable_fields = fields
            .iter()
            .flat_map(|field| [field.to_lowercase(), compact_search_text(field)])
            .collect::<Vec<_>>();
        if !terms.iter().all(|(term, compact_term)| {
            searchable_fields.iter().any(|field| {
                field.contains(term) || (!compact_term.is_empty() && field.contains(compact_term))
            })
        }) {
            continue;
        }

        let labels = [
            petname.as_deref(),
            profile_name.as_deref(),
            profile_display_name.as_deref(),
            indexed_name.as_deref(),
        ];
        let text_rank = if labels
            .into_iter()
            .flatten()
            .any(|label| search_text_equals(label, &normalized_query, &compact_query))
        {
            0u8
        } else if labels
            .into_iter()
            .flatten()
            .any(|label| search_text_starts_with(label, &normalized_query, &compact_query))
        {
            1u8
        } else if aliases
            .iter()
            .chain(nip05.iter())
            .any(|value| search_text_starts_with(value, &normalized_query, &compact_query))
        {
            2u8
        } else {
            3u8
        };
        let (social_source, social_distance, friend_support) =
            social_rank(&owner_hex, personalized_social, personalized_friend_support);
        matches.push((
            text_rank,
            follow_position.unwrap_or(u32::MAX),
            social_source,
            social_distance,
            friend_support,
            owner_hex.clone(),
            FollowedUserSearchResult {
                owner_pubkey_hex: owner_hex,
                display_label,
                profile_label,
                picture_url,
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
            .then_with(|| left.3.cmp(&right.3))
            .then_with(|| left.4.cmp(&right.4))
            .then_with(|| left.5.cmp(&right.5))
    });
    Ok(matches
        .into_iter()
        .map(|(_, _, _, _, _, _, row)| row)
        .collect())
}

fn social_rank(
    owner: &str,
    personalized_social: bool,
    personalized_friend_support: Option<usize>,
) -> (u8, u32, Reverse<usize>) {
    if let Some(support) = personalized_friend_support.filter(|support| *support > 0) {
        return (0, 2, Reverse(support));
    }
    let (distance, support) = default_social_rank(owner);
    (u8::from(personalized_social), distance, support)
}

fn default_social_rank(owner: &str) -> (u32, Reverse<usize>) {
    let Some(graph) = DEFAULT_SOCIAL_GRAPH.get().and_then(Option::as_ref) else {
        return (u32::MAX, Reverse(0));
    };
    let friend_support = graph
        .get_followers_by_user(owner)
        .into_iter()
        .filter(|follower| graph.is_following(DEFAULT_SOCIAL_GRAPH_ROOT, follower))
        .count();
    (graph.get_follow_distance(owner), Reverse(friend_support))
}

pub(crate) fn prewarm_default_social_graph() {
    DEFAULT_SOCIAL_GRAPH.get_or_init(|| {
        SocialGraph::from_binary(
            DEFAULT_SOCIAL_GRAPH_ROOT,
            include_bytes!("../../assets/socialGraph.bin"),
        )
        .ok()
    });
}

fn compact_user_id(user_id: &str) -> String {
    if user_id.len() > 16 {
        format!("{}…{}", &user_id[..10], &user_id[user_id.len() - 4..])
    } else {
        user_id.to_string()
    }
}

fn compact_search_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn search_text_equals(value: &str, query: &str, compact_query: &str) -> bool {
    value.to_lowercase() == query
        || (!compact_query.is_empty() && compact_search_text(value) == compact_query)
}

fn search_text_starts_with(value: &str, query: &str, compact_query: &str) -> bool {
    value.to_lowercase().starts_with(query)
        || (!compact_query.is_empty() && compact_search_text(value).starts_with(compact_query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use std::sync::{Arc, RwLock};
    use tempfile::TempDir;

    const GIGI_HEX: &str = "6e468422dfb74a5738702a8823b9b28168abab8655faacb6853cd0ee15deee93";

    fn test_core() -> (TempDir, AppCore) {
        let temp = TempDir::new().unwrap();
        let core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            temp.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        (temp, core)
    }

    fn log_in(core: &mut AppCore) {
        let owner = Keys::generate();
        let device = Keys::generate();
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner),
            device_keys: device.clone(),
            client: Client::new(device),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });
    }

    fn people_search_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE user_discovery_state (
                 id INTEGER PRIMARY KEY,
                 owner_pubkey_hex TEXT,
                 social_rank_ready INTEGER NOT NULL
             );
             CREATE TABLE user_discovery_users (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 follow_position INTEGER NOT NULL,
                 petname TEXT
             );
             CREATE TABLE user_discovery_social (
                 account_owner_pubkey_hex TEXT NOT NULL,
                 target_owner_pubkey_hex TEXT NOT NULL,
                 friend_support INTEGER NOT NULL,
                 PRIMARY KEY(account_owner_pubkey_hex, target_owner_pubkey_hex)
             );
             CREATE TABLE owner_profiles (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 name TEXT,
                 display_name TEXT,
                 picture TEXT,
                 about TEXT
             );
             CREATE TABLE profile_search_candidates (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 aliases_json TEXT NOT NULL,
                 nip05 TEXT,
                 picture TEXT,
                 created_at_secs INTEGER NOT NULL,
                 cached_at_secs INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn candidate_search_includes_profiles_pending_verification() {
        let conn = people_search_connection();
        let global = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO profile_search_candidates(
                 owner_pubkey_hex, name, aliases_json, nip05, picture,
                 created_at_secs, cached_at_secs
             ) VALUES (?1, 'Sirius', '[\"Sirius Business\"]', 'sirius@iris.to',
                       NULL, 1, 1)",
            [&global],
        )
        .unwrap();

        let rows = search_people_candidates(&conn, "sirius", &HashSet::new(), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owner_pubkey_hex, global);
        assert_eq!(rows[0].display_label, "Sirius");
    }

    #[test]
    fn people_search_matches_camel_case_against_spaced_names() {
        let conn = people_search_connection();
        let owner = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO profile_search_candidates VALUES
                 (?1, 'John Doe', '[]', NULL, NULL, 1, 1)",
            [&owner],
        )
        .unwrap();

        let rows = search_people_candidates(&conn, "JohnDoe", &HashSet::new(), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owner_pubkey_hex, owner);
    }

    #[test]
    fn equally_relevant_direct_follows_keep_social_order_before_global_hits() {
        let conn = people_search_connection();
        let first = Keys::generate().public_key().to_hex();
        let second = Keys::generate().public_key().to_hex();
        let global = Keys::generate().public_key().to_hex();
        let root = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO user_discovery_state VALUES (1, ?1, 0)",
            [&root],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_discovery_users VALUES (?1, 1, 'Alex Three')",
            [&second],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_discovery_users VALUES (?1, 0, 'Alex Two')",
            [&first],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO profile_search_candidates VALUES
                 (?1, 'Alex One', '[]', NULL, NULL, 1, 1)",
            [&global],
        )
        .unwrap();

        let owners = search_people_candidates(&conn, "alex", &HashSet::new(), Some(&root))
            .unwrap()
            .into_iter()
            .map(|row| row.owner_pubkey_hex)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec![first, second, global]);
    }

    #[test]
    fn default_graph_fills_personalized_gaps_without_filtering_unknowns() {
        prewarm_default_social_graph();
        let conn = people_search_connection();
        let root = Keys::generate().public_key().to_hex();
        let unknown = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO user_discovery_state VALUES (1, ?1, 1)",
            [&root],
        )
        .unwrap();
        for owner in [GIGI_HEX, &unknown] {
            conn.execute(
                "INSERT INTO profile_search_candidates VALUES
                     (?1, 'Alex', '[]', NULL, NULL, 1, 1)",
                [owner],
            )
            .unwrap();
        }

        let owners = search_people_candidates(&conn, "alex", &HashSet::new(), Some(&root))
            .unwrap()
            .into_iter()
            .map(|row| row.owner_pubkey_hex)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec![GIGI_HEX.to_string(), unknown]);
        let (distance, Reverse(friend_support)) = default_social_rank(GIGI_HEX);
        assert!(distance < 1_000);
        assert!(friend_support > 0);
    }

    #[test]
    fn personalized_global_rank_restores_and_is_account_scoped() {
        let temp = TempDir::new().unwrap();
        let mut store = AppStore::new(open_database(temp.path()).unwrap());
        let root = Keys::generate().public_key().to_hex();
        let other_root = Keys::generate().public_key().to_hex();
        let friend = Keys::generate().public_key().to_hex();
        let mut globals = [
            Keys::generate().public_key().to_hex(),
            Keys::generate().public_key().to_hex(),
        ];
        globals.sort();
        let unsupported = globals[0].clone();
        let supported = globals[1].clone();
        let cache = UserDiscoveryCache {
            owner_pubkey_hex: Some(root.clone()),
            follow_event_id: Some("verified-head".to_string()),
            follow_created_at_secs: 10,
            users: BTreeMap::from([(
                friend.clone(),
                DiscoveredUserRecord {
                    owner_pubkey_hex: friend.clone(),
                    follow_position: 0,
                    petname: None,
                },
            )]),
            social_rank_ready: true,
            social_friend_support: BTreeMap::from([(supported.clone(), 2)]),
        };
        store.replace_user_discovery(&cache).unwrap();
        store
            .upsert_profile_search_candidates(&[
                ProfileSearchCandidate {
                    owner_pubkey_hex: unsupported.clone(),
                    name: "Alex".to_string(),
                    aliases: Vec::new(),
                    nip05: None,
                    picture: None,
                    created_at_secs: 1,
                },
                ProfileSearchCandidate {
                    owner_pubkey_hex: supported.clone(),
                    name: "Alex".to_string(),
                    aliases: Vec::new(),
                    nip05: None,
                    picture: None,
                    created_at_secs: 1,
                },
            ])
            .unwrap();

        assert_eq!(store.load_user_discovery().unwrap(), cache);
        {
            let owners_for = |owner: &str| {
                let shared = store.shared();
                let conn = shared.lock().unwrap();
                search_people_candidates(&conn, "alex", &HashSet::new(), Some(owner))
                    .unwrap()
                    .into_iter()
                    .map(|row| row.owner_pubkey_hex)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                owners_for(&root),
                vec![supported.clone(), unsupported.clone()]
            );
            assert_eq!(
                owners_for(&other_root),
                vec![unsupported.clone(), supported]
            );
            let shared = store.shared();
            let conn = shared.lock().unwrap();
            assert!(
                search_people_candidates(&conn, &friend, &HashSet::new(), Some(&other_root))
                    .unwrap()
                    .is_empty()
            );
        }

        store
            .replace_user_discovery(&UserDiscoveryCache::default())
            .unwrap();
        let shared = store.shared();
        let conn = shared.lock().unwrap();
        let owners = search_people_candidates(&conn, "alex", &HashSet::new(), Some(&root))
            .unwrap()
            .into_iter()
            .map(|row| row.owner_pubkey_hex)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec![unsupported, globals[1].clone()]);
    }

    #[test]
    fn canonical_profile_fields_override_stale_index_hints() {
        let conn = people_search_connection();
        let owner = Keys::generate().public_key().to_hex();
        conn.execute(
            "INSERT INTO profile_search_candidates VALUES
                 (?1, 'Old Sirius', '[\"Old alias\"]', 'old@iris.to',
                  'https://example.com/old.jpg', 1, 1)",
            [&owner],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO owner_profiles VALUES (?1, 'Current', NULL, NULL, NULL)",
            [&owner],
        )
        .unwrap();

        assert!(
            search_people_candidates(&conn, "old sirius", &HashSet::new(), None)
                .unwrap()
                .is_empty()
        );
        let rows = search_people_candidates(&conn, "current", &HashSet::new(), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].profile_label.as_deref(), Some("Current"));
        assert!(rows[0].picture_url.is_none());
    }

    #[test]
    fn verified_search_completion_shows_people_and_revocation_hides_them() {
        let (_temp, mut core) = test_core();
        log_in(&mut core);
        let owner = Keys::generate();
        let device = Keys::generate();
        let now = unix_now().get();
        let candidate = ProfileSearchCandidate {
            owner_pubkey_hex: owner.public_key().to_hex(),
            name: "Alice".to_string(),
            aliases: Vec::new(),
            nip05: None,
            picture: None,
            created_at_secs: now,
        };
        core.app_store
            .upsert_profile_search_candidates(&[candidate.clone()])
            .unwrap();
        let search = |core: &AppCore| {
            search_people(
                &core.app_store.shared().lock().unwrap(),
                "alice",
                &HashSet::new(),
                None,
            )
            .unwrap()
        };
        assert!(search(&core).is_empty());
        for (token, devices, count) in [
            (1, vec![DeviceEntry::new(device.public_key(), now)], 1),
            (2, Vec::new(), 0),
        ] {
            let event = AppKeys::new(devices)
                .get_event_at(owner.public_key(), now + token)
                .sign_with_keys(&owner)
                .unwrap();
            core.profile_search_runtime.token = token;
            core.profile_search_runtime.query = "alice".to_string();
            core.profile_search_runtime.in_flight = true;
            let previous_revision = core.user_discovery_revision;
            core.handle_profile_search_fetch_finished(
                token,
                "alice",
                Ok(ProfileSearchFetchResult {
                    candidates: vec![candidate.clone()],
                    app_keys_events: vec![event],
                    detail: String::new(),
                }),
            );
            assert_eq!(search(&core).len(), count);
            assert!(core.user_discovery_revision > previous_revision);
        }
    }

    #[test]
    fn stale_profile_search_completion_cannot_mutate_the_cache() {
        let (_temp, mut core) = test_core();
        core.profile_search_runtime.token = 7;
        core.profile_search_runtime.query = "current".to_string();
        core.profile_search_runtime.in_flight = true;
        let candidate = ProfileSearchCandidate {
            owner_pubkey_hex: Keys::generate().public_key().to_hex(),
            name: "Stale result".to_string(),
            aliases: Vec::new(),
            nip05: None,
            picture: None,
            created_at_secs: 1,
        };

        core.handle_profile_search_fetch_finished(
            6,
            "stale",
            Ok(ProfileSearchFetchResult {
                app_keys_events: Vec::new(),
                candidates: vec![candidate],
                detail: "stale".to_string(),
            }),
        );

        let shared = core.app_store.shared();
        let count = shared
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM profile_search_candidates",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert!(core.profile_search_runtime.in_flight);
        assert_eq!(core.profile_search_runtime.query, "current");
    }

    #[test]
    fn in_flight_profile_search_keeps_only_the_latest_query() {
        let (_temp, mut core) = test_core();
        log_in(&mut core);
        core.profile_search_runtime.token = 9;
        core.profile_search_runtime.query = "alice".to_string();
        core.profile_search_runtime.in_flight = true;

        core.request_profile_search("sirius");
        assert_eq!(
            core.profile_search_runtime.pending,
            Some(PendingProfileSearch::Query("sirius".to_string()))
        );
        core.request_profile_search("alice");
        assert!(core.profile_search_runtime.pending.is_none());
        core.request_profile_search("gigi");

        core.handle_profile_search_fetch_finished(
            9,
            "alice",
            Ok(ProfileSearchFetchResult {
                app_keys_events: Vec::new(),
                candidates: Vec::new(),
                detail: "superseded".to_string(),
            }),
        );

        assert_eq!(core.profile_search_runtime.token, 10);
        assert_eq!(core.profile_search_runtime.query, "gigi");
        assert!(core.profile_search_runtime.debounce_pending);
        assert!(!core.profile_search_runtime.in_flight);
        assert!(core.profile_search_runtime.pending.is_none());
        assert!(core.user_discovery_syncing);
    }

    #[test]
    fn recent_query_cancels_a_different_debounce() {
        let (_temp, mut core) = test_core();
        log_in(&mut core);
        core.profile_search_runtime
            .recent_attempts
            .push_back(("gigi".to_string(), Instant::now()));

        core.request_profile_search("sirius");
        let sirius_token = core.profile_search_runtime.token;
        assert!(core.profile_search_runtime.debounce_pending);

        core.request_profile_search("gigi");

        assert!(core.profile_search_runtime.token > sirius_token);
        assert!(core.profile_search_runtime.query.is_empty());
        assert!(!core.profile_search_runtime.debounce_pending);
        assert!(!core.user_discovery_syncing);
        assert!(!core.state.user_discovery_syncing);
    }

    #[test]
    fn clearing_an_in_flight_query_discards_its_result() {
        let (_temp, mut core) = test_core();
        log_in(&mut core);
        core.profile_search_runtime.token = 4;
        core.profile_search_runtime.query = "sirius".to_string();
        core.profile_search_runtime.in_flight = true;
        core.user_discovery_syncing = true;
        core.request_profile_search("");
        assert_eq!(
            core.profile_search_runtime.pending,
            Some(PendingProfileSearch::Cancel)
        );

        core.handle_profile_search_fetch_finished(
            4,
            "sirius",
            Ok(ProfileSearchFetchResult {
                app_keys_events: Vec::new(),
                candidates: vec![ProfileSearchCandidate {
                    owner_pubkey_hex: Keys::generate().public_key().to_hex(),
                    name: "Discarded".to_string(),
                    aliases: Vec::new(),
                    nip05: None,
                    picture: None,
                    created_at_secs: 1,
                }],
                detail: "discarded".to_string(),
            }),
        );

        let count = core
            .app_store
            .shared()
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM profile_search_candidates",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert!(core.profile_search_runtime.query.is_empty());
        assert!(!core.profile_search_runtime.in_flight);
        assert!(!core.user_discovery_syncing);
        assert!(!core.state.user_discovery_syncing);
    }
}
