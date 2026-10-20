use super::*;

pub(super) fn configure_fips_lan(config: &mut Config, enabled: bool) {
    config.node.discovery.lan.enabled = enabled;
    config.transports.udp = if enabled {
        TransportInstances::Single(UdpConfig {
            bind_addr: Some("0.0.0.0:0".to_string()),
            advertise_on_nostr: Some(false),
            public: Some(false),
            outbound_only: Some(false),
            accept_connections: Some(true),
            ..UdpConfig::default()
        })
    } else {
        TransportInstances::default()
    };
}

pub(super) async fn run_recent_peer_observer(
    endpoint: Arc<FipsEndpoint>,
    recent_peers: Arc<RwLock<DeviceSyncRecentPeers>>,
) {
    loop {
        let peers = match peer_snapshot::query(|| endpoint.peers()).await {
            Ok(peers) => peers,
            Err(_) => return,
        };
        if let Ok(mut recent_peers) = recent_peers.write() {
            if let Err(error) =
                recent_peers.observe_and_flush_if_due(&peers, crate::perflog::now_ms())
            {
                crate::perflog!("fips_recent_peers.observe error={error}");
            }
        }
        sleep(RECENT_PEERS_OBSERVE_INTERVAL).await;
    }
}

pub(super) async fn shutdown_shared_fips(
    endpoint: Arc<FipsEndpoint>,
    pubsub: Option<Arc<FipsPubsubClient>>,
    recent_peers: Option<Arc<RwLock<DeviceSyncRecentPeers>>>,
) {
    if let Some(pubsub) = pubsub {
        pubsub.shutdown_shared().await;
    }
    if let (Some(recent_peers), Ok(peers)) = (recent_peers, endpoint.peers().await) {
        if let Ok(mut recent_peers) = recent_peers.write() {
            if let Err(error) = recent_peers.observe_and_flush(&peers, crate::perflog::now_ms()) {
                crate::perflog!("fips_recent_peers.shutdown error={error}");
            }
        }
    }
    let _ = endpoint.shutdown().await;
}

pub(super) fn same_host_hashtree_enabled() -> bool {
    same_host_hashtree_setting(std::env::var(SAME_HOST_HASHTREE_ENV).ok().as_deref())
}

pub(super) fn same_host_hashtree_setting(value: Option<&str>) -> bool {
    !value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    })
}

pub(super) fn configured_websocket_seeds() -> Option<WebSocketConfig> {
    let configured = std::env::var(WEBSOCKET_SEED_URLS_ENV).ok();
    let seed_urls = websocket_seed_urls(configured.as_deref());
    let bind_addr = std::env::var("IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR")
        .ok()
        .filter(|value| !value.trim().is_empty());
    (!seed_urls.is_empty() || bind_addr.is_some()).then_some(WebSocketConfig {
        bind_addr,
        seed_urls,
        ..WebSocketConfig::default()
    })
}

pub(super) fn configured_local_rendezvous_addr() -> Result<Option<SocketAddrV4>, String> {
    let Some(value) = std::env::var(LOCAL_RENDEZVOUS_ADDR_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    parse_local_rendezvous_addr(&value).map(Some)
}

pub(super) fn parse_local_rendezvous_addr(value: &str) -> Result<SocketAddrV4, String> {
    let address = value.trim().parse::<SocketAddrV4>().map_err(|error| {
        format!("{LOCAL_RENDEZVOUS_ADDR_ENV} must be an IPv4 loopback address: {error}")
    })?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(format!(
            "{LOCAL_RENDEZVOUS_ADDR_ENV} must be a non-zero IPv4 loopback address"
        ));
    }
    Ok(address)
}
