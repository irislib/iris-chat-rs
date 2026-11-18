//! Refresh the public profile-ranking projection only, over the anonymous discovery pool.
use super::*;
use crate::core::protocol::public_discovery::DiscoveryRelays;
use nostr_social_graph::{accepts_list_revision, NostrEvent};

fn window(graph: &SocialGraph, round: u64) -> Vec<PublicKey> {
    const PEERS: usize = 31;
    let mut neighbours = Vec::new();
    for distance in [1, 2] {
        let mut layer = graph.get_users_by_follow_distance(distance);
        layer.sort_unstable();
        neighbours.extend(layer);
    }
    let mut authors = vec![PublicKey::from_hex(graph.get_root()).expect("validated public root")];
    if !neighbours.is_empty() {
        let start = (round % neighbours.len() as u64) as usize * PEERS % neighbours.len();
        authors.extend(
            neighbours
                .iter()
                .cycle()
                .skip(start)
                .take(PEERS.min(neighbours.len()))
                .filter_map(|s| PublicKey::from_hex(s).ok()),
        );
    }
    authors
}

pub(super) async fn refresh(
    discovery: Arc<DiscoveryRelays>,
    relay: &RelayUrl,
    graph: Arc<SocialGraph>,
    round: u64,
) -> Option<Vec<u8>> {
    let scope = discovery.public_snapshot_scope(relay).await.ok()?;
    let authors = window(&graph, round);
    let mut updated = None;
    for kind in [Kind::ContactList, Kind::from(10_000)] {
        let Ok(reply) = scope.snapshot_opinions(kind, &authors, &graph).await else {
            continue;
        };
        for event in
            crate::core::user_discovery::newest_verified_events_by_author(reply.events, kind)
        {
            if !authors.contains(&event.pubkey) {
                continue;
            }
            let author = event.pubkey.to_hex();
            let previous = updated.as_ref().unwrap_or(&*graph);
            let list_kind = u32::from(kind.as_u16());
            let time = if list_kind == 3 {
                previous.get_follow_list_created_at(&author)
            } else {
                previous.get_mute_list_created_at(&author)
            };
            let revision = previous.get_list_revision(&author, list_kind);
            let at = event.created_at.as_secs();
            let id = event.id.to_hex();
            if !crate::core::user_discovery::discovery_event_time_is_acceptable(
                at,
                unix_now().get(),
            ) || !accepts_list_revision(time, revision.as_ref(), at, &id)
            {
                continue;
            }
            let next = updated.get_or_insert_with(|| (*graph).clone());
            let tags = event
                .tags
                .iter()
                .filter_map(|t| {
                    let tag = t.as_slice();
                    if tag.first()?.as_str() != "p" {
                        return None;
                    }
                    Some(vec![
                        "p".into(),
                        PublicKey::from_hex(tag.get(1)?).ok()?.to_hex(),
                    ])
                })
                .collect();
            next.handle_event(
                &NostrEvent {
                    pubkey: author,
                    kind: list_kind,
                    created_at: at,
                    tags,
                    content: String::new(),
                    id,
                    sig: String::new(),
                },
                true,
                f64::INFINITY,
            );
        }
    }
    let bytes = updated?.to_binary().ok()?;
    if bytes.len() > MOBILE_MAX_BYTES {
        return None;
    }
    SocialGraph::from_mobile_binary(DEFAULT_SOCIAL_GRAPH_ROOT, &bytes, MOBILE_MIN_USERS).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::protocol::public_discovery::test_support::graph_fixture;
    use nostr::{EventBuilder, Tag, Timestamp};

    fn list(keys: &Keys, kind: Kind, at: u64, label: &str, target: PublicKey) -> Event {
        EventBuilder::new(kind, label)
            .tags([Tag::public_key(target)])
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn install(graph: &mut SocialGraph, event: &Event) {
        graph.handle_event(
            &NostrEvent {
                created_at: event.created_at.as_secs(),
                content: String::new(),
                kind: u32::from(event.kind.as_u16()),
                pubkey: event.pubkey.to_hex(),
                id: event.id.to_hex(),
                sig: String::new(),
                tags: event.tags.iter().map(|t| t.as_slice().to_vec()).collect(),
            },
            true,
            f64::INFINITY,
        );
    }

    #[tokio::test]
    async fn live_public_seed_advertises_complete_ids_and_repairs_only_missing_bodies() {
        let mut graph = SocialGraph::from_mobile_binary(
            DEFAULT_SOCIAL_GRAPH_ROOT,
            include_bytes!("../../assets/socialGraph.bin"),
            MOBILE_MIN_USERS,
        )
        .unwrap();
        let account = Keys::generate();
        let author = Keys::generate();
        let who = author.public_key().to_hex();
        let target = Keys::generate().public_key();
        graph
            .add_positive_relation(DEFAULT_SOCIAL_GRAPH_ROOT, &who)
            .unwrap();
        let mut layer = graph.get_users_by_follow_distance(1);
        layer.sort_unstable();
        let round = (layer.iter().position(|s| s == &who).unwrap() / 31) as u64;
        let authors = window(&graph, round);
        assert!(
            authors.len() <= 32
                && authors.contains(&author.public_key())
                && !authors.contains(&account.public_key())
        );
        let follow = list(&author, Kind::ContactList, 100, "complete", target);
        let mut tied = [
            list(&author, Kind::MuteList, 101, "first", target),
            list(&author, Kind::MuteList, 101, "second", target),
        ];
        tied.sort_by_key(|e| e.id);
        let [competing, mute] = tied;
        install(&mut graph, &follow);
        install(&mut graph, &mute);
        graph.mark_list_incomplete(&who, 10_000);
        let stale = list(&author, Kind::MuteList, 90, "older", target);
        let relay = graph_fixture(vec![follow.clone(), mute.clone(), stale]).await;
        let graph = Arc::new(graph);
        let pool = Arc::new(DiscoveryRelays::default());
        let keeper = pool.open(std::slice::from_ref(&relay.url)).await.unwrap();

        relay.omitted.lock().unwrap().insert(mute.id);
        let no_body = refresh(pool.clone(), &relay.url, graph.clone(), round).await;
        assert!(no_body.is_none());
        assert!(!graph.get_list_revision(&who, 10_000).unwrap().complete);
        relay.omitted.lock().unwrap().clear();
        relay.wire.lock().unwrap().clear();

        let bytes = refresh(pool.clone(), &relay.url, graph, round)
            .await
            .unwrap();
        let repaired =
            SocialGraph::from_mobile_binary(DEFAULT_SOCIAL_GRAPH_ROOT, &bytes, MOBILE_MIN_USERS)
                .unwrap();
        let head = repaired.get_list_revision(&who, 10_000).unwrap();
        assert!(head.complete && head.id == mute.id.to_hex());
        assert_eq!(
            repaired.get_list_revision(&who, 3).unwrap().id,
            follow.id.to_hex()
        );
        let frames = relay.wire.lock().unwrap().clone();
        let mut kinds = frames
            .iter()
            .filter(|f| f[0] == "NEG-OPEN")
            .map(|f| f[2]["kinds"][0].as_u64().unwrap())
            .collect::<Vec<_>>();
        kinds.sort();
        assert_eq!(kinds, [3, 10_000]);
        let ids = frames
            .iter()
            .filter(|f| f[0] == "REQ")
            .flat_map(|f| f[2]["ids"].as_array().unwrap())
            .collect::<Vec<_>>();
        assert!(ids.iter().any(|id| **id == mute.id.to_hex()));
        assert!(ids.iter().all(|id| **id != follow.id.to_hex()));
        assert!(!serde_json::to_string(&frames)
            .unwrap()
            .contains(&account.public_key().to_hex()));
        relay.wire.lock().unwrap().clear();

        let simple_relay = graph_fixture(vec![follow.clone(), mute.clone()]).await;
        assert!(refresh(
            pool.clone(),
            &simple_relay.url,
            Arc::new(repaired.clone()),
            round
        )
        .await
        .is_none());
        assert!(simple_relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .all(|f| f[0] != "REQ"));
        // Private ranking and account evidence still require actual signed bodies, even when
        // the public graph already has a complete revision with the same ID.
        simple_relay.wire.lock().unwrap().clear();
        let private = pool
            .graph_scope(std::slice::from_ref(&simple_relay.url))
            .await
            .unwrap();
        let opinion = private
            .opinions(Kind::ContactList, &[author.public_key()], &[], 2)
            .await
            .unwrap();
        assert!(opinion.complete);
        assert_eq!(
            opinion
                .events
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [follow.id]
        );
        assert!(simple_relay
            .wire
            .lock()
            .unwrap()
            .iter()
            .any(|f| f[0] == "REQ"
                && f[2]["ids"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| *id == follow.id.to_hex()))));
        drop(private);
        // At equal time a smaller signed ID wins even when our current body is complete.
        let equal_relay = graph_fixture(vec![follow, competing.clone()]).await;
        let bytes = refresh(pool, &equal_relay.url, Arc::new(repaired), round)
            .await
            .unwrap();
        let replaced =
            SocialGraph::from_mobile_binary(DEFAULT_SOCIAL_GRAPH_ROOT, &bytes, MOBILE_MIN_USERS)
                .unwrap();
        assert_eq!(
            replaced.get_list_revision(&who, 10_000).unwrap().id,
            competing.id.to_hex()
        );
        drop(keeper);
    }
}
