use super::*;

impl AppCore {
    pub(in crate::core) fn has_pending_protocol_engine_retry_work(&self) -> bool {
        self.pending_outgoing_invite_acceptance.is_some()
            || self.pending_private_invite_cleanup_retry
            || self
                .protocol_engine
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
}
