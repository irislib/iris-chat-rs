use super::*;

impl AppCore {
    pub(in crate::core) fn build_device_sync_packets_for_test(
        &self,
        roster_at: u64,
        include_messages: bool,
    ) -> Vec<Vec<u8>> {
        encode_device_sync_chunks(self.build_device_sync_snapshot(roster_at, include_messages))
    }

    pub(in crate::core) fn install_device_sync_sender_for_test(
        &mut self,
        endpoint: Arc<FipsEndpoint>,
        tcp: DeviceSyncTcpSender,
        siblings: Vec<FipsPeerIdentity>,
    ) {
        self.device_sync = Some(DeviceSyncRuntime {
            direct_files: None,
            calls_tx: None,
            key: "test".to_string(),
            peer_refresh_key: "test".to_string(),
            endpoint,
            tcp: Some(tcp),
            siblings,
            snapshot_pending: false,
            nearby_enabled: false,
            nearby_bootstrap_payloads: Arc::new(RwLock::new(Vec::new())),
            nearby_outbox: Arc::new(RwLock::new(
                crate::core::fips_nearby::FipsNearbyOutbox::default(),
            )),
            _attachment_blobs: None,
            pubsub: None,
            protocol_subscriptions: crate::core::mesh_pubsub::MeshProtocolSubscriptions::default(),
            _update_provider: None,
            recent_peers: None,
            tasks: Vec::new(),
        });
    }

    pub(in crate::core) fn take_device_sync_control_for_test(
        &self,
        peer: FipsPeerIdentity,
    ) -> Option<Vec<u8>> {
        self.device_sync
            .as_ref()
            .and_then(|runtime| runtime.tcp.as_ref())
            .and_then(|tcp| tcp.take_control_for_test(peer))
    }
}
