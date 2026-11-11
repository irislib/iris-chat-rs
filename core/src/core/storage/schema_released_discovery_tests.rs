use super::*;
use crate::core::{storage::AppStore, UserDiscoveryCache};
use nostr::{Event, Kind, PublicKey};
use nostr_social_graph::SocialGraph;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

const DATABASE: &[u8] =
    include_bytes!("../../../tests/fixtures/discovery-schema46/schema46.sqlite3");
const EXPECTED: &str = include_str!("../../../tests/fixtures/discovery-schema46/expected.json");
const EVENTS: &str = include_str!("../../../tests/fixtures/discovery-schema46/events.json");
const PROVENANCE: &str = include_str!("../../../tests/fixtures/discovery-schema46/provenance.json");

fn user_version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn assert_contents(cache: &UserDiscoveryCache, expected: &Value, signed_revisions: bool) {
    let root: Event = serde_json::from_value(expected["root_follow"].clone()).unwrap();
    root.verify().unwrap();
    let owner = root.pubkey.to_hex();
    assert_eq!(cache.owner_pubkey_hex.as_deref(), Some(owner.as_str()));
    assert_eq!(cache.follow_event_id, Some(root.id.to_hex()));
    assert_eq!(cache.follow_created_at_secs, root.created_at.as_secs());
    let restored: Event =
        serde_json::from_str(cache.follow_event_json.as_deref().unwrap()).unwrap();
    assert_eq!(restored, root);
    assert!(cache.social_rank_ready);
    let users = expected["users"].as_array().unwrap();
    assert_eq!(cache.users.len(), users.len());
    for row in users {
        let user = &cache.users[row[0].as_str().unwrap()];
        assert_eq!(user.follow_position as u64, row[1].as_u64().unwrap());
        assert_eq!(user.petname.as_deref(), row[2].as_str());
    }
    let support = expected["support"].as_array().unwrap();
    assert_eq!(cache.social_friend_support.len(), support.len());
    for row in support {
        assert_eq!(row[0].as_str().unwrap(), owner);
        assert_eq!(
            u64::from(cache.social_friend_support[row[1].as_str().unwrap()]),
            row[2].as_u64().unwrap()
        );
    }
    let graph = SocialGraph::from_binary(&owner, cache.social_graph.as_deref().unwrap()).unwrap();
    for head in expected["graph_heads"].as_object().unwrap().values() {
        let author = head["author"].as_str().unwrap();
        let kind = head["kind"].as_u64().unwrap() as u32;
        let (time, targets) = if kind == 3 {
            (
                graph.get_follow_list_created_at(author),
                graph.get_followed_by_user(author),
            )
        } else {
            assert_eq!(kind, 10000);
            (
                graph.get_mute_list_created_at(author),
                graph.get_muted_by_user(author),
            )
        };
        assert_eq!(time, head["created_at"].as_u64());
        let expected_targets = head["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|target| target.as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            targets.into_iter().collect::<BTreeSet<_>>(),
            expected_targets
        );
        if signed_revisions {
            let revision = graph.get_list_revision(author, kind).unwrap();
            assert_eq!(revision.id, head["id"].as_str().unwrap());
            assert_eq!(Some(revision.created_at), time);
            assert!(revision.complete);
        } else {
            // The released v3 graph persisted edges and timestamps, not IDs
            // or a proof that its projection contains the complete event.
            assert!(graph.get_list_revision(author, kind).is_none());
        }
    }
}

#[test]
fn released_schema46_discovery_migrates_reopens_and_adopts_verified_heads() {
    let provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(provenance["producer_tag"], "v2026.11.3");
    assert_eq!(
        provenance["producer_commit"],
        "d052e6d3105c54d72209799542470894c8611796"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(DATABASE)),
        provenance["fixture_sha256"].as_str().unwrap()
    );
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();
    let events: Vec<Event> = serde_json::from_str(EVENTS).unwrap();
    for event in &events {
        event.verify().unwrap();
    }
    let signed_heads = events
        .iter()
        .filter(|event| matches!(event.kind.as_u16(), 3 | 10000))
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        signed_heads.len(),
        expected["graph_heads"].as_object().unwrap().len()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("released.sqlite3");
    std::fs::write(&path, DATABASE).unwrap();
    let legacy_graph;
    {
        let mut conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), 46);
        legacy_graph = conn
            .query_row(
                "SELECT social_graph FROM user_discovery_state WHERE id=1",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap();
        ensure_schema(&mut conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        let mut store = AppStore::new(Arc::new(Mutex::new(conn)));
        let cache = store.load_user_discovery().unwrap();
        assert_contents(&cache, &expected, false);
        assert_eq!(cache.social_graph.as_deref(), Some(legacy_graph.as_slice()));
        assert!(cache.social_opinion_events.is_empty());
        store.replace_user_discovery(&cache).unwrap();
    }
    {
        let mut conn = Connection::open(&path).unwrap();
        ensure_schema(&mut conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        let mut store = AppStore::new(Arc::new(Mutex::new(conn)));
        let mut cache = store.load_user_discovery().unwrap();
        assert_contents(&cache, &expected, false);
        assert_eq!(cache.social_graph.as_deref(), Some(legacy_graph.as_slice()));
        assert!(cache.social_opinion_events.is_empty());
        let root: Event = serde_json::from_value(expected["root_follow"].clone()).unwrap();
        let authors = expected["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| PublicKey::from_hex(row[0].as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let follows = events
            .iter()
            .filter(|event| event.kind == Kind::ContactList && event.pubkey != root.pubkey)
            .cloned()
            .collect::<Vec<_>>();
        let mutes = events
            .iter()
            .filter(|event| event.kind == Kind::MuteList)
            .cloned()
            .collect();
        cache.social_graph = crate::core::user_discovery_graph::update_people_graph(
            root.pubkey,
            Some(&root),
            &authors,
            &follows,
            mutes,
            cache.social_graph.as_deref(),
            root.created_at.as_secs() + 100,
        );
        cache.social_opinion_events = signed_heads.clone();
        assert_contents(&cache, &expected, true);
        store.replace_user_discovery(&cache).unwrap();
    }
    {
        let mut conn = Connection::open(&path).unwrap();
        ensure_schema(&mut conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        let cache = AppStore::new(Arc::new(Mutex::new(conn)))
            .load_user_discovery()
            .unwrap();
        assert_contents(&cache, &expected, true);
        assert_eq!(cache.social_opinion_events, signed_heads);
    }
}
