impl ProtocolEngine {
    pub fn load_or_create_for_local_device(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        device_keys: &Keys,
    ) -> anyhow::Result<Self> {
        let local_owner = ndr_owner(owner_pubkey);
        let local_device = ndr_device(device_keys.public_key());
        let device_secret = device_keys.secret_key().to_secret_bytes();
        let mut engine = Self::load_persisted_state(
            Arc::clone(&storage),
            owner_pubkey,
            local_owner,
            local_device,
            device_secret,
        )?
        .unwrap_or_else(|| Self {
            owner_pubkey,
            local_owner,
            local_device,
            storage,
            session_manager: SessionManager::new(local_owner, device_secret),
            group_manager: NostrGroupManager::new(local_owner),
            latest_app_keys_created_at: BTreeMap::new(),
            pending_outbound: Vec::new(),
            pending_inbound: Vec::new(),
            pending_group_fanouts: Vec::new(),
            pending_group_pairwise_payloads: Vec::new(),
            pending_group_sender_key_messages: Vec::new(),
            pending_group_sender_key_repairs: Vec::new(),
            delivered_group_sender_key_acks: Vec::new(),
            answered_group_sender_key_repairs: Vec::new(),
            pending_decrypted_deliveries: Vec::new(),
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            subscription_generation: 0,
            last_backfill_attempt_secs: 0,
            batch_depth: std::cell::Cell::new(0),
            batch_persist_dirty: std::cell::Cell::new(false),
        });

        let local_invite = if let Some(invite) = engine.session_manager.snapshot().local_invite {
            let invite = normalize_local_invite_owner(invite, owner_pubkey);
            engine.session_manager.replace_local_invite(invite.clone());
            invite
        } else {
            let device_id = device_keys.public_key().to_hex();
            let invite = load_or_create_local_invite(
                engine.storage.as_ref(),
                device_keys.public_key(),
                &device_id,
                owner_pubkey,
            )?;
            engine.session_manager.replace_local_invite(invite.clone());
            invite
        };
        engine.finish_local_device_startup(local_invite.created_at)?;
        Ok(engine)
    }

    fn load_persisted_state(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        local_owner: NdrOwnerPubkey,
        local_device: NdrDevicePubkey,
        device_secret: [u8; 32],
    ) -> anyhow::Result<Option<Self>> {
        let Some(raw) = storage.get(PROTOCOL_ENGINE_STATE_KEY)? else {
            return Ok(None);
        };
        let Ok(state) = serde_json::from_str::<ProtocolEnginePersistedState>(&raw) else {
            return Ok(None);
        };
        if state.version != PROTOCOL_ENGINE_STATE_VERSION {
            return Ok(None);
        }

        let session_manager = SessionManager::from_snapshot(state.session_manager, device_secret)?;
        let group_manager = NostrGroupManager::from_snapshot(state.group_manager)?;
        let mut delivered_group_sender_key_acks = state.delivered_group_sender_key_acks;
        let excess = delivered_group_sender_key_acks
            .len()
            .saturating_sub(DELIVERED_GROUP_SENDER_KEY_ACK_LIMIT);
        if excess > 0 {
            delivered_group_sender_key_acks.drain(0..excess);
        }
        let mut answered_group_sender_key_repairs = state.answered_group_sender_key_repairs;
        let excess = answered_group_sender_key_repairs
            .len()
            .saturating_sub(ANSWERED_GROUP_SENDER_KEY_REPAIR_LIMIT);
        if excess > 0 {
            answered_group_sender_key_repairs.drain(0..excess);
        }

        Ok(Some(Self {
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
            delivered_group_sender_key_acks,
            answered_group_sender_key_repairs,
            pending_decrypted_deliveries: state.pending_decrypted_deliveries,
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            subscription_generation: state.subscription_generation,
            last_backfill_attempt_secs: state.last_backfill_attempt_secs,
            batch_depth: std::cell::Cell::new(0),
            batch_persist_dirty: std::cell::Cell::new(false),
        }))
    }

    fn finish_local_device_startup(
        &mut self,
        local_invite_created_at: NdrUnixSeconds,
    ) -> anyhow::Result<()> {
        self.ensure_local_roster(local_invite_created_at);
        self.hydrate_pending_inbound_metadata();
        self.prune_untracked_pending_inbound();
        self.prune_pending_group_sender_key_work_for_inactive_local_groups();
        self.persist()
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

    pub fn debug_snapshot(&self) -> ProtocolEngineDebugSnapshot {
        let pending_group_sender_key_retry_count =
            self.pending_group_sender_key_retry_count();
        let pending_group_sender_key_unmapped_count = self
            .pending_group_sender_key_messages
            .len()
            .saturating_sub(pending_group_sender_key_retry_count);
        let known_message_author_pubkeys = self
            .known_message_author_pubkeys()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<Vec<_>>();
        let known_group_sender_key_author_pubkeys = self
            .known_group_sender_event_pubkeys()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<Vec<_>>();
        ProtocolEngineDebugSnapshot {
            known_message_author_count: known_message_author_pubkeys.len(),
            known_message_author_pubkeys,
            known_group_sender_key_author_count: known_group_sender_key_author_pubkeys.len(),
            known_group_sender_key_author_pubkeys,
            pending_outbound_count: self.pending_outbound.len(),
            pending_inbound_count: self.pending_inbound.len(),
            pending_group_fanout_count: self.pending_group_fanouts.len(),
            pending_group_pairwise_payload_count: self.pending_group_pairwise_payloads.len(),
            pending_group_sender_key_message_count: self.pending_group_sender_key_messages.len(),
            pending_group_sender_key_retry_count,
            pending_group_sender_key_unmapped_count,
            pending_group_sender_key_repair_count: self.pending_group_sender_key_repairs.len(),
            pending_group_sender_key_repair_last_requested_at_secs: self
                .pending_group_sender_key_repairs
                .iter()
                .map(|repair| repair.last_requested_at_secs)
                .max()
                .unwrap_or_default(),
            pending_group_sender_key_repair_next_retry_at_secs: self
                .pending_group_sender_key_repairs
                .iter()
                .map(Self::pending_group_sender_key_repair_due_at_secs)
                .min()
                .unwrap_or_default(),
            pending_group_sender_key_repair_max_request_count: self
                .pending_group_sender_key_repairs
                .iter()
                .map(|repair| repair.request_count)
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

    pub fn session_manager_snapshot_for_test(&self) -> SessionManagerSnapshot {
        self.session_manager.snapshot()
    }

    pub fn group_manager_snapshot_for_test(&self) -> GroupManagerSnapshot {
        self.group_manager.snapshot()
    }

    pub fn is_known_local_owner_device(&self, device_pubkey: PublicKey) -> bool {
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

    pub fn owner_hint_for_device(
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

    fn verified_roster_owner_for_device(
        &self,
        device_pubkey: NdrDevicePubkey,
    ) -> Option<NdrOwnerPubkey> {
        let provisional_owner = NdrOwnerPubkey::from_bytes(device_pubkey.to_bytes());
        let mut provisional_match = None;
        for user in self.session_manager.snapshot().users {
            let Some(roster) = user.roster.as_ref() else {
                continue;
            };
            if roster.get_device(&device_pubkey).is_none() {
                continue;
            }
            if user.owner_pubkey == self.local_owner {
                continue;
            }
            if user.owner_pubkey != provisional_owner {
                return Some(user.owner_pubkey);
            }
            provisional_match = Some(user.owner_pubkey);
        }
        provisional_match
    }

    pub fn has_pending_inbound_direct_events(&self) -> bool {
        !self.pending_inbound.is_empty()
    }

    pub fn has_pending_retry_work(&self) -> bool {
        !self.pending_inbound.is_empty()
            || !self.pending_group_fanouts.is_empty()
            || !self.pending_group_pairwise_payloads.is_empty()
            || self.has_pending_group_sender_key_retry_work()
            || !self.pending_group_sender_key_repairs.is_empty()
    }

    fn has_pending_group_sender_key_retry_work(&self) -> bool {
        self.pending_group_sender_key_retry_count() > 0
    }

    fn pending_group_sender_key_retry_count(&self) -> usize {
        self.pending_group_sender_key_messages
            .iter()
            .filter(|pending| {
                self.group_manager
                    .group_id_for_sender_event_pubkey(pending.sender_event_pubkey)
                    .is_some()
            })
            .count()
    }

    pub fn has_pending_inbound_direct_event_id(&self, event_id: &str) -> bool {
        self.pending_inbound.iter().any(|pending| {
            let pending_event_id = if pending.event_id.is_empty() {
                pending.event.id.to_string()
            } else {
                pending.event_id.clone()
            };
            pending_event_id == event_id
        })
    }

    pub fn queued_owner_claim_targets(&self) -> Vec<String> {
        let mut targets = self.pending_inbound_owner_claim_targets();
        targets.extend(self.pending_group_pairwise_owner_claim_targets());
        targets.sort();
        targets.dedup();
        targets
    }

    pub fn queued_protocol_backfill_effects(
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

    pub fn queued_group_target_hexes(&self) -> Vec<String> {
        self.queued_group_targets()
    }

    pub fn has_queued_invite_author(&self, author: PublicKey) -> bool {
        let target = ndr_device(author);
        let snapshot = self.session_manager.snapshot();
        self.pending_outbound.iter().any(|pending| {
            self.pending_outbound_targets_device_with_snapshot(pending, target, &snapshot)
        })
    }

    pub fn local_invite(&self) -> Option<Invite> {
        self.session_manager.snapshot().local_invite
    }

    pub fn local_invite_response_pubkey(&self) -> Option<PublicKey> {
        self.local_invite()?
            .inviter_ephemeral_public_key
            .to_nostr()
            .ok()
    }

    pub fn pending_inbound_for_test(&self) -> Vec<ProtocolPendingInboundTestDebug> {
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

    pub fn known_message_author_pubkeys(&self) -> Vec<PublicKey> {
        self.with_known_message_author_cache(|cache| cache.pubkeys.clone())
    }

    /// Walks every session and returns its expected event-author
    /// pubkeys, but only for sessions whose peer owner passes the
    /// `accept_owner` predicate. The owner-aware variant lets the
    /// caller drop blocked / non-accepted peers from the subscription
    /// filter without losing the device-ephemeral keys that nostr
    /// actually filters on.
    pub fn message_author_pubkeys_filtered<F>(&self, accept_owner: F) -> Vec<PublicKey>
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

    pub fn is_known_message_author(&self, author: PublicKey) -> bool {
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

    pub fn known_message_author_cache_build_count_for_test(&self) -> u64 {
        self.known_message_author_cache_build_count.get()
    }

    pub fn pending_decrypted_deliveries_len_for_test(&self) -> usize {
        self.pending_decrypted_deliveries.len()
    }

    pub fn known_group_sender_event_pubkeys(&self) -> Vec<PublicKey> {
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

    pub fn is_known_group_sender_event_author(&self, author: PublicKey) -> bool {
        self.group_manager
            .group_id_for_sender_event_pubkey(ndr_device(author))
            .is_some()
    }

    pub fn known_device_identity_pubkeys_for_owner(
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

    pub fn message_author_pubkeys_for_owner(&self, owner_pubkey: PublicKey) -> Vec<PublicKey> {
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
    pub fn session_manager_snapshot(&self) -> SessionManagerSnapshot {
        self.session_manager.snapshot()
    }

    pub fn message_session_debug_snapshots_with_snapshot(
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

    pub fn active_session_count_for_owner_with_snapshot(
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

    pub fn active_session_count_for_owner(&self, owner_pubkey: PublicKey) -> usize {
        Self::active_session_count_for_owner_with_snapshot(
            &self.session_manager.snapshot(),
            owner_pubkey,
        )
    }

    pub fn queued_message_diagnostics(&self, message_id: Option<&str>) -> Vec<String> {
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

    pub fn has_delivery_blocking_message_work(&self, message_id: &str) -> bool {
        self.pending_outbound
            .iter()
            .any(|pending| {
                pending.message_id == message_id
                    && self.pending_outbound_blocks_delivery(pending)
            })
            || self.pending_group_fanouts.iter().any(|pending| {
                pending.inner_event_id.as_deref() == Some(message_id)
            })
    }

    fn pending_outbound_blocks_delivery(&self, pending: &ProtocolPendingOutbound) -> bool {
        !self.pending_remote_target_hexes(pending).is_empty()
            || (pending.local_sibling_payload.is_some()
                && !self
                    .remaining_local_sibling_targets(&pending.delivered_local_device_hexes)
                    .is_empty())
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
            delivered_group_sender_key_acks: self.delivered_group_sender_key_acks.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
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
        self.delivered_group_sender_key_acks = checkpoint.delivered_group_sender_key_acks;
        self.answered_group_sender_key_repairs = checkpoint.answered_group_sender_key_repairs;
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

fn load_or_create_local_invite(
    storage: &dyn StorageAdapter,
    device_pubkey: PublicKey,
    device_id: &str,
    owner_pubkey: PublicKey,
) -> anyhow::Result<Invite> {
    let storage_key = format!("device-invite/{device_id}");
    if let Some(serialized) = storage.get(&storage_key)? {
        if let Ok(invite) = Invite::deserialize(&serialized) {
            return Ok(normalize_local_invite_owner(invite, owner_pubkey));
        }
    }

    let mut invite = Invite::create_new(device_pubkey, Some(device_id.to_string()), None)?;
    invite = normalize_local_invite_owner(invite, owner_pubkey);
    storage.put(&storage_key, invite.serialize()?)?;
    Ok(invite)
}

fn normalize_local_invite_owner(mut invite: Invite, owner_pubkey: PublicKey) -> Invite {
    invite.inviter_owner_pubkey = Some(ndr_owner(owner_pubkey));
    invite.owner_public_key = Some(owner_pubkey);
    invite
}
