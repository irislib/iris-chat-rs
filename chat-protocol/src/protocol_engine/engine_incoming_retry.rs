impl ProtocolEngine {
    pub fn has_due_pending_retry_work(&self, now: NdrUnixSeconds) -> bool {
        let now_secs = now.get();
        self.pending_inbound
                .iter()
                .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self
                .pending_group_fanouts
                .iter()
                .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self
                .pending_group_pairwise_payloads
                .iter()
                .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self
                .pending_group_sender_key_repairs
                .iter()
                .any(|pending| Self::pending_group_sender_key_repair_due_at_secs(pending) <= now_secs)
            || !self.pending_decrypted_deliveries.is_empty()
    }

    pub fn process_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;
        let resolution = self.resolve_message_sender_owner(&envelope);
        match resolution {
            ProtocolSenderOwnerResolution::Verified { .. }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { .. } => {}
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. } => {
                self.queue_header_group_sender_key_candidate(event)?;
                self.queue_pending_inbound_direct_event(
                    event.clone(),
                    event.created_at.as_secs(),
                    Some(&envelope),
                    Some(resolution),
                )?;
                return Ok(None);
            }
        };
        match self.decrypt_direct_message_envelope(event, &envelope, true) {
            Ok(Some(decrypted)) => return Ok(Some(decrypted)),
            Ok(None) => {}
            Err(error) => {
                if self.queue_header_group_sender_key_candidate_after_direct_error(event)? {
                    return Ok(None);
                }
                if protocol_event_has_tag(event, "header")
                    && !self.is_known_message_author(event.pubkey)
                {
                    self.queue_pending_inbound_direct_event(
                        event.clone(),
                        event.created_at.as_secs(),
                        Some(&envelope),
                        Some(resolution),
                    )?;
                    return Ok(None);
                }
                return Err(error);
            }
        }
        self.queue_header_group_sender_key_candidate(event)?;
        self.queue_pending_inbound_direct_event(
            event.clone(),
            event.created_at.as_secs(),
            Some(&envelope),
            Some(resolution),
        )?;
        Ok(None)
    }

    fn queue_header_group_sender_key_candidate(&mut self, event: &Event) -> anyhow::Result<()> {
        if !protocol_event_has_tag(event, "header") {
            return Ok(());
        }
        if self.is_known_message_author(event.pubkey) {
            return Ok(());
        }
        if let Ok(parsed) = parse_group_sender_key_message_event_unchecked(event) {
            self.queue_pending_group_sender_key_message(parsed)?;
        }
        Ok(())
    }

    fn queue_header_group_sender_key_candidate_after_direct_error(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<bool> {
        if !protocol_event_has_tag(event, "header") || self.is_known_message_author(event.pubkey) {
            return Ok(false);
        }
        let Ok(parsed) = parse_group_sender_key_message_event_unchecked(event) else {
            return Ok(false);
        };
        self.queue_pending_group_sender_key_message(parsed)?;
        Ok(true)
    }

    pub fn process_group_outer_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let has_header = protocol_event_has_tag(event, "header");
        let parsed = if has_header {
            parse_group_sender_key_message_event_unchecked(event)
        } else {
            parse_group_sender_key_message_event(event)
        };
        let Ok(parsed) = parsed else {
            return Ok(ProtocolGroupIncomingResult::default());
        };
        let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
            self.queue_pending_group_sender_key_message(parsed)?;
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        };
        let mut result = self.handle_group_sender_key_message(message)?;
        if result.pending {
            self.queue_pending_group_sender_key_message(parsed)?;
        } else if self.clear_pending_group_sender_key_candidate(&parsed) {
            self.persist()?;
        }
        if !result.pending {
            let retry = self.retry_pending_group_inputs(NdrUnixSeconds(unix_now().get()))?;
            result.events.extend(retry.events);
            result.effects.extend(retry.effects);
        }
        Ok(ProtocolGroupIncomingResult {
            consumed: true,
            ..result
        })
    }

    pub fn process_group_pairwise_payload(
        &mut self,
        payload: &[u8],
        from_owner_pubkey: PublicKey,
        from_sender_device_pubkey: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let (is_group_payload, is_supported_group_payload) =
            classify_group_pairwise_payload(payload).unwrap_or((false, false));
        if is_group_payload && !is_supported_group_payload {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        let sender_device = from_sender_device_pubkey.map(ndr_device);
        let sender_owner = ndr_owner(from_owner_pubkey);
        let sender_owner =
            match self.resolve_group_pairwise_sender_owner(sender_owner, sender_device) {
                ProtocolSenderOwnerResolution::Verified { owner }
                | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
                ProtocolSenderOwnerResolution::PendingOwnerClaim {
                    storage_owner,
                    claimed_owner: _,
                    sender_device,
                } => {
                    if is_supported_group_payload {
                        self.queue_pending_group_pairwise_payload(
                            storage_owner,
                            Some(sender_device),
                            payload.to_vec(),
                            unix_now().get(),
                        )?;
                        return Ok(ProtocolGroupIncomingResult {
                            consumed: true,
                            ..Default::default()
                        });
                    }
                    storage_owner
                }
            };
        let result = match sender_device {
            Some(device_pubkey) => {
                self.group_manager
                    .handle_pairwise_payload(sender_owner, device_pubkey, payload)
            }
            None => self.group_manager.handle_incoming(sender_owner, payload),
        };

        let now = NdrUnixSeconds(unix_now().get());
        match result {
            Ok(Some(event)) => {
                if let GroupIncomingEvent::SenderKeyRepairRequested(repair) = event {
                    let effects = self.sender_key_repair_response_effects(
                        repair.requester_owner,
                        &repair.request,
                        NdrUnixSeconds(unix_now().get()),
                    )?;
                    self.persist()?;
                    return Ok(ProtocolGroupIncomingResult {
                        effects,
                        consumed: true,
                        ..Default::default()
                    });
                }
                let mut effects = Vec::new();
                if sender_owner != self.local_owner {
                    if let GroupIncomingEvent::MetadataUpdated(group) = &event {
                        for pending in &mut self.pending_group_pairwise_payloads {
                            pending.next_retry_at_secs = 0;
                        }
                        let sync_effects = self.sync_group_to_local_siblings(group)?;
                        effects.extend(sync_effects);
                    }
                }
                let mut events = vec![event];
                let retry = self.retry_pending_group_inputs(now)?;
                events.extend(retry.events);
                effects.extend(retry.effects);
                let fanout_retry = self.retry_pending_group_fanouts(now)?;
                effects.extend(fanout_retry.effects);
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events,
                    effects,
                    consumed: true,
                    ..Default::default()
                })
            }
            Ok(None) => {
                let retry = self.retry_pending_group_inputs(now)?;
                Ok(ProtocolGroupIncomingResult {
                    events: retry.events,
                    effects: retry.effects,
                    consumed: is_group_payload || retry.consumed,
                    ..Default::default()
                })
            }
            Err(error) => {
                if is_supported_group_payload {
                    self.queue_pending_group_pairwise_payload(
                        sender_owner,
                        sender_device,
                        payload.to_vec(),
                        unix_now().get(),
                    )?;
                    Ok(ProtocolGroupIncomingResult {
                        consumed: true,
                        ..Default::default()
                    })
                } else {
                    Err(error.into())
                }
            }
        }
    }

    pub fn retry_pending_protocol(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let group_result = self.retry_pending_group_inputs(now)?;
        let group_fanout_result = self.retry_pending_group_fanouts(now)?;
        let mut group_result = group_result;
        group_result.effects.extend(group_fanout_result.effects);
        let mut direct_messages = self
            .pending_decrypted_deliveries
            .iter()
            .cloned()
            .map(ProtocolDecryptedMessage::from)
            .collect::<Vec<_>>();
        direct_messages.extend(self.retry_pending_inbound_direct_events(now)?);
        let batch = ProtocolRetryBatch {
            group_result,
            direct_messages,
            effects: Vec::new(),
        };
        if !batch.is_empty() {
            self.subscription_generation = self.subscription_generation.wrapping_add(1);
        }
        Ok(batch)
    }

    pub fn ack_pending_decrypted_deliveries(&mut self) -> anyhow::Result<()> {
        if self.pending_decrypted_deliveries.is_empty() {
            return Ok(());
        }
        self.pending_decrypted_deliveries.clear();
        self.persist()
    }

    fn retry_pending_inbound_direct_events(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolDecryptedMessage>> {
        if self.pending_inbound.is_empty() {
            return Ok(Vec::new());
        }
        let pending = std::mem::take(&mut self.pending_inbound);
        let mut still_pending = Vec::new();
        let mut messages = Vec::new();
        for mut pending in pending {
            if pending.next_retry_at_secs > now.get() {
                still_pending.push(pending);
                continue;
            }
            match self.decrypt_pending_direct_message_event(&pending)? {
                Some(message) => messages.push(message),
                None => {
                    pending.next_retry_at_secs =
                        next_pending_retry_at_secs(pending.created_at_secs, now);
                    still_pending.push(pending);
                }
            }
        }
        self.pending_inbound = still_pending;
        Ok(messages)
    }

    fn decrypt_pending_direct_message_event(
        &mut self,
        pending: &ProtocolPendingInbound,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        if let Some(envelope) = pending.envelope.as_ref() {
            return self.decrypt_direct_message_envelope(&pending.event, envelope, false);
        }
        self.decrypt_direct_message_event(&pending.event)
    }

    fn decrypt_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;
        self.decrypt_direct_message_envelope(event, &envelope, false)
    }

    fn decrypt_direct_message_envelope(
        &mut self,
        event: &Event,
        envelope: &MessageEnvelope,
        record_delivery: bool,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let sender_owner = match self.resolve_message_sender_owner(&envelope) {
            ProtocolSenderOwnerResolution::Verified { owner }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. } => {
                return Ok(None);
            }
        };
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let Some(received) = self
            .session_manager
            .receive(&mut ctx, sender_owner, &envelope)?
        else {
            return Ok(None);
        };
        self.clear_pending_group_sender_key_candidate_for_direct_event(event);
        self.invalidate_known_message_author_cache();
        let (conversation_owner, payload) = decode_local_sibling_payload(&received.payload)
            .map(|(owner, payload)| (Some(owner), payload))
            .unwrap_or((None, received.payload));
        let content = String::from_utf8(payload)?;
        let decrypted = ProtocolDecryptedMessage {
            sender: public_owner(received.owner_pubkey)?,
            sender_device: Some(public_device(received.device_pubkey)?),
            conversation_owner,
            content,
            event_id: Some(event.id.to_string()),
        };
        if record_delivery {
            self.record_pending_decrypted_delivery(decrypted.clone(), event.created_at.as_secs());
        }
        self.persist()?;
        Ok(Some(decrypted))
    }

    fn retry_pending_group_inputs(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let mut result = ProtocolGroupIncomingResult::default();
        result.consumed = false;

        let pairwise = std::mem::take(&mut self.pending_group_pairwise_payloads);
        let mut still_pairwise = Vec::new();
        let mut persist_needed = false;
        for mut pending in pairwise {
            if pending.next_retry_at_secs > now.get() {
                still_pairwise.push(pending);
                continue;
            }
            let (_, is_supported_group_payload) =
                classify_group_pairwise_payload(&pending.payload).unwrap_or((false, false));
            if !is_supported_group_payload {
                persist_needed = true;
                continue;
            }
            let sender_resolution = self
                .resolve_group_pairwise_sender_owner(pending.sender_owner, pending.sender_device);
            let sender_owner = match sender_resolution {
                ProtocolSenderOwnerResolution::Verified { owner }
                | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
                ProtocolSenderOwnerResolution::PendingOwnerClaim { .. } => {
                    pending.next_retry_at_secs =
                        next_pending_retry_at_secs(pending.created_at_secs, now);
                    still_pairwise.push(pending);
                    continue;
                }
            };
            let outcome = match pending.sender_device {
                Some(device_pubkey) => self.group_manager.handle_pairwise_payload(
                    sender_owner,
                    device_pubkey,
                    &pending.payload,
                ),
                None => self
                    .group_manager
                    .handle_incoming(sender_owner, &pending.payload),
            };
            match outcome {
                Ok(Some(event)) => {
                    if let GroupIncomingEvent::SenderKeyRepairRequested(repair) = event {
                        let effects = self.sender_key_repair_response_effects(
                            repair.requester_owner,
                            &repair.request,
                            now,
                        )?;
                        result.effects.extend(effects);
                    } else {
                        result.events.push(event);
                    }
                    persist_needed = true;
                }
                Ok(None) => {
                    persist_needed = true;
                }
                Err(_) => {
                    pending.next_retry_at_secs =
                        next_pending_retry_at_secs(pending.created_at_secs, now);
                    still_pairwise.push(pending);
                }
            }
        }
        self.pending_group_pairwise_payloads = still_pairwise;

        let sender_keys = std::mem::take(&mut self.pending_group_sender_key_messages);
        let mut still_sender_keys = Vec::new();
        let mut stale_sender_repairs = Vec::new();
        for parsed in sender_keys {
            if let Some(group_id) =
                self.inactive_local_group_id_for_sender_key_candidate(&parsed)
            {
                stale_sender_repairs.push((
                    group_id,
                    parsed.sender_event_pubkey,
                    parsed.encrypted_header.is_none().then_some(parsed.key_id),
                    parsed
                        .encrypted_header
                        .is_none()
                        .then_some(parsed.message_number),
                ));
                persist_needed = true;
                continue;
            }
            let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
                if self.unmapped_group_sender_key_candidate_is_known_message_author(&parsed) {
                    persist_needed = true;
                    continue;
                }
                still_sender_keys.push(parsed);
                continue;
            };
            if self.pending_group_sender_key_candidate_predates_known_distribution(&parsed) {
                stale_sender_repairs.push((
                    message.group_id,
                    parsed.sender_event_pubkey,
                    parsed.encrypted_header.is_none().then_some(parsed.key_id),
                    parsed
                        .encrypted_header
                        .is_none()
                        .then_some(parsed.message_number),
                ));
                persist_needed = true;
                continue;
            }
            let outcome = self.handle_group_sender_key_message(message)?;
            if outcome.pending {
                still_sender_keys.push(parsed);
            } else {
                persist_needed = true;
            }
            result.events.extend(outcome.events);
            result.effects.extend(outcome.effects);
        }
        self.pending_group_sender_key_messages = still_sender_keys;
        let repair_effects = self.retry_pending_group_sender_key_repairs(now)?;
        if !repair_effects.is_empty() {
            result.effects.extend(repair_effects);
            persist_needed = true;
        }
        if persist_needed || !result.events.is_empty() || !result.effects.is_empty() {
            self.persist()?;
        }
        Ok(result)
    }

    fn retry_pending_group_fanouts(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        if self.pending_group_fanouts.is_empty() {
            return Ok(ProtocolGroupIncomingResult::default());
        }
        let pending = std::mem::take(&mut self.pending_group_fanouts);
        let mut still_pending = Vec::new();
        let mut effects = Vec::new();
        let mut persist_needed = false;
        let mut session_changed = false;
        for mut pending in pending {
            if pending.next_retry_at_secs > now.get() {
                still_pending.push(pending);
                continue;
            }
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            let prepared = match &pending.fanout {
                GroupPendingFanout::Remote {
                    recipient_owner,
                    payload,
                } => self
                    .session_manager
                    .prepare_remote_send(&mut ctx, *recipient_owner, payload.clone())
                    .map(|prepared| {
                        group_publish_from_prepared_send(prepared, pending.fanout.clone())
                    }),
                GroupPendingFanout::LocalSiblings { payload } => self
                    .session_manager
                    .prepare_local_sibling_send_reusing_all_sessions(&mut ctx, payload.clone())
                    .map(|prepared| {
                        group_publish_from_prepared_send(prepared, pending.fanout.clone())
                    }),
            };
            let prepared = match prepared {
                Ok(prepared) => {
                    if !prepared.deliveries.is_empty()
                        || !prepared.invite_responses.is_empty()
                        || !prepared.sender_key_messages.is_empty()
                    {
                        session_changed = true;
                        persist_needed = true;
                    }
                    prepared
                }
                Err(_) => {
                    pending.next_retry_at_secs =
                        next_pending_retry_at_secs(pending.created_at_secs, now);
                    still_pending.push(pending);
                    continue;
                }
            };
            let still_has_gap = !prepared.relay_gaps.is_empty();
            let mut event_ids = Vec::new();
            let chat_id = group_chat_id(&pending.group_id);
            effects.extend(protocol_effects_from_group_prepared_publish(
                &prepared,
                pending.inner_event_id.clone(),
                chat_id,
                &mut event_ids,
            )?);
            if still_has_gap {
                pending.next_retry_at_secs =
                    next_pending_retry_at_secs(pending.created_at_secs, now);
                still_pending.push(pending);
            }
        }
        self.pending_group_fanouts = still_pending;
        if session_changed {
            self.invalidate_known_message_author_cache();
        }
        if persist_needed {
            self.persist()?;
        }
        Ok(ProtocolGroupIncomingResult {
            effects,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod incoming_retry_tests {
    use super::*;
    use crate::InMemoryStorage;
    use nostr_double_ratchet::message_event;

    fn test_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
        ProtocolEngine::load_or_create_for_local_device(
            Arc::new(InMemoryStorage::new()),
            owner.public_key(),
            device,
        )
        .expect("test protocol engine")
    }

    fn direct_message_before_receiver_observes_response(
        receiver: &ProtocolEngine,
        sender_owner: &Keys,
        body: &str,
        created_at_secs: u64,
    ) -> (Event, Event) {
        let invite = receiver.local_invite().expect("receiver invite");
        let (mut sender_session, response) = invite
            .accept_with_owner(
                sender_owner.public_key(),
                sender_owner.secret_key().to_secret_bytes(),
                Some(sender_owner.public_key().to_hex()),
                Some(sender_owner.public_key()),
            )
            .expect("sender accepts receiver invite");
        let response_event = invite_response_event(&response).expect("invite response event");
        let plan = sender_session
            .plan_send(body.as_bytes(), NdrUnixSeconds(created_at_secs))
            .expect("sender plans direct message");
        let sent = sender_session.apply_send(plan);
        let message_event = message_event(&sent.envelope).expect("direct message event");
        (message_event, response_event)
    }

    #[test]
    fn direct_message_retry_clears_header_sender_key_candidate() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello after backfill",
            100,
        );

        assert!(
            bob.process_direct_message_event(&message_event)
                .expect("unknown direct message queues")
                .is_none(),
            "receiver should not decrypt before it observes the session response"
        );
        assert_eq!(bob.pending_inbound.len(), 1);
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);
        assert!(
            bob.has_pending_retry_work(),
            "queued direct/group candidate should keep liveness retry work active"
        );

        let mut direct_messages = bob
            .observe_invite_response_event(&response_event)
            .expect("receiver observes session response");
        let retry = bob
            .retry_pending_protocol(NdrUnixSeconds(103))
            .expect("retry pending direct message");
        direct_messages
            .direct_messages
            .extend(retry.direct_messages);

        assert_eq!(direct_messages.direct_messages.len(), 1);
        assert_eq!(
            direct_messages.direct_messages[0].content,
            "hello after backfill"
        );
        assert!(bob.pending_inbound.is_empty());
        assert!(
            bob.pending_group_sender_key_messages.is_empty(),
            "the same event must not remain queued as a sender-key repair candidate after direct decrypt succeeds"
        );
        assert!(
            bob.pending_group_sender_key_repairs.is_empty(),
            "direct decrypt success should not leave sender-key repair bookkeeping behind"
        );
        assert!(
            !bob.has_pending_retry_work(),
            "all retry work should be clear after the pending direct message applies"
        );
    }

    #[test]
    fn unknown_group_sender_key_candidate_alone_does_not_keep_retry_work_alive() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, _response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello before metadata",
            100,
        );

        assert!(
            bob.process_direct_message_event(&message_event)
                .expect("unknown direct message queues")
                .is_none()
        );
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);
        bob.pending_inbound.clear();

        assert!(
            !bob.has_pending_retry_work(),
            "a header-shaped direct event with no known group sender key must not keep liveness hot by itself"
        );
    }

    #[test]
    fn known_direct_message_author_prunes_unmapped_sender_key_candidate_without_decrypt() {
        let bob_owner = Keys::generate();
        let bob_device = Keys::generate();
        let alice_owner = Keys::generate();
        let mut bob = test_engine(&bob_owner, &bob_device);
        let (message_event, response_event) = direct_message_before_receiver_observes_response(
            &bob,
            &alice_owner,
            "hello from another target",
            100,
        );

        assert!(
            bob.process_direct_message_event(&message_event)
                .expect("unknown direct message queues")
                .is_none()
        );
        assert_eq!(bob.pending_inbound.len(), 1);
        assert_eq!(bob.pending_group_sender_key_messages.len(), 1);

        bob.pending_inbound.clear();
        let retry = bob
            .observe_invite_response_event(&response_event)
            .expect("receiver observes session response");

        assert!(
            retry.direct_messages.is_empty(),
            "test setup removed the pending direct decrypt path"
        );
        assert!(bob.is_known_message_author(message_event.pubkey));
        assert!(
            bob.pending_group_sender_key_messages.is_empty(),
            "once the event pubkey is known to be a direct-message author, an unmapped group sender-key candidate should be pruned"
        );
        assert_eq!(
            bob.debug_snapshot().pending_group_sender_key_unmapped_count,
            0
        );
        assert!(
            !bob.has_pending_retry_work(),
            "pruning the stale candidate should leave no background retry work"
        );
    }
}
