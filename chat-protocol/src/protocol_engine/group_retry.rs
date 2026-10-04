const GROUP_SENDER_KEY_RETRY_LIMIT: usize = 8;
const GROUP_SENDER_KEY_TRIALS_PER_TURN: usize = 256;
const GROUP_SENDER_KEY_RETRY_SLICE: std::time::Duration = std::time::Duration::from_millis(25);

#[derive(Clone, PartialEq, Eq)]
struct GroupSenderKeyRetryInput {
    group_id: String,
    protocol: GroupProtocol,
    members: Vec<NdrOwnerPubkey>,
    revision: u64,
    sender_owner: NdrOwnerPubkey,
    sender_device: NdrDevicePubkey,
    is_local_sending_chain: bool,
    states: Vec<nostr_double_ratchet::SenderKeyState>,
    distributions: Vec<nostr_double_ratchet::SenderKeyDistribution>,
}

/// Scheduling is ephemeral; the existing durable ciphertext queue is authoritative.
/// One input change admits each affected candidate once, in FIFO order. A failed
/// blind key search cannot run again until its actual decryption inputs change.
#[derive(Clone)]
struct ProtocolGroupSenderKeyRetry {
    inputs: BTreeMap<NdrDevicePubkey, GroupSenderKeyRetryInput>,
    ready: std::collections::VecDeque<(NdrDevicePubkey, String)>,
    queued: HashSet<String>,
    started: Option<std::time::Instant>,
    attempts: usize,
    key_trials_remaining: usize,
    active: Option<ActiveGroupSenderKeyDecrypt>,
    prepared: Option<(String, nostr_double_ratchet::GroupSenderKeyReceivePlan)>,
    yielded: bool,
    #[cfg(test)]
    total_attempts: usize,
    #[cfg(test)]
    receive_checkpoints: usize,
}

impl Default for ProtocolGroupSenderKeyRetry {
    fn default() -> Self {
        Self {
            inputs: BTreeMap::new(),
            ready: Default::default(),
            queued: HashSet::new(),
            started: None,
            attempts: 0,
            key_trials_remaining: GROUP_SENDER_KEY_TRIALS_PER_TURN,
            active: None,
            prepared: None,
            yielded: false,
            #[cfg(test)]
            total_attempts: 0,
            #[cfg(test)]
            receive_checkpoints: 0,
        }
    }
}

impl ProtocolGroupSenderKeyRetry {
    fn reset_budget(&mut self) {
        self.started = None;
        self.attempts = 0;
        self.key_trials_remaining = GROUP_SENDER_KEY_TRIALS_PER_TURN;
        self.yielded = false;
    }

    fn next(&mut self) -> Option<(NdrDevicePubkey, String)> {
        if self.attempts >= GROUP_SENDER_KEY_RETRY_LIMIT
            || self
                .started
                .is_some_and(|start| start.elapsed() >= GROUP_SENDER_KEY_RETRY_SLICE)
        {
            return None;
        }
        let active_fingerprint = self
            .prepared
            .as_ref()
            .map(|(fingerprint, _)| fingerprint)
            .or_else(|| self.active.as_ref().map(|active| &active.fingerprint));
        let active_index = active_fingerprint.and_then(|fingerprint| {
            self.ready
                .iter()
                .position(|(_, queued)| queued == fingerprint)
        });
        let next = active_index
            .and_then(|index| self.ready.remove(index))
            .or_else(|| self.ready.pop_front())?;
        self.queued.remove(&next.1);
        self.started.get_or_insert_with(std::time::Instant::now);
        self.attempts += 1;
        Some(next)
    }

    fn record_attempt(&mut self) {
        #[cfg(test)]
        {
            self.total_attempts += 1;
        }
    }
}

impl ProtocolEngine {
    fn refresh_group_sender_key_retry_inputs(&self) {
        let mut retry = self.group_sender_key_retry.borrow_mut();
        if self.pending_group_sender_key_messages.is_empty() {
            retry.active = None;
            retry.prepared = None;
            retry.inputs.clear();
            retry.ready.clear();
            retry.queued.clear();
            return;
        }
        if retry.active.is_some() || retry.prepared.is_some() {
            // A frozen search's prepared result still belongs to its old inputs.
            // Observe new keys only after applying it, so a missing-key result
            // cannot consume the wakeup for a newly arrived distribution.
            return;
        }
        // Clone once per pass, never once per pending ciphertext. Distribution
        // bookkeeping is excluded: it cannot unlock a failed key search.
        let snapshot = self.group_manager.snapshot();
        let groups = snapshot
            .groups
            .into_iter()
            .map(|group| (group.group_id.clone(), group))
            .collect::<BTreeMap<_, _>>();
        let inputs = snapshot
            .sender_keys
            .into_iter()
            .filter_map(|record| {
                let group = groups.get(&record.group_id)?;
                Some((
                    record.sender_event_pubkey,
                    GroupSenderKeyRetryInput {
                        group_id: record.group_id.clone(),
                        protocol: group.protocol,
                        members: group.members.clone(),
                        revision: group.revision,
                        sender_owner: record.sender_owner,
                        sender_device: record.sender_device,
                        is_local_sending_chain: record.sender_event_secret_key.is_some(),
                        states: record.states,
                        distributions: record.distribution_history,
                    },
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let changed = inputs
            .iter()
            .filter_map(|(author, input)| {
                (retry.inputs.get(author) != Some(input)).then_some(*author)
            })
            .collect::<HashSet<_>>();
        retry.inputs = inputs;
        for parsed in &self.pending_group_sender_key_messages {
            if !changed.contains(&parsed.sender_event_pubkey) {
                continue;
            }
            let Some(message) = self.group_sender_key_message_from_parsed(parsed) else {
                continue;
            };
            let fingerprint = group_sender_key_fingerprint(&message);
            if retry.queued.insert(fingerprint.clone()) {
                retry
                    .ready
                    .push_back((parsed.sender_event_pubkey, fingerprint));
            }
        }
    }

    fn retry_eligible_group_sender_key_messages(
        &mut self,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        if self.batch_depth.get() == 0 {
            self.group_sender_key_retry.borrow_mut().reset_budget();
        }
        if self.advance_group_sender_key_continuation()? {
            return Ok(ProtocolGroupIncomingResult::default());
        }
        if self.batch_depth.get() > 0 {
            return self.retry_eligible_group_sender_key_messages_inner(&mut Vec::new());
        }
        self.refresh_group_sender_key_retry_inputs();
        if self.group_sender_key_retry.borrow().ready.is_empty()
            && !self.pending_group_sender_key_messages.iter().any(|parsed| {
                self.unmapped_group_sender_key_candidate_is_known_message_author(parsed)
            })
        {
            return Ok(ProtocolGroupIncomingResult::default());
        }
        // Standalone callers need one atomic save for the pass. Do not clone the
        // potentially large ciphertext backlog: journal only its removals. The
        // application's existing outer batch keeps its own persistence boundary.
        let checkpoint = (
            self.group_manager.clone(),
            self.session_manager.clone(),
            self.pending_group_sender_key_repairs.clone(),
            self.pending_group_fanouts.clone(),
            self.processed_group_sender_key_messages.clone(),
            self.group_sender_key_retry.borrow().clone(),
            self.batch_persist_dirty.get(),
            self.pending_decrypted_deliveries.clone(),
        );
        let mut removed = Vec::new();
        let remaining_key_trials = self.group_sender_key_retry.borrow().key_trials_remaining;
        self.enter_batch();
        self.group_sender_key_retry
            .borrow_mut()
            .key_trials_remaining = remaining_key_trials;
        let outcome = self.retry_eligible_group_sender_key_messages_inner(&mut removed);
        let outcome = outcome.and_then(|result| self.exit_batch().map(|()| result));
        if outcome.is_err() {
            self.batch_depth.set(0);
            self.group_manager = checkpoint.0;
            self.session_manager = checkpoint.1;
            self.pending_group_sender_key_repairs = checkpoint.2;
            self.pending_group_fanouts = checkpoint.3;
            self.processed_group_sender_key_messages = checkpoint.4;
            self.group_sender_key_retry.replace(checkpoint.5);
            // A failed save may be followed by new authenticated ratchet inputs.
            // Replan the still-queued candidate instead of restoring a stale plan.
            self.group_sender_key_retry.borrow_mut().prepared = None;
            self.batch_persist_dirty.set(checkpoint.6);
            self.pending_decrypted_deliveries = checkpoint.7;
            for (index, parsed) in removed.into_iter().rev() {
                self.pending_group_sender_key_messages.insert(index, parsed);
            }
            self.invalidate_known_message_author_cache();
        }
        outcome
    }

    fn retry_eligible_group_sender_key_messages_inner(
        &mut self,
        removed: &mut Vec<(
            usize,
            nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
        )>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        for index in (0..self.pending_group_sender_key_messages.len()).rev() {
            if self.unmapped_group_sender_key_candidate_is_known_message_author(
                &self.pending_group_sender_key_messages[index],
            ) {
                removed.push((index, self.pending_group_sender_key_messages.remove(index)));
                self.persist()?;
            }
        }
        self.refresh_group_sender_key_retry_inputs();
        if self.batch_depth.get() == 0 {
            self.group_sender_key_retry.borrow_mut().reset_budget();
        }
        let repairs_before = self.pending_group_sender_key_repairs.clone();
        let mut result = ProtocolGroupIncomingResult::default();
        loop {
            let next = self.group_sender_key_retry.borrow_mut().next();
            let Some((author, fingerprint)) = next else {
                break;
            };
            let Some(index) = self
                .pending_group_sender_key_messages
                .iter()
                .position(|parsed| {
                    parsed.sender_event_pubkey == author
                        && self
                            .group_sender_key_message_from_parsed(parsed)
                            .is_some_and(|message| {
                                group_sender_key_fingerprint(&message) == fingerprint
                            })
                })
            else {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                if retry
                    .prepared
                    .as_ref()
                    .is_some_and(|(ready, _)| ready == &fingerprint)
                {
                    retry.prepared = None;
                }
                continue;
            };
            let parsed = self.pending_group_sender_key_messages[index].clone();
            let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
                continue;
            };
            let discard = self.local_owner_is_inactive_for_group(&message.group_id)
                || self
                    .group_sender_key_retry
                    .borrow()
                    .inputs
                    .get(&author)
                    .is_some_and(|input| {
                        !input.members.contains(&self.local_owner)
                            || input
                                .distributions
                                .iter()
                                .map(|distribution| distribution.created_at.get())
                                .min()
                                .is_some_and(|first| parsed.created_at.get() < first)
                    });
            if discard {
                removed.push((index, self.pending_group_sender_key_messages.remove(index)));
                self.persist()?;
                continue;
            }
            {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                if retry.active.is_none() && retry.prepared.is_none() {
                    retry.record_attempt();
                }
            }
            // Leave the durable candidate in place on any error, including a
            // persistence failure. Only successful consumption removes it.
            let outcome = match self.handle_group_sender_key_message(message) {
                Ok(outcome) => outcome,
                Err(error) if error.downcast_ref::<StorageError>().is_some() => {
                    let mut retry = self.group_sender_key_retry.borrow_mut();
                    if retry.queued.insert(fingerprint.clone()) {
                        retry.ready.push_back((author, fingerprint));
                    }
                    return Err(error);
                }
                // A malformed ciphertext must not block other streams or spin
                // unchanged. Keep it durable for a later change in inputs.
                Err(_) => continue,
            };
            if self.group_sender_key_retry.borrow().yielded {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                if retry.queued.insert(fingerprint.clone()) {
                    if retry.active.is_some() {
                        retry.ready.push_front((author, fingerprint));
                    } else {
                        retry.ready.push_back((author, fingerprint));
                    }
                }
                break;
            }
            if !outcome.pending {
                removed.push((index, self.pending_group_sender_key_messages.remove(index)));
                self.persist()?;
            }
            result.events.extend(outcome.events);
            result.effects.extend(outcome.effects);
        }
        if self.pending_group_sender_key_repairs != repairs_before {
            self.persist()?;
        }
        Ok(result)
    }
}
