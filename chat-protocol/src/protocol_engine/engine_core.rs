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
            delivered_group_sender_key_acks: Vec::new(),
            answered_group_sender_key_repairs: Vec::new(),
            pending_decrypted_deliveries: Vec::new(),
            group_roster_fact_histories: BTreeMap::new(),
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            local_app_keys_observed: false,
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
            delivered_group_sender_key_acks,
            answered_group_sender_key_repairs,
            pending_decrypted_deliveries: state.pending_decrypted_deliveries,
            group_roster_fact_histories: state.group_roster_fact_histories,
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            local_app_keys_observed: false,
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
        self.persist()
    }

    pub fn debug_snapshot(&self) -> ProtocolEngineDebugSnapshot {
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
            pending_outbound_count: 0,
            pending_inbound_count: 0,
            pending_group_fanout_count: 0,
            pending_group_pairwise_payload_count: 0,
            pending_group_sender_key_message_count: 0,
            pending_group_sender_key_retry_count: 0,
            pending_group_sender_key_unmapped_count: 0,
            pending_group_sender_key_repair_count: 0,
            pending_group_sender_key_repair_last_requested_at_secs: 0,
            pending_group_sender_key_repair_next_retry_at_secs: 0,
            pending_group_sender_key_repair_max_request_count: 0,
            pending_outbound_targets: Vec::new(),
            pending_outbound_details: Vec::new(),
            pending_group_fanout_targets: Vec::new(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
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
            }
        }
        None
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
        false
    }

    pub fn has_pending_retry_work(&self) -> bool {
        false
    }

    pub fn has_pending_inbound_direct_event_id(&self, event_id: &str) -> bool {
        let _ = event_id;
        false
    }

    pub fn queued_owner_claim_targets(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn queued_group_target_hexes(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn has_queued_invite_author(&self, author: PublicKey) -> bool {
        let _ = author;
        false
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

    pub fn has_roster_for_owner(&self, owner_pubkey: PublicKey) -> bool {
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster.as_ref())
            .is_some_and(|roster| !roster.devices().is_empty())
    }

    pub fn has_direct_send_capability_for_owner(&self, owner_pubkey: PublicKey) -> bool {
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .iter()
            .find(|user| user.owner_pubkey == owner)
            .is_some_and(|user| {
                user.devices.iter().any(|device| {
                    device.authorized
                        && !device.is_stale
                        && (device.active_session.is_some() || device.public_invite.is_some())
                })
            })
    }

    pub fn queued_message_diagnostics(&self, message_id: Option<&str>) -> Vec<String> {
        let _ = message_id;
        Vec::new()
    }

    pub fn has_delivery_blocking_message_work(&self, message_id: &str) -> bool {
        let _ = message_id;
        false
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

    fn state_checkpoint(&self) -> ProtocolEngineCheckpoint {
        ProtocolEngineCheckpoint {
            session_manager: self.session_manager.clone(),
            group_manager: self.group_manager.clone(),
            delivered_group_sender_key_acks: self.delivered_group_sender_key_acks.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: ProtocolEngineCheckpoint) {
        self.session_manager = checkpoint.session_manager;
        self.group_manager = checkpoint.group_manager;
        self.delivered_group_sender_key_acks = checkpoint.delivered_group_sender_key_acks;
        self.answered_group_sender_key_repairs = checkpoint.answered_group_sender_key_repairs;
        self.pending_decrypted_deliveries = checkpoint.pending_decrypted_deliveries;
        self.group_roster_fact_histories = checkpoint.group_roster_fact_histories;
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
