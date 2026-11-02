use super::*;

pub(in crate::core) const APP: &str = "iris-mesh-discovery-test";

pub(in crate::core) fn advert(
    keys: &Keys,
    created_at: u64,
    expires_at: u64,
    endpoint: &str,
) -> Event {
    EventBuilder::new(
        Kind::Custom(37195),
        serde_json::json!({
            "identifier": "fips-overlay-v1", "version": 1,
            "endpoints": [{"transport": "udp", "addr": endpoint}],
        })
        .to_string(),
    )
    .custom_created_at(Timestamp::from(created_at))
    .tags([
        nostr::Tag::identifier(APP),
        nostr::Tag::parse(["protocol", APP]).unwrap(),
        nostr::Tag::parse(["version", "1"]).unwrap(),
        nostr::Tag::expiration(Timestamp::from(expires_at)),
    ])
    .sign_with_keys(keys)
    .unwrap()
}

/// Real external advert ingestion, with no remote peers or network discovery.
/// Reserved hostname addresses are signed fixture data and are never dialed.
pub(in crate::core) async fn endpoint() -> Arc<FipsEndpoint> {
    let mut config = Config::new();
    config.node.leaf_only = true;
    config.node.control.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = true;
    config.node.discovery.nostr.advertise = false;
    config.node.discovery.nostr.peerfinding_source =
        fips_core::config::NostrPeerfindingSource::External;
    config.node.discovery.nostr.policy = fips_core::config::NostrDiscoveryPolicy::ConfiguredOnly;
    config.node.discovery.nostr.app = APP.into();
    config.node.discovery.nostr.advert_ttl_secs = 3_600;
    config.node.discovery.nostr.advert_relays.clear();
    config.node.discovery.nostr.stun_servers.clear();
    config.peers.clear();
    config.transports.websocket = TransportInstances::Single(WebSocketConfig::default());
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        accept_connections: Some(false),
        public: Some(false),
        ..Default::default()
    });
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

#[cfg(target_os = "linux")]
pub(in crate::core) async fn shutdown(discovery: &mut PrivateMeshDiscovery) {
    discovery.workers.clear();
    discovery.relay_discovery = None;
    tokio::task::yield_now().await;
}

#[tokio::test]
async fn accepted_advert_renewals_keep_quiet_cadence_and_repair_an_empty_endpoint_cache() {
    let keys = Keys::generate();
    let now = unix_now().get();
    let live = endpoint().await;
    let provider = PrivateMeshProviders::new(vec![], None, vec![], vec![]);
    let mut observations = DiscoveryObservations::default();
    let mut cadence = DiscoveryCadence::default();
    let (sender, _events) = flume::unbounded();
    for (offset, addr, expected) in [
        (0, "mesh-a.invalid:9000", 30),
        (1, "mesh-a.invalid:9000", 60),
        (2, "mesh-a.invalid:9000", 120),
        (3, "mesh-b.invalid:9000", 30),
    ] {
        let event = advert(&keys, now - 10 + offset, now + 3600 + offset, addr);
        let mut poll = DiscoveryPoll {
            attempted: true,
            ..Default::default()
        };
        provider
            .deliver_discovery(event, &live, &sender, &mut observations, &mut poll)
            .await;
        assert_eq!(
            poll.next_delay(&mut cadence),
            Some(Duration::from_secs(expected))
        );
        assert_eq!(observations.advert_valid_until, Some(now + 3600 + offset));
    }
    let duplicate = observations.advert.clone().unwrap();
    let signed_deadline = observations.advert_valid_until;
    observations.advert_valid_until = None;
    let empty_cache = endpoint().await;
    let mut poll = DiscoveryPoll::default();
    provider
        .deliver_discovery(
            duplicate,
            &empty_cache,
            &sender,
            &mut observations,
            &mut poll,
        )
        .await;
    assert!(
        !poll.changed,
        "cache repair is not a semantic endpoint change"
    );
    assert_eq!(
        observations.advert_valid_until, signed_deadline,
        "duplicate must reach FIPS ingestion and restore its original signed bound"
    );
    empty_cache.shutdown().await.unwrap();
    live.shutdown().await.unwrap();
}

#[test]
fn advert_semantics_keep_all_non_expiration_tags_and_signed_max_age() {
    let keys = Keys::generate();
    let now = unix_now().get();
    let original = advert(&keys, now, now + 3600, "mesh-a.invalid:9000");
    let mut observations = DiscoveryObservations::default();
    assert!(observations.remember(&original, 60));
    assert_eq!(observations.advert_valid_until, Some(now + 120));
    let renewed = advert(&keys, now + 1, now + 4000, "mesh-a.invalid:9000");
    assert!(!observations.remember(&renewed, 60));
    assert_eq!(observations.advert_valid_until, Some(now + 121));
    let without_expiration = EventBuilder::new(renewed.kind, &renewed.content)
        .custom_created_at(renewed.created_at)
        .tags(advert_semantic_tags(&renewed).cloned())
        .sign_with_keys(&keys)
        .unwrap();
    assert!(!observations.remember(&without_expiration, 60));
    assert_eq!(observations.advert_valid_until, Some(now + 121));
    assert!(!observations.remember(&without_expiration, 60));
    assert_eq!(observations.advert_valid_until, Some(now + 121));
    for tag in [
        nostr::Tag::identifier("changed-app"),
        nostr::Tag::parse(["protocol", "changed-protocol"]).unwrap(),
        nostr::Tag::parse(["unknown-future-field", "must-not-be-ignored"]).unwrap(),
    ] {
        let changed = EventBuilder::new(original.kind, &original.content)
            .custom_created_at(original.created_at)
            .tags(original.tags.iter().cloned().chain([tag]))
            .sign_with_keys(&keys)
            .unwrap();
        assert!(observations.remember(&changed, 60));
        assert!(!observations.remember(&original, 60));
    }
    let other_author = EventBuilder::new(original.kind, &original.content)
        .tags(original.tags.iter().cloned())
        .sign_with_keys(&Keys::generate())
        .unwrap();
    assert!(observations.remember(&other_author, 60));
}

#[tokio::test]
async fn accepted_same_second_advert_replays_back_off_without_hiding_new_routes() {
    let keys = Keys::generate();
    let now = unix_now().get();
    let live = endpoint().await;
    let provider = PrivateMeshProviders::new(vec![], None, vec![], vec![]);
    let (sender, _events) = flume::unbounded();
    let mut observations = DiscoveryObservations::default();
    let mut cadence = DiscoveryCadence::default();
    let pair = [
        advert(&keys, now - 10, now + 3600, "mesh-a.invalid:9000"),
        advert(&keys, now - 10, now + 3700, "mesh-b.invalid:9000"),
    ];
    for expected in [30, 60, 120, 120] {
        observations.advert_valid_until = None;
        let mut poll = DiscoveryPoll {
            attempted: true,
            ..Default::default()
        };
        for event in &pair {
            provider
                .deliver_discovery(event.clone(), &live, &sender, &mut observations, &mut poll)
                .await;
        }
        assert_eq!(
            poll.next_delay(&mut cadence),
            Some(Duration::from_secs(expected))
        );
        assert_eq!(observations.seen.len(), 2);
        assert_eq!(observations.order.len(), 2);
        assert_eq!(observations.advert.as_ref().unwrap().id, pair[1].id);
        assert_eq!(
            observations.advert_valid_until,
            Some(now + 3700),
            "both replayed adverts must still be ingested with their original signed deadlines"
        );
    }
    for (offset, addr, expected) in [
        (1, "mesh-b.invalid:9000", 120),
        (2, "mesh-c.invalid:9000", 30),
    ] {
        let mut poll = DiscoveryPoll {
            attempted: true,
            ..Default::default()
        };
        provider
            .deliver_discovery(
                advert(&keys, now - 10 + offset, now + 3700 + offset, addr),
                &live,
                &sender,
                &mut observations,
                &mut poll,
            )
            .await;
        assert_eq!(
            poll.next_delay(&mut cadence),
            Some(Duration::from_secs(expected))
        );
        assert_eq!(observations.advert_valid_until, Some(now + 3700 + offset));
    }
    live.shutdown().await.unwrap();
}
