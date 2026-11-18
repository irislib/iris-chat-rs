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
    let mut changed = false;
    for kind in [Kind::ContactList, Kind::from(10_000)] {
        let Ok(reply) = scope.snapshot_opinions(kind, &authors, &graph).await else {
            continue;
        };
        let now = unix_now().get();
        let eligible = reply
            .events
            .into_iter()
            .filter(|e| {
                authors.contains(&e.pubkey)
                    && crate::core::user_discovery::discovery_event_time_is_acceptable(
                        e.created_at.as_secs(),
                        now,
                    )
            })
            .collect();
        for event in crate::core::user_discovery::newest_verified_events_by_author(eligible, kind) {
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
            if !accepts_list_revision(time, revision.as_ref(), at, &id) {
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
            let list = NostrEvent {
                pubkey: author,
                kind: list_kind,
                created_at: at,
                tags,
                content: String::new(),
                id,
                sig: String::new(),
            };
            next.handle_event(&list, true, f64::INFINITY);
            changed |= next.get_list_revision(&list.pubkey, list_kind) != revision;
        }
    }
    if !changed {
        return None;
    }
    let bytes = updated?.to_binary().ok()?;
    if bytes.len() > MOBILE_MAX_BYTES {
        return None;
    }
    SocialGraph::from_mobile_binary(DEFAULT_SOCIAL_GRAPH_ROOT, &bytes, MOBILE_MIN_USERS).ok()?;
    Some(bytes)
}

#[cfg(test)]
#[path = "profile_search_seed_sync_tests.rs"]
mod tests;
