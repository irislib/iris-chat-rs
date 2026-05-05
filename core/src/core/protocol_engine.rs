use super::*;

const PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
const PROTOCOL_ENGINE_STATE_VERSION: u32 = 1;
const LOCAL_SIBLING_PROTOCOL: &str = "ndr-local-sibling-copy";
const PENDING_RETRY_DELAY_SECS: u64 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct ProtocolEnginePersistedState {
    version: u32,
    session_manager: SessionManagerSnapshot,
    group_manager: GroupManagerSnapshot,
    #[serde(default)]
    latest_app_keys_created_at: BTreeMap<String, u64>,
    #[serde(default)]
    pending_outbound: Vec<ProtocolPendingOutbound>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProtocolPendingOutbound {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    recipient_owner_hex: String,
    remote_payload: Vec<u8>,
    local_sibling_payload: Option<Vec<u8>>,
    inner_event_id: Option<String>,
    #[serde(default)]
    delivered_remote_device_hexes: Vec<String>,
    #[serde(default)]
    delivered_local_device_hexes: Vec<String>,
    created_at_secs: u64,
    next_retry_at_secs: u64,
    reason: ProtocolPendingReason,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum ProtocolPendingReason {
    MissingRoster,
    MissingDeviceInvite,
    PublishRetry,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolDirectSendResult {
    pub(super) message_id: String,
    pub(super) event_ids: Vec<String>,
    pub(super) effects: Vec<RuntimeEffect>,
    pub(super) queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolRetryResult {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) event_ids: Vec<String>,
    pub(super) effects: Vec<RuntimeEffect>,
    pub(super) queued_targets: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct ProtocolDecryptedMessage {
    pub(super) sender: PublicKey,
    pub(super) sender_device: Option<PublicKey>,
    pub(super) conversation_owner: Option<PublicKey>,
    pub(super) content: String,
    pub(super) event_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct ProtocolEngineDebugSnapshot {
    pub(super) known_message_author_count: usize,
    pub(super) pending_outbound_count: usize,
    pub(super) pending_outbound_targets: Vec<String>,
    pub(super) latest_app_keys_owner_count: usize,
}

pub(super) struct ProtocolEngine {
    owner_pubkey: PublicKey,
    local_owner: NdrOwnerPubkey,
    local_device: NdrDevicePubkey,
    storage: Arc<dyn StorageAdapter>,
    session_manager: SessionManager,
    group_manager: NostrGroupManager,
    latest_app_keys_created_at: BTreeMap<String, u64>,
    pending_outbound: Vec<ProtocolPendingOutbound>,
}

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
            engine.session_manager.replace_local_invite(local_invite);
        }
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
        })
    }

    pub(super) fn debug_snapshot(&self) -> ProtocolEngineDebugSnapshot {
        ProtocolEngineDebugSnapshot {
            known_message_author_count: self.known_message_author_pubkeys().len(),
            pending_outbound_count: self.pending_outbound.len(),
            pending_outbound_targets: self.queued_message_diagnostics(None),
            latest_app_keys_owner_count: self.latest_app_keys_created_at.len(),
        }
    }

    pub(super) fn known_message_author_pubkeys(&self) -> Vec<PublicKey> {
        let mut authors = HashSet::new();
        for user in self.session_manager.snapshot().users {
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

    pub(super) fn ingest_app_keys_snapshot(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
    ) -> anyhow::Result<Vec<ProtocolRetryResult>> {
        let owner_hex = owner_pubkey.to_hex();
        let latest = self
            .latest_app_keys_created_at
            .get(&owner_hex)
            .copied()
            .unwrap_or(0);
        if created_at < latest {
            return Ok(Vec::new());
        }
        self.latest_app_keys_created_at
            .insert(owner_hex, created_at);
        let roster = DeviceRoster::new(
            NdrUnixSeconds(created_at),
            app_keys
                .get_all_devices()
                .into_iter()
                .map(|entry| {
                    AuthorizedDevice::new(
                        ndr_device(entry.identity_pubkey),
                        NdrUnixSeconds(entry.created_at),
                    )
                })
                .collect(),
        );
        if owner_pubkey == self.owner_pubkey {
            self.session_manager.apply_local_roster(roster);
        } else {
            self.session_manager
                .observe_peer_roster(ndr_owner(owner_pubkey), roster);
        }
        self.persist()?;
        self.retry_pending_outbound(NdrUnixSeconds(unix_now().get()))
    }

    pub(super) fn observe_invite_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Vec<ProtocolRetryResult>> {
        let invite = parse_invite_event(event)?;
        let invite_owner = invite
            .inviter_owner_pubkey
            .unwrap_or_else(|| NdrOwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        if invite.inviter_device_pubkey != self.local_device {
            self.session_manager
                .observe_device_invite(invite_owner, invite)?;
        }
        self.persist()?;
        self.retry_pending_outbound(NdrUnixSeconds(event.created_at.as_secs()))
    }

    pub(super) fn observe_invite_response_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Vec<ProtocolRetryResult>> {
        let Some(local_invite_recipient) = self
            .session_manager
            .snapshot()
            .local_invite
            .as_ref()
            .map(|invite| invite.inviter_ephemeral_public_key)
        else {
            return Ok(Vec::new());
        };
        let envelope = parse_invite_response_event(event)?;
        if envelope.recipient != local_invite_recipient {
            return Ok(Vec::new());
        }
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let _ = self
            .session_manager
            .observe_invite_response(&mut ctx, &envelope)?;
        self.persist()?;
        self.retry_pending_outbound(ctx.now)
    }

    pub(super) fn send_direct_text(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        text: &str,
        expires_at_secs: Option<u64>,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let now_ms = current_unix_millis();
        let mut options = pairwise_codec::EncodeOptions::new(now.get(), now_ms);
        if let Some(expires_at_secs) = expires_at_secs {
            options = options.with_expiration(expires_at_secs);
        }
        let rumor = pairwise_codec::message_event(self.owner_pubkey, text.to_string(), options)?;
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let remote_payload = serde_json::to_vec(&rumor)?;
        let local_sibling_payload = local_sibling_payload(peer_pubkey, &remote_payload)?;
        let recipient_owner = ndr_owner(peer_pubkey);
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let remote = self.session_manager.prepare_remote_send(
            &mut ctx,
            recipient_owner,
            remote_payload.clone(),
        )?;
        let local = self
            .session_manager
            .prepare_local_sibling_send_refreshing_one_way_sessions(
                &mut ctx,
                local_sibling_payload.clone(),
            )?;

        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        effects.extend(runtime_effects_from_prepared(
            &remote,
            Some(message_id.clone()),
            &mut event_ids,
        )?);
        effects.extend(runtime_effects_from_prepared(
            &local,
            Some(message_id.clone()),
            &mut event_ids,
        )?);

        let remote_delivered = delivered_device_hexes(&remote);
        let local_delivered = delivered_device_hexes(&local);
        let gaps = remote
            .relay_gaps
            .iter()
            .chain(local.relay_gaps.iter())
            .cloned()
            .collect::<Vec<_>>();
        if !gaps.is_empty() {
            self.upsert_pending_outbound(ProtocolPendingOutbound {
                message_id: message_id.clone(),
                chat_id: chat_id.to_string(),
                recipient_owner_hex: peer_pubkey.to_hex(),
                remote_payload,
                local_sibling_payload: Some(local_sibling_payload),
                inner_event_id: Some(message_id.clone()),
                delivered_remote_device_hexes: remote_delivered,
                delivered_local_device_hexes: local_delivered,
                created_at_secs: now.get(),
                next_retry_at_secs: now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                reason: pending_reason_from_gaps(&gaps),
            });
        }
        self.persist()?;
        let queued_targets = self.queued_message_diagnostics(Some(&message_id));
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
            queued_targets,
        })
    }

    pub(super) fn send_direct_unsigned_event(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        mut rumor: UnsignedEvent,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        rumor.ensure_id();
        let message_id = rumor
            .id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let remote_payload = serde_json::to_vec(&rumor)?;
        self.send_direct_payloads(
            peer_pubkey,
            chat_id,
            remote_payload.clone(),
            local_sibling_payload(peer_pubkey, &remote_payload)?,
            Some(message_id.clone()),
            message_id,
            now,
        )
    }

    fn send_direct_payloads(
        &mut self,
        peer_pubkey: PublicKey,
        chat_id: &str,
        remote_payload: Vec<u8>,
        local_sibling_payload: Vec<u8>,
        inner_event_id: Option<String>,
        message_id: String,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolDirectSendResult> {
        let recipient_owner = ndr_owner(peer_pubkey);
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let remote = self.session_manager.prepare_remote_send(
            &mut ctx,
            recipient_owner,
            remote_payload.clone(),
        )?;
        let local = self
            .session_manager
            .prepare_local_sibling_send_refreshing_one_way_sessions(
                &mut ctx,
                local_sibling_payload.clone(),
            )?;

        let mut event_ids = Vec::new();
        let mut effects = Vec::new();
        effects.extend(runtime_effects_from_prepared(
            &remote,
            inner_event_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(runtime_effects_from_prepared(
            &local,
            inner_event_id.clone(),
            &mut event_ids,
        )?);

        let remote_delivered = delivered_device_hexes(&remote);
        let local_delivered = delivered_device_hexes(&local);
        let gaps = remote
            .relay_gaps
            .iter()
            .chain(local.relay_gaps.iter())
            .cloned()
            .collect::<Vec<_>>();
        if !gaps.is_empty() {
            self.upsert_pending_outbound(ProtocolPendingOutbound {
                message_id: message_id.clone(),
                chat_id: chat_id.to_string(),
                recipient_owner_hex: peer_pubkey.to_hex(),
                remote_payload,
                local_sibling_payload: Some(local_sibling_payload),
                inner_event_id,
                delivered_remote_device_hexes: remote_delivered,
                delivered_local_device_hexes: local_delivered,
                created_at_secs: now.get(),
                next_retry_at_secs: now.get().saturating_add(PENDING_RETRY_DELAY_SECS),
                reason: pending_reason_from_gaps(&gaps),
            });
        }
        self.persist()?;
        let queued_targets = self.queued_message_diagnostics(Some(&message_id));
        Ok(ProtocolDirectSendResult {
            message_id,
            event_ids,
            effects,
            queued_targets,
        })
    }

    pub(super) fn process_direct_message_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<Option<ProtocolDecryptedMessage>> {
        let envelope = parse_message_event(event)?;
        let Some(sender_owner) =
            self.resolve_message_sender_owner(&envelope, event.created_at.as_secs())
        else {
            return Ok(None);
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
        self.persist()?;
        Ok(Some(ProtocolDecryptedMessage {
            sender: public_owner(received.owner_pubkey)?,
            sender_device: Some(public_device(received.device_pubkey)?),
            conversation_owner,
            content,
            event_id: Some(event.id.to_string()),
        }))
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
            let remote_targets = self
                .remaining_remote_targets(recipient_owner, &pending.delivered_remote_device_hexes);
            let local_targets =
                self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes);

            if remote_targets.is_empty() && local_targets.is_empty() {
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
                effects.extend(runtime_effects_from_prepared(
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
                    effects.extend(runtime_effects_from_prepared(
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
                pending.reason = pending_reason_from_gaps(&gaps);
                pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                still_pending.push(pending.clone());
            }
            if !event_ids.is_empty() || !effects.is_empty() {
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

    fn resolve_message_sender_owner(
        &self,
        envelope: &MessageEnvelope,
        created_at_secs: u64,
    ) -> Option<NdrOwnerPubkey> {
        let owners = self
            .session_manager
            .snapshot()
            .users
            .into_iter()
            .map(|user| user.owner_pubkey)
            .collect::<Vec<_>>();
        for owner in owners {
            let mut candidate = self.session_manager.clone();
            let mut rng = OsRng;
            let mut ctx = ProtocolContext::new(NdrUnixSeconds(created_at_secs), &mut rng);
            match candidate.receive(&mut ctx, owner, envelope) {
                Ok(Some(_)) => return Some(owner),
                Ok(None) | Err(_) => {}
            }
        }
        Some(NdrOwnerPubkey::from_bytes(envelope.sender.to_bytes()))
    }

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
        let delivered = delivered_device_hexes
            .iter()
            .filter_map(|hex| PublicKey::parse(hex).ok())
            .map(ndr_device)
            .collect::<HashSet<_>>();
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster)
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

    fn pending_target_hexes(&self, pending: &ProtocolPendingOutbound) -> Vec<String> {
        let mut targets = Vec::new();
        if let Ok(owner) = PublicKey::parse(&pending.recipient_owner_hex) {
            for target in self
                .remaining_remote_targets(ndr_owner(owner), &pending.delivered_remote_device_hexes)
            {
                targets.push(target.to_hex());
            }
        }
        for target in self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes) {
            targets.push(target.to_hex());
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn persist(&self) -> anyhow::Result<()> {
        let state = ProtocolEnginePersistedState {
            version: PROTOCOL_ENGINE_STATE_VERSION,
            session_manager: self.session_manager.snapshot(),
            group_manager: self.group_manager.snapshot(),
            latest_app_keys_created_at: self.latest_app_keys_created_at.clone(),
            pending_outbound: self.pending_outbound.clone(),
        };
        self.storage
            .put(PROTOCOL_ENGINE_STATE_KEY, serde_json::to_string(&state)?)?;
        Ok(())
    }
}

fn runtime_effects_from_prepared(
    prepared: &PreparedSend,
    inner_event_id: Option<String>,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<RuntimeEffect>> {
    let mut effects = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        effects.push(RuntimeEffect::PublishSigned(event));
    }
    for delivery in &prepared.deliveries {
        let event = message_event(&delivery.envelope)?;
        event_ids.push(event.id.to_string());
        effects.push(RuntimeEffect::PublishSignedForInnerEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_device_id: Some(public_device(delivery.device_pubkey)?.to_hex()),
        });
    }
    Ok(effects)
}

fn delivered_device_hexes(prepared: &PreparedSend) -> Vec<String> {
    let mut devices = prepared
        .deliveries
        .iter()
        .map(|delivery| delivery.device_pubkey.to_hex())
        .collect::<Vec<_>>();
    devices.sort();
    devices.dedup();
    devices
}

fn pending_reason_from_gaps(gaps: &[RelayGap]) -> ProtocolPendingReason {
    if gaps
        .iter()
        .any(|gap| matches!(gap, RelayGap::MissingRoster { .. }))
    {
        ProtocolPendingReason::MissingRoster
    } else if gaps.is_empty() {
        ProtocolPendingReason::PublishRetry
    } else {
        ProtocolPendingReason::MissingDeviceInvite
    }
}

fn collect_expected_sender_pubkeys(session: &SessionState, out: &mut HashSet<PublicKey>) {
    if let Some(current) = session.their_current_nostr_public_key {
        if let Ok(pubkey) = public_device(current) {
            out.insert(pubkey);
        }
    }
    if let Some(next) = session.their_next_nostr_public_key {
        if let Ok(pubkey) = public_device(next) {
            out.insert(pubkey);
        }
    }
    for device in session.skipped_keys.keys() {
        if let Ok(pubkey) = public_device(*device) {
            out.insert(pubkey);
        }
    }
}

fn local_sibling_payload(conversation_owner: PublicKey, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    let wrapper = LocalSiblingPayload {
        protocol: LOCAL_SIBLING_PROTOCOL.to_string(),
        version: 1,
        conversation_owner: conversation_owner.to_hex(),
        payload: base64::engine::general_purpose::STANDARD.encode(payload),
    };
    Ok(serde_json::to_vec(&wrapper)?)
}

fn decode_local_sibling_payload(payload: &[u8]) -> Option<(PublicKey, Vec<u8>)> {
    use base64::Engine;
    let wrapper: LocalSiblingPayload = serde_json::from_slice(payload).ok()?;
    if wrapper.protocol != LOCAL_SIBLING_PROTOCOL || wrapper.version != 1 {
        return None;
    }
    let owner = PublicKey::parse(&wrapper.conversation_owner).ok()?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(wrapper.payload)
        .ok()?;
    Some((owner, payload))
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalSiblingPayload {
    protocol: String,
    version: u32,
    conversation_owner: String,
    payload: String,
}

fn ndr_owner(pubkey: PublicKey) -> NdrOwnerPubkey {
    NdrOwnerPubkey::from_bytes(pubkey.to_bytes())
}

fn ndr_device(pubkey: PublicKey) -> NdrDevicePubkey {
    NdrDevicePubkey::from_bytes(pubkey.to_bytes())
}

fn public_owner(pubkey: NdrOwnerPubkey) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_slice(&pubkey.to_bytes())?)
}

fn public_device(pubkey: NdrDevicePubkey) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_slice(&pubkey.to_bytes())?)
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
