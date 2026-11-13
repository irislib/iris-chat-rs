use super::*;
use nostr::JsonUtil;

impl AppCore {
    pub(in crate::core) fn publish_runtime_event(
        &mut self,
        event: Event,
        label: &'static str,
        completion: Option<(String, String)>,
    ) -> bool {
        let (inner_event_id, chat_id) = completion
            .map(|(inner_event_id, chat_id)| (Some(inner_event_id), Some(chat_id)))
            .unwrap_or((None, None));
        self.publish_runtime_event_with_metadata(event, label, chat_id, inner_event_id, None, None)
    }

    pub(in crate::core) fn publish_protocol_event(&mut self, publish: ProtocolPublish) -> bool {
        // Retry batches may already contain sealed events when a block arrives.
        // Guard before call push as well as before the durable/network outboxes.
        if self.blocked_direct_publication(&publish.chat_id)
            || !self.protocol_publication_allowed(&publish)
        {
            return false;
        }
        self.publish_call_push_for_protocol(&publish);
        let pairwise_device = publish.carrier.pairwise_device();
        let label = if publish.carrier == ProtocolCarrier::SharedGroup {
            GROUP_DATA_PUBLISH_LABEL
        } else {
            APPCORE_PROTOCOL_LABEL
        };
        self.publish_runtime_event_with_metadata(
            publish.event,
            label,
            Some(publish.chat_id),
            publish.inner_event_id,
            publish.authored_at_secs,
            pairwise_device,
        )
    }

    /// Commit a shared group event and all its private wakes before any carrier
    /// can acknowledge the shared event. Restart then retains every recipient.
    pub(in crate::core) fn persist_group_publish_batch(
        &mut self,
        effects: &[ProtocolEffect],
    ) -> bool {
        let Some(local) = &self.logged_in else {
            return true;
        };
        let owner = local.owner_pubkey;
        if !effects.iter().any(|effect| {
            let ProtocolEffect::Publish(publish) = effect;
            // Provenance, not tags: shared group ciphertext and pairwise
            // envelopes are indistinguishable on the wire.
            publish.carrier == ProtocolCarrier::SharedGroup
                || self.protocol_engine.as_ref().is_some_and(|engine| {
                    engine.has_pending_group_publish(&publish.event.id.to_hex())
                })
        }) {
            return true;
        }
        let pending = effects
            .iter()
            .filter_map(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                if !self.protocol_publication_allowed(publish)
                    || self.blocked_direct_publication(&publish.chat_id)
                    || self.is_removed_from_group(&publish.chat_id)
                {
                    return None;
                }
                Some((
                    PendingRelayPublish {
                        owner_pubkey_hex: owner.to_hex(),
                        event_id: publish.event.id.to_hex(),
                        label: if publish.carrier == ProtocolCarrier::SharedGroup {
                            GROUP_DATA_PUBLISH_LABEL
                        } else {
                            APPCORE_PROTOCOL_LABEL
                        }
                        .to_string(),
                        event_json: publish.event.as_json(),
                        inner_event_id: publish.inner_event_id.clone(),
                        chat_id: Some(publish.chat_id.clone()),
                        created_at_secs: publish.event.created_at.as_secs(),
                        attempt_count: 0,
                        last_error: None,
                        pairwise_device_hex: publish
                            .carrier
                            .pairwise_device()
                            .map(|device| device.to_hex()),
                    },
                    publish.authored_at_secs,
                ))
            })
            .collect::<Vec<_>>();
        let committed = match self.app_store.insert_pending_relay_publish_batch(&pending) {
            Ok(committed) => committed,
            Err(error) => {
                self.push_debug_log("group.private_wake.queue", error.to_string());
                return false;
            }
        };
        // Startup may reoffer protocol work before loading the application
        // outbox. Reuse its exact retry state instead of creating fresh rows.
        for pending in committed {
            self.pending_relay_publishes
                .entry(pending.event_id.clone())
                .or_insert(pending);
        }
        if let Some(engine) = self.protocol_engine.as_mut() {
            if let Err(error) = engine.ack_group_publish_effects(effects) {
                // The application outbox already owns the entire batch. A
                // repeated protocol offer uses the same IDs and is harmless.
                self.push_debug_log("group.private_wake.ack", error.to_string());
            }
        }
        true
    }

    pub(super) fn remember_pending_relay_publish_with_authored_at(
        &mut self,
        event: &Event,
        label: &str,
        chat_id: Option<String>,
        inner_event_id: Option<String>,
        authored_at_secs: Option<u64>,
        pairwise_device: Option<PublicKey>,
    ) -> Option<PendingPublishChange> {
        // The outbox feeds every carrier. A message event enters it only as
        // a pairwise envelope with a known target, never on its shape alone.
        if !self.private_publication_allowed(event)
            || (event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
                && pairwise_device.is_none()
                && !self.local_group_data_publication(event, label, chat_id.as_deref()))
        {
            return None;
        }
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
            pairwise_device_hex: pairwise_device.map(|device| device.to_hex()),
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
            pending.pairwise_device_hex = existing
                .pairwise_device_hex
                .clone()
                .or(pending.pairwise_device_hex);
            if &pending == existing && authored_at_secs.is_none() {
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
        if let Err(error) = self
            .app_store
            .upsert_pending_relay_publish_with_authored_at(&pending, authored_at_secs)
        {
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
