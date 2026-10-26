use super::*;
use nostr::JsonUtil;

impl AppCore {
    pub(in crate::core) fn capture_authorization_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<()> {
        if self.authorization_record_allowed(event) {
            self.store_sync_record(&DeviceSyncRecord::Authorization {
                event: event.clone(),
            })?;
        }
        Ok(())
    }

    pub(super) fn authorization_record_allowed(&self, event: &Event) -> bool {
        if event.verify().is_err()
            || event.as_json().len() > 64 * 1024
            || event.created_at.as_secs() > unix_now().get().saturating_add(300)
        {
            return false;
        }
        AppKeys::from_event(event).is_ok()
            || nostr_double_ratchet::parse_group_roster_fact_event(event).is_ok()
    }

    pub(super) fn merge_authorization_event(&mut self, event: &Event) -> bool {
        self.project_authorization_event(event).unwrap_or(false) && self.persist_app_state().is_ok()
    }

    // Restore coalesces app snapshots after replaying all durable heads. The
    // live merge above still requires immediate app durability before success.
    pub(super) fn project_authorization_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        // The authenticated sibling already admitted this original. Retain it
        // intact; the protocol engine still decides whether it advances authority.
        self.capture_authorization_event(event)?;
        if AppKeys::from_event(event).is_ok() {
            return self.apply_app_keys_event(event);
        }
        let Some(engine) = self.protocol_engine.as_mut() else {
            return Ok(false);
        };
        let result = engine.ingest_group_roster_fact_event(event)?;
        if let Some(result) = result {
            if let Some(snapshot) = result.snapshot {
                let previous = self.groups.get(&snapshot.group_id).cloned();
                self.apply_group_roster_snapshot(
                    snapshot.clone(),
                    unix_now().get().max(snapshot.updated_at.get()),
                );
                self.apply_group_metadata_notice(previous.as_ref(), &snapshot);
                self.request_protocol_subscription_refresh();
            }
            self.process_protocol_engine_retry_batch(
                "device_sync.authorization",
                result.retry_batch,
            );
        }
        Ok(true)
    }
}
