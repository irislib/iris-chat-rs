use super::user_discovery::{discovery_event_time_is_acceptable, newest_verified_events_by_author};
use super::*;
use nostr_social_graph::SocialGraph;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

pub(super) const MAX_SOCIAL_OPINION_AUTHORS: usize = 256;
const MAX_PEER_SOCIAL_EDGES: usize = 1_000;

pub(super) fn rank_followed_owners(
    local_owner: PublicKey,
    root_follow: &Event,
    followed_owners: &[PublicKey],
    opinion_authors: &[PublicKey],
    peer_follow_events: Vec<Event>,
    now_secs: u64,
) -> Option<Vec<PublicKey>> {
    let root_hex = local_owner.to_hex();
    let mut graph = SocialGraph::new(&root_hex);
    verify_event(root_follow, local_owner, Kind::ContactList, now_secs)?;
    let followed_set = followed_owners.iter().copied().collect::<HashSet<_>>();
    if !opinion_authors
        .iter()
        .all(|author| followed_set.contains(author))
    {
        return None;
    }
    for owner in followed_owners {
        graph
            .add_positive_relation(&root_hex, &owner.to_hex(), root_follow.created_at.as_secs())
            .ok()
            .filter(|added| *added)?;
    }
    let opinion_set = opinion_authors.iter().copied().collect::<HashSet<_>>();
    let mut peer_heads = newest_verified_events_by_author(peer_follow_events, Kind::ContactList)
        .into_iter()
        .filter(|event| opinion_set.contains(&event.pubkey))
        .map(|event| (event.pubkey, event))
        .collect::<BTreeMap<_, _>>();
    for author in opinion_authors {
        if let Some(event) = peer_heads.remove(author) {
            verify_event(&event, *author, Kind::ContactList, now_secs)?;
            let author_hex = author.to_hex();
            for target in bounded_peer_targets(&event, &followed_set)? {
                graph
                    .add_positive_relation(
                        &author_hex,
                        &target.to_hex(),
                        event.created_at.as_secs(),
                    )
                    .ok()
                    .filter(|added| *added)?;
            }
        }
    }
    let opinion_hex = opinion_authors
        .iter()
        .map(PublicKey::to_hex)
        .collect::<HashSet<_>>();
    let mut ranked = followed_owners
        .iter()
        .enumerate()
        .map(|(root_position, owner)| {
            let owner_hex = owner.to_hex();
            let support = graph
                .get_followers_by_user(&owner_hex)
                .iter()
                .filter(|follower| opinion_hex.contains(*follower))
                .count();
            ((Reverse(support), root_position, owner_hex), *owner)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0));
    Some(ranked.into_iter().map(|(_, owner)| owner).collect())
}

pub(super) fn apply_follow_order(
    users: &mut BTreeMap<String, DiscoveredUserRecord>,
    root_order: &[PublicKey],
    social_order: Option<&[PublicKey]>,
) {
    let root_set = root_order.iter().copied().collect::<HashSet<_>>();
    let order = match social_order {
        Some(order)
            if order.len() == root_order.len()
                && order.iter().copied().collect::<HashSet<_>>() == root_set =>
        {
            order
        }
        _ => root_order,
    };
    for (position, owner) in order.iter().enumerate() {
        if let Some(user) = users.get_mut(&owner.to_hex()) {
            user.follow_position = position as u32;
        }
    }
}

fn bounded_peer_targets(event: &Event, candidates: &HashSet<PublicKey>) -> Option<Vec<PublicKey>> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        let Some(owner) = (values.first().map(String::as_str) == Some("p"))
            .then(|| values.get(1))
            .flatten()
            .and_then(|owner| owner.parse::<PublicKey>().ok())
        else {
            continue;
        };
        if owner == event.pubkey || !candidates.contains(&owner) || !seen.insert(owner) {
            continue;
        }
        if seen.len() > MAX_PEER_SOCIAL_EDGES {
            return None;
        }
        targets.push(owner);
    }
    Some(targets)
}

fn verify_event(
    event: &Event,
    expected_author: PublicKey,
    expected_kind: Kind,
    now_secs: u64,
) -> Option<()> {
    if event.pubkey != expected_author
        || event.kind != expected_kind
        || !discovery_event_time_is_acceptable(event.created_at.as_secs(), now_secs)
        || event.verify().is_err()
    {
        return None;
    }
    Some(())
}

#[cfg(test)]
#[path = "user_discovery_social_tests.rs"]
mod tests;
