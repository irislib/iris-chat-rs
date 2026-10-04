use fips_core::{config::PeerConfig, PeerIdentity};

pub(super) fn webrtc_enabled(eligible: bool, configured: Option<&str>) -> bool {
    eligible
        && !configured.is_some_and(|value| {
            matches!(value.trim().to_ascii_lowercase().as_str(), "false" | "0")
        })
}

pub(super) fn pubsub_policy_options() -> nostr_pubsub_fips::FipsPubsubPolicyOptions {
    policy_options(
        &std::env::var_os("IRIS_CHAT_FIPS_TRUSTED_RATERS")
            .unwrap_or_default()
            .to_string_lossy(),
    )
}

fn policy_options(raters: &str) -> nostr_pubsub_fips::FipsPubsubPolicyOptions {
    let mut options = nostr_pubsub_fips::FipsPubsubPolicyOptions::default();
    // The shared adapter validates public keys and bounds this explicit list.
    options.reputation.trusted_raters = raters
        .split([',', ';'])
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect();
    options
}

pub(super) fn routed_peer_ids(
    siblings: &[PeerIdentity],
    peers: &[PeerIdentity],
    update_publisher: Option<&str>,
    max_connected_peers: usize,
) -> Vec<String> {
    let limit = max_connected_peers.saturating_sub(1);
    if limit == 0 {
        return Vec::new();
    }
    // Keep the trusted release publisher reachable even with a full roster,
    // while leaving one connection slot for a directly connected seed.
    let roster_limit = limit.saturating_sub(usize::from(update_publisher.is_some()));
    let mut selected = Vec::new();
    for peer in siblings.iter().chain(peers) {
        if selected.len() == roster_limit {
            break;
        }
        let npub = peer.npub();
        if Some(npub.as_str()) != update_publisher && !selected.contains(&npub) {
            selected.push(npub);
        }
    }
    if let Some(publisher) = update_publisher {
        selected.push(publisher.to_owned());
    }
    selected
}

pub(super) fn configured_peer_hints() -> Result<(Vec<PeerConfig>, Vec<PeerIdentity>), String> {
    parse_peer_hints(
        &std::env::var("IRIS_CHAT_FIPS_STATIC_PEERS").unwrap_or_default(),
        &std::env::var("IRIS_CHAT_FIPS_ROUTED_PEERS").unwrap_or_default(),
    )
}

fn parse_peer_hints(
    direct: &str,
    routed: &str,
) -> Result<(Vec<PeerConfig>, Vec<PeerIdentity>), String> {
    let mut peers = Vec::new();
    for entry in direct
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (npub, address) = entry.split_once('=').ok_or("expected npub=udp:address")?;
        let identity = PeerIdentity::from_npub(npub.trim()).map_err(|error| error.to_string())?;
        let address = address
            .trim()
            .strip_prefix("udp:")
            .ok_or("expected udp:address")?;
        let address = address
            .parse::<std::net::SocketAddr>()
            .map_err(|error| error.to_string())?;
        peers.push(PeerConfig::new(identity.npub(), "udp", address.to_string()));
    }
    let mut routed_peers = Vec::new();
    for npub in routed
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let peer = PeerIdentity::from_npub(npub).map_err(|error| error.to_string())?;
        if !routed_peers.contains(&peer) {
            routed_peers.push(peer);
        }
    }
    Ok((peers, routed_peers))
}

pub(super) fn valid_device_sync_chat_id(chat_id: &str) -> bool {
    chat_id
        .strip_prefix("group:")
        .is_some_and(|group_id| !group_id.is_empty() && group_id.len() <= 128)
        || nostr::PublicKey::from_hex(chat_id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_webrtc_opt_out_preserves_default_discovery() {
        for configured in [None, Some("true"), Some("1")] {
            assert!(webrtc_enabled(true, configured));
            assert!(!webrtc_enabled(false, configured));
        }
        for configured in [Some("false"), Some(" FALSE "), Some("0")] {
            assert!(!webrtc_enabled(true, configured));
            assert!(!webrtc_enabled(false, configured));
        }
    }

    #[test]
    fn machine_trust_entrypoints_are_explicit_and_empty_by_default() {
        assert!(policy_options("").reputation.trusted_raters.is_empty());
        let public_key = nostr::Keys::generate().public_key().to_hex();
        assert_eq!(
            policy_options(&format!(" {public_key}, ;{public_key} "))
                .reputation
                .trusted_raters,
            std::collections::BTreeSet::from([public_key]),
        );
        assert!(
            policy_options("invalid-public-key")
                .reputation
                .trusted_raters
                .contains("invalid-public-key"),
            "invalid explicit configuration must reach the adapter's validator"
        );
    }

    #[test]
    fn routed_identities_do_not_become_direct_addresses() {
        use nostr::ToBech32;
        let npub = nostr::Keys::generate().public_key().to_bech32().unwrap();
        let (direct, routed) = parse_peer_hints("", &format!("{npub},{npub}")).unwrap();
        assert!(direct.is_empty());
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].npub(), npub);
        let (direct, routed) = parse_peer_hints(&format!("{npub}=udp:127.0.0.1:7000"), "").unwrap();
        assert_eq!(direct.len(), 1);
        assert!(routed.is_empty());
    }

    #[test]
    fn sibling_identities_have_priority_within_the_bounded_roster() {
        use nostr::ToBech32;
        let peers = (0..70)
            .map(|_| {
                PeerIdentity::from_npub(&nostr::Keys::generate().public_key().to_bech32().unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let selected = routed_peer_ids(&[peers[69], peers[69]], &peers, None, 64);
        assert_eq!(selected.len(), 63);
        assert_eq!(selected[0], peers[69].npub());
        assert_eq!(
            selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            selected.len()
        );
    }

    #[test]
    fn invalid_hints_fail_instead_of_disabling_explicit_connectivity() {
        assert!(parse_peer_hints("broken", "").is_err());
        assert!(parse_peer_hints("", "broken").is_err());
    }

    #[test]
    fn roster_refresh_keeps_a_route_to_the_trusted_update_publisher() {
        use nostr::ToBech32;
        let publisher = nostr::Keys::generate().public_key().to_bech32().unwrap();
        let peers = (0..70)
            .map(|_| {
                PeerIdentity::from_npub(&nostr::Keys::generate().public_key().to_bech32().unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            routed_peer_ids(&[], &[], Some(&publisher), 64),
            std::slice::from_ref(&publisher)
        );
        for sibling in [peers[0], peers[69]] {
            let selected = routed_peer_ids(&[sibling], &peers, Some(&publisher), 64);
            assert_eq!(selected.len(), 63);
            assert_eq!(selected[0], sibling.npub());
            assert!(selected.contains(&publisher));
        }
        let publisher_peer = PeerIdentity::from_npub(&publisher).unwrap();
        assert_eq!(
            routed_peer_ids(&[publisher_peer], &[publisher_peer], Some(&publisher), 64),
            [publisher]
        );
    }

    #[test]
    fn update_publisher_respects_small_capacity_and_preserves_a_direct_seed_slot() {
        use nostr::ToBech32;
        let publisher = nostr::Keys::generate().public_key().to_bech32().unwrap();
        let sibling =
            PeerIdentity::from_npub(&nostr::Keys::generate().public_key().to_bech32().unwrap())
                .unwrap();
        for capacity in [0_usize, 1, 2, 3] {
            let selected = routed_peer_ids(&[sibling], &[], Some(&publisher), capacity);
            assert!(selected.len() <= capacity.saturating_sub(1));
            assert_eq!(selected.contains(&publisher), capacity >= 2);
            assert_eq!(selected.contains(&sibling.npub()), capacity >= 3);
        }
    }
}
