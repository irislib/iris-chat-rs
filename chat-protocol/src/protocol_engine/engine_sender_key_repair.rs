impl ProtocolEngine {
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
            .collect::<Vec<_>>();
        if sender_event_pubkeys.is_empty() {
            return false;
        }

        let mut changed = false;
        for sender_event_pubkey in sender_event_pubkeys {
            changed |= self.remember_delivered_group_sender_key_ack(
                group_id,
                sender_event_pubkey,
                created_at_secs,
            );
        }
        if changed {
            let _ = self.persist();
        }
        changed
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
        let result = match self.group_manager.handle_sender_key_message(message) {
            Ok(result) => result,
            Err(nostr_double_ratchet::Error::Decryption(error))
                if error == "duplicate or missing sender-key message" =>
            {
                self.persist()?;
                return Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    ..Default::default()
                });
            }
            Err(error) => return Err(error.into()),
        };

        match result {
            GroupSenderKeyHandleResult::Event(event) => {
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events: vec![event],
                    consumed: true,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::PendingDistribution { .. }
            | GroupSenderKeyHandleResult::PendingRevision { .. } => Ok(ProtocolGroupIncomingResult {
                consumed: true,
                pending: true,
                ..Default::default()
            }),
            GroupSenderKeyHandleResult::Ignored => Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            }),
        }
    }

    fn sender_key_repair_response_effects(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        if self.group_sender_key_repair_response_throttled(requester_owner, request, now) {
            return Ok(Vec::new());
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
        if !output.effects.is_empty() {
            self.remember_group_sender_key_repair_response(requester_owner, request, now);
        }
        self.invalidate_known_message_author_cache();
        Ok(output.effects)
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
}
