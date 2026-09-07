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
        {
            return;
        }
        let Some(session) = self.logged_in.as_ref() else {
            return;
        };
        let relay_urls = session
            .relay_urls
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let client = session.client.clone();
        let local_owner = Some(session.owner_pubkey.to_hex());
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
#[path = "profile_search_tests.rs"]
mod tests;
