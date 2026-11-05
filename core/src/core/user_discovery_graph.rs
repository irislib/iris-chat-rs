use super::*;
use nostr_social_graph::{NostrEvent, SocialGraph};

/// Keep signed follow/mute heads in the same graph format used by Iris Client.
/// Existing timestamps survive refresh failures, restarts, and stale relay replies.
pub(super) fn update_people_graph(
    owner: PublicKey,
    root_follow: Option<&Event>,
    opinion_authors: &[PublicKey],
    peer_follows: &[Event],
    mute_events: Vec<Event>,
    previous: Option<&[u8]>,
    now_secs: u64,
) -> Option<Vec<u8>> {
    let owner_hex = owner.to_hex();
    let mut graph = previous
        .and_then(|bytes| SocialGraph::from_binary(&owner_hex, bytes).ok())
        .or_else(|| {
            SocialGraph::from_binary(&owner_hex, include_bytes!("../../assets/socialGraph.bin"))
                .ok()
        })
        .unwrap_or_else(|| SocialGraph::new(&owner_hex));
    let allowed = opinion_authors
        .iter()
        .copied()
        .chain([owner])
        .collect::<HashSet<_>>();
    let events = root_follow
        .into_iter()
        .cloned()
        .chain(peer_follows.iter().cloned())
        .collect();
    for (kind, events) in [
        (Kind::ContactList, events),
        (Kind::from(10_000), mute_events),
    ] {
        let events = events
            .into_iter()
            .filter(|event| {
                super::user_discovery::discovery_event_time_is_acceptable(
                    event.created_at.as_secs(),
                    now_secs,
                )
            })
            .collect();
        for event in super::user_discovery::newest_verified_events_by_author(events, kind) {
            if !allowed.contains(&event.pubkey)
                || !super::user_discovery::discovery_event_time_is_acceptable(
                    event.created_at.as_secs(),
                    now_secs,
                )
            {
                continue;
            }
            // The relay input remains bounded even when signed by a followed account.
            let max_edges = if event.pubkey == owner { 5_000 } else { 1_000 };
            let mut seen = HashSet::new();
            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .filter_map(|tag| {
                    let values = tag.as_slice();
                    if values.first().map(String::as_str) != Some("p") {
                        return None;
                    }
                    let target = PublicKey::from_hex(values.get(1)?).ok()?.to_hex();
                    seen.insert(target.clone())
                        .then(|| vec!["p".to_string(), target])
                })
                .take(max_edges + 1)
                .collect();
            let incomplete = tags.len() > max_edges;
            let tags = tags.into_iter().take(max_edges).collect();
            let author = event.pubkey.to_hex();
            let list_kind = kind.as_u16().into();
            let previous_revision = graph.get_list_revision(&author, list_kind);
            graph.handle_event(
                &NostrEvent {
                    pubkey: event.pubkey.to_hex(),
                    kind: kind.as_u16().into(),
                    created_at: event.created_at.as_secs(),
                    tags,
                    content: String::new(),
                    id: event.id.to_hex(),
                    sig: String::new(),
                },
                true,
                1.0,
            );
            // A rejected stale/tie-losing event (or an already complete replay)
            // must not downgrade the retained signed head's completeness.
            let accepted_projection =
                graph
                    .get_list_revision(&author, list_kind)
                    .is_some_and(|revision| {
                        revision.id == event.id.to_hex()
                            && revision.created_at == event.created_at.as_secs()
                            && previous_revision.as_ref() != Some(&revision)
                    });
            if incomplete && accepted_projection {
                graph.mark_list_incomplete(&author, list_kind);
            }
        }
    }
    graph.to_binary().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Tag, Timestamp};

    fn event(keys: &Keys, kind: Kind, time: u64, targets: &[PublicKey]) -> Event {
        EventBuilder::new(kind, "")
            .tags(targets.iter().copied().map(Tag::public_key))
            .custom_created_at(Timestamp::from(time))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn signed_mute_heads_survive_restart_and_reject_stale_forged_and_unrequested_updates() {
        let root = Keys::generate();
        let friend = Keys::generate();
        let spam = Keys::generate();
        let outsider = Keys::generate();
        let follows = event(&root, Kind::ContactList, 1, &[friend.public_key()]);
        let muted = event(&friend, Kind::from(10_000), 10, &[spam.public_key()]);
        let first = update_people_graph(
            root.public_key(),
            Some(&follows),
            &[friend.public_key()],
            &[],
            vec![muted.clone()],
            None,
            100,
        )
        .unwrap();
        let read = |bytes: &[u8]| {
            SocialGraph::from_binary(&root.public_key().to_hex(), bytes)
                .unwrap()
                .is_overmuted(&spam.public_key().to_hex(), 1.0)
        };
        assert!(read(&first));
        let mut forged = event(&friend, Kind::from(10_000), 11, &[]);
        forged.sig = event(&outsider, Kind::from(10_000), 11, &[]).sig;
        let next = update_people_graph(
            root.public_key(),
            Some(&follows),
            &[friend.public_key()],
            &[],
            vec![
                forged,
                event(&friend, Kind::from(10_000), 9, &[]),
                event(&outsider, Kind::from(10_000), 20, &[spam.public_key()]),
            ],
            Some(&first),
            100,
        )
        .unwrap();
        assert!(read(&next));
        assert!(SocialGraph::from_binary(&root.public_key().to_hex(), &next)
            .unwrap()
            .get_muted_by_user(&outsider.public_key().to_hex())
            .is_empty());
        let unmuted = update_people_graph(
            root.public_key(),
            Some(&follows),
            &[friend.public_key()],
            &[],
            vec![event(&friend, Kind::from(10_000), 12, &[])],
            Some(&next),
            100,
        )
        .unwrap();
        assert!(!read(&unmuted));
    }

    #[test]
    fn projected_head_keeps_its_id_but_is_not_advertised_complete() {
        let root = Keys::generate();
        let friend = Keys::generate();
        let targets = (0..1001)
            .map(|_| Keys::generate().public_key())
            .collect::<Vec<_>>();
        let root_head = event(&root, Kind::ContactList, 1, &[friend.public_key()]);
        let friend_head = event(&friend, Kind::ContactList, 2, &targets);
        let bytes = update_people_graph(
            root.public_key(),
            Some(&root_head),
            &[friend.public_key()],
            &[friend_head.clone()],
            Vec::new(),
            None,
            100,
        )
        .unwrap();
        let graph = SocialGraph::from_binary(&root.public_key().to_hex(), &bytes).unwrap();
        let revision = graph
            .get_list_revision(&friend.public_key().to_hex(), 3)
            .unwrap();
        assert_eq!(revision.id, friend_head.id.to_hex());
        assert!(!revision.complete);
        assert_eq!(
            graph
                .get_followed_by_user(&friend.public_key().to_hex())
                .len(),
            1000
        );
    }

    fn graph_with_complete_head(root: PublicKey, head: &Event) -> Vec<u8> {
        let mut graph = SocialGraph::new(&root.to_hex());
        graph.handle_event(
            &NostrEvent {
                pubkey: head.pubkey.to_hex(),
                kind: head.kind.as_u16().into(),
                created_at: head.created_at.as_secs(),
                id: head.id.to_hex(),
                tags: head
                    .tags
                    .iter()
                    .map(|tag| tag.as_slice().to_vec())
                    .collect(),
                content: String::new(),
                sig: String::new(),
            },
            true,
            1.0,
        );
        graph.to_binary().unwrap()
    }

    #[test]
    fn rejected_oversized_heads_preserve_complete_follow_and_mute_revisions() {
        let root = Keys::generate();
        let friend = Keys::generate();
        let targets = (0..1001)
            .map(|_| Keys::generate().public_key())
            .collect::<Vec<_>>();
        for kind in [Kind::ContactList, Kind::from(10_000)] {
            let stale = event(&friend, kind, 19, &targets);
            let current = event(&friend, kind, 20, &targets[..1]);
            let a = event(&friend, kind, 20, &targets);
            let b = EventBuilder::new(kind, "tie")
                .tags(targets.iter().copied().map(Tag::public_key))
                .custom_created_at(Timestamp::from(20))
                .sign_with_keys(&friend)
                .unwrap();
            let (winner, loser) = if a.id < b.id { (a, b) } else { (b, a) };
            for (saved, rejected) in [(&current, &stale), (&winner, &loser), (&winner, &winner)] {
                let before = graph_with_complete_head(root.public_key(), saved);
                let (follows, mutes) = if kind == Kind::ContactList {
                    (vec![rejected.clone()], vec![])
                } else {
                    (vec![], vec![rejected.clone()])
                };
                let after = update_people_graph(
                    root.public_key(),
                    None,
                    &[friend.public_key()],
                    &follows,
                    mutes,
                    Some(&before),
                    100,
                )
                .unwrap();
                let graph = SocialGraph::from_binary(&root.public_key().to_hex(), &after).unwrap();
                let revision = graph
                    .get_list_revision(&friend.public_key().to_hex(), kind.as_u16().into())
                    .unwrap();
                assert_eq!(revision.id, saved.id.to_hex());
                assert!(
                    revision.complete,
                    "rejected oversized input must not invalidate a complete head"
                );
                let retained = if kind == Kind::ContactList {
                    graph.get_followed_by_user(&friend.public_key().to_hex())
                } else {
                    graph.get_muted_by_user(&friend.public_key().to_hex())
                };
                assert_eq!(retained.len(), saved.tags.len());
            }
        }
    }
    #[test]
    fn future_mute_does_not_hide_valid_head_in_the_same_reply() {
        let root = Keys::generate();
        let target = Keys::generate().public_key();
        let valid = event(&root, Kind::MuteList, 10, &[target]);
        let future = event(&root, Kind::MuteList, 10_000, &[]);
        let bytes = update_people_graph(
            root.public_key(),
            None,
            &[],
            &[],
            vec![valid.clone(), future],
            None,
            100,
        )
        .unwrap();
        let graph = SocialGraph::from_binary(&root.public_key().to_hex(), &bytes).unwrap();
        assert_eq!(
            graph
                .get_list_revision(&root.public_key().to_hex(), 10000)
                .unwrap()
                .id,
            valid.id.to_hex()
        );
        assert!(graph
            .get_muted_by_user(&root.public_key().to_hex())
            .contains(&target.to_hex()));
    }
}
