use super::user_discovery::{discovery_event_time_is_acceptable, newest_verified_events_by_author};
use super::*;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

pub(super) const MAX_SOCIAL_OPINION_AUTHORS: usize = 256;
const MAX_PEER_SOCIAL_EDGES: usize = 1_000;
// Root-follow order, then each signed event's tag order, decides which targets
// survive this persisted projection cap.
const MAX_SOCIAL_RANK_TARGETS: usize = 20_000;

pub(super) struct VerifiedSocialRanking {
    pub(super) followed_owners: Vec<PublicKey>,
    pub(super) friend_support: BTreeMap<String, u16>,
}

pub(super) fn build_verified_social_ranking(
    local_owner: PublicKey,
    root_follow: &Event,
    followed_owners: &[PublicKey],
    opinion_authors: &[PublicKey],
    peer_follow_events: Vec<Event>,
    now_secs: u64,
) -> Option<VerifiedSocialRanking> {
    verify_event(root_follow, local_owner, Kind::ContactList, now_secs)?;
    let followed_set = followed_owners.iter().copied().collect::<HashSet<_>>();
    if !opinion_authors
        .iter()
        .all(|author| followed_set.contains(author))
    {
        return None;
    }
    let opinion_set = opinion_authors.iter().copied().collect::<HashSet<_>>();
    let mut peer_heads = newest_verified_events_by_author(peer_follow_events, Kind::ContactList)
        .into_iter()
        .filter(|event| opinion_set.contains(&event.pubkey))
        .map(|event| (event.pubkey, event))
        .collect::<BTreeMap<_, _>>();
    let mut friend_support = BTreeMap::<String, u16>::new();
    for author in opinion_authors {
        if let Some(event) = peer_heads.remove(author) {
            verify_event(&event, *author, Kind::ContactList, now_secs)?;
            let direct_targets = bounded_peer_targets(&event, &followed_set)?;
            let mut targets = bounded_global_peer_targets(&event);
            let mut seen_targets = targets.iter().copied().collect::<HashSet<_>>();
            targets.extend(
                direct_targets
                    .into_iter()
                    .filter(|target| seen_targets.insert(*target)),
            );
            for target in targets {
                let target_hex = target.to_hex();
                if let Some(support) = friend_support.get_mut(&target_hex) {
                    *support = support.saturating_add(1);
                } else if friend_support.len() < MAX_SOCIAL_RANK_TARGETS
                    || followed_set.contains(&target)
                {
                    friend_support.insert(target_hex, 1);
                }
            }
        }
    }
    let mut ranked = followed_owners
        .iter()
        .enumerate()
        .map(|(root_position, owner)| {
            let owner_hex = owner.to_hex();
            let support = friend_support.get(&owner_hex).copied().unwrap_or(0);
            ((Reverse(support), root_position, owner_hex), *owner)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0));
    Some(VerifiedSocialRanking {
        followed_owners: ranked.into_iter().map(|(_, owner)| owner).collect(),
        friend_support,
    })
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

fn bounded_global_peer_targets(event: &Event) -> Vec<PublicKey> {
    let mut seen = HashSet::new();
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("p"))
                .then(|| values.get(1))
                .flatten()
                .and_then(|owner| owner.parse::<PublicKey>().ok())
                .filter(|owner| *owner != event.pubkey && seen.insert(*owner))
        })
        .take(MAX_PEER_SOCIAL_EDGES)
        .collect()
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
