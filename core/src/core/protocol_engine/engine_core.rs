impl ProtocolEngine {
    pub(super) fn load_or_seed(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        device_keys: &Keys,
        local_invite: Invite,
        seed_session_manager: SessionManagerSnapshot,
        seed_group_manager: GroupManagerSnapshot,
    ) -> anyhow::Result<Self> {
        let device_secret = device_keys.secret_key().to_secret_bytes();
        let local_owner = ndr_owner(owner_pubkey);
        let local_device = ndr_device(device_keys.public_key());

        let mut engine = match storage.get(PROTOCOL_ENGINE_STATE_KEY)? {
            Some(raw) => match serde_json::from_str::<ProtocolEnginePersistedState>(&raw) {
                Ok(state) if state.version == PROTOCOL_ENGINE_STATE_VERSION => {
                    let session_manager =
                        SessionManager::from_snapshot(state.session_manager, device_secret)?;
                    let group_manager = NostrGroupManager::from_snapshot(state.group_manager)?;
                    Self {
                        owner_pubkey,
                        local_owner,
                        local_device,
                        storage,
                        session_manager,
                        group_manager,
                        latest_app_keys_created_at: state.latest_app_keys_created_at,
                        pending_outbound: state.pending_outbound,
                        pending_inbound: state.pending_inbound,
                        pending_group_fanouts: state.pending_group_fanouts,
                        pending_group_pairwise_payloads: state.pending_group_pairwise_payloads,
                        pending_group_sender_key_messages: state.pending_group_sender_key_messages,
                        pending_group_sender_key_repairs: state.pending_group_sender_key_repairs,
                        pending_decrypted_deliveries: state.pending_decrypted_deliveries,
                        known_message_author_cache: std::cell::RefCell::new(None),
                        #[cfg(test)]
                        known_message_author_cache_build_count: std::cell::Cell::new(0),
                        subscription_generation: state.subscription_generation,
                        last_backfill_attempt_secs: state.last_backfill_attempt_secs,
                    }
                }
                _ => Self::from_seed(
                    storage,
                    owner_pubkey,
                    local_owner,
                    local_device,
                    device_secret,
                    seed_session_manager,
                    seed_group_manager,
                )?,
            },
            None => Self::from_seed(
                storage,
                owner_pubkey,
                local_owner,
                local_device,
                device_secret,
                seed_session_manager,
                seed_group_manager,
            )?,
        };

        if engine.session_manager.snapshot().local_invite.is_none() {
            engine
                .session_manager
                .replace_local_invite(local_invite.clone());
        }
        engine.ensure_local_roster(local_invite.created_at);
        engine.hydrate_pending_inbound_metadata();
        engine.prune_untracked_pending_inbound();
        engine.persist()?;
        Ok(engine)
    }

    fn from_seed(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        local_owner: NdrOwnerPubkey,
        local_device: NdrDevicePubkey,
        device_secret: [u8; 32],
        seed_session_manager: SessionManagerSnapshot,
        seed_group_manager: GroupManagerSnapshot,
    ) -> anyhow::Result<Self> {
        let session_manager = SessionManager::from_snapshot(seed_session_manager, device_secret)
            .unwrap_or_else(|_| SessionManager::new(local_owner, device_secret));
        let group_manager = NostrGroupManager::from_snapshot(seed_group_manager)
            .unwrap_or_else(|_| NostrGroupManager::new(local_owner));
        Ok(Self {
            owner_pubkey,
            local_owner,
            local_device,
            storage,
            session_manager,
            group_manager,
            latest_app_keys_created_at: BTreeMap::new(),
            pending_outbound: Vec::new(),
            pending_inbound: Vec::new(),
            pending_group_fanouts: Vec::new(),
            pending_group_pairwise_payloads: Vec::new(),
            pending_group_sender_key_messages: Vec::new(),
            pending_group_sender_key_repairs: Vec::new(),
            pending_decrypted_deliveries: Vec::new(),
            known_message_author_cache: std::cell::RefCell::new(None),
            #[cfg(test)]
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            subscription_generation: 0,
            last_backfill_attempt_secs: 0,
        })
    }

    fn hydrate_pending_inbound_metadata(&mut self) {
        let metadata = self
            .pending_inbound
            .iter()
            .map(|pending| {
                self.pending_inbound_metadata_for_event(
                    &pending.event,
                    pending.envelope.as_ref(),
                    None,
                )
            })
            .collect::<Vec<_>>();
        for (pending, metadata) in self.pending_inbound.iter_mut().zip(metadata) {
            apply_pending_inbound_metadata(pending, metadata);
        }
    }

    fn prune_untracked_pending_inbound(&mut self) {
        if self.pending_inbound.is_empty() {
            return;
        }
        let known_authors = self.known_message_author_hexes();
        self.pending_inbound.retain(|pending| {
            pending_inbound_sender_pubkey_hex(pending)
                .is_some_and(|sender| known_authors.contains(&sender))
        });
    }

    pub(super) fn debug_snapshot(&self) -> ProtocolEngineDebugSnapshot {
        ProtocolEngineDebugSnapshot {
            known_message_author_count: self.known_message_author_pubkeys().len(),
            pending_outbound_count: self.pending_outbound.len(),
            pending_inbound_count: self.pending_inbound.len(),
            pending_group_fanout_count: self.pending_group_fanouts.len(),
            pending_group_pairwise_payload_count: self.pending_group_pairwise_payloads.len(),
            pending_group_sender_key_message_count: self.pending_group_sender_key_messages.len(),
            pending_group_sender_key_repair_count: self.pending_group_sender_key_repairs.len(),
            pending_group_sender_key_repair_last_requested_at_secs: self
                .pending_group_sender_key_repairs
                .iter()
                .map(|repair| repair.last_requested_at_secs)
                .max()
                .unwrap_or_default(),
            pending_outbound_targets: self.queued_message_diagnostics(None),
            pending_outbound_details: self.pending_outbound_debug_details(),
            pending_group_fanout_targets: self.queued_group_targets(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
            latest_app_keys_owner_count: self.latest_app_keys_created_at.len(),
        }
    }

    #[cfg(test)]
    pub(super) fn session_manager_snapshot_for_test(&self) -> SessionManagerSnapshot {
        self.session_manager.snapshot()
    }

    #[cfg(test)]
    pub(super) fn group_manager_snapshot_for_test(&self) -> GroupManagerSnapshot {
        self.group_manager.snapshot()
    }

    pub(super) fn is_known_local_owner_device(&self, device_pubkey: PublicKey) -> bool {
        let device_pubkey = ndr_device(device_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == self.local_owner)
            .is_some_and(|user| {
                user.devices
                    .iter()
                    .any(|device| device.device_pubkey == device_pubkey)
            })
    }

    pub(super) fn owner_hint_for_device(
        &self,
        device_pubkey: PublicKey,
    ) -> Option<ProtocolDeviceOwnerHint> {
        let device = ndr_device(device_pubkey);
        let provisional_owner = ndr_owner(device_pubkey);
        let mut claimed_owner = None;
        for user in self.session_manager.snapshot().users {
            for record in user.devices {
                if record.device_pubkey != device {
                    continue;
                }
                if user.owner_pubkey != provisional_owner {
                    return public_owner(user.owner_pubkey).ok().map(|owner| {
                        ProtocolDeviceOwnerHint {
                            owner,
                            verified: true,
                        }
                    });
                }
                if claimed_owner.is_none() {
                    claimed_owner = record.claimed_owner_pubkey;
                }
            }
        }
        claimed_owner
            .and_then(|owner| public_owner(owner).ok())
            .map(|owner| ProtocolDeviceOwnerHint {
                owner,
                verified: false,
            })
    }

    pub(super) fn has_pending_inbound_direct_events(&self) -> bool {
        !self.pending_inbound.is_empty()
    }

    pub(super) fn has_pending_inbound_direct_event_id(&self, event_id: &str) -> bool {
        self.pending_inbound.iter().any(|pending| {
            let pending_event_id = if pending.event_id.is_empty() {
                pending.event.id.to_string()
            } else {
                pending.event_id.clone()
            };
            pending_event_id == event_id
        })
    }

    pub(super) fn queued_owner_claim_targets(&self) -> Vec<String> {
        let mut targets = self.pending_inbound_owner_claim_targets();
        targets.extend(self.pending_group_pairwise_owner_claim_targets());
        targets.sort();
        targets.dedup();
        targets
    }

    pub(super) fn queued_protocol_backfill_effects(
        &self,
        now: NdrUnixSeconds,
        reason: &'static str,
    ) -> (Vec<String>, Vec<ProtocolEffect>) {
        let mut targets = self.queued_message_diagnostics(None);
        let mut generic_targets = self.queued_owner_claim_targets();
        generic_targets.extend(self.queued_group_targets());
        targets.extend(generic_targets.clone());
        targets.sort();
        targets.dedup();
        let mut effects = self
            .pending_outbound
            .iter()
            .flat_map(|pending| {
                self.protocol_backfill_effects_for_pending_outbound(pending, now, reason)
            })
            .collect::<Vec<_>>();
        effects.extend(self.protocol_backfill_effects_for_targets(&generic_targets, now, reason));
        (targets, effects)
    }

    pub(super) fn queued_group_target_hexes(&self) -> Vec<String> {
        self.queued_group_targets()
    }

    pub(super) fn has_queued_invite_author(&self, author: PublicKey) -> bool {
        let target = ndr_device(author);
        let snapshot = self.session_manager.snapshot();
        self.pending_outbound.iter().any(|pending| {
            self.pending_outbound_targets_device_with_snapshot(pending, target, &snapshot)
        })
    }

    #[cfg(test)]
    pub(super) fn local_invite_for_test(&self) -> Option<Invite> {
        self.session_manager.snapshot().local_invite
    }

    #[cfg(test)]
    pub(super) fn pending_inbound_for_test(&self) -> Vec<ProtocolPendingInboundTestDebug> {
        self.pending_inbound
            .iter()
            .map(|pending| ProtocolPendingInboundTestDebug {
                event_id: if pending.event_id.is_empty() {
                    pending.event.id.to_string()
                } else {
                    pending.event_id.clone()
                },
                sender_message_pubkey_hex: pending.sender_message_pubkey_hex.clone(),
                claimed_owner_pubkey_hex: pending.claimed_owner_pubkey_hex.clone(),
                has_envelope: pending.envelope.is_some(),
                metadata_verified: pending.metadata_verified,
            })
            .collect()
    }

    pub(super) fn known_message_author_pubkeys(&self) -> Vec<PublicKey> {
        self.with_known_message_author_cache(|cache| cache.pubkeys.clone())
    }

    /// Walks every session and returns its expected event-author
    /// pubkeys, but only for sessions whose peer owner passes the
    /// `accept_owner` predicate. The owner-aware variant lets the
    /// caller drop blocked / non-accepted peers from the subscription
    /// filter without losing the device-ephemeral keys that nostr
    /// actually filters on.
    pub(super) fn message_author_pubkeys_filtered<F>(&self, accept_owner: F) -> Vec<PublicKey>
    where
        F: Fn(PublicKey) -> bool,
    {
        let mut authors = HashSet::new();
        for user in self.session_manager.snapshot().users {
            let Ok(owner) = PublicKey::parse(&user.owner_pubkey.to_string()) else {
                continue;
            };
            if !accept_owner(owner) {
                continue;
            }
            for device in user.devices {
                if let Some(session) = device.active_session.as_ref() {
                    collect_expected_sender_pubkeys(session, &mut authors);
                }
                for session in &device.inactive_sessions {
                    collect_expected_sender_pubkeys(session, &mut authors);
                }
            }
        }
        let mut authors = authors.into_iter().collect::<Vec<_>>();
        authors.sort_by_key(|pubkey| pubkey.to_hex());
        authors
    }

    pub(super) fn is_known_message_author(&self, author: PublicKey) -> bool {
        self.with_known_message_author_cache(|cache| cache.pubkey_set.contains(&author))
    }

    fn known_message_author_hexes(&self) -> HashSet<String> {
        self.with_known_message_author_cache(|cache| cache.hexes.clone())
    }

    fn with_known_message_author_cache<T>(
        &self,
        read: impl FnOnce(&KnownMessageAuthorCache) -> T,
    ) -> T {
        let mut cached = self.known_message_author_cache.borrow_mut();
        let cache = cached.get_or_insert_with(|| self.build_known_message_author_cache());
        read(cache)
    }

    fn build_known_message_author_cache(&self) -> KnownMessageAuthorCache {
        #[cfg(test)]
        self.known_message_author_cache_build_count
            .set(self.known_message_author_cache_build_count.get() + 1);

        let mut pubkeys = self.message_author_pubkeys_filtered(|_| true);
        pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
        pubkeys.dedup();
        KnownMessageAuthorCache {
            pubkey_set: pubkeys.iter().copied().collect(),
            hexes: pubkeys.iter().map(|pubkey| pubkey.to_hex()).collect(),
            pubkeys,
        }
    }

    fn invalidate_known_message_author_cache(&self) {
        self.known_message_author_cache.borrow_mut().take();
    }

    #[cfg(test)]
    pub(super) fn known_message_author_cache_build_count_for_test(&self) -> u64 {
        self.known_message_author_cache_build_count.get()
    }

    pub(super) fn known_group_sender_event_pubkeys(&self) -> Vec<PublicKey> {
        let mut authors = self
            .group_manager
            .known_sender_event_pubkeys()
            .into_iter()
            .filter_map(|pubkey| public_device(pubkey).ok())
            .collect::<Vec<_>>();
        authors.sort_by_key(|pubkey| pubkey.to_hex());
        authors.dedup();
        authors
    }

    pub(super) fn known_device_identity_pubkeys_for_owner(
        &self,
        owner_pubkey: PublicKey,
    ) -> Vec<PublicKey> {
        let owner = ndr_owner(owner_pubkey);
        let mut devices = self
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .map(|user| {
                user.devices
                    .into_iter()
                    .filter_map(|device| public_device(device.device_pubkey).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        devices.sort_by_key(|pubkey| pubkey.to_hex());
        devices.dedup();
        devices
    }

    pub(super) fn message_author_pubkeys_for_owner(
        &self,
        owner_pubkey: PublicKey,
    ) -> Vec<PublicKey> {
        let mut authors = HashSet::new();
        let owner = ndr_owner(owner_pubkey);
        for user in self.session_manager.snapshot().users {
            if user.owner_pubkey != owner {
                continue;
            }
            for device in user.devices {
                if let Some(session) = device.active_session.as_ref() {
                    collect_expected_sender_pubkeys(session, &mut authors);
                }
                for session in &device.inactive_sessions {
                    collect_expected_sender_pubkeys(session, &mut authors);
                }
            }
        }
        let mut authors = authors.into_iter().collect::<Vec<_>>();
        authors.sort_by_key(|pubkey| pubkey.to_hex());
        authors
    }

    /// `SessionManager::snapshot` clones every user record + every
    /// device state — the runtime debug builder fans out per known
    /// user, so callers that hit multiple owners in one pass must
    /// share a single snapshot via the `_with_snapshot` helpers
    /// below instead of paying that clone cost per owner.
    pub(super) fn session_manager_snapshot(&self) -> SessionManagerSnapshot {
        self.session_manager.snapshot()
    }

    pub(super) fn message_session_debug_snapshots_with_snapshot(
        snapshot: &SessionManagerSnapshot,
        owner_pubkey: PublicKey,
    ) -> Vec<ProtocolMessageSessionDebugSnapshot> {
        let owner = ndr_owner(owner_pubkey);
        snapshot
            .users
            .iter()
            .filter(|user| user.owner_pubkey == owner)
            .flat_map(|user| user.devices.iter())
            .flat_map(|device| {
                device
                    .active_session
                    .iter()
                    .chain(device.inactive_sessions.iter())
            })
            .map(|state| {
                let mut tracked = HashSet::new();
                collect_expected_sender_pubkeys(state, &mut tracked);
                let mut tracked_sender_pubkeys = tracked.into_iter().collect::<Vec<_>>();
                tracked_sender_pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
                ProtocolMessageSessionDebugSnapshot {
                    has_receiving_capability: state.receiving_chain_key.is_some()
                        || state.their_current_nostr_public_key.is_some(),
                    state: state.clone(),
                    tracked_sender_pubkeys,
                }
            })
            .collect()
    }

    pub(super) fn active_session_count_for_owner_with_snapshot(
        snapshot: &SessionManagerSnapshot,
        owner_pubkey: PublicKey,
    ) -> usize {
        let owner = ndr_owner(owner_pubkey);
        snapshot
            .users
            .iter()
            .filter(|user| user.owner_pubkey == owner)
            .flat_map(|user| user.devices.iter())
            .filter(|device| device.active_session.is_some())
            .count()
    }

    pub(super) fn active_session_count_for_owner(&self, owner_pubkey: PublicKey) -> usize {
        Self::active_session_count_for_owner_with_snapshot(
            &self.session_manager.snapshot(),
            owner_pubkey,
        )
    }

    pub(super) fn queued_message_diagnostics(&self, message_id: Option<&str>) -> Vec<String> {
        let mut targets = Vec::new();
        for pending in &self.pending_outbound {
            if message_id
                .map(|message_id| pending.message_id != message_id)
                .unwrap_or(false)
            {
                continue;
            }
            targets.extend(self.pending_target_hexes(pending));
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn pending_outbound_debug_details(&self) -> Vec<ProtocolPendingOutboundDebug> {
        self.pending_outbound
            .iter()
            .map(|pending| {
                let remaining_remote_targets = PublicKey::parse(&pending.recipient_owner_hex)
                    .ok()
                    .map(|owner| {
                        self.remaining_remote_targets(
                            ndr_owner(owner),
                            &pending.delivered_remote_device_hexes,
                        )
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|target| target.to_hex())
                    .collect::<Vec<_>>();
                let remaining_local_sibling_targets = self
                    .remaining_local_sibling_targets(&pending.delivered_local_device_hexes)
                    .into_iter()
                    .map(|target| target.to_hex())
                    .collect::<Vec<_>>();
                ProtocolPendingOutboundDebug {
                    message_id: pending.message_id.clone(),
                    chat_id: pending.chat_id.clone(),
                    recipient_owner_hex: pending.recipient_owner_hex.clone(),
                    reason: format!("{:?}", pending.reason),
                    probe_local_sibling_roster: pending.probe_local_sibling_roster,
                    delivered_remote_device_hexes: pending.delivered_remote_device_hexes.clone(),
                    delivered_local_device_hexes: pending.delivered_local_device_hexes.clone(),
                    remaining_remote_targets,
                    remaining_local_sibling_targets,
                    queued_targets: self.pending_target_hexes(pending),
                    next_retry_at_secs: pending.next_retry_at_secs,
                }
            })
            .collect()
    }

    pub(super) fn has_queued_remote_message_work(&self, message_id: &str) -> bool {
        self.pending_outbound.iter().any(|pending| {
            pending.message_id == message_id
                && !self.pending_remote_target_hexes(pending).is_empty()
        })
    }

    fn state_checkpoint(&self) -> ProtocolEngineCheckpoint {
        ProtocolEngineCheckpoint {
            session_manager: self.session_manager.clone(),
            group_manager: self.group_manager.clone(),
            latest_app_keys_created_at: self.latest_app_keys_created_at.clone(),
            pending_outbound: self.pending_outbound.clone(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: ProtocolEngineCheckpoint) {
        self.session_manager = checkpoint.session_manager;
        self.group_manager = checkpoint.group_manager;
        self.latest_app_keys_created_at = checkpoint.latest_app_keys_created_at;
        self.pending_outbound = checkpoint.pending_outbound;
        self.pending_inbound = checkpoint.pending_inbound;
        self.pending_group_fanouts = checkpoint.pending_group_fanouts;
        self.pending_group_pairwise_payloads = checkpoint.pending_group_pairwise_payloads;
        self.pending_group_sender_key_messages = checkpoint.pending_group_sender_key_messages;
        self.pending_group_sender_key_repairs = checkpoint.pending_group_sender_key_repairs;
        self.pending_decrypted_deliveries = checkpoint.pending_decrypted_deliveries;
        self.subscription_generation = checkpoint.subscription_generation;
        self.last_backfill_attempt_secs = checkpoint.last_backfill_attempt_secs;
        self.invalidate_known_message_author_cache();
    }

    fn with_state_checkpoint<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let checkpoint = self.state_checkpoint();
        match operation(self) {
            Ok(value) => {
                self.invalidate_known_message_author_cache();
                Ok(value)
            }
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

}
