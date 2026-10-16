//! Only explicit development fixtures may isolate the production LAN browser.
const SCOPE_ENV: &str = "IRIS_CHAT_OFFLINE_LAN_SCOPE";

pub(in crate::core) fn requested_scope() -> Result<Option<String>, &'static str> {
    let scope = std::env::var_os(SCOPE_ENV);
    let scope = scope
        .as_ref()
        .map(|value| value.to_str().ok_or("offline LAN scope must be UTF-8"))
        .transpose()?;
    resolve_scope(
        cfg!(debug_assertions),
        std::env::var("IRIS_CHAT_OFFLINE_LAN_FIXTURE").as_deref() == Ok("1"),
        scope,
    )
    .map(|value| value.map(str::to_owned))
}

fn resolve_scope(
    debug: bool,
    fixture: bool,
    scope: Option<&str>,
) -> Result<Option<&str>, &'static str> {
    if !fixture && scope.is_none() {
        return Ok(None);
    }
    if !debug || !fixture {
        return Err("offline LAN scope requires a development build and explicit fixture mode");
    }
    let scope = scope.ok_or("offline LAN fixture requires a dedicated discovery scope")?;
    let suffix = scope
        .strip_prefix("iris-offline-lan-")
        .ok_or("offline LAN scope must use the dedicated fixture namespace")?;
    if !(16..=64).contains(&suffix.len())
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("offline LAN scope must contain 16 to 64 lowercase hexadecimal characters");
    }
    Ok(Some(scope))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "iris-offline-lan-0123456789abcdef";

    #[test]
    fn fixture_scope_requires_both_development_and_explicit_fixture_gates() {
        for (debug, fixture) in [(false, false), (false, true), (true, false)] {
            assert!(resolve_scope(debug, fixture, Some(SCOPE)).is_err());
        }
        assert!(resolve_scope(false, true, None).is_err());
        assert_eq!(resolve_scope(true, true, Some(SCOPE)).unwrap(), Some(SCOPE));
    }

    #[test]
    fn fixture_scope_rejects_missing_malformed_and_production_names() {
        assert!(resolve_scope(true, true, None).is_err());
        for invalid in [
            "",
            "iris-chat-nearby-v1",
            "iris-offline-lan-0123456789abcde",
            "iris-offline-lan-0123456789abcdeF",
            "iris-offline-lan-0123456789abcdef ",
            "iris-offline-lan-0123456789abcdef\n",
            "iris-offline-lan-0123456789abcdeg",
            "iris-offline-lan-0123456789abcdeé",
        ] {
            assert!(
                resolve_scope(true, true, Some(invalid)).is_err(),
                "{invalid:?}"
            );
        }
        let maximum = format!("iris-offline-lan-{}", "a".repeat(64));
        assert_eq!(
            resolve_scope(true, true, Some(&maximum)).unwrap(),
            Some(maximum.as_str())
        );
        assert!(resolve_scope(true, true, Some(&(maximum + "a"))).is_err());
    }

    #[test]
    fn ordinary_debug_and_release_configuration_remain_unchanged() {
        for debug in [false, true] {
            assert_eq!(resolve_scope(debug, false, None).unwrap(), None);
        }
    }

    // Multicast-loopback semantics differ between CI hosts. Run explicitly on
    // the physical proof host; final cross-device evidence remains mandatory.
    #[ignore = "requires working local multicast; run explicitly before the physical LAN proof"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn production_lan_browser_filters_other_scope_before_dial_candidates() {
        use fips_core::config::{TransportInstances, UdpConfig};
        use fips_core::discovery::lan::{LanDiscovery, LanDiscoveryConfig, LanEvent};
        use std::collections::BTreeSet;
        use std::time::{Duration, Instant};

        // DNS-SD service names are limited to 15 bytes; the daemon checks this
        // asynchronously after registration has returned successfully.
        let service = format!("_if{:08x}._udp.local.", rand::random::<u32>());
        let mut config = fips_core::Config::new();
        config.node.control.enabled = false;
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.lan.scope = Some(SCOPE.into());
        config.node.discovery.lan.service_type = service.clone();
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some("0.0.0.0:0".into()),
            advertise_on_nostr: Some(false),
            public: Some(false),
            ..UdpConfig::default()
        });
        let endpoint = fips_core::FipsEndpoint::builder()
            .config(config)
            .identity_nsec(nostr::Keys::generate().secret_key().to_secret_hex())
            .discovery_scope("iris-chat-nearby-v1")
            .without_system_tun()
            .bind()
            .await
            .unwrap();
        let endpoint_npub = endpoint.npub().to_owned();
        let identities: Vec<_> = (0..4).map(|_| fips_core::Identity::generate()).collect();
        let expected = identities[1].npub();
        let excluded = identities[2].npub();
        let scopes = [
            resolve_scope(true, true, Some(SCOPE))
                .unwrap()
                .map(str::to_owned),
            Some(SCOPE.to_owned()),
            Some("iris-chat-nearby-v1".to_owned()),
            None,
        ];
        let mut browsers = Vec::new();
        for (index, (identity, scope)) in identities.iter().zip(scopes).enumerate() {
            let config = LanDiscoveryConfig {
                enabled: true,
                service_type: service.clone(),
                scope: scope.clone(),
            };
            browsers.push(
                LanDiscovery::start(identity, scope, 62000 + index as u16, config)
                    .await
                    .unwrap(),
            );
        }
        let mut scoped = BTreeSet::new();
        let mut unscoped = BTreeSet::new();
        let mut endpoint_scope = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            for event in browsers[0].drain_events().await {
                let LanEvent::Discovered(peer) = event;
                scoped.insert(peer.npub);
            }
            for event in browsers[3].drain_events().await {
                let LanEvent::Discovered(peer) = event;
                if peer.npub == endpoint_npub {
                    endpoint_scope = peer.scope;
                }
                unscoped.insert(peer.npub);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        for browser in browsers {
            browser.shutdown().await;
        }
        endpoint.shutdown().await.unwrap();
        assert!(
            unscoped.contains(&expected) && unscoped.contains(&excluded),
            "control must observe both actual adverts: {unscoped:?}"
        );
        assert_eq!(endpoint_scope.as_deref(), Some(SCOPE), "prepared_config must retain explicit LAN scope despite different endpoint discovery metadata");
        assert_eq!(
            scoped,
            BTreeSet::from([expected, endpoint_npub]),
            "only the matching scope may reach the node's dial-candidate boundary"
        );
        println!("scope control verified: matching candidates={}, unfiltered adverts={}, excluded advert seen={}, effective endpoint scope={endpoint_scope:?}", scoped.len(), unscoped.len(), unscoped.contains(&excluded));
    }
}
