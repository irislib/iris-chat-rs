// The shared public graph supplies profile ranking only. The signed personal
// lists used for private message admission have separate storage and lifetime.
use super::*;
use nostr_social_graph::{public_snapshot, MOBILE_MAX_BYTES, MOBILE_MIN_USERS};
use std::sync::Mutex;

#[path = "profile_search_seed_sync.rs"]
mod sync;

type CachedSeed = (String, Arc<SocialGraph>);
static PARSED: OnceLock<Mutex<Option<CachedSeed>>> = OnceLock::new();

fn stored_digest(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='public_discovery_seed')",
        [], |row| row.get(0),
    )?;
    if !table {
        return Ok(None);
    }
    conn.query_row(
        "SELECT sha256 FROM public_discovery_seed WHERE id=1",
        [],
        |row| row.get(0),
    )
    .optional()
}

pub(super) fn load(conn: &Connection) -> Option<Arc<SocialGraph>> {
    let digest = stored_digest(conn).ok()??;
    let mut cached = PARSED.get_or_init(Default::default).lock().ok()?;
    if let Some((known, graph)) = cached.as_ref() {
        if *known == digest {
            return Some(graph.clone());
        }
    }
    let bytes: Vec<u8> = conn.query_row(
        "SELECT bytes FROM public_discovery_seed WHERE id=1 AND sha256=?1 AND length(bytes) BETWEEN 1 AND ?2",
        rusqlite::params![digest, MOBILE_MAX_BYTES], |r| r.get(0),
    ).ok()?;
    if hashtree_core::to_hex(&hashtree_core::sha256(&bytes)) != digest {
        return None;
    }
    let graph = Arc::new(
        SocialGraph::from_mobile_binary(DEFAULT_SOCIAL_GRAPH_ROOT, &bytes, MOBILE_MIN_USERS)
            .ok()?,
    );
    *cached = Some((digest, graph.clone()));
    Some(graph)
}

fn insert(conn: &Connection, bytes: &[u8]) -> rusqlite::Result<bool> {
    // This optional local cache can also run on existing databases. An unreadable
    // row is kept intact. A different account can reuse the identical public file.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS public_discovery_seed (
        id INTEGER PRIMARY KEY CHECK(id=1), sha256 TEXT NOT NULL, bytes BLOB NOT NULL
    )",
    )?;
    let hash = hashtree_core::to_hex(&hashtree_core::sha256(bytes));
    Ok(conn.execute(
        "INSERT OR IGNORE INTO public_discovery_seed VALUES(1,?1,?2)",
        rusqlite::params![hash, bytes],
    )? == 1)
}

fn replace_public_projection(
    conn: &Connection,
    previous: &str,
    bytes: &[u8],
) -> rusqlite::Result<bool> {
    let digest = hashtree_core::to_hex(&hashtree_core::sha256(bytes));
    if digest == previous {
        return Ok(false);
    }
    Ok(conn.execute(
        "UPDATE public_discovery_seed SET sha256=?1, bytes=?2 WHERE id=1 AND sha256=?3",
        rusqlite::params![digest, bytes, previous],
    )? == 1)
}

fn default_network(relays: &[RelayUrl]) -> bool {
    use std::collections::BTreeSet;
    let actual: BTreeSet<_> = relays
        .iter()
        .map(RelayUrl::as_str)
        .map(|url| url.trim_end_matches('/'))
        .collect();
    let defaults = compiled_default_relays();
    actual
        == defaults
            .iter()
            .map(|url| url.trim_end_matches('/'))
            .collect()
}

impl AppCore {
    pub(in crate::core) fn start_public_discovery_seed(
        &mut self,
        relays: &[RelayUrl],
        owner: PublicKey,
    ) {
        if self.user_discovery_runtime.public_seed_abort.is_some() || !default_network(relays) {
            return;
        }
        let connection = self.app_store.shared();
        let Some(stored) = connection
            .lock()
            .ok()
            .and_then(|db| stored_digest(&db).ok())
        else {
            return;
        };
        let round = unix_now().get() / 300;
        if stored.is_some() {
            if self.user_discovery_runtime.public_seed_round == Some(round) {
                return;
            }
            self.user_discovery_runtime.public_seed_round = Some(round);
        } else if self.user_discovery.social_graph.is_some() {
            return;
        }
        let generation = self.user_discovery_runtime.public_seed_generation;
        let sender = self.core_sender.clone();
        let discovery = self.discovery_relays.clone();
        let mut relays = relays.to_vec();
        relays.sort();
        relays.dedup();
        let task = self.runtime.spawn(async move {
            let bytes = if let Some(digest) = &stored {
                let cached = connection.lock().ok().and_then(|db| {
                    (stored_digest(&db).ok().flatten().as_ref() == Some(digest))
                        .then(|| load(&db))
                        .flatten()
                });
                match cached {
                    Some(graph) => {
                        let relay_count = relays.len() as u64;
                        let relay = &relays[(round % relay_count) as usize];
                        sync::refresh(discovery, relay, graph, round / relay_count).await
                    }
                    None => None,
                }
            } else {
                public_snapshot::fetch(DEFAULT_SOCIAL_GRAPH_ROOT, MOBILE_MIN_USERS)
                    .await
                    .ok()
                    .map(|seed| seed.bytes)
            };
            let _ = sender.send(CoreMsg::Internal(Box::new(
                InternalEvent::PublicDiscoverySeedFinished {
                    generation,
                    owner: owner.to_hex(),
                    previous_digest: stored,
                    bytes,
                },
            )));
        });
        self.user_discovery_runtime.public_seed_abort = Some(task.abort_handle());
    }

    pub(in crate::core) fn cancel_public_discovery_seed(&mut self) {
        if let Some(task) = self.user_discovery_runtime.public_seed_abort.take() {
            task.abort();
        }
        self.user_discovery_runtime.public_seed_generation = self
            .user_discovery_runtime
            .public_seed_generation
            .wrapping_add(1);
    }

    pub(in crate::core) fn finish_public_discovery_seed(
        &mut self,
        generation: u64,
        owner: &str,
        previous_digest: Option<String>,
        bytes: Option<Vec<u8>>,
    ) {
        if generation != self.user_discovery_runtime.public_seed_generation {
            return;
        }
        self.user_discovery_runtime.public_seed_abort = None;
        if !self
            .logged_in
            .as_ref()
            .is_some_and(|s| s.owner_pubkey.to_hex() == owner && default_network(&s.relay_urls))
        {
            return;
        }
        let Some(bytes) = bytes else { return };
        let connection = self.app_store.shared();
        let changed = connection.lock().ok().is_some_and(|db| {
            match &previous_digest {
                Some(digest) => replace_public_projection(&db, digest, &bytes),
                None => insert(&db, &bytes),
            }
            .unwrap_or(false)
        });
        if changed {
            self.bump_user_discovery_revision();
            self.rebuild_state();
            self.emit_state();
            if previous_digest.is_none() {
                if let Some((relays, owner)) = self
                    .logged_in
                    .as_ref()
                    .map(|s| (s.relay_urls.clone(), s.owner_pubkey))
                {
                    self.start_public_discovery_seed(&relays, owner);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EMBEDDED: &[u8] = include_bytes!("../../assets/socialGraph.bin");

    #[test]
    fn qualified_seed_is_a_separate_immutable_profile_search_cache() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE user_discovery_state(id INTEGER PRIMARY KEY, owner_pubkey_hex TEXT, social_graph BLOB);
            INSERT INTO user_discovery_state VALUES(1,'personal',x'010203');").unwrap();
        assert!(stored_digest(&db).unwrap().is_none());
        assert!(insert(&db, EMBEDDED).unwrap());
        assert!(!insert(&db, b"replacement").unwrap());
        let graph = load(&db).unwrap();
        assert_eq!(graph.get_root(), DEFAULT_SOCIAL_GRAPH_ROOT);
        let retained: Vec<u8> = db
            .query_row(
                "SELECT social_graph FROM user_discovery_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retained, [1, 2, 3]);
    }

    #[test]
    fn public_deltas_compare_the_seed_digest_and_leave_personal_rows_untouched() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE user_discovery_state(id INTEGER, social_graph BLOB);
            INSERT INTO user_discovery_state VALUES(1,x'010203');",
        )
        .unwrap();
        insert(&db, EMBEDDED).unwrap();
        let baseline = stored_digest(&db).unwrap().unwrap();
        let mut graph = (*load(&db).unwrap()).clone();
        graph
            .add_positive_relation(
                DEFAULT_SOCIAL_GRAPH_ROOT,
                &Keys::generate().public_key().to_hex(),
                100,
            )
            .unwrap();
        let changed = graph.to_binary().unwrap();
        assert!(!replace_public_projection(&db, "unrelated", &changed).unwrap());
        assert!(replace_public_projection(&db, &baseline, &changed).unwrap());
        assert!(!replace_public_projection(&db, &baseline, EMBEDDED).unwrap());
        assert!(load(&db).is_some());
        let bytes: Vec<u8> = db
            .query_row("SELECT social_graph FROM user_discovery_state", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(bytes, [1, 2, 3]);
    }

    #[test]
    fn unreadable_saved_seed_uses_the_embedded_fallback_and_survives() {
        let db = Connection::open_in_memory().unwrap();
        assert!(insert(&db, b"retained unreadable data").unwrap());
        assert!(load(&db).is_none());
        assert!(!insert(&db, EMBEDDED).unwrap());
        assert!(stored_digest(&db).unwrap().is_some());
    }

    #[test]
    fn custom_relays_and_empty_network_stay_private() {
        assert!(!default_network(&[]));
        assert!(!default_network(&[
            RelayUrl::parse("wss://example.org").unwrap()
        ]));
        let defaults: Vec<_> = compiled_default_relays()
            .iter()
            .map(|u| RelayUrl::parse(u).unwrap())
            .collect();
        assert!(default_network(&defaults));
    }
}
