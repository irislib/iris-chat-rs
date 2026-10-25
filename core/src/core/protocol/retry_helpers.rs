use super::*;

impl AppCore {
    pub(in crate::core) fn has_pending_protocol_engine_retry_work(&self) -> bool {
        self.pending_outgoing_invite_acceptance.is_some()
            || self.pending_private_invite_cleanup_retry
            || self.has_queued_direct_text_messages()
            || self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_pending_retry_work())
    }

    pub(in crate::core) fn schedule_fast_protocol_retry_if_pending(&mut self) {
        if self.has_ready_protocol_retry_work() {
            let delay = self
                .protocol_engine
                .as_ref()
                .map_or(Duration::ZERO, |engine| {
                    engine.group_sender_key_retry_delay()
                })
                .max(Duration::from_millis(25));
            let due_at = Instant::now() + delay;
            if self
                .protocol_subscription_runtime
                .ready_retry_due_at
                .is_none_or(|previous| due_at < previous)
            {
                // Newly arrived messages can have an earlier work allowance
                // than old history. The due_at identity rejects the superseded
                // callback; equal/later requests still share the current timer.
                self.protocol_subscription_runtime.ready_retry_due_at = Some(due_at);
                let tx = self.core_sender.clone();
                self.runtime.spawn(async move {
                    sleep_until(due_at).await;
                    let _ = tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::RetryReadyProtocolWork { due_at },
                    )));
                });
            }
        } else {
            self.protocol_subscription_runtime.ready_retry_due_at = None;
        }
        if self.has_pending_protocol_engine_retry_work()
            || (!self.pending_relay_publishes.is_empty()
                && self
                    .logged_in
                    .as_ref()
                    .is_some_and(|session| !session.relay_urls.is_empty()))
            || self.has_mesh_outbox_work()
            || self.has_mesh_protocol_retry_work()
        {
            self.schedule_protocol_subscription_liveness_check(Duration::from_secs(
                PROTOCOL_RECONNECT_CHECK_SECS,
            ));
        }
    }

    fn has_ready_protocol_retry_work(&self) -> bool {
        !self.suspended
            && self.logged_in.is_some()
            && self
                .protocol_engine
                .as_ref()
                .is_some_and(|engine| engine.has_ready_group_sender_key_retry_work())
    }

    pub(in crate::core) fn handle_ready_protocol_retry(&mut self, due_at: Instant) {
        if self.protocol_subscription_runtime.ready_retry_due_at != Some(due_at) {
            return;
        }
        self.protocol_subscription_runtime.ready_retry_due_at = None;
        if self.has_ready_protocol_retry_work() {
            self.retry_protocol_engine_pending_work("ready_continuation");
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
            || self.has_mesh_protocol_liveness_work()
    }
}
