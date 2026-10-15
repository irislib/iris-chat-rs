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
        // The authenticated sibling already admitted this original. Retain it
        // intact; the protocol engine still decides whether it advances authority.
        if self.capture_authorization_event(event).is_err() {
            return false;
        }
        if AppKeys::from_event(event).is_ok() {
            return self.apply_app_keys_event(event).unwrap_or(false)
                && self.persist_app_state().is_ok();
        }
        let Some(engine) = self.protocol_engine.as_mut() else {
            return false;
        };
        let result = match engine.ingest_group_roster_fact_event(event) {
            Ok(value) => value,
            Err(_) => return false,
        };
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
        self.persist_app_state().is_ok()
    }
}
