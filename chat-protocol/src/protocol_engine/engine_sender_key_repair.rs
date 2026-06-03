impl ProtocolEngine {
    fn queue_pending_group_sender_key_message(
        &mut self,
        parsed: nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> anyhow::Result<()> {
        if self.unmapped_group_sender_key_candidate_is_known_message_author(&parsed) {
            return Ok(());
        }

        if let Some(group_id) = self.delivered_group_sender_key_ack_match(&parsed) {
            if !self.has_pending_group_sender_key_candidate(&group_id, parsed.sender_event_pubkey)
                && self.clear_group_sender_key_repairs(
                    &group_id,
                    parsed.sender_event_pubkey,
                    None,
                    None,
                )
            {
                self.persist()?;
            }
            return Ok(());
        }

        if let Some(group_id) = self.inactive_local_group_id_for_sender_key_candidate(&parsed) {
            if self.clear_group_sender_key_repairs(
                &group_id,
                parsed.sender_event_pubkey,
                parsed.encrypted_header.is_none().then_some(parsed.key_id),
                parsed
                    .encrypted_header
                    .is_none()
                    .then_some(parsed.message_number),
            ) {
                self.persist()?;
            }
            return Ok(());
        }

        if !self.pending_group_sender_key_messages.contains(&parsed) {
            self.pending_group_sender_key_messages.push(parsed);
            self.persist()?;
        }
        Ok(())
    }

    fn clear_pending_group_sender_key_candidate_for_direct_event(&mut self, event: &Event) {
        if !protocol_event_has_tag(event, "header") {
            return;
        }
        let Ok(parsed) = parse_group_sender_key_message_event_unchecked(event) else {
            return;
        };
        self.clear_pending_group_sender_key_candidate(&parsed);
    }

    fn clear_pending_group_sender_key_candidate(
        &mut self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> bool {
        let original_len = self.pending_group_sender_key_messages.len();
        self.pending_group_sender_key_messages
            .retain(|pending| pending != parsed);
        if self.pending_group_sender_key_messages.len() == original_len {
            return false;
        }

        let Some(group_id) = self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)
        else {
            return true;
        };
        if !self.has_pending_group_sender_key_candidate(&group_id, parsed.sender_event_pubkey) {
            self.clear_group_sender_key_repairs(
                &group_id,
                parsed.sender_event_pubkey,
                parsed.encrypted_header.is_none().then_some(parsed.key_id),
                parsed
                    .encrypted_header
                    .is_none()
                    .then_some(parsed.message_number),
            );
        }
        true
    }

    fn delivered_group_sender_key_ack_match(
        &self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> Option<String> {
        let group_id = self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)?;
        let sender_event_pubkey_hex = parsed.sender_event_pubkey.to_hex();
        self.delivered_group_sender_key_acks
            .iter()
            .any(|ack| {
                ack.group_id == group_id
                    && ack.sender_event_pubkey_hex == sender_event_pubkey_hex
                    && ack.created_at_secs == parsed.created_at.get()
            })
            .then_some(group_id)
    }

    fn remember_delivered_group_sender_key_ack(
        &mut self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        created_at_secs: u64,
    ) -> bool {
        let sender_event_pubkey_hex = sender_event_pubkey.to_hex();
        if self.delivered_group_sender_key_acks.iter().any(|ack| {
            ack.group_id == group_id
                && ack.sender_event_pubkey_hex == sender_event_pubkey_hex
                && ack.created_at_secs == created_at_secs
        }) {
            return false;
        }

        self.delivered_group_sender_key_acks
            .push(ProtocolDeliveredGroupSenderKeyAck {
                group_id: group_id.to_string(),
                sender_event_pubkey_hex,
                created_at_secs,
            });
        let excess = self
            .delivered_group_sender_key_acks
            .len()
            .saturating_sub(DELIVERED_GROUP_SENDER_KEY_ACK_LIMIT);
        if excess > 0 {
            self.delivered_group_sender_key_acks.drain(0..excess);
        }
        true
    }

    fn has_pending_group_sender_key_candidate(
        &self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
    ) -> bool {
        self.pending_group_sender_key_messages
            .iter()
            .any(|pending| {
                pending.sender_event_pubkey == sender_event_pubkey
                    && self
                        .group_manager
                        .group_id_for_sender_event_pubkey(pending.sender_event_pubkey)
                        .as_deref()
                        == Some(group_id)
                })
    }

    fn unmapped_group_sender_key_candidate_is_known_message_author(
        &self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> bool {
        if self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)
            .is_some()
        {
            return false;
        }

        public_device(parsed.sender_event_pubkey)
            .is_ok_and(|author| self.is_known_message_author(author))
    }

    fn pending_group_sender_key_candidate_predates_known_distribution(
        &self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> bool {
        self.group_manager
            .snapshot()
            .sender_keys
            .iter()
            .filter(|record| record.sender_event_pubkey == parsed.sender_event_pubkey)
            .flat_map(|record| record.distribution_history.iter())
            .map(|distribution| distribution.created_at.get())
            .min()
            .is_some_and(|first_distribution_at| parsed.created_at.get() < first_distribution_at)
    }

    fn inactive_local_group_id_for_sender_key_candidate(
        &self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> Option<String> {
        let group_id = self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)?;
        self.local_owner_is_inactive_for_group(&group_id)
            .then_some(group_id)
    }

    fn local_owner_is_inactive_for_group(&self, group_id: &str) -> bool {
        self.group_manager
            .group(group_id)
            .is_some_and(|group| !group.members.contains(&self.local_owner))
    }

    fn inactive_local_group_ids(&self) -> HashSet<String> {
        self.group_manager
            .snapshot()
            .groups
            .into_iter()
            .filter(|group| !group.members.contains(&self.local_owner))
            .map(|group| group.group_id)
            .collect()
    }

    fn prune_pending_group_sender_key_work_for_inactive_local_groups(&mut self) -> bool {
        let inactive_group_ids = self.inactive_local_group_ids();
        if inactive_group_ids.is_empty() {
            return false;
        }
        let inactive_sender_event_pubkeys = self
            .group_manager
            .snapshot()
            .sender_keys
            .into_iter()
            .filter(|record| inactive_group_ids.contains(&record.group_id))
            .map(|record| record.sender_event_pubkey)
            .collect::<HashSet<_>>();

        let original_message_len = self.pending_group_sender_key_messages.len();
        self.pending_group_sender_key_messages
            .retain(|pending| !inactive_sender_event_pubkeys.contains(&pending.sender_event_pubkey));

        let original_repair_len = self.pending_group_sender_key_repairs.len();
        self.pending_group_sender_key_repairs
            .retain(|pending| !inactive_group_ids.contains(&pending.group_id));

        self.pending_group_sender_key_messages.len() != original_message_len
            || self.pending_group_sender_key_repairs.len() != original_repair_len
    }

    pub fn acknowledge_delivered_group_sender_key_message(
        &mut self,
        group_id: &str,
        sender_owner: PublicKey,
        sender_device: Option<PublicKey>,
        created_at_secs: u64,
    ) -> bool {
        let Some(sender_device) = sender_device else {
            return false;
        };
        let sender_owner = ndr_owner(sender_owner);
        let sender_device = ndr_device(sender_device);
        let sender_event_pubkeys = self
            .group_manager
            .snapshot()
            .sender_keys
            .into_iter()
            .filter(|record| {
                record.group_id == group_id
                    && record.sender_owner == sender_owner
                    && record.sender_device == sender_device
            })
            .map(|record| record.sender_event_pubkey)
            .collect::<HashSet<_>>();
        if sender_event_pubkeys.is_empty() {
            return false;
        }

        let mut changed = false;
        for sender_event_pubkey in sender_event_pubkeys.iter().copied() {
            changed |= self.remember_delivered_group_sender_key_ack(
                group_id,
                sender_event_pubkey,
                created_at_secs,
            );
        }

        let mut removed_senders = HashSet::new();
        let original_len = self.pending_group_sender_key_messages.len();
        self.pending_group_sender_key_messages.retain(|pending| {
            let should_remove = sender_event_pubkeys.contains(&pending.sender_event_pubkey)
                && pending.created_at.get() == created_at_secs;
            if should_remove {
                removed_senders.insert(pending.sender_event_pubkey);
            }
            !should_remove
        });
        changed |= self.pending_group_sender_key_messages.len() != original_len;

        for sender_event_pubkey in removed_senders {
            if !self.has_pending_group_sender_key_candidate(group_id, sender_event_pubkey) {
                changed |=
                    self.clear_group_sender_key_repairs(group_id, sender_event_pubkey, None, None);
            }
        }
        if changed {
            let _ = self.persist();
        }
        changed
    }

    fn group_sender_key_message_from_parsed(
        &self,
        parsed: &nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> Option<GroupSenderKeyMessage> {
        let group_id = self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)?;
        Some(GroupSenderKeyMessage {
            group_id,
            sender_event_pubkey: parsed.sender_event_pubkey,
            key_id: parsed.key_id,
            message_number: parsed.message_number,
            encrypted_header: parsed.encrypted_header.clone(),
            created_at: parsed.created_at,
            ciphertext: parsed.ciphertext.clone(),
        })
    }

    fn handle_group_sender_key_message(
        &mut self,
        message: GroupSenderKeyMessage,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let message_repair_group_id = message.group_id.clone();
        let message_repair_sender = message.sender_event_pubkey;
        let message_repair_key_id = message.encrypted_header.is_none().then_some(message.key_id);
        let message_repair_number = message
            .encrypted_header
            .is_none()
            .then_some(message.message_number);
        let result = match self
            .group_manager
            .handle_sender_key_message(message.clone())
        {
            Ok(result) => result,
            Err(nostr_double_ratchet::Error::Decryption(error))
                if error == "duplicate or missing sender-key message" =>
            {
                self.clear_group_sender_key_repairs(
                    &message_repair_group_id,
                    message_repair_sender,
                    message_repair_key_id,
                    message_repair_number,
                );
                self.persist()?;
                return Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    ..Default::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        let now = NdrUnixSeconds(unix_now().get());
        let repair_request =
            SenderKeyRepairRequest::from_pending_sender_key_message(&message, &result, now);
        match result {
            GroupSenderKeyHandleResult::Event(event) => {
                self.clear_group_sender_key_repairs(
                    &message_repair_group_id,
                    message_repair_sender,
                    message_repair_key_id,
                    message_repair_number,
                );
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events: vec![event],
                    consumed: true,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::PendingDistribution { .. }
            | GroupSenderKeyHandleResult::PendingRevision { .. } => {
                let Some(request) = repair_request else {
                    anyhow::bail!("pending sender-key result did not produce a repair request");
                };
                let effects = self.sender_key_repair_request_effects(request, now)?;
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    pending: true,
                    effects,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::Ignored => {
                self.clear_group_sender_key_repairs(
                    &message_repair_group_id,
                    message_repair_sender,
                    message_repair_key_id,
                    message_repair_number,
                );
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    ..Default::default()
                })
            }
        }
    }

    fn sender_key_repair_request_effects(
        &mut self,
        request: SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        if self.local_owner_is_inactive_for_group(&request.group_id) {
            if self.clear_group_sender_key_repairs(
                &request.group_id,
                request.sender_event_pubkey,
                request.key_id,
                request.message_number,
            ) {
                self.persist()?;
            }
            return Ok(Vec::new());
        }

        let sender_event_pubkey_hex = request.sender_event_pubkey.to_hex();
        let position = self
            .pending_group_sender_key_repairs
            .iter()
            .position(|pending| {
                pending.group_id == request.group_id
                    && pending.sender_event_pubkey_hex == sender_event_pubkey_hex
                    && pending.key_id == request.key_id
                    && pending.message_number == request.message_number
                    && pending.required_revision == request.required_revision
            });
        let index = if let Some(index) = position {
            index
        } else {
            self.pending_group_sender_key_repairs
                .push(ProtocolPendingGroupSenderKeyRepair {
                    group_id: request.group_id.clone(),
                    sender_event_pubkey_hex,
                    key_id: request.key_id,
                    message_number: request.message_number,
                    required_revision: request.required_revision,
                    created_at_secs: now.get(),
                    last_requested_at_secs: 0,
                    request_count: 0,
                    next_retry_at_secs: 0,
                });
            self.pending_group_sender_key_repairs.len() - 1
        };
        let Some(pending_repair) = self.pending_group_sender_key_repairs.get(index) else {
            anyhow::bail!("pending sender-key repair index disappeared");
        };
        if Self::pending_group_sender_key_repair_due_at_secs(pending_repair) > now.get() {
            return Ok(Vec::new());
        }

        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.group_manager.request_sender_key_repair(
            &mut self.session_manager,
            &mut ctx,
            &request,
        )?;
        let output = self.protocol_group_send_from_prepared(&prepared, None)?;
        if let Some(pending) = self.pending_group_sender_key_repairs.get_mut(index) {
            pending.last_requested_at_secs = now.get();
            pending.request_count = pending.request_count.saturating_add(1);
            pending.next_retry_at_secs =
                protocol_sender_key_repair_next_retry_at(now, pending.request_count).get();
        }
        self.invalidate_known_message_author_cache();
        Ok(output.effects)
    }

    fn retry_pending_group_sender_key_repairs(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        let requests = self
            .pending_group_sender_key_repairs
            .iter()
            .filter(|pending| {
                Self::pending_group_sender_key_repair_due_at_secs(pending) <= now.get()
            })
            .filter_map(|pending| {
                let sender = PublicKey::parse(&pending.sender_event_pubkey_hex).ok()?;
                Some(SenderKeyRepairRequest {
                    group_id: pending.group_id.clone(),
                    sender_event_pubkey: ndr_device(sender),
                    key_id: pending.key_id,
                    message_number: pending.message_number,
                    required_revision: pending.required_revision,
                    created_at: NdrUnixSeconds(pending.created_at_secs),
                })
            })
            .collect::<Vec<_>>();
        let mut effects = Vec::new();
        for request in requests {
            let repair_effects = self.sender_key_repair_request_effects(request, now)?;
            effects.extend(repair_effects);
        }
        Ok(effects)
    }

    fn pending_group_sender_key_repair_due_at_secs(
        pending: &ProtocolPendingGroupSenderKeyRepair,
    ) -> u64 {
        if pending.request_count == 0 || pending.last_requested_at_secs == 0 {
            return pending.next_retry_at_secs;
        }
        let capped_due_at = pending
            .last_requested_at_secs
            .saturating_add(protocol_sender_key_repair_retry_delay_secs(
                pending.request_count,
            ));
        if pending.next_retry_at_secs == 0 {
            capped_due_at
        } else {
            pending.next_retry_at_secs.min(capped_due_at)
        }
    }

    fn sender_key_repair_response_effects(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, Vec<String>)> {
        if self.group_sender_key_repair_response_throttled(requester_owner, request, now) {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.group_manager.respond_to_sender_key_repair_request(
            &mut self.session_manager,
            &mut ctx,
            requester_owner,
            request,
        )?;
        let output = self.protocol_group_send_from_prepared(&prepared, None)?;
        if !output.effects.is_empty() || !output.queued_targets.is_empty() {
            self.remember_group_sender_key_repair_response(requester_owner, request, now);
        }
        self.invalidate_known_message_author_cache();
        Ok((output.effects, output.queued_targets))
    }

    fn group_sender_key_repair_response_throttled(
        &self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> bool {
        let requester_owner_hex = requester_owner.to_hex();
        let sender_event_pubkey_hex = request.sender_event_pubkey.to_hex();
        self.answered_group_sender_key_repairs
            .iter()
            .any(|answered| {
                answered.requester_owner_hex == requester_owner_hex
                    && answered.group_id == request.group_id.as_str()
                    && answered.sender_event_pubkey_hex == sender_event_pubkey_hex
                    && answered.key_id == request.key_id
                    && answered.message_number == request.message_number
                    && answered.required_revision == request.required_revision
                    && answered.request_created_at_secs == request.created_at.get()
                    && answered.next_response_at_secs > now.get()
            })
    }

    fn remember_group_sender_key_repair_response(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) {
        let requester_owner_hex = requester_owner.to_hex();
        let sender_event_pubkey_hex = request.sender_event_pubkey.to_hex();
        let position = self
            .answered_group_sender_key_repairs
            .iter()
            .position(|answered| {
                answered.requester_owner_hex == requester_owner_hex
                    && answered.group_id == request.group_id.as_str()
                    && answered.sender_event_pubkey_hex == sender_event_pubkey_hex
                    && answered.key_id == request.key_id
                    && answered.message_number == request.message_number
                    && answered.required_revision == request.required_revision
                    && answered.request_created_at_secs == request.created_at.get()
            });
        let response_count = position
            .and_then(|index| {
                self.answered_group_sender_key_repairs
                    .get(index)
                    .map(|answered| answered.response_count.saturating_add(1))
            })
            .unwrap_or(1);
        let next_response_at_secs =
            protocol_sender_key_repair_next_retry_at(now, response_count).get();
        if let Some(index) = position {
            if let Some(answered) = self.answered_group_sender_key_repairs.get_mut(index) {
                answered.last_responded_at_secs = now.get();
                answered.response_count = response_count;
                answered.next_response_at_secs = next_response_at_secs;
            }
        } else {
            self.answered_group_sender_key_repairs
                .push(ProtocolAnsweredGroupSenderKeyRepair {
                    requester_owner_hex,
                    group_id: request.group_id.clone(),
                    sender_event_pubkey_hex,
                    key_id: request.key_id,
                    message_number: request.message_number,
                    required_revision: request.required_revision,
                    request_created_at_secs: request.created_at.get(),
                    last_responded_at_secs: now.get(),
                    response_count,
                    next_response_at_secs,
                });
        }
        let excess = self
            .answered_group_sender_key_repairs
            .len()
            .saturating_sub(ANSWERED_GROUP_SENDER_KEY_REPAIR_LIMIT);
        if excess > 0 {
            self.answered_group_sender_key_repairs.drain(0..excess);
        }
    }

    fn clear_group_sender_key_repairs(
        &mut self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        key_id: Option<u32>,
        message_number: Option<u32>,
    ) -> bool {
        let original_len = self.pending_group_sender_key_repairs.len();
        let sender_event_pubkey_hex = sender_event_pubkey.to_hex();
        self.pending_group_sender_key_repairs.retain(|pending| {
            let position_matches = match (key_id, message_number) {
                (Some(key_id), Some(message_number)) => {
                    pending.key_id == Some(key_id) && pending.message_number == Some(message_number)
                }
                _ => true,
            };
            !(pending.group_id == group_id
                && pending.sender_event_pubkey_hex == sender_event_pubkey_hex
                && position_matches)
        });
        self.pending_group_sender_key_repairs.len() != original_len
    }
}
