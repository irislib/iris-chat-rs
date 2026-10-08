use super::*;

impl AppCore {
    pub(in crate::core) fn blocked_direct_publication(&self, chat_id: &str) -> bool {
        !is_group_chat_id(chat_id) && self.is_owner_blocked(chat_id)
    }

    pub(in crate::core) fn cancel_relay_publish_drain(&mut self) {
        // Dropping the task aborts its JoinSet as well as the not-yet-started
        // remainder of the cloned batch. Unaffected durable rows remain retryable.
        self.relay_transport_runtime.publish_drain_task.take();
        self.relay_transport_runtime.publish_drain_token = self
            .relay_transport_runtime
            .publish_drain_token
            .wrapping_add(1);
        self.relay_transport_runtime.publish_drain_in_flight = false;
        self.relay_transport_runtime.publish_drain_dirty = false;
        self.relay_transport_runtime.publish_drain_started_at = None;
        self.relay_transport_runtime.publish_drain_failed_count = 0;
        self.pending_relay_publish_inflight.clear();
    }

    fn cancel_direct_publications(&mut self, target: &str, through: Option<u64>) -> bool {
        self.cancel_pending_publication_tasks(target);
        let Some(owner) = self
            .logged_in
            .as_ref()
            .map(|local| local.owner_pubkey.to_hex())
        else {
            return true;
        };
        let interrupted = self.pending_relay_publishes.values().any(|pending| {
            pending.chat_id.as_deref() == Some(target)
                && self
                    .pending_relay_publish_inflight
                    .contains(&pending.event_id)
        });
        if interrupted {
            self.cancel_relay_publish_drain();
        }
        let ids = match self
            .app_store
            .retire_pending_direct_publishes(&owner, target, through)
        {
            Ok(stored) => stored,
            Err(error) => {
                self.push_debug_log("block.cancel_failed", error.to_string());
                return false;
            }
        };
        if let Some(thread) = self.threads.get_mut(target) {
            for message in &mut thread.messages {
                if message.is_outgoing
                    && matches!(message.delivery, DeliveryState::Queued)
                    && through.is_none_or(|cutoff| message.created_at_secs <= cutoff)
                {
                    message.delivery = DeliveryState::Failed;
                    message.delivery_trace.pending_relay_event_ids.clear();
                }
            }
        }
        for id in ids {
            self.pending_relay_publishes.remove(&id);
            self.pending_relay_publish_inflight.remove(&id);
            if let Some(mesh) = &self.device_sync {
                if let Ok(mut outbox) = mesh.nearby_outbox.write() {
                    outbox.forget(&id);
                }
            }
        }
        if !self.pending_relay_publishes.is_empty() {
            // A historical transition can cancel only a carrier task while
            // retaining its newer row. Retry it even if no row was retired.
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
        true
    }

    pub(in crate::core) fn cancel_direct_sends_for_block(
        &mut self,
        target: &str,
        through: Option<u64>,
    ) -> bool {
        if !self.cancel_direct_publications(target, through) {
            return false;
        }
        let Ok(peer) = PublicKey::from_hex(target) else {
            return false;
        };
        if let Some(mut engine) = self.protocol_engine.take() {
            if let Err(error) = engine.retire_pending_direct_sends(peer, through) {
                // Keep the engine unavailable if the durable retirement failed.
                // Startup reprojects persisted blocks before exposing sends again.
                self.push_debug_log("block.cancel_failed", error.to_string());
                return false;
            }
            self.protocol_engine = Some(engine);
        }
        true
    }
}
