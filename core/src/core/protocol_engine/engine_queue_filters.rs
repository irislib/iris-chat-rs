impl ProtocolEngine {
    fn upsert_pending_outbound(&mut self, pending: ProtocolPendingOutbound) {
        if let Some(existing) = self
            .pending_outbound
            .iter_mut()
            .find(|existing| existing.message_id == pending.message_id)
        {
            existing
                .delivered_remote_device_hexes
                .extend(pending.delivered_remote_device_hexes);
            existing.delivered_remote_device_hexes.sort();
            existing.delivered_remote_device_hexes.dedup();
            existing
                .delivered_local_device_hexes
                .extend(pending.delivered_local_device_hexes);
            existing.delivered_local_device_hexes.sort();
            existing.delivered_local_device_hexes.dedup();
            existing.probe_local_sibling_roster |= pending.probe_local_sibling_roster;
            existing.reason = pending.reason;
            existing.next_retry_at_secs = pending.next_retry_at_secs;
        } else {
            self.pending_outbound.push(pending);
        }
    }

    fn remaining_remote_targets(
        &self,
        owner: NdrOwnerPubkey,
        delivered_device_hexes: &[String],
    ) -> Vec<NdrDevicePubkey> {
        let snapshot = self.session_manager.snapshot();
        self.remaining_remote_targets_with_snapshot(&snapshot, owner, delivered_device_hexes)
    }

    fn remaining_remote_targets_with_snapshot(
        &self,
        snapshot: &SessionManagerSnapshot,
        owner: NdrOwnerPubkey,
        delivered_device_hexes: &[String],
    ) -> Vec<NdrDevicePubkey> {
        let delivered = delivered_device_hexes
            .iter()
            .filter_map(|hex| PublicKey::parse(hex).ok())
            .map(ndr_device)
            .collect::<HashSet<_>>();
        snapshot
            .users
            .iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster.as_ref())
            .map(|roster| {
                roster
                    .devices()
                    .iter()
                    .map(|device| device.device_pubkey)
                    .filter(|device| !delivered.contains(device))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn remaining_local_sibling_targets(
        &self,
        delivered_device_hexes: &[String],
    ) -> Vec<NdrDevicePubkey> {
        self.remaining_remote_targets(self.local_owner, delivered_device_hexes)
            .into_iter()
            .filter(|device| *device != self.local_device)
            .collect()
    }

    fn has_roster_for_owner(&self, owner: NdrOwnerPubkey) -> bool {
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| !roster.devices().is_empty())
    }

    fn needs_local_sibling_roster_probe(&self, prepared: &PreparedSend) -> bool {
        prepared.deliveries.is_empty() && prepared.relay_gaps.is_empty()
    }

    fn append_queued_protocol_backfill(
        &self,
        effects: &mut Vec<ProtocolEffect>,
        queued_targets: &[String],
        now: NdrUnixSeconds,
        reason: &'static str,
    ) {
        effects.extend(self.protocol_backfill_effects_for_targets(queued_targets, now, reason));
    }

    fn protocol_backfill_effects_for_targets(
        &self,
        queued_targets: &[String],
        now: NdrUnixSeconds,
        reason: &'static str,
    ) -> Vec<ProtocolEffect> {
        let filters = self.queued_protocol_filters(queued_targets, now);
        if filters.is_empty() {
            Vec::new()
        } else {
            vec![ProtocolEffect::FetchProtocolState { filters, reason }]
        }
    }

    fn protocol_backfill_effects_for_pending_outbound(
        &self,
        pending: &ProtocolPendingOutbound,
        now: NdrUnixSeconds,
        reason: &'static str,
    ) -> Vec<ProtocolEffect> {
        let filters = self.pending_outbound_protocol_filters(pending, now);
        if filters.is_empty() {
            Vec::new()
        } else {
            vec![ProtocolEffect::FetchProtocolState { filters, reason }]
        }
    }

    fn pending_outbound_protocol_filters(
        &self,
        pending: &ProtocolPendingOutbound,
        now: NdrUnixSeconds,
    ) -> Vec<Filter> {
        let mut owner_authors = Vec::new();
        let mut invite_authors = Vec::new();

        if let Ok(owner) = PublicKey::parse(&pending.recipient_owner_hex) {
            let ndr_owner = ndr_owner(owner);
            let remote_targets =
                self.remaining_remote_targets(ndr_owner, &pending.delivered_remote_device_hexes);
            if !remote_targets.is_empty()
                || (matches!(pending.reason, ProtocolPendingReason::MissingRoster)
                    && !self.has_roster_for_owner(ndr_owner))
            {
                owner_authors.push(owner);
            }
            invite_authors.extend(
                remote_targets
                    .into_iter()
                    .filter_map(|target| public_device(target).ok()),
            );
        }

        let local_targets =
            self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes);
        if !local_targets.is_empty() || pending.probe_local_sibling_roster {
            if let Ok(owner) = public_owner(self.local_owner) {
                owner_authors.push(owner);
            }
        }
        invite_authors.extend(
            local_targets
                .into_iter()
                .filter_map(|target| public_device(target).ok()),
        );

        self.protocol_backfill_filters(owner_authors, invite_authors, now)
    }

    fn queued_protocol_filters(
        &self,
        queued_targets: &[String],
        now: NdrUnixSeconds,
    ) -> Vec<Filter> {
        let mut owner_authors = Vec::new();
        let mut invite_authors = Vec::new();
        for target in queued_targets {
            if let Some(owner_hex) = target.strip_prefix("owner:") {
                if let Ok(owner) = PublicKey::parse(owner_hex) {
                    owner_authors.push(owner);
                }
                continue;
            }
            if let Ok(pubkey) = PublicKey::parse(target) {
                owner_authors.push(pubkey);
                invite_authors.push(pubkey);
            }
        }
        self.protocol_backfill_filters(owner_authors, invite_authors, now)
    }

    fn protocol_backfill_filters(
        &self,
        mut owner_authors: Vec<PublicKey>,
        mut invite_authors: Vec<PublicKey>,
        now: NdrUnixSeconds,
    ) -> Vec<Filter> {
        sort_dedup_protocol_pubkeys(&mut owner_authors);
        sort_dedup_protocol_pubkeys(&mut invite_authors);

        let mut filters = Vec::new();
        if !owner_authors.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                    .authors(owner_authors)
                    .identifier(NDR_APP_KEYS_D_TAG)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }
        if !invite_authors.is_empty() {
            filters.push(
                Filter::new()
                    .kind(Kind::from(INVITE_EVENT_KIND as u16))
                    .authors(invite_authors)
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::L), NDR_INVITES_L_TAG)
                    .since(Timestamp::from(
                        now.get()
                            .saturating_sub(DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS),
                    ))
                    .limit(DEVICE_INVITE_DISCOVERY_LIMIT),
            );
        }
        filters
    }

    fn pending_target_hexes(&self, pending: &ProtocolPendingOutbound) -> Vec<String> {
        let mut targets = self.pending_remote_target_hexes(pending);
        for target in self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes) {
            targets.push(target.to_hex());
        }
        if pending.probe_local_sibling_roster
            && self
                .remaining_local_sibling_targets(&pending.delivered_local_device_hexes)
                .is_empty()
        {
            targets.push(format!("owner:{}", self.local_owner.to_hex()));
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn pending_remote_target_hexes(&self, pending: &ProtocolPendingOutbound) -> Vec<String> {
        let mut targets = Vec::new();
        if let Ok(owner) = PublicKey::parse(&pending.recipient_owner_hex) {
            let ndr_owner = ndr_owner(owner);
            let remote_targets =
                self.remaining_remote_targets(ndr_owner, &pending.delivered_remote_device_hexes);
            for target in remote_targets {
                targets.push(target.to_hex());
            }
            if targets.is_empty()
                && matches!(pending.reason, ProtocolPendingReason::MissingRoster)
                && !self.has_roster_for_owner(ndr_owner)
            {
                targets.push(format!("owner:{}", owner.to_hex()));
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn pending_outbound_targets_device_with_snapshot(
        &self,
        pending: &ProtocolPendingOutbound,
        target: NdrDevicePubkey,
        snapshot: &SessionManagerSnapshot,
    ) -> bool {
        let delivered_remote = delivered_device_set(&pending.delivered_remote_device_hexes);
        if !delivered_remote.contains(&target)
            && PublicKey::parse(&pending.recipient_owner_hex).is_ok_and(|owner| {
                self.remaining_remote_targets_with_snapshot(
                    snapshot,
                    ndr_owner(owner),
                    &pending.delivered_remote_device_hexes,
                )
                .contains(&target)
            })
        {
            return true;
        }

        let delivered_local = delivered_device_set(&pending.delivered_local_device_hexes);
        !delivered_local.contains(&target)
            && self
                .remaining_remote_targets_with_snapshot(
                    snapshot,
                    self.local_owner,
                    &pending.delivered_local_device_hexes,
                )
                .into_iter()
                .any(|device| device != self.local_device && device == target)
    }

    fn persist(&self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            latest_app_keys_created_at: self.latest_app_keys_created_at.clone(),
            pending_outbound: self.pending_outbound.clone(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
        };
        self.storage
            .put(PROTOCOL_ENGINE_STATE_KEY, serde_json::to_string(&state)?)?;
        Ok(())
    }
}
