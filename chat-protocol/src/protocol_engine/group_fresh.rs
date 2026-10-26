const GROUP_FRESH_QUEUE_LIMIT: usize = 32;
const GROUP_FRESH_KEY_TRIALS: usize = 4;

#[derive(Clone)]
struct FreshGroupSenderKeyCandidate {
    fingerprint: String,
    parsed: Arc<nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent>,
    pending_index: usize,
}

impl ProtocolEngine {
    fn admit_fresh_group_sender_key_candidate(
        &self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
        pending_index: usize,
    ) {
        if !self.group_retry_pacing.borrow().enabled || parsed.encrypted_header.is_none() {
            return;
        }
        let Some(message) = self.group_sender_key_message_from_parsed(parsed) else {
            return;
        };
        let fingerprint = group_sender_key_fingerprint(&message);
        let mut retry = self.group_sender_key_retry.borrow_mut();
        if retry.fresh.len() < GROUP_FRESH_QUEUE_LIMIT
            && !retry
                .fresh
                .iter()
                .any(|fresh| fresh.fingerprint == fingerprint)
        {
            // Only ingress admits a new candidate. Restored queues and duplicate
            // events cannot repeatedly acquire this reserved, bounded allowance.
            retry.fresh.push_back(FreshGroupSenderKeyCandidate {
                fingerprint,
                parsed: Arc::new(parsed.clone()),
                pending_index,
            });
        }
    }

    fn fresh_group_sender_key_work_is_due(&self) -> bool {
        !self.group_sender_key_retry.borrow().fresh.is_empty()
            && self.group_retry_pacing.borrow().fresh_delay().is_zero()
    }

    fn pending_group_sender_key_retry_index(
        &self,
        author: NdrDevicePubkey,
        fingerprint: &str,
    ) -> Option<usize> {
        let mut retry = self.group_sender_key_retry.borrow_mut();
        if let Some(fresh) = retry
            .fresh
            .iter_mut()
            .find(|fresh| fresh.fingerprint == fingerprint)
        {
            if self
                .pending_group_sender_key_messages
                .get(fresh.pending_index)
                .is_some_and(|parsed| parsed == fresh.parsed.as_ref())
            {
                return Some(fresh.pending_index);
            }
            // Queue removals can move the index; compare parsed bytes only in
            // that case, never rehash every ciphertext on each fresh receive.
            let index = self
                .pending_group_sender_key_messages
                .iter()
                .position(|parsed| parsed == fresh.parsed.as_ref())?;
            fresh.pending_index = index;
            return Some(index);
        }
        if let Some(active) = retry
            .active
            .as_mut()
            .filter(|active| active.fingerprint == fingerprint)
        {
            if self
                .pending_group_sender_key_messages
                .get(active.pending_index)
                .is_some_and(|parsed| active.matches(parsed))
            {
                return Some(active.pending_index);
            }
            let index = self
                .pending_group_sender_key_messages
                .iter()
                .position(|parsed| active.matches(parsed))?;
            active.pending_index = index;
            return Some(index);
        }
        self.pending_group_sender_key_messages
            .iter()
            .position(|parsed| {
                parsed.sender_event_pubkey == author
                    && self
                        .group_sender_key_message_from_parsed(parsed)
                        .is_some_and(|message| {
                            group_sender_key_fingerprint(&message) == fingerprint
                        })
            })
    }

    fn prepare_fresh_group_sender_key_message(
        &self,
        message: &GroupSenderKeyMessage,
    ) -> nostr_double_ratchet::Result<Option<nostr_double_ratchet::GroupSenderKeyReceivePlan>> {
        let mut remaining = GROUP_FRESH_KEY_TRIALS;
        let started = std::time::Instant::now();
        let result = self
            .group_manager
            .plan_sender_key_message_frontier_with_budget(message, &mut remaining);
        let elapsed = started.elapsed();
        let mut pacing = self.group_retry_pacing.borrow_mut();
        pacing.record_fresh_work(elapsed);
        #[cfg(test)]
        {
            pacing.fresh_key_trials += GROUP_FRESH_KEY_TRIALS - remaining;
            pacing.fresh_work_calls += 1;
        }
        // A miss leaves the durable candidate and any old history cursor intact.
        // A hit returns the usual receive plan for authenticated atomic apply.
        result
    }
}
