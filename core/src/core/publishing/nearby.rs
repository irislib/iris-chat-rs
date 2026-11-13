use super::*;
use std::ops::Bound::{Excluded, Unbounded};

impl AppCore {
    pub(in crate::core) fn emit_nearby_published_event(&self, event: &Event) {
        if !self.carrier_publication_allowed(event) {
            return;
        }
        self.publish_fips_nearby(event);
        self.publish_mesh_event(event);
        if crate::core::fips_nearby::is_fips_nearby_bootstrap_event(event) {
            self.refresh_fips_nearby_bootstrap();
        }
        send_nearby_published_event(&self.update_tx, event);
    }

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
        // Continue after the last scanned key, even if its row was removed.
        // Advance past blocked or malformed rows without exceeding the batch.
        let pending = &self.pending_relay_publishes;
        let start = self
            .relay_transport_runtime
            .nearby_replay_cursor
            .as_ref()
            .map_or(Unbounded, Excluded);
        let batch = pending
            .range::<String, _>((start, Unbounded))
            .chain(pending.iter())
            .take(64.min(pending.len()));
        let mut last = None;
        for (id, row) in batch {
            last = Some(id);
            if row
                .chat_id
                .as_deref()
                .is_some_and(|chat| self.blocked_direct_publication(chat))
            {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<Event>(&row.event_json) {
                self.publish_fips_nearby(&event);
            }
        }
        self.relay_transport_runtime.nearby_replay_cursor = last.cloned();
    }
}
