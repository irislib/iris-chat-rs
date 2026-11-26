use super::*;

impl ProtocolEngine {
    /// Resolve an authenticated private carrier without advertising group authors.
    pub fn group_id_for_sender_event_author(&self, author: PublicKey) -> Option<String> {
        self.group_manager
            .group_id_for_sender_event_pubkey(ndr_device(author))
    }

    pub fn has_pending_group_pairwise_payloads(&self) -> bool {
        !self.pending_group_pairwise_payloads.is_empty()
    }

    /// A bounded rotating slice of durable, authenticated key distributions
    /// still waiting for their group metadata, including after a restart.
    pub fn missing_group_metadata_sources(&self) -> Vec<(PublicKey, String, PublicKey)> {
        let pending = &self.pending_group_pairwise_payloads;
        if pending.is_empty() {
            return Vec::new();
        }
        let start = ((unix_now().get() / 60) as usize % pending.len().div_ceil(32)) * 32;
        pending
            .iter()
            .skip(start)
            .take(32)
            .filter_map(|pending| {
                let owner = match self.resolve_group_pairwise_sender_owner(
                    pending.sender_owner,
                    pending.sender_device,
                ) {
                    ProtocolSenderOwnerResolution::Verified { owner } => owner,
                    _ => return None,
                };
                let GroupPairwiseCommand::SenderKeyDistribution { distribution } =
                    JsonGroupPayloadCodecV1
                        .decode_pairwise_command(&pending.payload)
                        .ok()??
                else {
                    return None;
                };
                if self.group_manager.group(&distribution.group_id).is_some() {
                    return None;
                }
                Some((
                    public_owner(owner).ok()?,
                    distribution.group_id,
                    public_device(distribution.sender_event_pubkey).ok()?,
                ))
            })
            .collect()
    }

    /// Resend only to an existing member. Ordinary members' metadata copies still
    /// pass the normal admin admission check; they cannot establish authority.
    pub fn resend_private_group_context(
        &mut self,
        group_id: &str,
        requester: PublicKey,
        author: Option<PublicKey>,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        let Some(group) = self.group_manager.group(group_id) else {
            return Ok(Vec::new());
        };
        if !group.members.contains(&self.local_owner)
            || !group.members.contains(&ndr_owner(requester))
        {
            return Ok(Vec::new());
        }
        self.with_state_checkpoint(|engine| {
            let effects = engine.sender_key_repair_response_effects(
                ndr_owner(requester),
                &SenderKeyRepairRequest {
                    group_id: group.group_id,
                    sender_event_pubkey: author.map(ndr_device).unwrap_or(engine.local_device),
                    key_id: None,
                    message_number: None,
                    required_revision: Some(group.revision),
                    created_at: NdrUnixSeconds(unix_now().get()),
                },
                NdrUnixSeconds(unix_now().get()),
            )?;
            engine.retain_group_publications(&effects, Some(ndr_owner(requester)))?;
            engine.persist()?;
            Ok(effects)
        })
    }
    pub(super) fn queue_pending_group_sender_key_message(
        &mut self,
        parsed: nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
    ) -> anyhow::Result<()> {
        self.queue_group_sender_key_candidate(parsed, true)
    }

    pub(super) fn queue_group_sender_key_candidate(
        &mut self,
        parsed: nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
        fresh: bool,
    ) -> anyhow::Result<()> {
        if self.unmapped_group_sender_key_candidate_is_known_message_author(&parsed) {
            return Ok(());
        }

        if self
            .group_sender_key_message_from_parsed(&parsed)
            .is_some_and(|message| {
                self.processed_group_sender_key_messages
                    .contains(&group_sender_key_fingerprint(&message))
            })
        {
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
            if fresh {
                self.admit_fresh_group_sender_key_candidate(
                    &parsed,
                    self.pending_group_sender_key_messages.len(),
                );
            }
            // Authenticated group ingress can add a candidate after this
            // stream's old backlog was already attempted. Admit that candidate
            // once even if its key/revision inputs have not changed.
            if let Some(message) = self.group_sender_key_message_from_parsed(&parsed) {
                let fingerprint = group_sender_key_fingerprint(&message);
                let mut retry = self.group_sender_key_retry.borrow_mut();
                if let Some(fresh) = retry
                    .fresh
                    .iter_mut()
                    .find(|fresh| fresh.fingerprint == fingerprint)
                {
                    fresh.pending_index = self.pending_group_sender_key_messages.len();
                }
                if retry.queued.insert(fingerprint.clone()) {
                    retry
                        .ready
                        .push_back((parsed.sender_event_pubkey, fingerprint));
                }
            }
            self.pending_group_sender_key_messages.push(parsed);
            self.persist()?;
        }
        Ok(())
    }

    pub(super) fn clear_pending_group_sender_key_candidate_for_direct_event(
        &mut self,
        event: &Event,
    ) {
        if !protocol_event_has_tag(event, "header") {
            return;
        }
        let Ok(parsed) = parse_group_sender_key_message_event_unchecked(event) else {
            return;
        };
        self.clear_pending_group_sender_key_candidate(&parsed);
    }

    pub(super) fn clear_pending_group_sender_key_candidate(
        &mut self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
    ) -> bool {
        {
            let mut retry = self.group_sender_key_retry.borrow_mut();
            retry.fresh.retain(|fresh| fresh.parsed.as_ref() != parsed);
            if retry
                .active
                .as_ref()
                .is_some_and(|active| active.matches(parsed))
            {
                retry.active = None;
            }
        }
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

    fn has_other_pending_group_sender_key_candidate(
        &self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        exclude_fingerprint: &str,
    ) -> bool {
        self.pending_group_sender_key_messages
            .iter()
            .any(|pending| {
                pending.sender_event_pubkey == sender_event_pubkey
                    && self
                        .group_sender_key_message_from_parsed(pending)
                        .is_some_and(|message| {
                            message.group_id == group_id
                                && group_sender_key_fingerprint(&message) != exclude_fingerprint
                        })
            })
    }

    fn clear_group_sender_key_repairs_for_resolved_message(
        &mut self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        key_id: Option<u32>,
        message_number: Option<u32>,
        fingerprint: &str,
    ) {
        // Camouflaged messages carry no plaintext key/position selector. Only
        // retire the stream's blind repair once no other pending ciphertexts
        // remain for that stream, so decrypting a newer live message never
        // resets an existing repair's retry backoff or answered-round counter.
        if key_id.is_none()
            && self.has_other_pending_group_sender_key_candidate(
                group_id,
                sender_event_pubkey,
                fingerprint,
            )
        {
            return;
        }
        self.clear_group_sender_key_repairs(group_id, sender_event_pubkey, key_id, message_number);
    }

    pub(super) fn clear_satisfied_revision_repairs(
        &mut self,
        group_id: &str,
        current_revision: u64,
    ) -> bool {
        let original_len = self.pending_group_sender_key_repairs.len();
        self.pending_group_sender_key_repairs.retain(|pending| {
            !(pending.group_id == group_id
                && pending
                    .required_revision
                    .is_some_and(|required| current_revision >= required))
        });
        self.pending_group_sender_key_repairs.len() != original_len
    }

    pub(super) fn record_sender_key_distribution_repair_feedback(
        &mut self,
        group_id: &str,
        sender_event_pubkey: NdrDevicePubkey,
        group_changed: bool,
        from_local_sibling_wrapper: bool,
    ) -> bool {
        let sender_hex = sender_event_pubkey.to_hex();

        if group_changed {
            for pending in &mut self.pending_group_sender_key_repairs {
                if pending.group_id == group_id
                    && pending.sender_event_pubkey_hex == sender_hex
                    && (pending.unchanged_response_rounds > 0
                        || pending.last_unchanged_response_request_count > 0)
                {
                    pending.unchanged_response_rounds = 0;
                    pending.last_unchanged_response_request_count = 0;
                }
            }
            return false;
        }

        if from_local_sibling_wrapper {
            return false;
        }

        // If the stream's own writer keeps answering repair rounds with keys
        // this device already holds, back off the repair schedule steeply
        // (5m -> 15m -> 1h) without deleting ciphertexts: genuinely pre-join
        // or during-removal traffic goes quiet, while harsh-network packet
        // loss or a newer key rotation can still recover.
        for pending in &mut self.pending_group_sender_key_repairs {
            if pending.group_id != group_id
                || pending.sender_event_pubkey_hex != sender_hex
                || pending.required_revision.is_some()
                || pending.request_count == 0
            {
                continue;
            }
            if pending.last_unchanged_response_request_count < pending.request_count {
                pending.last_unchanged_response_request_count = pending.request_count;
                pending.unchanged_response_rounds =
                    pending.unchanged_response_rounds.saturating_add(1);
                if pending.unchanged_response_rounds >= 3 && pending.last_requested_at_secs > 0 {
                    pending.next_retry_at_secs = protocol_sender_key_repair_next_retry_at_for(
                        NdrUnixSeconds(pending.last_requested_at_secs),
                        pending.request_count,
                        pending.unchanged_response_rounds,
                    )
                    .get();
                }
            }
        }
        false
    }

    pub(super) fn unmapped_group_sender_key_candidate_is_known_message_author(
        &self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
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

    fn inactive_local_group_id_for_sender_key_candidate(
        &self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
    ) -> Option<String> {
        let group_id = self
            .group_manager
            .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)?;
        self.local_owner_is_inactive_for_group(&group_id)
            .then_some(group_id)
    }

    pub(super) fn local_owner_is_inactive_for_group(&self, group_id: &str) -> bool {
        self.group_manager
            .group_has_member(group_id, self.local_owner)
            == Some(false)
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

    pub(super) fn prune_pending_group_sender_key_work_for_inactive_local_groups(&mut self) -> bool {
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
        self.pending_group_sender_key_messages.retain(|pending| {
            !inactive_sender_event_pubkeys.contains(&pending.sender_event_pubkey)
        });

        let original_repair_len = self.pending_group_sender_key_repairs.len();
        self.pending_group_sender_key_repairs
            .retain(|pending| !inactive_group_ids.contains(&pending.group_id));

        self.pending_group_sender_key_messages.len() != original_message_len
            || self.pending_group_sender_key_repairs.len() != original_repair_len
    }

    pub(super) fn group_sender_key_message_from_parsed(
        &self,
        parsed: &nostr_double_ratchet::wire::ParsedGroupSenderKeyMessageEvent,
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

    pub(super) fn group_sender_key_message_has_pending_repair(
        &self,
        message: &GroupSenderKeyMessage,
    ) -> bool {
        let sender = message.sender_event_pubkey.to_hex();
        let key_id = message.encrypted_header.is_none().then_some(message.key_id);
        let number = message
            .encrypted_header
            .is_none()
            .then_some(message.message_number);
        self.pending_group_sender_key_repairs.iter().any(|pending| {
            pending.group_id == message.group_id
                && pending.sender_event_pubkey_hex == sender
                && (message.encrypted_header.is_some()
                    || (pending.key_id == key_id && pending.message_number == number))
        })
    }

    pub(super) fn handle_group_sender_key_message(
        &mut self,
        message: GroupSenderKeyMessage,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let fingerprint = group_sender_key_fingerprint(&message);
        if self.local_owner_is_inactive_for_group(&message.group_id) {
            let mut retry = self.group_sender_key_retry.borrow_mut();
            if retry
                .active
                .as_ref()
                .is_some_and(|active| active.fingerprint == fingerprint)
            {
                retry.active = None;
            }
            retry.prepared = None;
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        if self
            .processed_group_sender_key_messages
            .contains(&fingerprint)
        {
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        // Our publishing chain has already advanced, so decrypting its relay echo
        // searches every skipped key and starts a pointless repair. This durable
        // secret identifies this device's own stream, including after restart.
        if self.group_manager.is_local_sender_key_stream(
            &message.group_id,
            message.sender_event_pubkey,
            self.local_device,
        ) {
            if self.clear_group_sender_key_repairs(
                &message.group_id,
                message.sender_event_pubkey,
                None,
                None,
            ) {
                self.persist()?;
            }
            return Ok(ProtocolGroupIncomingResult {
                consumed: true,
                ..Default::default()
            });
        }
        // Planning and duplicate/self-echo checks do not mutate durable state.
        // Only a ready receive (or duplicate marker) needs a rollback checkpoint.
        let plan = match self.prepare_group_sender_key_message(&message, &fingerprint) {
            Ok(Some(plan)) => Ok(plan),
            Ok(None) => {
                return Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    pending: true,
                    ..Default::default()
                })
            }
            Err(error @ nostr_double_ratchet::Error::Decryption(_))
                if matches!(&error, nostr_double_ratchet::Error::Decryption(reason)
                    if reason == "duplicate or missing sender-key message") =>
            {
                Err(error)
            }
            Err(error) => return Err(error.into()),
        };
        if self.batch_depth.get() > 0 {
            return self.apply_group_sender_key_message(message, fingerprint, plan);
        }
        // The sender ratchet, replay marker and app delivery must advance together.
        // The ciphertext queue is unchanged here; avoid cloning its backlog.
        #[cfg(test)]
        {
            self.group_sender_key_retry.borrow_mut().receive_checkpoints += 1;
        }
        let checkpoint = (
            self.group_manager.clone(),
            self.session_manager.clone(),
            self.pending_group_sender_key_repairs.clone(),
            self.pending_group_fanouts.clone(),
            self.processed_group_sender_key_messages.clone(),
            self.pending_decrypted_deliveries.clone(),
            self.batch_persist_dirty.get(),
        );
        let result = self.apply_group_sender_key_message(message, fingerprint, plan);
        if result.is_err() {
            self.group_manager = checkpoint.0;
            self.session_manager = checkpoint.1;
            self.pending_group_sender_key_repairs = checkpoint.2;
            self.invalidate_pending_group_fanout_retry_deadline();
            self.pending_group_fanouts = checkpoint.3;
            self.processed_group_sender_key_messages = checkpoint.4;
            self.pending_decrypted_deliveries = checkpoint.5;
            self.batch_persist_dirty.set(checkpoint.6);
            self.invalidate_known_message_author_cache();
        }
        result
    }

    fn apply_group_sender_key_message(
        &mut self,
        message: GroupSenderKeyMessage,
        fingerprint: String,
        plan: nostr_double_ratchet::Result<nostr_double_ratchet::GroupSenderKeyReceivePlan>,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let message_repair_group_id = message.group_id.clone();
        let message_repair_sender = message.sender_event_pubkey;
        let message_repair_key_id = message.encrypted_header.is_none().then_some(message.key_id);
        let message_repair_number = message
            .encrypted_header
            .is_none()
            .then_some(message.message_number);
        let result =
            match plan.and_then(|plan| self.group_manager.apply_sender_key_receive_plan(plan)) {
                Ok(result) => result,
                Err(nostr_double_ratchet::Error::Decryption(error))
                    if error == "duplicate or missing sender-key message" =>
                {
                    self.group_sender_key_retry
                        .borrow_mut()
                        .pending_revision_fingerprints
                        .remove(&fingerprint);
                    self.clear_group_sender_key_repairs_for_resolved_message(
                        &message_repair_group_id,
                        message_repair_sender,
                        message_repair_key_id,
                        message_repair_number,
                        &fingerprint,
                    );
                    self.processed_group_sender_key_messages.insert(fingerprint);
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
                self.group_sender_key_retry
                    .borrow_mut()
                    .pending_revision_fingerprints
                    .remove(&fingerprint);
                let discard = matches!(&event, GroupIncomingEvent::Message(received)
                    if received.sender_owner == self.local_owner
                        && is_message_mutation_payload(&received.body));
                if let GroupIncomingEvent::Message(received) = &event {
                    if !discard {
                        // Replay through the authenticated group envelope, retaining its
                        // scope even if the inner rumor contains a conflicting group tag.
                        let payload = JsonGroupPayloadCodecV1.encode_pairwise_command(
                            nostr_double_ratchet::GroupPayloadEncodeContext {
                                local_device_pubkey: received
                                    .sender_device
                                    .unwrap_or(self.local_device),
                                created_at: message.created_at,
                            },
                            &GroupPairwiseCommand::GroupMessage {
                                group_id: received.group_id.clone(),
                                revision: received.revision,
                                body: received.body.clone(),
                            },
                        )?;
                        self.record_pending_decrypted_delivery(
                            ProtocolDecryptedMessage {
                                sender: public_owner(received.sender_owner)?,
                                sender_device: received
                                    .sender_device
                                    .map(public_device)
                                    .transpose()?,
                                conversation_owner: None,
                                content: String::from_utf8(payload)?,
                                event_id: Some(format!("group-delivery:{fingerprint}")),
                                created_at_secs: message.created_at.get(),
                            },
                            message.created_at.get(),
                        );
                    }
                }
                self.clear_group_sender_key_repairs_for_resolved_message(
                    &message_repair_group_id,
                    message_repair_sender,
                    message_repair_key_id,
                    message_repair_number,
                    &fingerprint,
                );
                self.processed_group_sender_key_messages.insert(fingerprint);
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events: if discard { Vec::new() } else { vec![event] },
                    consumed: true,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::PendingDistribution { .. } => {
                self.group_sender_key_retry
                    .borrow_mut()
                    .pending_revision_fingerprints
                    .remove(&fingerprint);
                let Some(request) = repair_request else {
                    anyhow::bail!("pending sender-key result did not produce a repair request");
                };
                let effects = self.sender_key_repair_request_effects_for_message(
                    request,
                    Some(message.created_at),
                    now,
                )?;
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    pending: true,
                    effects,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::PendingRevision { .. } => {
                self.group_sender_key_retry
                    .borrow_mut()
                    .pending_revision_fingerprints
                    .insert(fingerprint);
                let Some(request) = repair_request else {
                    anyhow::bail!("pending sender-key result did not produce a repair request");
                };
                let effects = self.sender_key_repair_request_effects_for_message(
                    request,
                    Some(message.created_at),
                    now,
                )?;
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    pending: true,
                    effects,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::Ignored => {
                self.group_sender_key_retry
                    .borrow_mut()
                    .pending_revision_fingerprints
                    .remove(&fingerprint);
                self.clear_group_sender_key_repairs_for_resolved_message(
                    &message_repair_group_id,
                    message_repair_sender,
                    message_repair_key_id,
                    message_repair_number,
                    &fingerprint,
                );
                self.processed_group_sender_key_messages.insert(fingerprint);
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    ..Default::default()
                })
            }
        }
    }

    pub(super) fn sender_key_repair_request_effects(
        &mut self,
        request: SenderKeyRepairRequest,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        self.sender_key_repair_request_effects_for_message(request, None, now)
    }

    fn sender_key_repair_request_effects_for_message(
        &mut self,
        request: SenderKeyRepairRequest,
        message_created_at: Option<NdrUnixSeconds>,
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
        // Coalesce pending revision repairs per (group_id, required_revision)
        // since the message already decrypted its sender key and only needs
        // metadata at that revision. Coalesce explicit key-id repairs per
        // (group_id, sender, key_id) keeping the lowest message_number.
        let position = if let Some(required_revision) = request.required_revision {
            self.pending_group_sender_key_repairs
                .iter()
                .position(|pending| {
                    pending.group_id == request.group_id
                        && pending.required_revision == Some(required_revision)
                })
        } else if let Some(key_id) = request.key_id {
            self.pending_group_sender_key_repairs
                .iter()
                .position(|pending| {
                    pending.group_id == request.group_id
                        && pending.sender_event_pubkey_hex == sender_event_pubkey_hex
                        && pending.key_id == Some(key_id)
                        && pending.required_revision.is_none()
                })
        } else {
            self.pending_group_sender_key_repairs
                .iter()
                .position(|pending| {
                    pending.group_id == request.group_id
                        && pending.sender_event_pubkey_hex == sender_event_pubkey_hex
                        && pending.key_id == request.key_id
                        && pending.message_number == request.message_number
                        && pending.required_revision == request.required_revision
                })
        };
        let index = if let Some(index) = position {
            if request.required_revision.is_none() {
                if let Some(pending) = self.pending_group_sender_key_repairs.get_mut(index) {
                    if let Some(incoming_number) = request.message_number {
                        if pending
                            .message_number
                            .is_none_or(|existing| incoming_number < existing)
                        {
                            pending.message_number = Some(incoming_number);
                        }
                    }
                    // A genuinely newer un-decryptable ciphertext must wake a
                    // repair that backed off after unchanged replies for older
                    // history (e.g. a later key rotation missed after rejoin).
                    if pending.unchanged_response_rounds >= 3
                        && message_created_at
                            .is_some_and(|created| created.get() > pending.created_at_secs)
                    {
                        pending.created_at_secs = now.get();
                        pending.request_count = 0;
                        pending.last_requested_at_secs = 0;
                        pending.next_retry_at_secs = 0;
                        pending.unchanged_response_rounds = 0;
                        pending.last_unchanged_response_request_count = 0;
                    }
                }
            }
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
                    unchanged_response_rounds: 0,
                    last_unchanged_response_request_count: 0,
                });
            self.pending_group_sender_key_repairs.len() - 1
        };
        let Some(pending_repair) = self.pending_group_sender_key_repairs.get(index) else {
            anyhow::bail!("pending sender-key repair index disappeared");
        };
        if Self::pending_group_sender_key_repair_due_at_secs(pending_repair) > now.get() {
            return Ok(Vec::new());
        }
        let prior_request_count = pending_repair.request_count;
        let effective_request = SenderKeyRepairRequest {
            message_number: pending_repair.message_number.or(request.message_number),
            ..request
        };

        let prepared =
            self.prepare_group_repair_request_send(&effective_request, prior_request_count, now)?;
        let session_progress =
            [&prepared.remote, &prepared.local_sibling]
                .into_iter()
                .any(|part| {
                    !part.deliveries.is_empty()
                        || !part.invite_responses.is_empty()
                        || !part.sender_key_messages.is_empty()
                });
        // An attempted sibling produces a delivery or a gap. Keep gap-only
        // sibling work durable too: it may reorganize retained inactive sessions.
        let local_sibling_state_may_change = !prepared.local_sibling.relay_gaps.is_empty()
            || !prepared.local_sibling.pending_fanouts.is_empty();
        let (output, fanouts_changed) =
            self.protocol_group_send_from_prepared_with_progress(&prepared, None)?;
        if let Some(pending) = self.pending_group_sender_key_repairs.get_mut(index) {
            pending.last_requested_at_secs = now.get();
            pending.request_count = pending.request_count.saturating_add(1);
            pending.next_retry_at_secs = protocol_sender_key_repair_next_retry_at_for(
                now,
                pending.request_count,
                pending.unchanged_response_rounds,
            )
            .get();
        }
        if session_progress || fanouts_changed || local_sibling_state_may_change {
            // Persist after updating the clocks so a delivered request remains
            // throttled across restart, together with its ratchet and targets.
            self.persist()?;
        }
        self.invalidate_known_message_author_cache();
        Ok(output.effects)
    }

    pub(super) fn retry_pending_group_sender_key_repairs(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolEffect>> {
        let before_prune = self.pending_group_sender_key_repairs.len();
        self.pending_group_sender_key_repairs.retain(|pending| {
            !pending.required_revision.is_some_and(|required| {
                self.group_manager
                    .group(&pending.group_id)
                    .is_some_and(|group| group.revision >= required)
            })
        });
        if self.pending_group_sender_key_repairs.len() != before_prune {
            self.persist()?;
        }
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

    pub(super) fn pending_group_sender_key_repair_due_at_secs(
        pending: &ProtocolPendingGroupSenderKeyRepair,
    ) -> u64 {
        if pending.request_count == 0 || pending.last_requested_at_secs == 0 {
            return pending.next_retry_at_secs;
        }
        let capped_due_at = pending.last_requested_at_secs.saturating_add(
            protocol_sender_key_repair_retry_delay_secs_for(
                pending.request_count,
                pending.unchanged_response_rounds,
            ),
        );
        if pending.next_retry_at_secs == 0 {
            capped_due_at
        } else {
            pending.next_retry_at_secs.min(capped_due_at)
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
