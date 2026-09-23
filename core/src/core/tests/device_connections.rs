#[test]
fn device_roster_tracks_live_connections_and_ignores_retired_runtime_updates() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let offline = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("device-connections", &owner, &local);
    core.preferences.nearby_enabled = false;
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(
                [&local, &sibling, &offline]
                    .iter()
                    .map(|keys| DeviceEntry::new(keys.public_key(), 1))
                    .collect(),
            ),
            1,
        ),
    );
    core.rebuild_state();
    let initial = core.state.device_roster.clone().expect("device list");
    assert!(initial.devices.iter().all(|device| !device.is_connected));
    let generation = core.fips_connection_generation;
    let links = vec![crate::updates::FipsNearbyLinkSnapshot {
        device_pubkey_hex: sibling.public_key().to_hex().to_ascii_uppercase(),
        transport_type: "websocket".to_string(),
        transport_addr: None,
    }];
    core.handle_internal(InternalEvent::FipsNearbyPeersChanged {
        generation,
        peers: links.clone(),
    });
    let connected = core.state.device_roster.clone().expect("live device list");
    assert_eq!(
        connected.devices.iter().filter(|device| device.is_connected)
            .map(|device| device.device_pubkey_hex.clone()).collect::<Vec<_>>(),
        vec![sibling.public_key().to_hex()],
        "only the authenticated sibling link is connected, even with Nearby off"
    );
    assert!(updates.try_iter().any(|update| matches!(update,
        AppUpdate::FullState(state) if state.device_roster == Some(connected.clone())
    )), "connection changes must reach the UI without another action");
    core.handle_internal(InternalEvent::FipsNearbyPeersChanged {
        generation,
        peers: Vec::new(),
    });
    assert_eq!(core.state.device_roster, Some(initial.clone()));
    core.handle_internal(InternalEvent::FipsNearbyPeersChanged {
        generation,
        peers: links.clone(),
    });
    core.stop_device_sync();
    assert_eq!(core.state.device_roster, Some(initial.clone()));
    core.handle_internal(InternalEvent::FipsNearbyPeersChanged { generation, peers: links });
    assert_eq!(core.state.device_roster, Some(initial), "retired runtime cannot revive a green dot");
}
