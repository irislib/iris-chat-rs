use super::*;
use nostr::{EventBuilder, Keys, Tag, Timestamp};
use tempfile::TempDir;

fn follow_event(keys: &Keys, created_at: u64, followed: &[PublicKey]) -> Event {
    let tags = followed
        .iter()
        .map(|owner| Tag::parse(vec!["p".to_string(), owner.to_hex()]).unwrap())
        .collect::<Vec<_>>();
    follow_event_with_tags(keys, created_at, tags)
}

fn follow_event_with_tags(keys: &Keys, created_at: u64, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::ContactList, "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn record(owner: PublicKey, position: u32) -> DiscoveredUserRecord {
    DiscoveredUserRecord {
        owner_pubkey_hex: owner.to_hex(),
        follow_position: position,
        petname: None,
    }
}

fn positions(users: &BTreeMap<String, DiscoveredUserRecord>, owners: &[PublicKey]) -> Vec<u32> {
    owners
        .iter()
        .map(|owner| users[&owner.to_hex()].follow_position)
        .collect()
}

#[test]
fn direct_follow_opinions_change_order_without_unknown_authors() {
    let root = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let carol = Keys::generate();
    let owners = [alice.public_key(), bob.public_key(), carol.public_key()];
    let root_event = follow_event(&root, 100, &owners);

    let obsolete = follow_event(&alice, 150, &[bob.public_key()]);
    let mut newest_tags = (0..MAX_PEER_SOCIAL_EDGES + 1)
        .map(|_| Tag::public_key(Keys::generate().public_key()))
        .collect::<Vec<_>>();
    newest_tags.push(Tag::public_key(carol.public_key()));
    let newest = follow_event_with_tags(&alice, 200, newest_tags);
    let mut invalid_newest = follow_event(&alice, 300, &[bob.public_key()]);
    invalid_newest.content.push_str("tampered");
    let bob_opinion = follow_event(&bob, 200, &[carol.public_key()]);

    let ranked = rank_followed_owners(
        root.public_key(),
        &root_event,
        &owners,
        &owners,
        vec![obsolete, newest, invalid_newest, bob_opinion],
        1_000,
    )
    .unwrap();

    assert_eq!(
        ranked,
        [carol.public_key(), alice.public_key(), bob.public_key()]
    );
}

#[test]
fn future_dated_peer_head_degrades_social_order() {
    let root = Keys::generate();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let owners = [alice.public_key(), bob.public_key()];
    let root_event = follow_event(&root, 100, &owners);
    let future = follow_event(&alice, 1_601, &[bob.public_key()]);

    assert!(rank_followed_owners(
        root.public_key(),
        &root_event,
        &owners,
        &owners,
        vec![future],
        1_000,
    )
    .is_none());
}

#[test]
fn verified_empty_root_is_a_successful_empty_rank() {
    let root = Keys::generate();
    let root_event = follow_event(&root, 100, &[]);

    assert_eq!(
        rank_followed_owners(root.public_key(), &root_event, &[], &[], Vec::new(), 1_000),
        Some(Vec::new())
    );
}

#[test]
fn peer_edge_bound_counts_unique_effective_targets() {
    let author = Keys::generate();
    let targets = (0..MAX_PEER_SOCIAL_EDGES)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    let mut candidates = targets.iter().copied().collect::<HashSet<_>>();
    let mut tags = vec![
        Tag::parse(vec!["p".to_string(), author.public_key().to_hex()]).unwrap(),
        Tag::parse(["subject", "ignored"]).unwrap(),
    ];
    for target in &targets {
        let tag = Tag::parse(vec!["p".to_string(), target.to_hex()]).unwrap();
        tags.push(tag.clone());
        tags.push(tag);
    }
    let at_limit = follow_event_with_tags(&author, 100, tags.clone());
    assert_eq!(
        bounded_peer_targets(&at_limit, &candidates).unwrap().len(),
        MAX_PEER_SOCIAL_EDGES
    );

    let extra = Keys::generate().public_key();
    candidates.insert(extra);
    tags.push(Tag::parse(vec!["p".to_string(), extra.to_hex()]).unwrap());
    let over_limit = follow_event_with_tags(&author, 101, tags);
    assert!(bounded_peer_targets(&over_limit, &candidates).is_none());
}

#[test]
fn malformed_social_orders_restore_every_fresh_root_position() {
    let owners = (0..3)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    let foreign = Keys::generate().public_key();
    let mut users = owners
        .iter()
        .enumerate()
        .map(|(position, owner)| (owner.to_hex(), record(*owner, 90 + position as u32)))
        .collect::<BTreeMap<_, _>>();
    let valid = [owners[2], owners[0], owners[1]];
    apply_follow_order(&mut users, &owners, Some(&valid));
    assert_eq!(positions(&users, &owners), [1, 2, 0]);

    let malformed = [
        vec![owners[2], owners[0]],
        vec![owners[2], owners[2], owners[1]],
        vec![owners[2], owners[0], foreign],
    ];
    for order in malformed {
        for user in users.values_mut() {
            user.follow_position = 99;
        }
        apply_follow_order(&mut users, &owners, Some(&order));
        assert_eq!(positions(&users, &owners), [0, 1, 2]);
    }
}

#[test]
fn derived_order_persists_offline_and_degraded_refresh_restores_root() {
    let temp = TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(temp.path()).unwrap());
    let owners = (0..3)
        .map(|_| Keys::generate().public_key())
        .collect::<Vec<_>>();
    let mut cache = UserDiscoveryCache {
        follow_event_id: Some("root".to_string()),
        follow_created_at_secs: 100,
        users: owners
            .iter()
            .enumerate()
            .map(|(position, owner)| (owner.to_hex(), record(*owner, position as u32)))
            .collect(),
    };
    let social = [owners[2], owners[0], owners[1]];
    apply_follow_order(&mut cache.users, &owners, Some(&social));
    store.replace_user_discovery(&cache).unwrap();
    let mut restored = store.load_user_discovery().unwrap();
    assert_eq!(positions(&restored.users, &owners), [1, 2, 0]);

    apply_follow_order(&mut restored.users, &owners, Some(&social[..2]));
    store.replace_user_discovery(&restored).unwrap();
    let restored = store.load_user_discovery().unwrap();
    assert_eq!(positions(&restored.users, &owners), [0, 1, 2]);
}
