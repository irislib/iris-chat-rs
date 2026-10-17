use super::*;

// Environment-controlled fixture policy is exercised in a child so no other
// test can observe its discovery scope or transport overrides.
#[test]
fn scoped_offline_lan_keeps_blob_service_without_local_rendezvous() {
    const CHILD: &str = "IRIS_TEST_OFFLINE_BLOB_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        check_runtime(mode == "fixture");
        return;
    }
    for mode in ["ordinary", "fixture"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "core::device_sync::runtime::offline_blob_tests::scoped_offline_lan_keeps_blob_service_without_local_rendezvous",
                "--nocapture",
            ])
            .env(CHILD, mode)
            .env("IRIS_CHAT_SAME_HOST_HASHTREE", "0")
            .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
            .env("IRIS_CHAT_FIPS_STATIC_PEERS", "")
            .env("IRIS_CHAT_FIPS_ROUTED_PEERS", "")
            .env("IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR", "")
            .env("IRIS_CHAT_FIPS_ENABLE_WEBRTC", "0")
            .env("IRIS_UPDATE_RELAYS", "")
            .env_remove("IRIS_CHAT_OFFLINE_LAN_FIXTURE")
            .env_remove("IRIS_CHAT_OFFLINE_LAN_SCOPE");
        if mode == "fixture" {
            command.env("IRIS_CHAT_OFFLINE_LAN_FIXTURE", "1").env(
                "IRIS_CHAT_OFFLINE_LAN_SCOPE",
                "iris-offline-lan-0123456789abcdef",
            );
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{mode} child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

fn check_runtime(fixture: bool) {
    let temp = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let owner = nostr::Keys::generate();
    let device = nostr::Keys::generate();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: nostr_sdk::Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_lan_enabled = true;
    core.preferences.nearby_bluetooth_enabled = false;
    core.reconcile_shared_fips(SharedFipsOptions {
        same_host_hashtree: false,
        udp_bind_addr: Some("127.0.0.1:0".into()),
        ..SharedFipsOptions::default()
    });
    let runtime = core.device_sync.as_ref().expect("LAN endpoint starts");
    assert_eq!(
        runtime._attachment_blobs.is_some(),
        fixture,
        "the explicit offline fixture must retain the production blob service",
    );
    assert!(
        runtime
            .endpoint
            .local_instance_advertisements()
            .unwrap()
            .is_empty(),
        "same-host rendezvous must remain disabled",
    );
    let addresses = core
        .runtime
        .block_on(runtime.endpoint.bound_udp_listen_addrs())
        .unwrap();
    assert_eq!(addresses.len(), 1);
    assert!(addresses[0].ip().is_loopback());
    assert_ne!(addresses[0].port(), 0);
    core.stop_device_sync_now();
}
