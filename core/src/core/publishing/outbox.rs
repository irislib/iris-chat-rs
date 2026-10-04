use super::*;

impl AppCore {
    pub(super) fn remember_pending_relay_publish(
        &mut self,
        event: &Event,
        label: &str,
        chat_id: Option<String>,
        inner_event_id: Option<String>,
    ) -> Option<PendingPublishChange> {
        let logged_in = self.logged_in.as_ref()?;
        let owner_pubkey_hex = logged_in.owner_pubkey.to_hex();
        let event_json = match serde_json::to_string(event) {
            Ok(json) => json,
            Err(error) => {
                self.push_debug_log("publish.runtime.queue", format!("serialize_failed={error}"));
                return None;
            }
        };
        let mut pending = PendingRelayPublish {
            owner_pubkey_hex,
            event_id: event.id.to_string(),
            label: label.to_string(),
            event_json,
            inner_event_id,
            chat_id,
            created_at_secs: event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        };
        let existing = self.pending_relay_publishes.get(&pending.event_id);
        if let Some(existing) = existing {
            // The durable outbox owns this signed event's retry lifecycle.
            // Replayed protocol effects may add delivery metadata, but must
            // not reset attempts, erase existing metadata, or rewrite the event.
            pending.event_json = existing.event_json.clone();
            pending.attempt_count = existing.attempt_count;
            pending.last_error = existing.last_error.clone();
            pending.inner_event_id = pending
                .inner_event_id
                .or_else(|| existing.inner_event_id.clone());
            pending.chat_id = pending.chat_id.or_else(|| existing.chat_id.clone());
            if &pending == existing {
                return Some(PendingPublishChange::Existing);
            }
        }
        let change = if existing.is_some() {
            PendingPublishChange::Existing
        } else {
            PendingPublishChange::Inserted
        };
        if !self.prune_or_skip_superseded_app_keys_publish(event) {
            return None;
        }
        if !self.prune_or_skip_superseded_protocol_invite_response_publish(&pending, event) {
            return None;
        }
        if !self.prune_or_skip_superseded_local_invite_publish(&pending, event) {
            return None;
        }
        if let Err(error) = self.app_store.upsert_pending_relay_publish(&pending) {
            self.push_debug_log("publish.runtime.queue", format!("store_failed={error}"));
            return None;
        }
        if !self.prune_stored_superseded_protocol_control_publish(&pending, event) {
            return None;
        }
        if let (Some(message_id), Some(chat_id)) = (
            pending.inner_event_id.as_deref(),
            pending.chat_id.as_deref(),
        ) {
            self.record_message_outer_event(chat_id, message_id, &pending.event_id);
        }
        self.pending_relay_publishes
            .insert(pending.event_id.clone(), pending);
        if let Some(pending) = self.pending_relay_publishes.get(&event.id.to_string()) {
            if let (Some(message_id), Some(chat_id)) =
                (pending.inner_event_id.clone(), pending.chat_id.clone())
            {
                self.sync_message_delivery_trace(&chat_id, &message_id);
            }
        }
        self.prune_pending_relay_control_publish_backlog_to_limit(
            PENDING_RELAY_CONTROL_PUBLISH_MAX_ROWS,
            "enqueue",
        );
        Some(change)
    }
}
