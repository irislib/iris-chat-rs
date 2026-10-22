use super::*;

impl AppCore {
    pub(crate) fn device_sync_has_sibling_tcp_for_test(&self) -> bool {
        self.device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.tcp.is_some())
    }

    pub(in crate::core) fn device_sync_endpoint_for_test(&self) -> Option<Arc<FipsEndpoint>> {
        self.device_sync
            .as_ref()
            .map(|runtime| runtime.endpoint.clone())
    }

    pub(in crate::core) fn replace_device_sync_sender_for_test(
        &mut self,
        tcp: DeviceSyncTcpSender,
    ) {
        self.device_sync.as_mut().expect("device sync runtime").tcp = Some(tcp);
    }

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
        self.install_device_sync_endpoint_for_test(endpoint);
        let runtime = self.device_sync.as_mut().expect("device sync runtime");
        runtime.tcp = Some(tcp);
        runtime.siblings = siblings;
    }

    pub(in crate::core) fn install_device_sync_endpoint_for_test(
        &mut self,
        endpoint: Arc<FipsEndpoint>,
    ) {
        self.device_sync = Some(DeviceSyncRuntime {
            direct_files: None,
            calls_tx: None,
            key: "test".to_string(),
            peer_refresh_key: "test".to_string(),
            endpoint,
            configured_direct_peers: Default::default(),
            tcp: None,
            siblings: Vec::new(),
            snapshot_pending: false,
            history: history::HistoryState::default(),
            nearby_enabled: false,
            nearby_bootstrap_payloads: Arc::new(RwLock::new(Vec::new())),
            nearby_outbox: Arc::new(RwLock::new(
                crate::core::fips_nearby::FipsNearbyOutbox::default(),
            )),
            _attachment_blobs: None,
            pubsub: None,
            private_providers: None,
            peer_advert_authors: Vec::new(),
            protocol_subscriptions: crate::core::mesh_pubsub::MeshProtocolSubscriptions::default(),
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

impl AppCore {
    pub(in crate::core) fn device_history_session_count_for_test(&self) -> usize {
        self.device_sync
            .as_ref()
            .map_or(0, |runtime| runtime.history.session_count())
    }
}
