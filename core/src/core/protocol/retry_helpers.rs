use super::*;

impl AppCore {
    pub(in crate::core) fn has_pending_protocol_engine_retry_work(&self) -> bool {
        self.protocol_engine
            .as_ref()
            .is_some_and(|engine| engine.has_pending_retry_work())
    }

    pub(in crate::core) fn schedule_fast_protocol_retry_if_pending(&mut self) {
        if self.has_pending_protocol_engine_retry_work() || !self.pending_relay_publishes.is_empty()
        {
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
    }

    pub(in crate::core) fn has_protocol_liveness_work(&self) -> bool {
        self.protocol_subscription_runtime.desired_plan.is_some()
            || self.protocol_subscription_runtime.applying_plan.is_some()
            || self.protocol_subscription_runtime.applied_plan.is_some()
            || self.protocol_subscription_runtime.refresh_in_flight
            || self.protocol_subscription_runtime.refresh_dirty
            || !self.pending_relay_publishes.is_empty()
            || self.has_pending_protocol_engine_retry_work()
    }

    pub(in crate::core) fn tracked_peer_protocol_backfill_needed(&self) -> bool {
        let tracked_peer_owners = self.tracked_peer_owner_hexes();
        if tracked_peer_owners.is_empty() {
            return false;
        }

        tracked_peer_owners
            .iter()
            .any(|owner_hex| !self.app_keys.contains_key(owner_hex))
            || self.protocol_engine.as_ref().is_some_and(|engine| {
                tracked_peer_owners.iter().any(|owner_hex| {
                    PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                        let owner_prefix = owner_pubkey.to_hex();
                        engine
                            .queued_message_diagnostics(None)
                            .iter()
                            .any(|target| target == &owner_prefix)
                    })
                })
            })
            || self.protocol_engine.as_ref().is_some_and(|engine| {
                tracked_peer_owners.iter().any(|owner_hex| {
                    PublicKey::parse(owner_hex).is_ok_and(|owner_pubkey| {
                        engine
                            .message_author_pubkeys_for_owner(owner_pubkey)
                            .is_empty()
                    })
                })
            })
    }

    pub(in crate::core) fn current_queued_protocol_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        if let Some(protocol_engine) = self.protocol_engine.as_ref() {
            targets.extend(protocol_engine.queued_message_diagnostics(None));
            targets.extend(protocol_engine.queued_owner_claim_targets());
            targets.extend(protocol_engine.queued_group_target_hexes());
        }
        targets.sort();
        targets.dedup();
        targets
    }
}
