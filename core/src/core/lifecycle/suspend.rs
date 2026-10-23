use super::*;

impl AppCore {
    pub(in crate::core) fn prepare_for_suspend(&mut self) -> bool {
        if self.calls.active.is_some() {
            return true;
        }
        // Set the gate first so any relay/internal events that arrive in
        // the FFI queue after this point (whether already-queued or sent
        // by an in-flight tokio task before disconnect lands) are dropped
        // without touching SQLite. The persist below is the only write we
        // want before iOS suspends us.
        self.suspended = true;
        self.cancel_all_publication_tasks();
        self.private_contacts.stop_network();
        self.push_debug_log("app.suspend", "pausing network and flushing storage");
        self.pause_pending_linked_device();
        // The iOS bridge retains the OS-managed radio while the core is asleep.
        // Only an acknowledged, bounded native wake may process its packets.
        if !self.keep_background_ble() {
            self.stop_device_sync();
        }
        self.device_invite_poll_token = self.device_invite_poll_token.saturating_add(1);
        self.message_expiry_token = self.message_expiry_token.saturating_add(1);
        self.chat_mute_expiry_token = self.chat_mute_expiry_token.wrapping_add(1);
        self.protocol_reconnect_token = self.protocol_reconnect_token.saturating_add(1);
        self.protocol_liveness_token = self.protocol_liveness_token.saturating_add(1);
        self.relay_status_watch_generation = self.relay_status_watch_generation.wrapping_add(1);
        self.relay_status_watch_urls.clear();
        self.relay_status_by_url.clear();
        self.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
        self.relay_transport_runtime = RelayTransportRuntime::default();
        self.cancel_people_fetches_for_suspend();
        self.reset_direct_chat_capability_runtime();
        self.profile_metadata_fetch_inflight.clear();
        self.pending_relay_publish_inflight.clear();
        self.relay_connected_count = 0;
        self.all_relays_offline_since_secs = None;
        self.debug_snapshot_write_generation = self.debug_snapshot_write_generation.wrapping_add(1);
        self.debug_snapshot_write_inflight = false;
        self.debug_snapshot_write_dirty = false;
        self.state.busy.syncing_network = false;
        self.persist_best_effort();

        if let Some(logged_in) = self.logged_in.as_ref() {
            let client = logged_in.client.clone();
            self.runtime.block_on(async move {
                let _ = tokio::time::timeout(Duration::from_millis(750), async move {
                    client.unsubscribe_all().await;
                    client.disconnect().await;
                })
                .await;
            });
        }

        #[cfg(any(target_os = "ios", test))]
        if let Some(service) = &self.attachment_service {
            if let Err(error) = service.store.suspend() {
                self.push_debug_log("attachment.suspend.error", error.to_string());
            }
        }
        let storage_idle = match self.app_store.prepare_for_suspend() {
            Ok(()) => true,
            Err(error) => {
                self.push_debug_log("storage.suspend.error", error.to_string());
                self.state.toast = Some("Iris needs to restart. Please reopen it.".to_owned());
                false
            }
        };
        self.rebuild_state();
        self.emit_state();
        storage_idle
    }
}
