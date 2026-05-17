impl ProtocolEngine {
    pub(super) fn process_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;
        let resolution = self.resolve_message_sender_owner(&envelope);
        match resolution {
            ProtocolSenderOwnerResolution::Verified { .. }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { .. } => {}
            ProtocolSenderOwnerResolution::PendingOwnerClaim { .. } => {
                self.queue_pending_inbound_direct_event(
                    event.clone(),
                    event.created_at.as_secs(),
                    Some(&envelope),
                    Some(resolution),
                )?;
                return Ok(None);
            }
        };
        if let Some(decrypted) = self.decrypt_direct_message_envelope(event, &envelope, true)? {
            return Ok(Some(decrypted));
        }
        self.queue_pending_inbound_direct_event(
            event.clone(),
            event.created_at.as_secs(),
            Some(&envelope),
            Some(resolution),
        )?;
        Ok(None)
    }

    pub(super) fn process_group_outer_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let Ok(parsed) = parse_group_sender_key_message_event(event) else {
            return Ok(ProtocolGroupIncomingResult::default());
        };
        let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
            self.queue_pending_group_sender_key_message(parsed)?;
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        };
        let result = self.handle_group_sender_key_message(message)?;
        if result.pending {
            self.queue_pending_group_sender_key_message(parsed)?;
        }
        Ok(ProtocolGroupIncomingResult {
            consumed: true,
            ..result
        })
    }

    pub(super) fn process_group_pairwise_payload(
        &mut self,
        payload: &[u8],
        from_owner_pubkey: PublicKey,
        from_sender_device_pubkey: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let is_group_payload = self.group_manager.is_pairwise_payload(payload);
        let sender_device = from_sender_device_pubkey.map(ndr_device);
        let sender_owner = ndr_owner(from_owner_pubkey);
        let sender_owner =
            match self.resolve_group_pairwise_sender_owner(sender_owner, sender_device) {
                ProtocolSenderOwnerResolution::Verified { owner }
                | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
                ProtocolSenderOwnerResolution::PendingOwnerClaim {
                    storage_owner,
                    claimed_owner,
                    sender_device,
                } => {
                    if is_group_payload {
                        let queued_targets = vec![format!("owner:{}", claimed_owner.to_hex())];
                        let effects = self.protocol_backfill_effects_for_targets(
                            &queued_targets,
                            NdrUnixSeconds(unix_now().get()),
                            "group_pairwise_owner_claim",
                        );
                        self.queue_pending_group_pairwise_payload(
                            storage_owner,
                            Some(sender_device),
                            payload.to_vec(),
                            unix_now().get(),
                        )?;
                        return Ok(ProtocolGroupIncomingResult {
                            effects,
                            queued_targets,
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

        match result {
            Ok(Some(event)) => {
                let mut events = vec![event];
                let retry = self.retry_pending_group_inputs(NdrUnixSeconds(unix_now().get()))?;
                events.extend(retry.events);
                let mut effects = retry.effects;
                effects.extend(self.retry_pending_group_fanouts(NdrUnixSeconds(unix_now().get()))?);
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events,
                    effects,
                    queued_targets: self.queued_group_targets(),
                    consumed: true,
                    ..Default::default()
                })
            }
            Ok(None) => Ok(ProtocolGroupIncomingResult {
                consumed: is_group_payload,
                ..Default::default()
            }),
            Err(error) => {
                if is_group_payload {
                    self.queue_pending_group_pairwise_payload(
                        sender_owner,
                        sender_device,
                        payload.to_vec(),
                        unix_now().get(),
                    )?;
                    let queued_targets = self.queued_group_targets();
                    let effects = self.protocol_backfill_effects_for_targets(
                        &queued_targets,
                        NdrUnixSeconds(unix_now().get()),
                        "group_pairwise_retry",
                    );
                    Ok(ProtocolGroupIncomingResult {
                        effects,
                        queued_targets,
                        consumed: true,
                        ..Default::default()
                    })
                } else {
                    Err(error.into())
                }
            }
        }
    }

    pub(super) fn retry_pending_outbound(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolRetryResult>> {
        if self.pending_outbound.is_empty() {
            return Ok(Vec::new());
        }
        let pending = std::mem::take(&mut self.pending_outbound);
        let mut still_pending = Vec::new();
        let mut results = Vec::new();

        for mut pending in pending {
            if pending.next_retry_at_secs > now.get() {
                still_pending.push(pending);
                continue;
            }

            let recipient_owner = match PublicKey::parse(&pending.recipient_owner_hex) {
                Ok(pubkey) => ndr_owner(pubkey),
                Err(_) => continue,
            };
            if pending.probe_local_sibling_roster
                && now.get().saturating_sub(pending.created_at_secs)
                    > LOCAL_SIBLING_ROSTER_PROBE_TTL_SECS
                && self
                    .remaining_local_sibling_targets(&pending.delivered_local_device_hexes)
                    .is_empty()
            {
                pending.probe_local_sibling_roster = false;
            }
            let remote_targets = self
                .remaining_remote_targets(recipient_owner, &pending.delivered_remote_device_hexes);
            let local_targets =
                self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes);

            if remote_targets.is_empty() && local_targets.is_empty() {
                let queued_targets = self.pending_target_hexes(&pending);
                if (pending.waits_for_remote_protocol_state() || pending.probe_local_sibling_roster)
                    && !queued_targets.is_empty()
                {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    still_pending.push(pending.clone());
                    let effects =
                        self.protocol_backfill_effects_for_pending_outbound(&pending, now, "retry");
                    results.push(ProtocolRetryResult {
                        message_id: pending.message_id.clone(),
                        chat_id: pending.chat_id.clone(),
                        event_ids: Vec::new(),
                        effects,
                        queued_targets,
                    });
                }
                continue;
            }

            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(now, &mut rng);
            let mut event_ids = Vec::new();
            let mut effects = Vec::new();
            let mut gaps = Vec::new();

            if !remote_targets.is_empty() {
                let remote = self.session_manager.prepare_remote_send_to_devices(
                    &mut ctx,
                    recipient_owner,
                    remote_targets,
                    pending.remote_payload.clone(),
                )?;
                pending
                    .delivered_remote_device_hexes
                    .extend(delivered_device_hexes(&remote));
                gaps.extend(remote.relay_gaps.clone());
                effects.extend(protocol_effects_from_prepared(
                    &remote,
                    pending.inner_event_id.clone(),
                    &mut event_ids,
                )?);
            }

            if let Some(local_payload) = pending.local_sibling_payload.clone() {
                if !local_targets.is_empty() {
                    let local = self.session_manager.prepare_local_sibling_send_to_devices(
                        &mut ctx,
                        local_targets,
                        local_payload,
                    )?;
                    pending
                        .delivered_local_device_hexes
                        .extend(delivered_device_hexes(&local));
                    gaps.extend(local.relay_gaps.clone());
                    effects.extend(protocol_effects_from_prepared(
                        &local,
                        pending.inner_event_id.clone(),
                        &mut event_ids,
                    )?);
                }
            }

            pending.delivered_remote_device_hexes.sort();
            pending.delivered_remote_device_hexes.dedup();
            pending.delivered_local_device_hexes.sort();
            pending.delivered_local_device_hexes.dedup();

            let queued_targets = self.pending_target_hexes(&pending);
            if !queued_targets.is_empty() || !gaps.is_empty() {
                if !gaps.is_empty() {
                    pending.reason = pending_reason_from_gaps(&gaps);
                }
                pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                still_pending.push(pending.clone());
            }
            if !event_ids.is_empty() || !effects.is_empty() || !queued_targets.is_empty() {
                effects.extend(
                    self.protocol_backfill_effects_for_pending_outbound(&pending, now, "retry"),
                );
                results.push(ProtocolRetryResult {
                    message_id: pending.message_id.clone(),
                    chat_id: pending.chat_id.clone(),
                    event_ids,
                    effects,
                    queued_targets,
                });
            }
        }

        self.pending_outbound = still_pending;
        self.persist()?;
        Ok(results)
    }

    pub(super) fn retry_pending_protocol(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        self.last_backfill_attempt_secs = now.get();
        let direct_results = self.retry_pending_outbound(now)?;
        let group_result = self.retry_pending_group_inputs(now)?;
        let group_effects = self.retry_pending_group_fanouts(now)?;
        let mut group_result = group_result;
        group_result.effects.extend(group_effects);
        group_result.queued_targets = self.queued_group_targets();
        self.append_queued_protocol_backfill(
            &mut group_result.effects,
            &group_result.queued_targets,
            now,
            "group_retry",
        );
        let mut direct_messages = self
            .pending_decrypted_deliveries
            .iter()
            .cloned()
            .map(ProtocolDecryptedMessage::from)
            .collect::<Vec<_>>();
        direct_messages.extend(self.retry_pending_inbound_direct_events(now)?);
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.persist()?;
        Ok(ProtocolRetryBatch {
            direct_results,
            group_result,
            direct_messages,
            effects: Vec::new(),
        })
    }

    pub(super) fn ack_pending_decrypted_deliveries(&mut self) -> anyhow::Result<()> {
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
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
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
        let mut pairwise_changed = false;
        for mut pending in pairwise {
            if pending.next_retry_at_secs > now.get() {
                still_pairwise.push(pending);
                continue;
            }
            let sender_resolution = self
                .resolve_group_pairwise_sender_owner(pending.sender_owner, pending.sender_device);
            let sender_owner = match sender_resolution {
                ProtocolSenderOwnerResolution::Verified { owner }
                | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
                ProtocolSenderOwnerResolution::PendingOwnerClaim { claimed_owner, .. } => {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    result
                        .queued_targets
                        .push(format!("owner:{}", claimed_owner.to_hex()));
                    still_pairwise.push(pending);
                    pairwise_changed = true;
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
                    result.events.push(event);
                    pairwise_changed = true;
                }
                Ok(None) => {
                    pairwise_changed = true;
                }
                Err(_) => {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    still_pairwise.push(pending);
                    pairwise_changed = true;
                }
            }
        }
        self.pending_group_pairwise_payloads = still_pairwise;
        result.queued_targets.sort();
        result.queued_targets.dedup();

        let sender_keys = std::mem::take(&mut self.pending_group_sender_key_messages);
        let mut still_sender_keys = Vec::new();
        let mut sender_keys_changed = false;
        for parsed in sender_keys {
            let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
                still_sender_keys.push(parsed);
                continue;
            };
            let outcome = self.handle_group_sender_key_message(message)?;
            if outcome.pending {
                still_sender_keys.push(parsed);
            } else {
                sender_keys_changed = true;
            }
            result.events.extend(outcome.events);
            result.effects.extend(outcome.effects);
        }
        self.pending_group_sender_key_messages = still_sender_keys;
        if pairwise_changed
            || sender_keys_changed
            || !result.events.is_empty()
            || !result.effects.is_empty()
        {
            self.persist()?;
        }
        Ok(result)
    }

    fn retry_pending_group_fanouts(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        if self.pending_group_fanouts.is_empty() {
            return Ok(Vec::new());
        }
        let pending = std::mem::take(&mut self.pending_group_fanouts);
        let mut still_pending = Vec::new();
        let mut effects = Vec::new();
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
                Ok(prepared) => prepared,
                Err(_) => {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    still_pending.push(pending);
                    continue;
                }
            };
            let still_has_gap = !prepared.relay_gaps.is_empty();
            let mut event_ids = Vec::new();
            effects.extend(protocol_effects_from_group_prepared_publish(
                &prepared,
                pending.inner_event_id.clone(),
                &mut event_ids,
            )?);
            if still_has_gap {
                pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                still_pending.push(pending);
            }
        }
        self.pending_group_fanouts = still_pending;
        self.persist()?;
        Ok(effects)
    }

}
