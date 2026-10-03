use super::*;

#[test]
fn nearby_snapshot_excludes_self_before_and_after_device_list_arrives() {
    let directory = tempfile::TempDir::new().unwrap();
    let (updates_tx, updates_rx) = flume::unbounded();
    let mut core = AppCore::new(
        updates_tx,
        flume::unbounded().0,
        directory.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.create_account("Me");
    let login = core.logged_in.as_ref().unwrap();
    let owner = login.owner_keys.as_ref().unwrap().clone();
    let local_device = login.device_keys.public_key();
    let sibling_device = Keys::generate().public_key();
    let other_device = Keys::generate().public_key().to_hex();
    let sibling_id = sibling_device.to_hex().to_ascii_uppercase();
    // A restored account can discover links before its device list arrives.
    core.app_keys.remove(&owner.public_key().to_hex());
    let latest_snapshot = || {
        updates_rx
            .try_iter()
            .filter_map(|update| match update {
                AppUpdate::NearbyPeersChanged {
                    snapshot,
                    bluetooth_peer_ids,
                    lan_peer_ids,
                } => Some((snapshot, bluetooth_peer_ids, lan_peer_ids)),
                _ => None,
            })
            .last()
            .expect("nearby snapshot")
    };
    let link = |id: String, transport: &str| crate::updates::FipsNearbyLinkSnapshot {
        device_pubkey_hex: id,
        transport_type: transport.to_string(),
        transport_addr: Some("192.168.1.25:7000".to_string()),
    };
    core.handle_internal(InternalEvent::FipsNearbyPeersChanged {
        generation: core.fips_connection_generation,
        peers: vec![
            link(local_device.to_hex().to_ascii_uppercase(), "BLE"),
            link(sibling_id.clone(), "UDP"),
            link(other_device.clone(), "Bluetooth"),
        ],
    });
    let (snapshot, bluetooth, lan) = latest_snapshot();
    assert_eq!(
        snapshot
            .peers
            .iter()
            .map(|peer| &peer.id)
            .collect::<Vec<_>>(),
        vec![&sibling_id, &other_device],
        "the current device must be hidden even before its device list arrives"
    );
    assert_eq!(bluetooth, vec![other_device.clone()]);
    assert_eq!(lan, vec![sibling_id]);

    let created_at = unix_now().get().saturating_add(1);
    let device_list = AppKeys::new(vec![
        DeviceEntry::new(local_device, created_at),
        DeviceEntry::new(sibling_device, created_at),
    ])
    .get_event_at(owner.public_key(), created_at)
    .sign_with_keys(&owner)
    .unwrap();
    let previous_generation = core.fips_connection_generation;
    let links = core.fips_nearby_links.clone();
    core.handle_relay_event(device_list);
    reconnect_fixture_after_runtime_change(&mut core, previous_generation, links);
    let (snapshot, bluetooth, lan) = latest_snapshot();
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.peers[0].id, other_device);
    assert_eq!(bluetooth, vec![other_device]);
    assert!(lan.is_empty(), "linked devices are not other nearby users");
    assert_eq!(
        core.fips_nearby_links.len(),
        3,
        "keep links for device sync"
    );
    assert!(core.app_keys[&owner.public_key().to_hex()]
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == sibling_device.to_hex()));
}

#[test]
fn nearby_snapshot_excludes_transit_connections() {
    let directory = tempfile::TempDir::new().unwrap();
    let (updates_tx, updates_rx) = flume::unbounded();
    let mut core = AppCore::new(
        updates_tx,
        flume::unbounded().0,
        directory.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let link = |id: &str, transport: &str| crate::updates::FipsNearbyLinkSnapshot {
        device_pubkey_hex: id.repeat(32),
        transport_type: transport.to_string(),
        transport_addr: Some("192.168.1.25:7000".to_string()),
    };

    for links in [
        vec![link("01", "websocket"), link("02", "WebSocket")],
        vec![
            link("01", "websocket"),
            link("02", "WebSocket"),
            link("03", "BLE"),
            link("04", "udp"),
            link("05", "Ethernet"),
            link("06", "Bluetooth"),
            link("07", "webrtc"),
            link("08", ""),
        ],
    ] {
        let has_local_peers = links.len() > 2;
        while updates_rx.try_recv().is_ok() {}
        core.handle_internal(InternalEvent::FipsNearbyPeersChanged {
            generation: core.fips_connection_generation,
            peers: links,
        });
        let (snapshot, bluetooth_peer_ids, lan_peer_ids) = updates_rx
            .try_iter()
            .find_map(|update| match update {
                AppUpdate::NearbyPeersChanged {
                    snapshot,
                    bluetooth_peer_ids,
                    lan_peer_ids,
                } => Some((snapshot, bluetooth_peer_ids, lan_peer_ids)),
                _ => None,
            })
            .expect("nearby snapshot");

        let expected_ids = if has_local_peers {
            vec![
                "03".repeat(32),
                "04".repeat(32),
                "05".repeat(32),
                "06".repeat(32),
            ]
        } else {
            Vec::new()
        };
        assert_eq!(
            snapshot
                .peers
                .iter()
                .map(|peer| peer.id.clone())
                .collect::<Vec<_>>(),
            expected_ids,
            "the chat-list preview must contain only Bluetooth and LAN peers"
        );
        assert_eq!(
            bluetooth_peer_ids,
            if has_local_peers {
                vec!["03".repeat(32), "06".repeat(32)]
            } else {
                Vec::new()
            }
        );
        assert_eq!(
            lan_peer_ids,
            if has_local_peers {
                vec!["04".repeat(32), "05".repeat(32)]
            } else {
                Vec::new()
            }
        );
    }
}

#[test]
fn nearby_snapshot_does_not_treat_internet_udp_contacts_as_local() {
    let directory = tempfile::TempDir::new().unwrap();
    let (tx, rx) = flume::unbounded();
    let mut core = AppCore::new(
        tx,
        flume::unbounded().0,
        directory.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let addresses = [
        Some("8.8.8.8:7000"),
        Some("[2606:4700:4700::1111]:7000"),
        None,
        Some("unrecognized"),
        Some("127.0.0.1:7000"),
        Some("192.168.1.25:7000"),
        Some("[fe80::abcd%4]:7000"),
    ];
    core.fips_nearby_links = addresses
        .iter()
        .enumerate()
        .map(|(index, addr)| crate::updates::FipsNearbyLinkSnapshot {
            device_pubkey_hex: format!("{index:064x}"),
            transport_type: "UDP".to_string(),
            transport_addr: addr.map(str::to_string),
        })
        .collect();
    core.emit_fips_nearby_peers();
    let ids = rx
        .try_iter()
        .find_map(|update| match update {
            AppUpdate::NearbyPeersChanged { lan_peer_ids, .. } => Some(lan_peer_ids),
            _ => None,
        })
        .unwrap();
    assert_eq!(ids, vec![format!("{:064x}", 5), format!("{:064x}", 6)]);
}
