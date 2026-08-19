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
    core.reset_user_discovery_runtime();
}
