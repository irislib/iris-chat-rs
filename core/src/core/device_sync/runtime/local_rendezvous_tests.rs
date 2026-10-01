use super::*;

#[test]
fn same_host_files_are_enabled_unless_explicitly_disabled() {
    assert!(same_host_hashtree_setting(None));
    for value in ["", "1", "true", "YES", " on "] {
        assert!(same_host_hashtree_setting(Some(value)));
    }
    for value in ["0", "false", "NO", " off "] {
        assert!(!same_host_hashtree_setting(Some(value)));
    }
}

#[test]
fn websocket_seeds_default_to_osiris_then_lnvps() {
    assert_eq!(
        websocket_seed_urls(None),
        vec![
            "wss://fips2.iris.to/fips".to_string(),
            "wss://fips1.iris.to/fips".to_string(),
        ]
    );
}

#[test]
fn websocket_seed_override_can_replace_or_disable_defaults() {
    assert_eq!(
        websocket_seed_urls(Some(" wss://one.example/fips, wss://two.example/fips ")),
        vec![
            "wss://one.example/fips".to_string(),
            "wss://two.example/fips".to_string(),
        ]
    );
    assert!(websocket_seed_urls(Some("  ")).is_empty());
}

#[test]
fn local_rendezvous_override_requires_nonzero_ipv4_loopback() {
    assert_eq!(
        parse_local_rendezvous_addr("127.0.0.1:32112").unwrap(),
        "127.0.0.1:32112".parse::<SocketAddrV4>().unwrap()
    );
    assert!(parse_local_rendezvous_addr("0.0.0.0:32112").is_err());
    assert!(parse_local_rendezvous_addr("127.0.0.1:0").is_err());
    assert!(parse_local_rendezvous_addr("[::1]:32112").is_err());
}

#[test]
fn fips_lan_uses_scoped_bidirectional_ephemeral_udp() {
    let mut config = Config::new();
    configure_fips_lan(&mut config, true);

    assert!(config.node.discovery.lan.enabled);
    let TransportInstances::Single(udp) = config.transports.udp else {
        panic!("expected one UDP transport");
    };
    assert_eq!(udp.bind_addr.as_deref(), Some("0.0.0.0:0"));
    assert_eq!(udp.advertise_on_nostr, Some(false));
    assert_eq!(udp.public, Some(false));
    assert_eq!(udp.outbound_only, Some(false));
    assert_eq!(udp.accept_connections, Some(true));
}

#[test]
fn disabling_fips_lan_removes_udp_transport() {
    let mut config = Config::new();
    configure_fips_lan(&mut config, true);
    configure_fips_lan(&mut config, false);

    assert!(!config.node.discovery.lan.enabled);
    assert!(config.transports.udp.is_empty());
}

#[test]
fn fips_lan_can_start_before_any_remote_peer_is_known() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.create_account("LAN discovery");
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_lan_enabled = true;

    let config = core.device_sync_config().expect("nearby-only config");

    assert!(config.peers.is_empty());
    assert!(config.nearby_ip_enabled);
}
