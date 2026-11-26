use super::*;

pub(super) const GROUP_SENDER_KEY_RETRY_LIMIT: usize = 8;
const GROUP_SENDER_KEY_TRIALS_PER_TURN: usize = 256;
const GROUP_SENDER_KEY_RETRY_SLICE: std::time::Duration = std::time::Duration::from_millis(25);
// Ordinary live messages advance state.iteration by one, which cannot unlock
// failed blind history unless the ratchet jumps far enough to bring a
// >10,000-skip ciphertext into window, or a new key / backward extension arrives.
const GROUP_RETRY_STATE_JUMP_WINDOW: u32 = 1_024;

#[derive(Clone, PartialEq, Eq)]
struct GroupSenderKeyStateAnchor {
    key_id: u32,
    base_iteration: u32,
    last_iteration: u32,
    last_chain_key: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct GroupSenderKeyRetryInput {
    group_id: String,
    protocol: GroupProtocol,
    sender_is_member: bool,
    local_is_member: bool,
    pub(super) revision: u64,
    sender_owner: NdrOwnerPubkey,
    sender_device: NdrDevicePubkey,
    is_local_sending_chain: bool,
    state_anchors: Vec<GroupSenderKeyStateAnchor>,
    distributions: Vec<nostr_double_ratchet::SenderKeyDistribution>,
}

impl GroupSenderKeyRetryInput {
    pub(super) fn has_multiple_sender_keys(&self) -> bool {
        self.state_anchors.len() > 1
    }
}

/// Scheduling is ephemeral; the existing durable ciphertext queue is authoritative.
/// One input change admits each affected candidate once, in FIFO order. A failed
/// blind key search cannot run again until its actual decryption inputs change.
#[derive(Clone)]
pub(super) struct ProtocolGroupSenderKeyRetry {
    pub(super) inputs: BTreeMap<NdrDevicePubkey, GroupSenderKeyRetryInput>,
    pub(super) ready: std::collections::VecDeque<(NdrDevicePubkey, String)>,
    pub(super) queued: HashSet<String>,
    pub(super) pending_revision_fingerprints: HashSet<String>,
    pub(super) started: Option<std::time::Instant>,
    pub(super) attempts: usize,
    pub(super) key_trials_remaining: usize,
    pub(super) active: Option<ActiveGroupSenderKeyDecrypt>,
    pub(super) prepared: Option<(String, nostr_double_ratchet::GroupSenderKeyReceivePlan)>,
    pub(super) fresh: std::collections::VecDeque<FreshGroupSenderKeyCandidate>,
    pub(super) yielded: bool,
    #[cfg(test)]
    pub(super) total_attempts: usize,
    #[cfg(test)]
    pub(super) receive_checkpoints: usize,
    #[cfg(test)]
    pub(super) continuation_ready_comparisons: usize,
}

impl Default for ProtocolGroupSenderKeyRetry {
    fn default() -> Self {
        Self {
            inputs: BTreeMap::new(),
            ready: Default::default(),
            queued: HashSet::new(),
            pending_revision_fingerprints: HashSet::new(),
            started: None,
            attempts: 0,
            key_trials_remaining: GROUP_SENDER_KEY_TRIALS_PER_TURN,
            active: None,
            prepared: None,
            fresh: Default::default(),
            yielded: false,
            #[cfg(test)]
            total_attempts: 0,
            #[cfg(test)]
            receive_checkpoints: 0,
            #[cfg(test)]
            continuation_ready_comparisons: 0,
        }
    }
}

impl ProtocolGroupSenderKeyRetry {
    pub(super) fn reset_budget(&mut self) {
        self.started = None;
        self.attempts = 0;
        self.key_trials_remaining = GROUP_SENDER_KEY_TRIALS_PER_TURN;
        self.yielded = false;
    }

    pub(super) fn remove_ready_for_continuation(&mut self, fingerprint: &str) {
        // Admission through `queued` makes each fingerprint unique. An active
        // continuation is normally first; stop there instead of visiting the
        // entire backlog. A displaced entry still retains every other position.
        #[cfg(test)]
        let mut comparisons = 0;
        let index = self.ready.iter().position(|(_, queued)| {
            #[cfg(test)]
            {
                comparisons += 1;
            }
            queued == fingerprint
        });
        #[cfg(test)]
        {
            self.continuation_ready_comparisons += comparisons;
        }
        if let Some(index) = index {
            self.ready.remove(index);
        }
    }

    fn next(&mut self, fresh_due: bool) -> Option<(NdrDevicePubkey, String)> {
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
            .or_else(|| {
                fresh_due
                    .then(|| self.fresh.front())
                    .flatten()
                    .map(|fresh| &fresh.fingerprint)
            })
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
    pub(super) fn refresh_group_sender_key_retry_inputs(&self) {
        let mut retry = self.group_sender_key_retry.borrow_mut();
        if self.pending_group_sender_key_messages.is_empty() {
            retry.active = None;
            retry.prepared = None;
            retry.fresh.clear();
            retry.inputs.clear();
            retry.ready.clear();
            retry.queued.clear();
            retry.pending_revision_fingerprints.clear();
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
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum StreamWakeup {
            Full,
            RevisionOnly,
        }
        let mut inputs = BTreeMap::new();
        let mut wakeups: BTreeMap<NdrDevicePubkey, StreamWakeup> = BTreeMap::new();
        for record in snapshot.sender_keys {
            let Some(group) = groups.get(&record.group_id) else {
                continue;
            };
            let author = record.sender_event_pubkey;
            let previous = retry.inputs.get(&author);
            let mut states_materially_changed =
                previous.is_none_or(|prev| prev.state_anchors.len() != record.states.len());
            let mut state_anchors = Vec::with_capacity(record.states.len());
            for state in &record.states {
                let key_id = state.key_id();
                let iteration = state.iteration();
                let chain_key = state.chain_key();
                let prev_anchor = previous.and_then(|prev| {
                    prev.state_anchors
                        .iter()
                        .find(|anchor| anchor.key_id == key_id)
                });
                let anchor = match prev_anchor {
                    Some(prev)
                        if iteration == prev.last_iteration && chain_key == prev.last_chain_key =>
                    {
                        prev.clone()
                    }
                    Some(prev)
                        if iteration > prev.last_iteration
                            && iteration.saturating_sub(prev.base_iteration)
                                < GROUP_RETRY_STATE_JUMP_WINDOW =>
                    {
                        GroupSenderKeyStateAnchor {
                            key_id,
                            base_iteration: prev.base_iteration,
                            last_iteration: iteration,
                            last_chain_key: chain_key,
                        }
                    }
                    _ => {
                        states_materially_changed = true;
                        GroupSenderKeyStateAnchor {
                            key_id,
                            base_iteration: iteration,
                            last_iteration: iteration,
                            last_chain_key: chain_key,
                        }
                    }
                };
                state_anchors.push(anchor);
            }
            let input = GroupSenderKeyRetryInput {
                group_id: record.group_id.clone(),
                protocol: group.protocol,
                sender_is_member: group.members.contains(&record.sender_owner),
                local_is_member: group.members.contains(&self.local_owner),
                revision: group.revision,
                sender_owner: record.sender_owner,
                sender_device: record.sender_device,
                is_local_sending_chain: record.sender_event_secret_key.is_some(),
                state_anchors,
                distributions: record.distribution_history,
            };
            let wakeup = match previous {
                None => Some(StreamWakeup::Full),
                Some(prev) => {
                    let decrypt_inputs_changed = states_materially_changed
                        || prev.group_id != input.group_id
                        || prev.protocol != input.protocol
                        || prev.sender_is_member != input.sender_is_member
                        || prev.local_is_member != input.local_is_member
                        || prev.sender_owner != input.sender_owner
                        || prev.sender_device != input.sender_device
                        || prev.is_local_sending_chain != input.is_local_sending_chain
                        || prev.distributions != input.distributions;
                    if decrypt_inputs_changed {
                        Some(StreamWakeup::Full)
                    } else if prev.revision != input.revision {
                        Some(StreamWakeup::RevisionOnly)
                    } else {
                        None
                    }
                }
            };
            if let Some(wakeup) = wakeup {
                wakeups.insert(author, wakeup);
            }
            inputs.insert(author, input);
        }
        #[cfg(all(test, target_os = "linux"))]
        {
            let changed = wakeups.keys().copied().collect::<HashSet<_>>();
            tests::audit_group_retry_input_changes(&changed);
        }
        retry.inputs = inputs;
        for parsed in &self.pending_group_sender_key_messages {
            let Some(wakeup) = wakeups.get(&parsed.sender_event_pubkey).copied() else {
                continue;
            };
            let Some(message) = self.group_sender_key_message_from_parsed(parsed) else {
                continue;
            };
            let fingerprint = group_sender_key_fingerprint(&message);
            if wakeup == StreamWakeup::RevisionOnly
                && !retry.pending_revision_fingerprints.contains(&fingerprint)
            {
                continue;
            }
            if retry.queued.insert(fingerprint.clone()) {
                retry
                    .ready
                    .push_back((parsed.sender_event_pubkey, fingerprint));
            }
        }
    }

    pub(super) fn retry_eligible_group_sender_key_messages(
        &mut self,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        if self.batch_depth.get() == 0 {
            self.group_sender_key_retry.borrow_mut().reset_budget();
        }
        if !self.fresh_group_sender_key_work_is_due()
            && self.advance_group_sender_key_continuation()?
        {
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
            self.invalidate_pending_group_fanout_retry_deadline();
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
        let mut repairs_before = self.pending_group_sender_key_repairs.clone();
        let mut result = ProtocolGroupIncomingResult::default();
        loop {
            let fresh_due = self.fresh_group_sender_key_work_is_due();
            let next = self.group_sender_key_retry.borrow_mut().next(fresh_due);
            let Some((author, fingerprint)) = next else {
                break;
            };
            let Some(index) = self.pending_group_sender_key_retry_index(author, &fingerprint)
            else {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                retry.fresh.retain(|fresh| fresh.fingerprint != fingerprint);
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
            // Never discard ciphertexts solely because their created_at precedes
            // the earliest distribution currently installed: relay backfill and
            // repair replies can deliver a newer epoch key before an older key.
            let discard = self.local_owner_is_inactive_for_group(&message.group_id)
                || self
                    .group_sender_key_retry
                    .borrow()
                    .inputs
                    .get(&author)
                    .is_some_and(|input| !input.local_is_member);
            if discard {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                retry.fresh.retain(|fresh| fresh.fingerprint != fingerprint);
                if retry
                    .prepared
                    .as_ref()
                    .is_some_and(|(ready, _)| ready == &fingerprint)
                {
                    retry.prepared = None;
                }
                drop(retry);
                removed.push((index, self.pending_group_sender_key_messages.remove(index)));
                self.persist()?;
                continue;
            }
            {
                let mut retry = self.group_sender_key_retry.borrow_mut();
                if retry.active.is_none()
                    && retry.prepared.is_none()
                    && (message.encrypted_header.is_none()
                        || self.group_retry_pacing.borrow().delay().is_zero())
                {
                    retry.record_attempt();
                    #[cfg(all(test, target_os = "linux"))]
                    if message.encrypted_header.is_none() || retry.key_trials_remaining > 0 {
                        // A candidate selected after the last trial was spent
                        // only yields; its first real search starts next turn.
                        tests::audit_group_retry_attempt(author, &fingerprint);
                    }
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
                self.group_sender_key_retry
                    .borrow_mut()
                    .fresh
                    .retain(|fresh| fresh.fingerprint != fingerprint);
                removed.push((index, self.pending_group_sender_key_messages.remove(index)));
                self.persist()?;
            }
            result.events.extend(outcome.events);
            result.effects.extend(outcome.effects);
        }
        // The request helper saves real send/fanout progress with its clocks.
        // An unchanged, still-undeliverable intent only advances volatile backoff.
        // Keep every other field, queue position and addition/removal significant.
        for (before, after) in repairs_before
            .iter_mut()
            .zip(&self.pending_group_sender_key_repairs)
        {
            before.last_requested_at_secs = after.last_requested_at_secs;
            before.request_count = after.request_count;
            before.next_retry_at_secs = after.next_retry_at_secs;
        }
        if self.pending_group_sender_key_repairs != repairs_before {
            self.persist()?;
        }
        Ok(result)
    }
}
