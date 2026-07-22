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
            local_device_secret: device_secret,
            storage,
            session_manager: SessionManager::new(local_owner, device_secret),
            group_manager: GroupEventManager::new(local_owner),
            pending_inbound: Vec::new(),
            pending_group_fanouts: Vec::new(),
            pending_group_pairwise_payloads: Vec::new(),
            pending_group_sender_key_messages: Vec::new(),
            pending_group_sender_key_repairs: Vec::new(),
            delivered_group_sender_key_acks: Vec::new(),
            answered_group_sender_key_repairs: Vec::new(),
            pending_decrypted_deliveries: Vec::new(),
            group_roster_fact_histories: BTreeMap::new(),
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            verified_app_keys_owners: BTreeSet::new(),
            invite_owner_app_keys_evidence: BTreeMap::new(),
            processed_private_invite_response_ids: Vec::new(),
            local_app_keys_observed: false,
            local_owner_authenticated: false,
            subscription_generation: 0,
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
        let Ok(mut state) = serde_json::from_str::<ProtocolEnginePersistedState>(&raw) else {
            return Ok(None);
        };
        if state.version != PROTOCOL_ENGINE_STATE_VERSION {
            return Ok(None);
        }
        sanitize_invite_owner_persisted_state(&mut state);
        quarantine_unverified_owner_rosters(
            &mut state.session_manager,
            &state.verified_app_keys_owners,
        );
        let session_manager = SessionManager::from_snapshot(state.session_manager, device_secret)?;
        let group_manager = GroupEventManager::from_snapshot(state.group_manager)?;
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
            local_device_secret: device_secret,
            storage,
            session_manager,
            group_manager,
            pending_inbound: state.pending_inbound,
            pending_group_fanouts: state.pending_group_fanouts,
            pending_group_pairwise_payloads: state.pending_group_pairwise_payloads,
            pending_group_sender_key_messages: state.pending_group_sender_key_messages,
            pending_group_sender_key_repairs: state.pending_group_sender_key_repairs,
            delivered_group_sender_key_acks,
            answered_group_sender_key_repairs,
            pending_decrypted_deliveries: state.pending_decrypted_deliveries,
            group_roster_fact_histories: state.group_roster_fact_histories,
            known_message_author_cache: std::cell::RefCell::new(None),
            known_message_author_cache_build_count: std::cell::Cell::new(0),
            local_app_keys_observed: state.verified_app_keys_owners.contains(&local_owner),
            verified_app_keys_owners: state.verified_app_keys_owners,
            invite_owner_app_keys_evidence: state.invite_owner_app_keys_evidence,
            processed_private_invite_response_ids: state
                .processed_private_invite_response_ids,
            local_owner_authenticated: false,
            subscription_generation: state.subscription_generation,
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
        self.prune_superseded_local_group_sync_fanouts();
        self.prune_pending_group_sender_key_work_for_inactive_local_groups();
        self.persist()
    }

    fn prune_superseded_local_group_sync_fanouts(&mut self) -> bool {
        let original_len = self.pending_group_fanouts.len();
        if original_len < 2 {
            return false;
        }

        let mut seen_groups = HashSet::new();
        let mut compacted = Vec::with_capacity(original_len);
        for pending in std::mem::take(&mut self.pending_group_fanouts)
            .into_iter()
            .rev()
        {
            let keep = if pending.inner_event_id.is_none()
                && matches!(
                    &pending.fanout,
                    GroupPendingFanout::LocalSiblings { .. }
                )
            {
                seen_groups.insert(pending.group_id.clone())
            } else {
                true
            };
            if keep {
                compacted.push(pending);
            }
        }
        compacted.reverse();
        self.pending_group_fanouts = compacted;
        self.pending_group_fanouts.len() != original_len
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
        self.pending_inbound
            .retain(|pending| known_authors.contains(&pending_inbound_sender_pubkey_hex(pending)));
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
            pending_group_fanout_targets: self.queued_group_targets(),
            subscription_generation: self.subscription_generation,
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
        if device_pubkey == self.local_device {
            return true;
        }
        if !self.verified_app_keys_owners.contains(&self.local_owner) {
            return false;
        }
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == self.local_owner)
            .is_some_and(|user| {
                user.roster.as_ref().is_some_and(|roster| {
                    roster.get_device(&device_pubkey).is_some()
                }) && user
                    .devices
                    .iter()
                    .any(|device| {
                        device.device_pubkey == device_pubkey
                            && device.authorized
                            && !device.is_stale
                    })
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
                    if self.verified_app_keys_owners.contains(&user.owner_pubkey)
                        && user.roster.as_ref().is_some_and(|roster| {
                            roster.get_device(&record.device_pubkey).is_some()
                        })
                    {
                        return public_owner(user.owner_pubkey).ok().map(|owner| {
                            ProtocolDeviceOwnerHint {
                                owner,
                                verified: true,
                            }
                        });
                    }
                    if claimed_owner.is_none() {
                        claimed_owner = Some(user.owner_pubkey);
                    }
                    continue;
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
                if self.verified_app_keys_owners.contains(&user.owner_pubkey) {
                    return Some(user.owner_pubkey);
                }
                continue;
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

    pub fn direct_send_readiness(&self, peer_pubkey: PublicKey) -> DirectSendReadiness {
        let snapshot = self.session_manager.snapshot();
        if !self.local_owner_authenticated
            && !self.local_app_keys_observed
            && !self.has_authoritative_local_roster()
        {
            return DirectSendReadiness::MissingLocalAppKeys;
        }

        let peer_owner = ndr_owner(peer_pubkey);
        let Some(peer_user) = user_record_snapshot(&snapshot, peer_owner) else {
            return DirectSendReadiness::MissingPeerAppKeys;
        };
        let Some(peer_devices) = roster_device_pubkeys(peer_user) else {
            return DirectSendReadiness::MissingPeerAppKeys;
        };
        if peer_devices.is_empty() {
            return DirectSendReadiness::MissingPeerAppKeys;
        }
        if peer_devices
            .iter()
            .any(|device| !user_can_send_to_device(peer_user, *device))
        {
            return DirectSendReadiness::MissingPeerInviteOrSession;
        }

        if let Some(local_user) = user_record_snapshot(&snapshot, self.local_owner) {
            if let Some(local_devices) = roster_device_pubkeys(local_user) {
                if local_devices
                    .into_iter()
                    .filter(|device| *device != self.local_device)
                    .any(|device| !user_can_send_to_device(local_user, device))
                {
                    return DirectSendReadiness::MissingLocalSiblingInviteOrSession;
                }
            }
        }

        DirectSendReadiness::Ready
    }

    pub fn authenticate_local_owner_for_sending(
        &mut self,
        owner_keys: &Keys,
    ) -> anyhow::Result<()> {
        if owner_keys.public_key() != self.owner_pubkey {
            anyhow::bail!("local owner key does not match protocol owner");
        }
        self.local_owner_authenticated = true;
        Ok(())
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
            let roster = user.roster.clone();
            for device in user.devices {
                if !device.authorized
                    || device.is_stale
                    || !self.owner_device_binding_is_verified_in_roster(
                        user.owner_pubkey,
                        device.device_pubkey,
                        roster.as_ref(),
                    )
                {
                    continue;
                }
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

    pub fn is_potential_group_sender_key_event(&self, event: &Event) -> bool {
        parse_group_sender_key_message_event_unchecked(event).is_ok_and(|parsed| {
            self.group_manager
                .group_id_for_sender_event_pubkey(parsed.sender_event_pubkey)
                .is_some()
        })
    }

    pub fn is_group_sender_key_candidate_with_local_group_context(&self, event: &Event) -> bool {
        if !protocol_event_has_tag(event, "header") || self.group_manager.snapshot().groups.is_empty()
        {
            return false;
        }
        parse_group_sender_key_message_event_unchecked(event).is_ok()
    }

    pub fn header_message_sender_has_verified_owner(&self, event: &Event) -> bool {
        if !protocol_event_has_tag(event, "header") {
            return false;
        }
        let Ok(envelope) = parse_message_event(event) else {
            return false;
        };
        let Ok(sender) = public_device(envelope.sender) else {
            return false;
        };
        self.owner_hint_for_device(sender)
            .is_some_and(|hint| hint.verified)
    }

    pub fn header_message_sender_has_tracked_session(&self, event: &Event) -> bool {
        if !protocol_event_has_tag(event, "header") {
            return false;
        }
        let Ok(envelope) = parse_message_event(event) else {
            return false;
        };
        !matches!(
            self.resolve_message_sender_owner(&envelope),
            ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { .. }
        )
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
            .and_then(|user| {
                user.roster.map(|roster| {
                    roster
                        .devices()
                        .iter()
                        .filter(|device| {
                            self.owner_device_binding_is_verified_in_roster(
                                owner,
                                device.device_pubkey,
                                Some(&roster),
                            )
                        })
                        .filter_map(|device| public_device(device.device_pubkey).ok())
                        .collect::<Vec<_>>()
                })
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
            let roster = user.roster.clone();
            for device in user.devices {
                if !device.authorized
                    || device.is_stale
                    || !self.owner_device_binding_is_verified_in_roster(
                        owner,
                        device.device_pubkey,
                        roster.as_ref(),
                    )
                {
                    continue;
                }
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

    pub fn verified_message_session_snapshots_for_owner(
        &self,
        owner_pubkey: PublicKey,
    ) -> Vec<ProtocolMessageSessionDebugSnapshot> {
        let owner = ndr_owner(owner_pubkey);
        let mut snapshot = self.session_manager.snapshot();
        for user in &mut snapshot.users {
            if user.owner_pubkey != owner {
                continue;
            }
            let roster = user.roster.clone();
            user.devices.retain(|device| {
                device.authorized
                    && !device.is_stale
                    && self.owner_device_binding_is_verified_in_roster(
                        owner,
                        device.device_pubkey,
                        roster.as_ref(),
                    )
            });
        }
        Self::message_session_debug_snapshots_with_snapshot(&snapshot, owner_pubkey)
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
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .filter(|user| user.owner_pubkey == owner)
            .map(|user| {
                let roster = user.roster;
                user.devices
                    .into_iter()
                    .filter(|device| {
                        device.active_session.is_some()
                            && device.authorized
                            && !device.is_stale
                            && self.owner_device_binding_is_verified_in_roster(
                                owner,
                                device.device_pubkey,
                                roster.as_ref(),
                            )
                    })
                    .count()
            })
            .sum()
    }

    pub fn active_roster_session_count_for_owner(&self, owner_pubkey: PublicKey) -> usize {
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| {
                let roster = user.roster?;
                Some(
                    user.devices
                        .iter()
                        .filter(|device| {
                            device.active_session.is_some()
                                && device.authorized
                                && !device.is_stale
                                && self.owner_device_binding_is_verified_in_roster(
                                    owner,
                                    device.device_pubkey,
                                    Some(&roster),
                                )
                                && roster.devices().iter().any(|entry| {
                                    entry.device_pubkey == device.device_pubkey
                                })
                        })
                        .count(),
                )
            })
            .unwrap_or_default()
    }

    pub fn owner_device_binding_is_verified(
        &self,
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
    ) -> bool {
        self.has_verified_device_owner_claim(
            ndr_owner(owner_pubkey),
            ndr_device(device_pubkey),
        )
    }

    fn owner_device_binding_is_verified_in_roster(
        &self,
        owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
        roster: Option<&DeviceRoster>,
    ) -> bool {
        owner == provisional_owner_from_sender_pubkey(device)
            || (self.verified_app_keys_owners.contains(&owner)
                && roster.is_some_and(|roster| roster.get_device(&device).is_some()))
    }

    pub fn known_verified_peer_owner_pubkeys(&self) -> Vec<PublicKey> {
        let mut owners = self
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .filter(|user| {
                let roster = user.roster.as_ref();
                user.devices.iter().any(|device| {
                    self.owner_device_binding_is_verified_in_roster(
                        user.owner_pubkey,
                        device.device_pubkey,
                        roster,
                    )
                })
            })
            .filter_map(|user| public_owner(user.owner_pubkey).ok())
            .collect::<Vec<_>>();
        owners.sort_by_key(|owner| owner.to_hex());
        owners.dedup();
        owners
    }

    pub fn has_delivery_blocking_message_work(&self, message_id: &str) -> bool {
        self.pending_group_fanouts
            .iter()
            .any(|pending| pending.inner_event_id.as_deref() == Some(message_id))
    }

    fn state_checkpoint(&self) -> ProtocolEngineCheckpoint {
        ProtocolEngineCheckpoint {
            session_manager: self.session_manager.clone(),
            group_manager: self.group_manager.clone(),
            pending_inbound: self.pending_inbound.clone(),
            pending_group_fanouts: self.pending_group_fanouts.clone(),
            pending_group_pairwise_payloads: self.pending_group_pairwise_payloads.clone(),
            pending_group_sender_key_messages: self.pending_group_sender_key_messages.clone(),
            pending_group_sender_key_repairs: self.pending_group_sender_key_repairs.clone(),
            delivered_group_sender_key_acks: self.delivered_group_sender_key_acks.clone(),
            answered_group_sender_key_repairs: self.answered_group_sender_key_repairs.clone(),
            pending_decrypted_deliveries: self.pending_decrypted_deliveries.clone(),
            group_roster_fact_histories: self.group_roster_fact_histories.clone(),
            verified_app_keys_owners: self.verified_app_keys_owners.clone(),
            invite_owner_app_keys_evidence: self.invite_owner_app_keys_evidence.clone(),
            processed_private_invite_response_ids: self
                .processed_private_invite_response_ids
                .clone(),
            local_app_keys_observed: self.local_app_keys_observed,
            subscription_generation: self.subscription_generation,
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: ProtocolEngineCheckpoint) {
        self.session_manager = checkpoint.session_manager;
        self.group_manager = checkpoint.group_manager;
        self.pending_inbound = checkpoint.pending_inbound;
        self.pending_group_fanouts = checkpoint.pending_group_fanouts;
        self.pending_group_pairwise_payloads = checkpoint.pending_group_pairwise_payloads;
        self.pending_group_sender_key_messages = checkpoint.pending_group_sender_key_messages;
        self.pending_group_sender_key_repairs = checkpoint.pending_group_sender_key_repairs;
        self.delivered_group_sender_key_acks = checkpoint.delivered_group_sender_key_acks;
        self.answered_group_sender_key_repairs = checkpoint.answered_group_sender_key_repairs;
        self.pending_decrypted_deliveries = checkpoint.pending_decrypted_deliveries;
        self.group_roster_fact_histories = checkpoint.group_roster_fact_histories;
        self.verified_app_keys_owners = checkpoint.verified_app_keys_owners;
        self.invite_owner_app_keys_evidence = checkpoint.invite_owner_app_keys_evidence;
        self.processed_private_invite_response_ids =
            checkpoint.processed_private_invite_response_ids;
        self.local_app_keys_observed = checkpoint.local_app_keys_observed;
        self.subscription_generation = checkpoint.subscription_generation;
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
