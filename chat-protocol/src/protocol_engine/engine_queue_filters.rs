impl ProtocolEngine {
    fn has_authoritative_local_roster(&self) -> bool {
        if self.local_app_keys_observed {
            return true;
        }
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == self.local_owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| {
                let devices = roster.devices();
                !devices.is_empty()
                    && (devices.len() > 1 || devices[0].device_pubkey != self.local_device)
            })
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

    pub fn protocol_discovery_effects_for_owners(
        &self,
        owners: impl IntoIterator<Item = PublicKey>,
        now: UnixSeconds,
        reason: &'static str,
    ) -> Vec<ProtocolEffect> {
        let owners = owners.into_iter().collect::<Vec<_>>();
        let filters =
            self.protocol_backfill_filters(owners.clone(), owners, NdrUnixSeconds(now.get()));
        if filters.is_empty() {
            Vec::new()
        } else {
            vec![ProtocolEffect::FetchProtocolState { filters, reason }]
        }
    }

    fn protocol_backfill_filters(
        &self,
        mut owner_authors: Vec<PublicKey>,
        mut invite_authors: Vec<PublicKey>,
        _now: NdrUnixSeconds,
    ) -> Vec<Filter> {
        sort_dedup_protocol_pubkeys(&mut owner_authors);
        sort_dedup_protocol_pubkeys(&mut invite_authors);

        build_protocol_discovery_filters(
            owner_authors,
            invite_authors,
            DEVICE_INVITE_DISCOVERY_LIMIT,
        )
    }

    fn persist(&self) -> anyhow::Result<()> {
        if self.batch_depth.get() > 0 {
            self.batch_persist_dirty.set(true);
            return Ok(());
        }
        self.persist_now()
    }

    fn persist_now(&self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            delivered_group_sender_key_acks: self.delivered_group_sender_key_acks.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
        };
        self.batch_persist_dirty.set(false);
        self.storage
            .put(PROTOCOL_ENGINE_STATE_KEY, serde_json::to_string(&state)?)?;
        Ok(())
    }

    pub fn enter_batch(&self) {
        self.batch_depth
            .set(self.batch_depth.get().saturating_add(1));
    }

    pub fn exit_batch(&self) -> anyhow::Result<()> {
        let depth = self.batch_depth.get();
        if depth == 0 {
            return Ok(());
        }
        self.batch_depth.set(depth - 1);
        if self.batch_depth.get() == 0 && self.batch_persist_dirty.get() {
            self.persist_now()?;
        }
        Ok(())
    }
}
