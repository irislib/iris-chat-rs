#[derive(Clone)]
struct ActiveGroupSenderKeyDecrypt {
    fingerprint: String,
    cursor: nostr_double_ratchet::GroupSenderKeyDecryptCursor,
    message: GroupSenderKeyMessage,
    // Validated against the durable queue before each continuation. Only a queue
    // mutation can force a scan; ordinary slices touch just this one ciphertext.
    pending_index: usize,
}

impl ActiveGroupSenderKeyDecrypt {
    fn matches(
        &self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
    ) -> bool {
        self.message.sender_event_pubkey == parsed.sender_event_pubkey
            && self.message.created_at == parsed.created_at
            && self.message.key_id == parsed.key_id
            && self.message.message_number == parsed.message_number
            && self.message.encrypted_header == parsed.encrypted_header
            && self.message.ciphertext == parsed.ciphertext
    }
}

impl ProtocolEngine {
    fn prepare_group_sender_key_message(
        &mut self,
        message: &GroupSenderKeyMessage,
        fingerprint: &str,
    ) -> nostr_double_ratchet::Result<Option<nostr_double_ratchet::GroupSenderKeyReceivePlan>> {
        {
            let mut retry = self.group_sender_key_retry.borrow_mut();
            retry.yielded = false;
            if retry
                .prepared
                .as_ref()
                .is_some_and(|(ready, _)| ready == fingerprint)
            {
                return Ok(retry.prepared.take().map(|(_, plan)| plan));
            }
        }
        if message.encrypted_header.is_none() {
            let mut remaining = usize::MAX;
            return self.group_manager.plan_sender_key_message_with_budget(
                message.clone(),
                &mut None,
                &mut remaining,
            );
        }
        let (active, mut remaining) = {
            let mut retry = self.group_sender_key_retry.borrow_mut();
            if retry
                .active
                .as_ref()
                .is_some_and(|active| active.fingerprint != fingerprint)
                || retry.key_trials_remaining == 0
            {
                retry.yielded = true;
                return Ok(None);
            }
            (retry.active.take(), retry.key_trials_remaining)
        };
        let (mut cursor, cached_message, pending_index) = active.map_or_else(
            || (None, message.clone(), usize::MAX),
            |active| (Some(active.cursor), active.message, active.pending_index),
        );
        let result = self.group_manager.plan_sender_key_message_with_budget(
            message.clone(),
            &mut cursor,
            &mut remaining,
        );
        let mut retry = self.group_sender_key_retry.borrow_mut();
        retry.key_trials_remaining = remaining;
        retry.yielded = matches!(result, Ok(None));
        retry.active = cursor.map(|cursor| ActiveGroupSenderKeyDecrypt {
            fingerprint: fingerprint.to_owned(),
            cursor,
            message: cached_message,
            pending_index,
        });
        result
    }

    /// Pure search does not clone rollback state or write a checkpoint per slice.
    /// Final ratchet application still uses the caller's atomic persistence batch.
    fn advance_group_sender_key_continuation(&mut self) -> anyhow::Result<bool> {
        let Some(mut active) = self.group_sender_key_retry.borrow_mut().active.take() else {
            return Ok(false);
        };
        let matches_cached = self
            .pending_group_sender_key_messages
            .get(active.pending_index)
            .is_some_and(|parsed| active.matches(parsed));
        if !matches_cached {
            let Some(index) = self
                .pending_group_sender_key_messages
                .iter()
                .position(|parsed| active.matches(parsed))
            else {
                return Ok(false);
            };
            active.pending_index = index;
        }
        if self.local_owner_is_inactive_for_group(&active.message.group_id)
            || self.unmapped_group_sender_key_candidate_is_known_message_author(
                &self.pending_group_sender_key_messages[active.pending_index],
            )
        {
            return Ok(false);
        }
        let fingerprint = active.fingerprint.clone();
        let message = active.message.clone();
        self.group_sender_key_retry.borrow_mut().active = Some(active);
        let result = self.prepare_group_sender_key_message(&message, &fingerprint)?;
        let mut retry = self.group_sender_key_retry.borrow_mut();
        retry.ready.retain(|(_, queued)| queued != &fingerprint);
        retry.queued.insert(fingerprint.clone());
        let pending = result.is_none();
        if let Some(plan) = result {
            retry.prepared = Some((fingerprint.clone(), plan));
        }
        if retry.active.is_some() || retry.prepared.is_some() {
            retry
                .ready
                .push_front((message.sender_event_pubkey, fingerprint));
        } else {
            // A changed active ratchet cancels this generation. Requeue behind
            // other senders so fresh traffic cannot repeatedly win the front.
            retry
                .ready
                .push_back((message.sender_event_pubkey, fingerprint));
        }
        Ok(pending)
    }
}
