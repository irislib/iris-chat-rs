use fips_core::config::PeerConfig;
use fips_core::{FipsEndpointPeer, RecentPeers};
use fips_endpoint::RecentPeersFileStore;
use std::path::PathBuf;

pub(super) const RECENT_PEERS_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const RECENT_PEERS_FLUSH_INTERVAL_MS: u64 = 60 * 1_000;

pub(super) struct DeviceSyncRecentPeers {
    store: RecentPeersFileStore,
    recent: RecentPeers,
    dirty: bool,
    last_flush_at_ms: u64,
}

impl DeviceSyncRecentPeers {
    pub(super) fn load(
        path: PathBuf,
        local_npub: &str,
        scope: &str,
        now_ms: u64,
    ) -> Result<(Self, Option<String>), String> {
        let store = RecentPeersFileStore::new(path, local_npub, scope)
            .map_err(|error| error.to_string())?;
        let (mut recent, warning, mut dirty) = match store.load() {
            Ok(recent) => (recent, None, false),
            Err(error) => (
                RecentPeers::new(local_npub, scope).map_err(|error| error.to_string())?,
                Some(error.to_string()),
                true,
            ),
        };
        let before_prune = recent.clone();
        recent.prune(now_ms, RECENT_PEERS_TTL_MS);
        dirty |= recent != before_prune;
        Ok((
            Self {
                store,
                recent,
                dirty,
                last_flush_at_ms: 0,
            },
            warning,
        ))
    }

    pub(super) fn merge_into(&self, peers: &mut [PeerConfig]) -> usize {
        self.recent.merge_into_peer_configs(peers)
    }

    pub(super) fn observe_and_flush_if_due(
        &mut self,
        peers: &[FipsEndpointPeer],
        now_ms: u64,
    ) -> Result<bool, String> {
        self.observe(peers, now_ms)?;
        if self.dirty
            && now_ms.saturating_sub(self.last_flush_at_ms) >= RECENT_PEERS_FLUSH_INTERVAL_MS
        {
            return self.flush(now_ms);
        }
        Ok(false)
    }

    pub(super) fn observe_and_flush(
        &mut self,
        peers: &[FipsEndpointPeer],
        now_ms: u64,
    ) -> Result<bool, String> {
        self.observe(peers, now_ms)?;
        self.flush(now_ms)
    }

    fn observe(&mut self, peers: &[FipsEndpointPeer], now_ms: u64) -> Result<(), String> {
        for peer in peers {
            self.dirty |= self
                .recent
                .observe_authenticated_peer(peer, now_ms)
                .map_err(|error| error.to_string())?;
        }
        let before_prune = self.recent.clone();
        self.recent.prune(now_ms, RECENT_PEERS_TTL_MS);
        self.dirty |= self.recent != before_prune;
        Ok(())
    }

    fn flush(&mut self, now_ms: u64) -> Result<bool, String> {
        if !self.dirty {
            return Ok(false);
        }
        self.store
            .save(&self.recent)
            .map_err(|error| error.to_string())?;
        self.dirty = false;
        self.last_flush_at_ms = now_ms;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fips_core::config::PeerAddressProvenance;
    use fips_core::{FipsEndpointPeer, Identity, NodeAddr};

    fn endpoint_peer(npub: String, transport: &str, addr: &str) -> FipsEndpointPeer {
        FipsEndpointPeer {
            npub,
            node_addr: NodeAddr::from_bytes([7; 16]),
            connected: true,
            transport_addr: Some(addr.to_string()),
            transport_type: Some(transport.to_string()),
            link_id: 1,
            srtt_ms: None,
            srtt_age_ms: None,
            packets_sent: 0,
            packets_recv: 0,
            bytes_sent: 0,
            bytes_recv: 0,
            rekey_in_progress: false,
            rekey_draining: false,
            current_k_bit: None,
            last_outbound_route: None,
            direct_probe_pending: false,
            direct_probe_after_ms: None,
            direct_probe_retry_count: 0,
            direct_probe_auto_reconnect: false,
            direct_probe_expires_at_ms: None,
            nostr_traversal_consecutive_failures: 0,
            nostr_traversal_in_cooldown: false,
            nostr_traversal_cooldown_until_ms: None,
            nostr_traversal_last_observed_skew_ms: None,
        }
    }

    #[test]
    fn load_prunes_old_routes_and_only_augments_configured_membership() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent-peers.json");
        let local = Identity::generate().npub();
        let authorized = Identity::generate().npub();
        let expired = Identity::generate().npub();
        let unconfigured = Identity::generate().npub();
        let scope = "iris-chat-device-sync-v1:test";
        let now_ms = RECENT_PEERS_TTL_MS + 10_000;
        let store = RecentPeersFileStore::new(&path, &local, scope).unwrap();
        let mut recent = RecentPeers::new(&local, scope).unwrap();
        recent
            .observe_authenticated_peer(
                &endpoint_peer(authorized.clone(), "udp", "192.0.2.1:32112"),
                now_ms - 1_000,
            )
            .unwrap();
        recent
            .observe_authenticated_peer(
                &endpoint_peer(expired.clone(), "udp", "192.0.2.2:32112"),
                now_ms - RECENT_PEERS_TTL_MS - 1,
            )
            .unwrap();
        recent
            .observe_authenticated_peer(
                &endpoint_peer(unconfigured, "udp", "192.0.2.3:32112"),
                now_ms - 500,
            )
            .unwrap();
        store.save(&recent).unwrap();

        let (cache, warning) = DeviceSyncRecentPeers::load(path, &local, scope, now_ms).unwrap();
        assert!(warning.is_none());
        let mut configured = vec![
            PeerConfig {
                npub: authorized,
                ..PeerConfig::default()
            },
            PeerConfig {
                npub: expired,
                ..PeerConfig::default()
            },
        ];

        assert_eq!(cache.merge_into(&mut configured), 1);
        assert_eq!(configured.len(), 2);
        assert_eq!(configured[0].addresses.len(), 1);
        assert_eq!(
            configured[0].addresses[0].provenance,
            PeerAddressProvenance::Authenticated
        );
        assert!(configured[1].addresses.is_empty());
    }

    #[test]
    fn malformed_cache_is_discarded_and_replaced_with_bound_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent-peers.json");
        std::fs::write(&path, b"not json").unwrap();
        let local = Identity::generate().npub();
        let scope = "iris-chat-nearby-v1";
        let now_ms = 1_000_000;

        let (mut cache, warning) =
            DeviceSyncRecentPeers::load(path.clone(), &local, scope, now_ms).unwrap();
        assert!(warning.is_some());
        assert!(cache.observe_and_flush(&[], now_ms).unwrap());

        let restored = RecentPeersFileStore::new(path, &local, scope)
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(restored.local_npub(), local);
        assert_eq!(restored.scope(), scope);
        assert!(restored.peers.is_empty());
    }

    #[test]
    fn observation_persists_only_restart_safe_authenticated_udp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent-peers.json");
        let local = Identity::generate().npub();
        let remote = Identity::generate().npub();
        let scope = "iris-chat-nearby-v1";
        let (mut cache, _) =
            DeviceSyncRecentPeers::load(path.clone(), &local, scope, 1_000).unwrap();

        cache
            .observe_and_flush(
                &[
                    endpoint_peer(remote.clone(), "websocket", "wss://example.invalid/fips"),
                    endpoint_peer(remote.clone(), "udp", "0.0.0.0:32112"),
                    endpoint_peer(remote.clone(), "udp", "192.0.2.4:32112"),
                ],
                2_000,
            )
            .unwrap();

        let restored = RecentPeersFileStore::new(path, &local, scope)
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(restored.peers[&remote].endpoints.len(), 1);
        assert_eq!(restored.peers[&remote].endpoints[0].addr, "192.0.2.4:32112");
    }
}
