impl ProtocolEngine {
    fn resolve_message_sender_owner(
        &self,
        envelope: &MessageEnvelope,
    ) -> ProtocolSenderOwnerResolution {
        self.resolve_message_sender_owner_for_sender(envelope.sender)
    }

    fn resolve_message_sender_owner_for_sender(
        &self,
        sender: NdrDevicePubkey,
    ) -> ProtocolSenderOwnerResolution {
        self.session_record_matching_message_sender(sender)
            .map(|record| self.owner_resolution_for_sender_record(record))
            .unwrap_or_else(|| ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                owner: provisional_owner_from_sender_pubkey(sender),
            })
    }

    fn resolve_group_pairwise_sender_owner(
        &self,
        sender_owner: NdrOwnerPubkey,
        sender_device: Option<NdrDevicePubkey>,
    ) -> ProtocolSenderOwnerResolution {
        if let Some(sender_device) = sender_device {
            if let Some(record) = self.session_record_for_device_identity(sender_device) {
                return self.owner_resolution_for_sender_record(record);
            }
            if sender_owner == provisional_owner_from_sender_pubkey(sender_device) {
                return ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                    owner: sender_owner,
                };
            }
        }
        ProtocolSenderOwnerResolution::Verified {
            owner: sender_owner,
        }
    }

    fn owner_resolution_for_sender_record(
        &self,
        record: ProtocolSenderDeviceRecord,
    ) -> ProtocolSenderOwnerResolution {
        if let Some(claimed_owner) = record
            .claimed_owner_pubkey
            .filter(|claimed_owner| *claimed_owner != record.storage_owner)
        {
            if self.has_verified_device_owner_claim(claimed_owner, record.device_pubkey) {
                return ProtocolSenderOwnerResolution::Verified {
                    owner: claimed_owner,
                };
            }
            return ProtocolSenderOwnerResolution::PendingOwnerClaim {
                storage_owner: record.storage_owner,
                claimed_owner,
                sender_device: record.device_pubkey,
            };
        }
        if record.storage_owner == provisional_owner_from_sender_pubkey(record.device_pubkey) {
            ProtocolSenderOwnerResolution::ProvisionalDeviceOwner {
                owner: record.storage_owner,
            }
        } else {
            ProtocolSenderOwnerResolution::Verified {
                owner: record.storage_owner,
            }
        }
    }

    fn session_record_matching_message_sender(
        &self,
        sender: NdrDevicePubkey,
    ) -> Option<ProtocolSenderDeviceRecord> {
        for user in self.session_manager.snapshot().users {
            for record in user.devices {
                let matches_active = record
                    .active_session
                    .as_ref()
                    .is_some_and(|state| session_state_matches_sender(state, sender));
                let matches_inactive = record
                    .inactive_sessions
                    .iter()
                    .any(|state| session_state_matches_sender(state, sender));
                if matches_active || matches_inactive {
                    return Some(ProtocolSenderDeviceRecord {
                        storage_owner: user.owner_pubkey,
                        device_pubkey: record.device_pubkey,
                        claimed_owner_pubkey: record.claimed_owner_pubkey,
                    });
                }
            }
        }
        None
    }

    fn session_record_for_device_identity(
        &self,
        sender_device: NdrDevicePubkey,
    ) -> Option<ProtocolSenderDeviceRecord> {
        for user in self.session_manager.snapshot().users {
            for record in user.devices {
                if record.device_pubkey == sender_device {
                    return Some(ProtocolSenderDeviceRecord {
                        storage_owner: user.owner_pubkey,
                        device_pubkey: record.device_pubkey,
                        claimed_owner_pubkey: record.claimed_owner_pubkey,
                    });
                }
            }
        }
        None
    }

    fn has_verified_device_owner_claim(
        &self,
        owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
    ) -> bool {
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| roster.get_device(&device).is_some())
    }

    fn wake_pending_protocol_for_owner(&mut self, owner: NdrOwnerPubkey) {
        let owner_hex = owner.to_hex();
        for pending in &mut self.pending_outbound {
            if pending.recipient_owner_hex == owner_hex
                || (pending.probe_local_sibling_roster && owner == self.local_owner)
                || (owner == self.local_owner && pending.local_sibling_payload.is_some())
            {
                pending.next_retry_at_secs = 0;
            }
        }
        for pending in &mut self.pending_group_fanouts {
            if matches!(
                &pending.fanout,
                GroupPendingFanout::Remote {
                    recipient_owner,
                    ..
                } if *recipient_owner == owner
            ) {
                pending.next_retry_at_secs = 0;
            }
        }

        let pending_inbound_ids = self
            .pending_inbound
            .iter()
            .filter_map(|pending| {
                self.pending_inbound_matches_owner(pending, owner)
                    .then(|| pending.event.id)
            })
            .collect::<HashSet<_>>();
        for pending in &mut self.pending_inbound {
            if pending_inbound_ids.contains(&pending.event.id) {
                pending.next_retry_at_secs = 0;
            }
        }

        let pending_pairwise_keys = self
            .pending_group_pairwise_payloads
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| {
                sender_resolution_owner_matches(
                    self.resolve_group_pairwise_sender_owner(
                        pending.sender_owner,
                        pending.sender_device,
                    ),
                    owner,
                )
                .then_some(index)
            })
            .collect::<HashSet<_>>();
        for (index, pending) in self.pending_group_pairwise_payloads.iter_mut().enumerate() {
            if pending_pairwise_keys.contains(&index) {
                pending.next_retry_at_secs = 0;
            }
        }
    }

    fn ensure_local_roster(&mut self, created_at: NdrUnixSeconds) {
        let has_local_roster = self
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .any(|user| user.owner_pubkey == self.local_owner && user.roster.is_some());
        if !has_local_roster {
            self.session_manager.apply_local_roster(DeviceRoster::new(
                created_at,
                vec![AuthorizedDevice::new(self.local_device, created_at)],
            ));
            self.invalidate_known_message_author_cache();
        }
    }

    fn protocol_group_send_from_prepared(
        &mut self,
        prepared: &GroupPreparedSend,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        self.queue_group_pending_fanouts(
            &prepared.group_id,
            &prepared.remote,
            inner_event_id.clone(),
        );
        self.queue_group_pending_fanouts(
            &prepared.group_id,
            &prepared.local_sibling,
            inner_event_id.clone(),
        );
        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        effects.extend(protocol_effects_from_group_prepared_publish(
            &prepared.local_sibling,
            inner_event_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_group_prepared_publish(
            &prepared.remote,
            inner_event_id,
            &mut event_ids,
        )?);
        let mut queued_targets = self.queued_group_targets();
        queued_targets.sort();
        queued_targets.dedup();
        self.append_queued_protocol_backfill(
            &mut effects,
            &queued_targets,
            NdrUnixSeconds(unix_now().get()),
            "group_send",
        );
        Ok(ProtocolGroupSendResult {
            event_ids,
            effects,
            queued_targets,
            ..Default::default()
        })
    }

    fn sync_group_to_local_siblings(
        &mut self,
        group: &GroupSnapshot,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, Vec<String>)> {
        let now = NdrUnixSeconds(unix_now().get());
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.group_manager.sync_group_to_local_siblings(
            &mut self.session_manager,
            &mut ctx,
            &group.group_id,
        )?;
        self.queue_group_pending_fanouts(&group.group_id, &prepared, None);
        let mut event_ids = Vec::new();
        let mut effects = protocol_effects_from_group_prepared_publish(
            &prepared,
            None,
            &mut event_ids,
        )?;
        let mut queued_targets = self.queued_group_targets();
        queued_targets.sort();
        queued_targets.dedup();
        self.append_queued_protocol_backfill(
            &mut effects,
            &queued_targets,
            now,
            "group_local_sibling_sync",
        );
        Ok((effects, queued_targets))
    }

    fn queue_group_pending_fanouts(
        &mut self,
        group_id: &str,
        prepared: &GroupPreparedPublish,
        inner_event_id: Option<String>,
    ) {
        if prepared.pending_fanouts.is_empty() {
            return;
        }
        for fanout in &prepared.pending_fanouts {
            let pending = ProtocolPendingGroupFanout {
                group_id: group_id.to_string(),
                fanout: fanout.clone(),
                inner_event_id: inner_event_id.clone(),
                created_at_secs: unix_now().get(),
                next_retry_at_secs: unix_now().get().saturating_add(PENDING_RETRY_DELAY_SECS),
            };
            if !self.pending_group_fanouts.contains(&pending) {
                self.pending_group_fanouts.push(pending);
            }
        }
    }

    fn queued_group_targets(&self) -> Vec<String> {
        let mut targets = self
            .pending_group_fanouts
            .iter()
            .map(|pending| match &pending.fanout {
                GroupPendingFanout::Remote {
                    recipient_owner, ..
                } => recipient_owner.to_hex(),
                GroupPendingFanout::LocalSiblings { .. } => self.local_owner.to_hex(),
            })
            .collect::<Vec<_>>();
        targets.extend(self.pending_group_pairwise_owner_claim_targets());
        targets.sort();
        targets.dedup();
        targets
    }

    fn pending_inbound_owner_claim_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        for pending in &self.pending_inbound {
            if let Some(sender) = pending_inbound_sender_pubkey(pending) {
                if let ProtocolSenderOwnerResolution::PendingOwnerClaim { claimed_owner, .. } =
                    self.resolve_message_sender_owner_for_sender(sender)
                {
                    targets.push(format!("owner:{}", claimed_owner.to_hex()));
                }
                continue;
            }
            if let Some(claimed_owner_hex) = pending.claimed_owner_pubkey_hex.as_ref() {
                targets.push(format!("owner:{claimed_owner_hex}"));
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn pending_group_pairwise_owner_claim_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        for pending in &self.pending_group_pairwise_payloads {
            if let ProtocolSenderOwnerResolution::PendingOwnerClaim { claimed_owner, .. } = self
                .resolve_group_pairwise_sender_owner(pending.sender_owner, pending.sender_device)
            {
                targets.push(format!("owner:{}", claimed_owner.to_hex()));
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn queue_pending_inbound_direct_event(
        &mut self,
        event: Event,
        now_secs: u64,
        envelope: Option<&MessageEnvelope>,
        resolution: Option<ProtocolSenderOwnerResolution>,
    ) -> anyhow::Result<()> {
        let event_id = event.id.to_string();
        let metadata = self.pending_inbound_metadata_for_event(&event, envelope, resolution);
        if let Some(existing) = self.pending_inbound.iter_mut().find(|pending| {
            let pending_event_id = if pending.event_id.is_empty() {
                pending.event.id.to_string()
            } else {
                pending.event_id.clone()
            };
            pending_event_id == event_id
        }) {
            let changed = apply_pending_inbound_metadata(existing, metadata);
            if changed {
                self.persist()?;
            }
        } else {
            let mut pending = ProtocolPendingInbound {
                event,
                created_at_secs: now_secs,
                next_retry_at_secs: now_secs.saturating_add(PENDING_RETRY_DELAY_SECS),
                event_id: String::new(),
                envelope: None,
                sender_message_pubkey_hex: None,
                resolved_owner_pubkey_hex: None,
                claimed_owner_pubkey_hex: None,
                metadata_verified: false,
            };
            apply_pending_inbound_metadata(&mut pending, metadata);
            if pending.event_id.is_empty() {
                pending.event_id = event_id;
            }
            self.pending_inbound.push(pending);
            self.persist()?;
        }
        Ok(())
    }

    fn pending_inbound_metadata_for_event(
        &self,
        event: &Event,
        envelope: Option<&MessageEnvelope>,
        resolution: Option<ProtocolSenderOwnerResolution>,
    ) -> ProtocolPendingInboundMetadata {
        let parsed = envelope
            .cloned()
            .map(|envelope| (envelope, true))
            .or_else(|| {
                parse_message_event(event)
                    .ok()
                    .map(|envelope| (envelope, true))
            });
        let event_id = event.id.to_string();
        let Some((envelope, metadata_verified)) = parsed else {
            return ProtocolPendingInboundMetadata {
                event_id,
                envelope: None,
                sender_message_pubkey_hex: Some(event.pubkey.to_hex()),
                resolved_owner_pubkey_hex: None,
                claimed_owner_pubkey_hex: None,
                metadata_verified: false,
            };
        };
        let resolution = resolution.unwrap_or_else(|| self.resolve_message_sender_owner(&envelope));
        let (resolved_owner_pubkey_hex, claimed_owner_pubkey_hex) =
            pending_inbound_owner_hexes_from_resolution(resolution);
        ProtocolPendingInboundMetadata {
            event_id,
            sender_message_pubkey_hex: public_device(envelope.sender)
                .ok()
                .map(|pubkey| pubkey.to_hex())
                .or_else(|| Some(event.pubkey.to_hex())),
            envelope: Some(envelope),
            resolved_owner_pubkey_hex,
            claimed_owner_pubkey_hex,
            metadata_verified,
        }
    }

    fn pending_inbound_matches_owner(
        &self,
        pending: &ProtocolPendingInbound,
        owner: NdrOwnerPubkey,
    ) -> bool {
        let owner_hex = owner.to_hex();
        if pending
            .claimed_owner_pubkey_hex
            .as_ref()
            .is_some_and(|claimed_owner| claimed_owner == &owner_hex)
            || pending
                .resolved_owner_pubkey_hex
                .as_ref()
                .is_some_and(|resolved_owner| resolved_owner == &owner_hex)
        {
            return true;
        }
        pending_inbound_sender_pubkey(pending)
            .map(|sender| {
                sender_resolution_owner_matches(
                    self.resolve_message_sender_owner_for_sender(sender),
                    owner,
                )
            })
            .unwrap_or(false)
    }

    fn record_pending_decrypted_delivery(
        &mut self,
        decrypted: ProtocolDecryptedMessage,
        created_at_secs: u64,
    ) {
        let pending = ProtocolPendingDecryptedDelivery {
            sender: decrypted.sender,
            sender_device: decrypted.sender_device,
            conversation_owner: decrypted.conversation_owner,
            content: decrypted.content,
            event_id: decrypted.event_id,
            created_at_secs,
        };
        if !self
            .pending_decrypted_deliveries
            .iter()
            .any(|existing| existing.event_id == pending.event_id)
        {
            self.pending_decrypted_deliveries.push(pending);
        }
    }

    fn queue_pending_group_pairwise_payload(
        &mut self,
        sender_owner: NdrOwnerPubkey,
        sender_device: Option<NdrDevicePubkey>,
        payload: Vec<u8>,
        now_secs: u64,
    ) -> anyhow::Result<()> {
        let pending = ProtocolPendingGroupPairwisePayload {
            sender_owner,
            sender_device,
            payload,
            created_at_secs: now_secs,
            next_retry_at_secs: now_secs.saturating_add(PENDING_RETRY_DELAY_SECS),
        };
        if !self.pending_group_pairwise_payloads.iter().any(|existing| {
            existing.sender_owner == pending.sender_owner
                && existing.sender_device == pending.sender_device
                && existing.payload == pending.payload
        }) {
            self.pending_group_pairwise_payloads.push(pending);
            self.persist()?;
        }
        Ok(())
    }

    fn queue_pending_group_sender_key_message(
        &mut self,
        parsed: nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent,
    ) -> anyhow::Result<()> {
        if !self.pending_group_sender_key_messages.contains(&parsed) {
            self.pending_group_sender_key_messages.push(parsed);
            self.persist()?;
        }
        Ok(())
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
        let message_repair_key_id = message.key_id;
        let message_repair_number = message.message_number;
        let result = match self
            .group_manager
            .handle_sender_key_message(message.clone())
        {
            Ok(result) => result,
            Err(nostr_double_ratchet::Error::Decryption(error))
                if error == "duplicate or missing sender-key message" =>
            {
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
                    return Err(anyhow::anyhow!(
                        "pending sender-key result did not produce a repair request"
                    ));
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
        if self
            .pending_group_sender_key_repairs
            .get(index)
            .is_some_and(|pending| pending.next_retry_at_secs > now.get())
        {
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
                sender_key_repair_default_next_retry_at(now, pending.request_count).get();
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
            .filter(|pending| pending.next_retry_at_secs <= now.get())
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
            effects.extend(self.sender_key_repair_request_effects(request, now)?);
        }
        Ok(effects)
    }

    fn sender_key_repair_response_effects(
        &mut self,
        requester_owner: NdrOwnerPubkey,
        request: &SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<(Vec<ProtocolEffect>, Vec<String>)> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(now, &mut rng);
        let prepared = self.group_manager.respond_to_sender_key_repair_request(
            &mut self.session_manager,
            &mut ctx,
            requester_owner,
            request,
        )?;
        let output = self.protocol_group_send_from_prepared(&prepared, None)?;
        self.invalidate_known_message_author_cache();
        Ok((output.effects, output.queued_targets))
    }

    fn clear_group_sender_key_repairs(
        &mut self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        key_id: u32,
        message_number: u32,
    ) {
        let sender_event_pubkey_hex = sender_event_pubkey.to_hex();
        self.pending_group_sender_key_repairs.retain(|pending| {
            !(pending.group_id == group_id
                && pending.sender_event_pubkey_hex == sender_event_pubkey_hex
                && pending.key_id == key_id
                && pending.message_number == message_number)
        });
    }

}
