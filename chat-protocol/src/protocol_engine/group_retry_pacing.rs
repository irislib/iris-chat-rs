/// Resource policy for automatic history recovery, separate from ratchet state
/// and rollback checkpoints. Time spent on a failed transaction is still work.
#[derive(Default)]
struct GroupRetryPacing {
    enabled: bool,
    not_before: Option<std::time::Instant>,
    fresh_not_before: Option<std::time::Instant>,
    #[cfg(test)]
    clock: Option<std::time::Instant>,
    #[cfg(test)]
    key_trials: usize,
    #[cfg(test)]
    work_calls: usize,
    #[cfg(test)]
    fresh_key_trials: usize,
    #[cfg(test)]
    fresh_work_calls: usize,
}

impl GroupRetryPacing {
    fn now(&self) -> std::time::Instant {
        #[cfg(test)]
        if let Some(now) = self.clock {
            return now;
        }
        std::time::Instant::now()
    }

    fn delay(&self) -> std::time::Duration {
        if !self.enabled {
            return std::time::Duration::ZERO;
        }
        self.not_before
            .map_or(std::time::Duration::ZERO, |deadline| {
                deadline.saturating_duration_since(self.now())
            })
    }

    fn record_work(&mut self, elapsed: std::time::Duration) {
        if self.enabled {
            #[cfg(test)]
            if let Some(now) = self.clock {
                // Explicit virtual clocks advance through the measured work as
                // well as the cooldown; latency evidence must count both.
                self.clock = now.checked_add(elapsed);
            }
            // At most 2% sustained blind-search duty, leaving room for the rest
            // of the application. This is wall work, conservatively including
            // preemption; it is not a claim about total application CPU usage.
            self.not_before = self
                .not_before
                .unwrap_or_else(|| self.now())
                .max(self.now())
                .checked_add(elapsed.saturating_mul(49));
        }
    }

    fn fresh_delay(&self) -> std::time::Duration {
        self.fresh_not_before
            .map_or(std::time::Duration::ZERO, |deadline| {
                deadline.saturating_duration_since(self.now())
            })
    }

    fn record_fresh_work(&mut self, elapsed: std::time::Duration) {
        // A small current-position probe may run while history is cooling down.
        // Its measured cost also extends the GLOBAL debt: sustained fresh work
        // replaces history work instead of creating another ongoing CPU budget.
        self.record_work(elapsed);
        self.fresh_not_before = self.now().checked_add(elapsed.saturating_mul(49));
    }
}

impl ProtocolEngine {
    /// Pace automatic blind history recovery by its measured work cost.
    /// Manual protocol drivers retain their existing per-call work bounds unless
    /// they opt in. Hosts must use `group_sender_key_retry_delay` when scheduling
    /// continuations; calls from other events obey the same internal work gate.
    pub fn enable_background_history_recovery_pacing(&self) {
        self.group_retry_pacing.borrow_mut().enabled = true;
    }

    /// Earliest useful blind-search continuation. Prepared receive plans can be
    /// applied immediately; membership and ratchet validation remain mandatory.
    pub fn group_sender_key_retry_delay(&self) -> std::time::Duration {
        let retry = self.group_sender_key_retry.borrow();
        if retry.prepared.is_some() {
            return std::time::Duration::ZERO;
        }
        let pacing = self.group_retry_pacing.borrow();
        if retry.fresh.is_empty() {
            pacing.delay()
        } else {
            pacing.delay().min(pacing.fresh_delay())
        }
    }
}
