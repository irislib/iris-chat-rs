use super::*;

impl AppCore {
    pub(super) fn logout(&mut self) {
        self.finish_call("Call ended");
        self.calls = calls::CallRuntime::default();
        self.push_debug_log("session.logout", "clearing runtime state");
        let previous_rev = self.state.rev;
        self.stop_pending_linked_device();
        self.stop_device_sync();
        self.reset_pending_invite_acceptance();
        self.private_chat_invites.clear();
        self.pending_private_invite_responses.clear();
        self.pending_private_invite_cleanup_retry = false;
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.wrapping_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.protocol_engine = None;
        if let Some(logged_in) = self.logged_in.take() {
            let client = logged_in.client.clone();
            self.runtime.spawn(async move {
                client.unsubscribe_all().await;
                let _ = client.shutdown().await;
            });
        }

        self.clear_chats_and_deletions();
        self.active_chat_id = None;
        self.screen_stack.clear();
        self.owner_profiles.clear();
        self.profile_metadata_fetch_inflight.clear();
        self.app_keys.clear();
        self.reset_direct_chat_capability_runtime();
        self.reset_user_discovery_runtime();
        self.groups.clear();
        self.chat_message_ttl_seconds.clear();
        self.recent_handshake_peers.clear();
        self.seen_event_ids.clear();
        self.seen_event_order.clear();
        self.typing_floor_secs.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.cached_mobile_push = MobilePushSyncSnapshot::default();
        self.mobile_push_dirty = true;
        self.last_emitted_state = None;
        self.next_message_id = 1;
        self.state = AppState::empty();
        self.state.rev = previous_rev;
        self.clear_persistence_best_effort();
        self.emit_state();
    }
}
