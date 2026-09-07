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
        .upsert_profile_search_candidates(std::slice::from_ref(&candidate))
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
