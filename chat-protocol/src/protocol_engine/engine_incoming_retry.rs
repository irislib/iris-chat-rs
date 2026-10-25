impl ProtocolEngine {
    /// Shared bootstrap authors can expose ciphertext addressed to other devices.
    /// Untargeted legacy messages and group broadcasts remain eligible.
    pub fn message_targets_another_device(&self, event: &Event) -> bool {
        if event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
            || !protocol_event_has_tag(event, "header")
        {
            return false;
        }
        let mut has_recipient = false;
        for recipient in event.tags.public_keys() {
            has_recipient = true;
            if *recipient == self.owner_pubkey || ndr_device(*recipient) == self.local_device {
                return false;
            }
        }
        has_recipient
    }

    fn invalidate_pending_inbound_retry_deadline(&self) {
        self.pending_inbound_retry_deadline.set(None);
    }

    fn refresh_pending_inbound_retry_deadline(&self) -> PendingInboundRetryDeadline {
        let deadline = PendingInboundRetryDeadline {
            queue_len: self.pending_inbound.len(),
            next_retry_at_secs: self
                .pending_inbound
                .iter()
                .map(|pending| {
                    #[cfg(test)]
                    tests::record_inbound_retry_deadline_scan();
                    pending.next_retry_at_secs
                })
                .min(),
        };
        self.pending_inbound_retry_deadline.set(Some(deadline));
        deadline
    }

    fn has_due_pending_inbound_retry(&self, now: NdrUnixSeconds) -> bool {
        let deadline = self
            .pending_inbound_retry_deadline
            .get()
            .filter(|deadline| deadline.queue_len == self.pending_inbound.len())
            .unwrap_or_else(|| self.refresh_pending_inbound_retry_deadline());
        deadline
            .next_retry_at_secs
            .is_some_and(|next_retry| next_retry <= now.get())
    }

    fn invalidate_pending_group_fanout_retry_deadline(&self) {
        self.pending_group_fanout_retry_deadline.set(None);
    }

    fn has_due_pending_group_fanout_retry(&self, now: NdrUnixSeconds) -> bool {
        if let Some(deadline) = self
            .pending_group_fanout_retry_deadline
            .get()
            .filter(|deadline| deadline.queue_len == self.pending_group_fanouts.len())
        {
            return deadline
                .next_retry_at_secs
                .is_some_and(|next_retry| next_retry <= now.get());
        }
        let mut next_retry_at_secs: Option<u64> = None;
        for pending in &self.pending_group_fanouts {
            #[cfg(test)]
            tests::record_fanout_retry_deadline_scan();
            let next_retry = pending.next_retry_at_secs;
            // Preserve the early positive query across successive due batches;
            // only a fully deferred queue needs a cached minimum.
            if next_retry <= now.get() {
                return true;
            }
            next_retry_at_secs =
                Some(next_retry_at_secs.map_or(next_retry, |earliest| earliest.min(next_retry)));
        }
        self.pending_group_fanout_retry_deadline
            .set(Some(PendingGroupFanoutRetryDeadline {
                queue_len: self.pending_group_fanouts.len(),
                next_retry_at_secs,
            }));
        false
    }

    pub fn has_due_pending_retry_work(&self, now: NdrUnixSeconds) -> bool {
        // Signed group batches remain immediately runnable until the app outbox
        // acknowledges them. A retry also retires any now-ineligible entries.
        if !self.pending_decrypted_deliveries.is_empty()
            || !self.ready_group_publications.is_empty()
        {
            return true;
        }
        if !self.pending_group_sender_key_messages.is_empty() {
            let retry = self.group_sender_key_retry.borrow();
            // Refresh leaves a frozen search's inputs untouched. Its established
            // continuation already makes this turn runnable, without due scans.
            if !retry.ready.is_empty() && (retry.active.is_some() || retry.prepared.is_some()) {
                return true;
            }
        }
        let now_secs = now.get();
        self.pending_remote_sends
            .iter()
            .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self
                .pending_local_sibling_sends
                .iter()
                .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self.has_due_pending_inbound_retry(now)
            || self.has_due_pending_group_fanout_retry(now)
            || self
                .pending_group_pairwise_payloads
                .iter()
                .any(|pending| pending.next_retry_at_secs <= now_secs)
            || self.pending_group_sender_key_repairs.iter().any(|pending| {
                Self::pending_group_sender_key_repair_due_at_secs(pending) <= now_secs
            })
            || self.has_ready_group_sender_key_retry_work()
            || !self.pending_decrypted_deliveries.is_empty()
    }

    pub fn process_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;
        if self.message_targets_another_device(event) {
            return Ok(None);
        }
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
            Ok(DirectReceive::Message(decrypted)) => return Ok(Some(decrypted)),
            Ok(DirectReceive::Discarded) => return Ok(None),
            Ok(DirectReceive::Pending) => {}
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
        if self.message_targets_another_device(event) {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
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
        if self
            .processed_group_sender_key_messages
            .contains(&group_sender_key_fingerprint(&message))
        {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        // Direct-message probing can queue a candidate before its first group
        // attempt. Suppress only candidates already waiting for a repair.
        if self.pending_group_sender_key_messages.contains(&parsed)
            && self.group_sender_key_message_has_pending_repair(&message)
        {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                pending: true,
                ..Default::default()
            });
        }
        let mut result = self.handle_group_sender_key_message(message)?;
        if result.pending {
            self.queue_pending_group_sender_key_message(parsed)?;
        } else if self.clear_pending_group_sender_key_candidate(&parsed) {
            self.persist()?;
        }
        result.source_event_count = result.events.len();
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
        self.process_group_pairwise_payload_inner(
            payload,
            from_owner_pubkey,
            from_sender_device_pubkey,
            true,
            false,
        )
    }

    pub fn process_local_sibling_group_pairwise_payload(
        &mut self,
        payload: &[u8],
        from_owner_pubkey: PublicKey,
        from_sender_device_pubkey: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        self.process_group_pairwise_payload_inner(
            payload,
            from_owner_pubkey,
            from_sender_device_pubkey,
            false,
            true,
        )
    }

    fn process_group_pairwise_payload_inner(
        &mut self,
        payload: &[u8],
        from_owner_pubkey: PublicKey,
        from_sender_device_pubkey: Option<PublicKey>,
        forward_to_local_siblings: bool,
        trust_local_sibling_attribution: bool,
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
        if is_supported_group_payload && sender_device.is_none() && !trust_local_sibling_attribution
        {
            // A remote owner claim without the authenticated device identity D
            // cannot be checked against AppKeys. Consume the control payload
            // without applying it; normal pairwise decrypts always supply D.
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        let sender_owner = if trust_local_sibling_attribution {
            // The outer pairwise session already authenticated a current local
            // sibling. Its wrapper can carry the original remote owner/device
            // so every sibling does not need to race the same AppKeys event.
            sender_owner
        } else {
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
            }
        };
        let remote_sender_key_distribution_group_id =
            if sender_owner != self.local_owner && sender_device.is_some() {
                JsonGroupPayloadCodecV1
                    .decode_pairwise_command(payload)
                    .ok()
                    .flatten()
                    .and_then(|command| match command {
                        GroupPairwiseCommand::SenderKeyDistribution { distribution } => {
                            Some(distribution.group_id)
                        }
                        _ => None,
                    })
            } else {
                None
            };
        let previous_groups = is_supported_group_payload.then(|| self.group_manager.snapshot());
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
                let group_changed = previous_groups
                    .as_ref()
                    .is_some_and(|previous| *previous != self.group_manager.snapshot());
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
                if group_changed && matches!(&event, GroupIncomingEvent::MetadataUpdated(_)) {
                    for pending in &mut self.pending_group_pairwise_payloads {
                        pending.next_retry_at_secs = 0;
                    }
                }
                if group_changed && forward_to_local_siblings && sender_owner != self.local_owner {
                    if let GroupIncomingEvent::MetadataUpdated(group) = &event {
                        let sync_effects = self.sync_group_to_local_siblings(group)?;
                        effects.extend(sync_effects);
                    }
                    if let (Some(group_id), Some(sender_device)) =
                        (&remote_sender_key_distribution_group_id, sender_device)
                    {
                        effects.extend(
                            self.sync_remote_group_sender_key_distribution_to_local_siblings(
                                group_id,
                                sender_owner,
                                sender_device,
                                payload,
                            )?,
                        );
                    }
                }
                let mut events = vec![event];
                let retry = self.retry_pending_group_inputs(now)?;
                events.extend(retry.events);
                effects.extend(retry.effects);
                let fanout_retry = self.retry_pending_group_fanouts(now)?;
                effects.extend(fanout_retry.effects);
                if group_changed || !effects.is_empty() {
                    self.persist()?;
                }
                Ok(ProtocolGroupIncomingResult {
                    events,
                    effects,
                    consumed: true,
                    ..Default::default()
                })
            }
            Ok(None) => Ok(ProtocolGroupIncomingResult {
                consumed: is_group_payload,
                ..Default::default()
            }),
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

    /// Resolve a sender-key delivery already authenticated and durably journaled.
    /// External pairwise group-message payloads remain unsupported. The caller
    /// acknowledges this ID only after its application snapshot is saved.
    pub fn pending_group_decrypted_delivery(
        &self,
        event_id: &str,
    ) -> anyhow::Result<Option<GroupIncomingEvent>> {
        if !event_id.starts_with("group-delivery:") {
            return Ok(None);
        }
        let pending = self
            .pending_decrypted_deliveries
            .iter()
            .find(|pending| pending.event_id.as_deref() == Some(event_id))
            .ok_or_else(|| anyhow::anyhow!("group delivery is not in the authenticated journal"))?;
        let Some(GroupPairwiseCommand::GroupMessage {
            group_id,
            revision,
            body,
        }) = JsonGroupPayloadCodecV1.decode_pairwise_command(pending.content.as_bytes())?
        else {
            anyhow::bail!("invalid group delivery journal envelope");
        };
        Ok(Some(GroupIncomingEvent::Message(
            nostr_double_ratchet::GroupReceivedMessage {
                group_id,
                revision,
                body,
                sender_owner: ndr_owner(pending.sender),
                sender_device: pending.sender_device.map(ndr_device),
            },
        )))
    }

    pub fn retry_pending_protocol(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("retirement");
            self.retire_ineligible_group_publications()?;
            self.flush_discarded_direct_receives()?;
        }
        let group_result = {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("group_inputs");
            self.retry_pending_group_inputs(now)?
        };
        let group_fanout_result = {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("group_fanouts");
            self.retry_pending_group_fanouts(now)?
        };
        let mut group_result = group_result;
        group_result.effects.extend(group_fanout_result.effects);
        {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("inbound");
            self.retry_pending_inbound_direct_events(now)?;
        }
        let direct_messages = self
            .pending_decrypted_deliveries
            .iter()
            .filter(|delivery| !delivery.discarded)
            .cloned()
            .map(ProtocolDecryptedMessage::from)
            .collect::<Vec<_>>();
        let mut effects = {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("remote_sends");
            self.retry_pending_remote_sends(now)?
        };
        {
            #[cfg(all(test, target_os = "linux"))]
            let _phase = tests::RetryPhaseTimer::start("local_siblings");
            effects.extend(self.retry_pending_local_sibling_sends(now)?);
        }
        let offered = group_result
            .effects
            .iter()
            .chain(&effects)
            .map(|effect| {
                let ProtocolEffect::Publish(publish) = effect;
                publish.event.id
            })
            .collect::<HashSet<_>>();
        effects.extend(
            self.pending_group_publish_effects()
                .into_iter()
                .filter(|effect| {
                    let ProtocolEffect::Publish(publish) = effect;
                    !offered.contains(&publish.event.id)
                }),
        );
        let batch = ProtocolRetryBatch {
            group_result,
            direct_messages,
            effects,
        };
        if !batch.is_empty() {
            self.subscription_generation = self.subscription_generation.wrapping_add(1);
        }
        Ok(batch)
    }

    fn discard_pending_messages_for_other_devices(&mut self) -> anyhow::Result<()> {
        let foreign_events = self
            .pending_inbound
            .iter()
            .filter(|pending| self.message_targets_another_device(&pending.event))
            .map(|pending| pending.event.clone())
            .collect::<Vec<_>>();
        if foreign_events.is_empty() {
            return Ok(());
        }
        let ids = foreign_events
            .iter()
            .map(|event| event.id)
            .collect::<HashSet<_>>();
        self.with_state_checkpoint(|engine| {
            engine.invalidate_pending_inbound_retry_deadline();
            engine
                .pending_inbound
                .retain(|pending| !ids.contains(&pending.event.id));
            // Older clients also tried these packets as camouflaged group messages.
            for event in &foreign_events {
                engine.clear_pending_group_sender_key_candidate_for_direct_event(event);
            }
            engine.persist()
        })
    }

    pub fn ack_pending_decrypted_deliveries(&mut self) -> anyhow::Result<()> {
        let ids = self
            .pending_decrypted_deliveries
            .iter()
            .filter(|delivery| !delivery.discarded)
            .filter_map(|delivery| delivery.event_id.clone())
            .collect();
        self.ack_decrypted_delivery_ids(&ids)
    }

    /// Only acknowledge deliveries that the application has durably applied.
    /// Other saves must not discard plaintext waiting for its first delivery.
    pub fn ack_decrypted_delivery_ids(
        &mut self,
        event_ids: &HashSet<String>,
    ) -> anyhow::Result<()> {
        if !self.pending_decrypted_deliveries.iter().any(|delivery| {
            delivery
                .event_id
                .as_ref()
                .is_some_and(|id| event_ids.contains(id))
        }) {
            return Ok(());
        }
        let before = self.pending_decrypted_deliveries.clone();
        self.pending_decrypted_deliveries.retain(|delivery| {
            delivery.discarded
                || !delivery
                    .event_id
                    .as_ref()
                    .is_some_and(|id| event_ids.contains(id))
        });
        if let Err(error) = self.persist() {
            self.pending_decrypted_deliveries = before;
            return Err(error);
        }
        Ok(())
    }

    fn retry_pending_inbound_direct_events_inner(
        &mut self,
        now: NdrUnixSeconds,
        fingerprints: &mut InboundReceiveFingerprints,
    ) -> anyhow::Result<Vec<ProtocolDecryptedMessage>> {
        // A failed save can leave a ratchet advanced with its delivery journal.
        // Keep the hint unknown until the entire pass succeeds, so retry and
        // rollback paths rederive it from the retained queue.
        self.invalidate_pending_inbound_retry_deadline();
        // Keep the queue installed while decrypting: decryption persists the
        // ratchet and its delivery journal, including all not-yet-tried events.
        // Group continuations run every 25 ms even while inbound packets are
        // backed off. Copy only due packets, not the entire deferred backlog.
        let pending = self
            .pending_inbound
            .iter()
            .enumerate()
            .filter(|(_, pending)| {
                #[cfg(test)]
                tests::record_inbound_retry_deadline_scan();
                pending.next_retry_at_secs <= now.get()
            })
            .map(|(index, pending)| (index, pending.clone()))
            .collect::<Vec<_>>();
        let mut messages = Vec::new();
        for (original_index, pending) in pending {
            match self.decrypt_pending_direct_message_with_cache(&pending, fingerprints) {
                Ok(DirectReceive::Message(message)) => {
                    self.pending_inbound
                        .retain(|item| item.event.id != pending.event.id);
                    messages.push(message);
                }
                Ok(DirectReceive::Discarded) => {
                    self.pending_inbound
                        .retain(|item| item.event.id != pending.event.id);
                }
                Err(error) if error.downcast_ref::<StorageError>().is_some() => {
                    // A failed save may follow a successful ratchet advance.
                    // Keep the delivery journal intact and let the caller retry.
                    return Err(error);
                }
                Ok(DirectReceive::Pending) | Err(_) => {
                    // One undecryptable event must not block other senders or
                    // outgoing acknowledgements. Retain it with backoff so a
                    // later handshake can still make it decryptable.
                    // Failed receives normally leave the queue in place. Reuse
                    // that position instead of searching the whole queue for
                    // every failed packet. Earlier deliveries can remove entries,
                    // so verify the ID and fall back to a search after a shift.
                    let matches_id = |item: &ProtocolPendingInbound| {
                        #[cfg(test)]
                        tests::record_inbound_retry_lookup();
                        item.event.id == pending.event.id
                    };
                    let index = if self
                        .pending_inbound
                        .get(original_index)
                        .is_some_and(matches_id)
                    {
                        Some(original_index)
                    } else {
                        self.pending_inbound.iter().position(matches_id)
                    };
                    if let Some(index) = index {
                        self.pending_inbound[index].next_retry_at_secs =
                            next_pending_retry_at_secs(pending.created_at_secs, now);
                    }
                }
            }
        }
        self.refresh_pending_inbound_retry_deadline();
        Ok(messages)
    }

    #[cfg(test)]
    fn decrypt_pending_direct_message_event(
        &mut self,
        pending: &ProtocolPendingInbound,
    ) -> anyhow::Result<DirectReceive> {
        if let Some(envelope) = pending.envelope.as_ref() {
            return self.decrypt_direct_message_envelope(&pending.event, envelope, true);
        }
        self.decrypt_direct_message_event(&pending.event)
    }

    #[cfg(test)]
    fn decrypt_direct_message_event(&mut self, event: &Event) -> anyhow::Result<DirectReceive> {
        let envelope = parse_message_event(event)?;
        self.decrypt_direct_message_envelope(event, &envelope, true)
    }

    fn decrypt_direct_message_envelope(
        &mut self,
        event: &Event,
        envelope: &MessageEnvelope,
        record_delivery: bool,
    ) -> anyhow::Result<DirectReceive> {
        self.decrypt_direct_message_envelope_with_cache(event, envelope, record_delivery, None)
    }

    fn decrypt_direct_message_envelope_with_cache(
        &mut self,
        event: &Event,
        envelope: &MessageEnvelope,
        record_delivery: bool,
        fingerprints: Option<&mut InboundReceiveFingerprints>,
    ) -> anyhow::Result<DirectReceive> {
        if let Some(delivery) = self
            .pending_decrypted_deliveries
            .iter()
            .find(|delivery| delivery.event_id.as_deref() == Some(event.id.to_hex().as_str()))
            .cloned()
        {
            // The ratchet may already have advanced before a previous save failed.
            if delivery.discarded {
                self.flush_discarded_direct_receives()?;
                return Ok(DirectReceive::Discarded);
            }
            self.persist()?;
            return Ok(DirectReceive::Message(delivery.into()));
        }
        let sender_owner = match self.resolve_message_sender_owner(envelope) {
            ProtocolSenderOwnerResolution::Verified { owner }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. } => {
                return Ok(DirectReceive::Pending);
            }
        };
        let Some(received) =
            self.receive_direct_message_with_cache(event, envelope, sender_owner, fingerprints)?
        else {
            return Ok(DirectReceive::Pending);
        };
        self.clear_pending_group_sender_key_candidate_for_direct_event(event);
        self.invalidate_known_message_author_cache();
        let from_local_sibling = received.owner_pubkey == self.local_owner;
        let local_sibling = from_local_sibling
            .then(|| decode_local_sibling_payload(&received.payload))
            .flatten();
        let (conversation_owner, sender, sender_device, payload) = match local_sibling {
            Some((owner, original_device, payload)) => (
                Some(owner),
                original_device.map_or(self.owner_pubkey, |_| owner),
                original_device.or(Some(public_device(received.device_pubkey)?)),
                payload,
            ),
            _ => (
                None,
                public_owner(received.owner_pubkey)?,
                Some(public_device(received.device_pubkey)?),
                received.payload,
            ),
        };
        // Own-device controls use history-aware reconciliation. Never put their
        // replacement text in the delivery journal before application admission.
        if from_local_sibling && is_message_mutation_payload(&payload) {
            self.pending_decrypted_deliveries
                .push(ProtocolPendingDecryptedDelivery {
                    discarded: true,
                    sender,
                    sender_device,
                    conversation_owner,
                    content: String::new(),
                    event_id: Some(event.id.to_string()),
                    created_at_secs: event.created_at.as_secs(),
                });
            self.invalidate_pending_inbound_retry_deadline();
            self.pending_inbound
                .retain(|pending| pending.event.id != event.id);
            self.flush_discarded_direct_receives()?;
            return Ok(DirectReceive::Discarded);
        }
        let content = String::from_utf8(payload)?;
        let decrypted = ProtocolDecryptedMessage {
            created_at_secs: event.created_at.as_secs(),
            sender,
            sender_device,
            conversation_owner,
            content,
            event_id: Some(event.id.to_string()),
        };
        if record_delivery {
            self.record_pending_decrypted_delivery(decrypted.clone(), event.created_at.as_secs());
        }
        self.persist()?;
        Ok(DirectReceive::Message(decrypted))
    }

    fn retry_pending_group_inputs(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let mut result = ProtocolGroupIncomingResult {
            consumed: false,
            ..Default::default()
        };

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
            if pending.sender_device.is_none() {
                // Legacy device-less entries have no cryptographic identity to
                // authorize against the claimed owner and can never be promoted.
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

        let sender_keys = self.retry_eligible_group_sender_key_messages()?;
        result.events.extend(sender_keys.events);
        result.effects.extend(sender_keys.effects);
        let repair_effects = self.retry_pending_group_sender_key_repairs(now)?;
        if !repair_effects.is_empty() {
            result.effects.extend(repair_effects);
            persist_needed = true;
        }
        if persist_needed {
            self.persist()?;
        }
        Ok(result)
    }
}

include!("incoming_retry_tests.rs");
