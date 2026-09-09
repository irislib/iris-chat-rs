use fips_core::{config::PeerConfig, PeerIdentity};

pub(super) fn routed_peer_ids(siblings: &[PeerIdentity], peers: &[PeerIdentity]) -> Vec<String> {
    let limit = nostr_pubsub_fips::FipsPubsubClientOptions::default()
        .max_connected_peers
        .saturating_sub(1);
    let mut selected = Vec::new();
    for peer in siblings.iter().chain(peers) {
        let npub = peer.npub();
        if !selected.contains(&npub) {
            selected.push(npub);
        }
        if selected.len() == limit {
            break;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let selected = routed_peer_ids(&[peers[69], peers[69]], &peers);
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
}
