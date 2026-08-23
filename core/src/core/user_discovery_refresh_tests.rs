use super::*;
use std::sync::{Arc, RwLock};
use tempfile::TempDir;

#[test]
fn completed_fetch_starts_one_pending_refresh_despite_reconnect_floor() {
    let temp = TempDir::new().unwrap();
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: Client::new(device),
        relay_urls: relay_urls_from_strings(&["wss://example.invalid".to_string()]),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.user_discovery_runtime.token = 7;
    core.user_discovery_runtime.in_flight = true;
    core.user_discovery_runtime.refresh_pending = false;
    core.user_discovery_runtime.last_started_at = Some(Instant::now());
    core.user_discovery_syncing = true;
    core.request_user_discovery_refresh(false);
    assert!(core.user_discovery_runtime.refresh_pending);
    let result = UserDiscoveryFetchResult {
        cache: UserDiscoveryCache::default(),
        metadata_events: Vec::new(),
        detail: "test".to_string(),
    };

    core.handle_user_discovery_fetch_finished(7, result);

    assert_eq!(core.user_discovery_runtime.token, 8);
    assert!(core.user_discovery_runtime.in_flight);
    assert!(!core.user_discovery_runtime.refresh_pending);
    core.handle_user_discovery_fetch_finished(
        7,
        UserDiscoveryFetchResult {
            cache: UserDiscoveryCache::default(),
            metadata_events: Vec::new(),
            detail: "stale".to_string(),
        },
    );
    assert_eq!(core.user_discovery_runtime.token, 8);
    core.profile_search_runtime.token = 12;
    core.profile_search_runtime.query = "alice".to_string();
    core.profile_search_runtime.in_flight = true;
    core.reset_user_discovery_runtime();
    assert_eq!(core.user_discovery_runtime.token, 9);
    assert!(!core.user_discovery_runtime.in_flight);
    assert_eq!(core.profile_search_runtime.token, 13);
    assert!(core.profile_search_runtime.query.is_empty());
    assert!(!core.profile_search_runtime.in_flight);
    assert!(!core.user_discovery_syncing);
}

#[test]
fn restore_adopts_legacy_unowned_cache_without_social_rank() {
    let temp = TempDir::new().unwrap();
    let owner = Keys::generate();
    let friend = Keys::generate().public_key().to_hex();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_store
        .replace_user_discovery(&UserDiscoveryCache {
            owner_pubkey_hex: None,
            follow_event_id: Some("legacy-head".to_string()),
            follow_created_at_secs: 1,
            users: BTreeMap::from([(
                friend.clone(),
                DiscoveredUserRecord {
                    owner_pubkey_hex: friend,
                    follow_position: 0,
                    petname: None,
                },
            )]),
            social_rank_ready: true,
            social_friend_support: BTreeMap::from([("stale".to_string(), 1)]),
        })
        .unwrap();

    core.restore_user_discovery_cache(owner.public_key());

    assert_eq!(
        core.user_discovery.owner_pubkey_hex,
        Some(owner.public_key().to_hex())
    );
    assert_eq!(core.user_discovery.users.len(), 1);
    assert!(!core.user_discovery.social_rank_ready);
    assert!(core.user_discovery.social_friend_support.is_empty());
    assert_eq!(
        core.app_store.load_user_discovery().unwrap(),
        core.user_discovery
    );
}

#[test]
fn restoring_another_account_clears_personalized_ranking() {
    let temp = TempDir::new().unwrap();
    let owner = Keys::generate();
    let other_owner = Keys::generate();
    let target = Keys::generate().public_key().to_hex();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_store
        .replace_user_discovery(&UserDiscoveryCache {
            owner_pubkey_hex: Some(owner.public_key().to_hex()),
            follow_event_id: Some("head".to_string()),
            follow_created_at_secs: 1,
            users: BTreeMap::new(),
            social_rank_ready: true,
            social_friend_support: BTreeMap::from([(target, 1)]),
        })
        .unwrap();

    core.restore_user_discovery_cache(other_owner.public_key());

    assert_eq!(core.user_discovery, UserDiscoveryCache::default());
    assert_eq!(
        core.app_store.load_user_discovery().unwrap(),
        UserDiscoveryCache::default()
    );
}
