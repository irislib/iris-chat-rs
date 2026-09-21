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
            let tags = event
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
                .take(max_edges)
                .collect();
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
}
