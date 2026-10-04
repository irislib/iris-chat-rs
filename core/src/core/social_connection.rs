use super::*;
use crate::state::{SocialBadge, SocialConnectionSnapshot};
use nostr_social_graph::SocialGraph;

impl AppCore {
    pub(super) fn refresh_social_graph(&mut self) {
        self.social_graph = self
            .user_discovery
            .owner_pubkey_hex
            .as_deref()
            .zip(self.user_discovery.social_graph.as_deref())
            .and_then(|(owner, bytes)| SocialGraph::from_binary(owner, bytes).ok());
    }

    pub(super) fn social_connection(&self, target: &str) -> Option<SocialConnectionSnapshot> {
        let owner = self.logged_in.as_ref()?.owner_pubkey.to_hex();
        social_connection(
            Some(&owner),
            target,
            self.social_graph.as_ref(),
            self.owner_profiles
                .get(target)
                .is_some_and(|profile| profile.contact_memory.favorite),
        )
    }
}

pub(super) fn social_connection(
    owner: Option<&str>,
    target: &str,
    graph: Option<&SocialGraph>,
    is_favorite: bool,
) -> Option<SocialConnectionSnapshot> {
    let owner = owner?;
    // Group IDs and device IDs must never acquire a person's social badge.
    PublicKey::from_hex(target).ok()?;
    if owner == target {
        return Some(SocialConnectionSnapshot {
            is_favorite: false,
            badge: Some(SocialBadge::Following),
            follow_distance: Some(0),
            followed_by_friends: 0,
            description: "You".into(),
        });
    }
    let Some(graph) = graph.filter(|graph| graph.get_root() == owner) else {
        return is_favorite.then(|| SocialConnectionSnapshot {
            badge: None,
            follow_distance: None,
            followed_by_friends: 0,
            description: "Favorite · Only you".into(),
            is_favorite: true,
        });
    };
    let distance = graph.get_follow_distance(target);
    let friends = graph
        .get_followers_by_user(target)
        .iter()
        .filter(|friend| graph.get_follow_distance(friend) == 1)
        .count() as u32;
    let muted = graph
        .get_muted_by_user(owner)
        .iter()
        .any(|user| user == target);
    let overmuted = graph.is_overmuted(target, 1.0);
    let badge = if muted {
        Some(SocialBadge::Muted)
    } else if overmuted {
        Some(SocialBadge::Warning)
    } else {
        match distance {
            1 => Some(SocialBadge::Following),
            2 if friends > 10 => Some(SocialBadge::Trusted),
            2 => Some(SocialBadge::Friend),
            _ => None,
        }
    };
    let description = if muted {
        "Muted".into()
    } else if overmuted {
        "More mutes than follows in your network".into()
    } else if distance == 1 {
        "Followed by you".into()
    } else if friends == 1 {
        "Followed by 1 friend".into()
    } else if friends > 1 {
        format!("Followed by {friends} friends")
    } else if distance == 3 {
        "Followed by friends of friends".into()
    } else {
        "Not followed by anyone you follow".into()
    };
    Some(SocialConnectionSnapshot {
        is_favorite,
        badge,
        follow_distance: (distance < 1_000).then_some(distance),
        followed_by_friends: friends,
        description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_social_graph::NostrEvent;

    fn key(n: u64) -> String {
        Keys::parse(&format!("{n:064x}"))
            .unwrap()
            .public_key()
            .to_hex()
    }
    fn event(graph: &mut SocialGraph, author: u64, targets: &[u64], kind: u32, time: u64) {
        graph.handle_event(
            &NostrEvent {
                pubkey: key(author),
                kind,
                created_at: time,
                tags: targets.iter().map(|n| vec!["p".into(), key(*n)]).collect(),
                content: String::new(),
                id: String::new(),
                sig: String::new(),
            },
            true,
            1.0,
        );
    }

    #[test]
    fn connections_match_iris_client_and_refresh_after_follow_changes() {
        let mut graph = SocialGraph::new(&key(1));
        event(&mut graph, 1, &(2..=12).collect::<Vec<_>>(), 3, 1);
        for n in 2..=12 {
            event(&mut graph, n, &[20], 3, 1);
        }
        event(&mut graph, 2, &[20, 21], 3, 2);
        event(&mut graph, 21, &[30], 3, 1);
        let connection =
            |n| social_connection(Some(&key(1)), &key(n), Some(&graph), false).unwrap();
        assert_eq!(connection(1).description, "You");
        assert_eq!(connection(2).badge, Some(SocialBadge::Following));
        assert_eq!(connection(20).badge, Some(SocialBadge::Trusted));
        assert_eq!(connection(20).followed_by_friends, 11);
        assert_eq!(connection(21).badge, Some(SocialBadge::Friend));
        assert_eq!(connection(21).description, "Followed by 1 friend");
        assert_eq!(connection(30).description, "Followed by friends of friends");
        assert_eq!(connection(30).badge, None);
        assert_eq!(connection(40).follow_distance, None);
        event(&mut graph, 1, &[20], 10_000, 3);
        assert_eq!(
            social_connection(Some(&key(1)), &key(20), Some(&graph), false)
                .unwrap()
                .badge,
            Some(SocialBadge::Muted)
        );
        event(&mut graph, 1, &[2], 3, 4);
        assert_eq!(
            social_connection(Some(&key(1)), &key(20), Some(&graph), false)
                .unwrap()
                .followed_by_friends,
            1
        );
    }

    #[test]
    fn overmuted_warning_respects_closest_opinions_and_personal_choices() {
        let mut graph = SocialGraph::new(&key(1));
        event(&mut graph, 1, &[2, 3, 4], 3, 1);
        event(&mut graph, 2, &[20], 3, 1);
        event(&mut graph, 3, &[20], 10_000, 1);
        event(&mut graph, 4, &[20], 10_000, 1);
        let warning = social_connection(Some(&key(1)), &key(20), Some(&graph), false).unwrap();
        assert_eq!(warning.badge, Some(SocialBadge::Warning));
        assert_eq!(
            warning.description,
            "More mutes than follows in your network"
        );
        event(&mut graph, 1, &[2, 3, 4, 20], 3, 2);
        assert_eq!(
            social_connection(Some(&key(1)), &key(20), Some(&graph), false)
                .unwrap()
                .badge,
            Some(SocialBadge::Following)
        );
        event(&mut graph, 1, &[20], 10_000, 2);
        assert_eq!(
            social_connection(Some(&key(1)), &key(20), Some(&graph), false)
                .unwrap()
                .badge,
            Some(SocialBadge::Muted)
        );
    }

    #[test]
    fn missing_or_other_accounts_graph_never_claims_a_connection() {
        let graph = SocialGraph::new(&key(1));
        assert!(social_connection(None, &key(1), Some(&graph), false).is_none());
        assert!(social_connection(Some(&key(2)), &key(1), Some(&graph), false).is_none());
        assert!(social_connection(Some(&key(1)), &key(2), None, false).is_none());
        assert!(social_connection(Some(&key(1)), "group:abc", Some(&graph), false).is_none());
        assert_eq!(
            social_connection(Some(&key(1)), &key(1), None, false)
                .unwrap()
                .description,
            "You"
        );
    }
    #[test]
    fn private_favorite_is_independent_of_public_relationships_and_available_offline() {
        let owner = key(1);
        let peer = key(2);
        let mut graph = SocialGraph::new(&owner);
        event(&mut graph, 1, &[2], 3, 1);
        let favorite = social_connection(Some(&owner), &peer, Some(&graph), true).unwrap();
        assert!(favorite.is_favorite);
        assert_eq!(favorite.badge, Some(SocialBadge::Following));
        event(&mut graph, 1, &[2], 10_000, 2);
        let muted = social_connection(Some(&owner), &peer, Some(&graph), true).unwrap();
        assert!(muted.is_favorite);
        assert_eq!(muted.badge, Some(SocialBadge::Muted));
        let offline = social_connection(Some(&owner), &peer, None, true).unwrap();
        assert!(offline.is_favorite);
        assert_eq!(offline.badge, None);
        assert_eq!(offline.follow_distance, None);
        assert!(social_connection(Some(&owner), &peer, None, false).is_none());
        assert!(social_connection(None, &peer, None, true).is_none());
        assert!(social_connection(Some(&owner), "group:test", None, true).is_none());
        assert!(
            !social_connection(Some(&owner), &owner, None, true)
                .unwrap()
                .is_favorite
        );
    }
}
