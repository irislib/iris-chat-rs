use super::*;

impl AppCore {
    pub(super) fn replay_pending_nearby_publishes(&mut self) {
        if self.pending_relay_publishes.is_empty() || self.device_sync.is_none() {
            return;
        }
        // Every new protocol publication and every relay drain completion reaches
        // this path. Replaying 64 older events each time amplifies a repair burst
        // and monopolizes the core thread before Send can update the chat.
        // New events already go directly through emit_nearby_published_event;
        // only backlog replay shares the regular retry cadence.
        let now = Instant::now();
        if self
            .relay_transport_runtime
            .nearby_replay_started_at
            .is_some_and(|last| {
                now.saturating_duration_since(last)
                    < Duration::from_secs(PROTOCOL_RECONNECT_CHECK_SECS)
            })
        {
            return;
        }
        self.relay_transport_runtime.nearby_replay_started_at = Some(now);
        self.replay_mesh_outbox();
        let nearby_events = self
            .pending_relay_publishes
            .values()
            .rev()
            .filter(|pending| {
                !pending
                    .chat_id
                    .as_deref()
                    .is_some_and(|chat| self.blocked_direct_publication(chat))
            })
            .take(64)
            .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
            .collect::<Vec<_>>();
        for event in &nearby_events {
            self.publish_fips_nearby(event);
        }
    }
}
