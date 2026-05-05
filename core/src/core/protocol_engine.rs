use super::*;

const PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
const LEGACY_RUNTIME_STATE_KEY: &str = "v2/runtime-state";
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
    #[serde(default)]
    pending_inbound: Vec<ProtocolPendingInbound>,
    #[serde(default)]
    pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    #[serde(default)]
    pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    #[serde(default)]
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    #[serde(default)]
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    #[serde(default)]
    subscription_generation: u64,
    #[serde(default)]
    last_backfill_attempt_secs: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyRuntimePersistedState {
    core: SessionManagerSnapshot,
    #[serde(default)]
    group_manager: Option<GroupManagerSnapshot>,
    #[serde(default)]
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    #[serde(default)]
    pending_group_pairwise_payloads: Vec<LegacyPendingGroupPairwisePayload>,
    #[serde(default)]
    pending_group_fanouts: Vec<LegacyPendingGroupFanout>,
    #[serde(default)]
    pending_pairwise_message_events: Vec<LegacyPendingPairwiseMessageEvent>,
    #[serde(default)]
    latest_app_keys_created_at: BTreeMap<String, u64>,
}

#[derive(Debug, Deserialize)]
struct LegacyPendingPairwiseMessageEvent {
    event: Event,
    created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyPendingGroupPairwisePayload {
    sender_owner: NdrOwnerPubkey,
    sender_device: Option<NdrDevicePubkey>,
    payload: Vec<u8>,
    created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyPendingGroupFanout {
    group_id: String,
    fanout: GroupPendingFanout,
    inner_event_id: Option<String>,
    created_at_ms: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) enum ProtocolEffect {
    Subscribe {
        subid: String,
        filters: Vec<Filter>,
    },
    Unsubscribe(String),
    FetchBackfill,
    PublishUnsigned(UnsignedEvent),
    PublishSigned(Event),
    PublishSignedForInnerEvent {
        event: Event,
        inner_event_id: Option<String>,
        target_owner_pubkey_hex: Option<String>,
        target_device_id: Option<String>,
    },
    EmitDecrypted {
        sender: PublicKey,
        sender_device: Option<PublicKey>,
        conversation_owner: Option<PublicKey>,
        content: String,
        event_id: Option<String>,
    },
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

impl ProtocolPendingOutbound {
    fn waits_for_remote_protocol_state(&self) -> bool {
        matches!(
            self.reason,
            ProtocolPendingReason::MissingRoster | ProtocolPendingReason::MissingDeviceInvite
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolPendingInbound {
    event: Event,
    created_at_secs: u64,
    next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolPendingGroupFanout {
    group_id: String,
    fanout: GroupPendingFanout,
    inner_event_id: Option<String>,
    created_at_secs: u64,
    next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolPendingGroupPairwisePayload {
    sender_owner: NdrOwnerPubkey,
    sender_device: Option<NdrDevicePubkey>,
    payload: Vec<u8>,
    created_at_secs: u64,
    next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolPendingDecryptedDelivery {
    sender: PublicKey,
    sender_device: Option<PublicKey>,
    conversation_owner: Option<PublicKey>,
    content: String,
    event_id: Option<String>,
    created_at_secs: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolDirectSendResult {
    pub(super) message_id: String,
    pub(super) event_ids: Vec<String>,
    pub(super) effects: Vec<ProtocolEffect>,
    pub(super) queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolRetryResult {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) event_ids: Vec<String>,
    pub(super) effects: Vec<ProtocolEffect>,
    pub(super) queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolGroupSendResult {
    pub(super) snapshot: Option<GroupSnapshot>,
    pub(super) message_id: Option<String>,
    pub(super) event_ids: Vec<String>,
    pub(super) effects: Vec<ProtocolEffect>,
    pub(super) queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolGroupIncomingResult {
    pub(super) events: Vec<GroupIncomingEvent>,
    pub(super) effects: Vec<ProtocolEffect>,
    pub(super) consumed: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolRetryBatch {
    pub(super) direct_results: Vec<ProtocolRetryResult>,
    pub(super) group_result: ProtocolGroupIncomingResult,
    pub(super) direct_messages: Vec<ProtocolDecryptedMessage>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct ProtocolAcceptInviteResult {
    pub(super) owner_pubkey: PublicKey,
    pub(super) inviter_device_pubkey: PublicKey,
    pub(super) device_id: String,
    pub(super) effects: Vec<ProtocolEffect>,
}

#[derive(Clone, Debug)]
pub(super) struct ProtocolDecryptedMessage {
    pub(super) sender: PublicKey,
    pub(super) sender_device: Option<PublicKey>,
    pub(super) conversation_owner: Option<PublicKey>,
    pub(super) content: String,
    pub(super) event_id: Option<String>,
}

impl From<ProtocolPendingDecryptedDelivery> for ProtocolDecryptedMessage {
    fn from(pending: ProtocolPendingDecryptedDelivery) -> Self {
        Self {
            sender: pending.sender,
            sender_device: pending.sender_device,
            conversation_owner: pending.conversation_owner,
            content: pending.content,
            event_id: pending.event_id,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct ProtocolEngineDebugSnapshot {
    pub(super) known_message_author_count: usize,
    pub(super) pending_outbound_count: usize,
    pub(super) pending_inbound_count: usize,
    pub(super) pending_group_fanout_count: usize,
    pub(super) pending_group_pairwise_payload_count: usize,
    pub(super) pending_group_sender_key_message_count: usize,
    pub(super) pending_outbound_targets: Vec<String>,
    pub(super) subscription_generation: u64,
    pub(super) last_backfill_attempt_secs: u64,
    pub(super) latest_app_keys_owner_count: usize,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct ProtocolMessageSessionDebugSnapshot {
    pub(super) state: SessionState,
    pub(super) tracked_sender_pubkeys: Vec<PublicKey>,
    pub(super) has_receiving_capability: bool,
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
    pending_inbound: Vec<ProtocolPendingInbound>,
    pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    subscription_generation: u64,
    last_backfill_attempt_secs: u64,
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
                        pending_inbound: state.pending_inbound,
                        pending_group_fanouts: state.pending_group_fanouts,
                        pending_group_pairwise_payloads: state.pending_group_pairwise_payloads,
                        pending_group_sender_key_messages: state.pending_group_sender_key_messages,
                        pending_decrypted_deliveries: state.pending_decrypted_deliveries,
                        subscription_generation: state.subscription_generation,
                        last_backfill_attempt_secs: state.last_backfill_attempt_secs,
                    }
                }
                _ => Self::from_legacy_or_seed(
                    storage,
                    owner_pubkey,
                    local_owner,
                    local_device,
                    device_secret,
                    seed_session_manager,
                    seed_group_manager,
                )?,
            },
            None => Self::from_legacy_or_seed(
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
        engine.persist()?;
        Ok(engine)
    }

    fn from_legacy_or_seed(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        local_owner: NdrOwnerPubkey,
        local_device: NdrDevicePubkey,
        device_secret: [u8; 32],
        seed_session_manager: SessionManagerSnapshot,
        seed_group_manager: GroupManagerSnapshot,
    ) -> anyhow::Result<Self> {
        if let Some(raw) = storage.get(LEGACY_RUNTIME_STATE_KEY)? {
            if let Ok(legacy) = serde_json::from_str::<LegacyRuntimePersistedState>(&raw) {
                if let Ok(session_manager) =
                    SessionManager::from_snapshot(legacy.core, device_secret)
                {
                    let group_manager = legacy
                        .group_manager
                        .map(NostrGroupManager::from_snapshot)
                        .transpose()?
                        .unwrap_or_else(|| NostrGroupManager::new(local_owner));
                    let pending_inbound = legacy
                        .pending_pairwise_message_events
                        .into_iter()
                        .map(|pending| {
                            let created_at_secs = pending.created_at_ms / 1_000;
                            ProtocolPendingInbound {
                                event: pending.event,
                                created_at_secs,
                                next_retry_at_secs: created_at_secs,
                            }
                        })
                        .collect();
                    let pending_group_pairwise_payloads = legacy
                        .pending_group_pairwise_payloads
                        .into_iter()
                        .map(|pending| {
                            let created_at_secs = pending.created_at_ms / 1_000;
                            ProtocolPendingGroupPairwisePayload {
                                sender_owner: pending.sender_owner,
                                sender_device: pending.sender_device,
                                payload: pending.payload,
                                created_at_secs,
                                next_retry_at_secs: created_at_secs,
                            }
                        })
                        .collect();
                    let pending_group_fanouts = legacy
                        .pending_group_fanouts
                        .into_iter()
                        .map(|pending| {
                            let created_at_secs = pending.created_at_ms / 1_000;
                            ProtocolPendingGroupFanout {
                                group_id: pending.group_id,
                                fanout: pending.fanout,
                                inner_event_id: pending.inner_event_id,
                                created_at_secs,
                                next_retry_at_secs: created_at_secs,
                            }
                        })
                        .collect();
                    return Ok(Self {
                        owner_pubkey,
                        local_owner,
                        local_device,
                        storage,
                        session_manager,
                        group_manager,
                        latest_app_keys_created_at: legacy.latest_app_keys_created_at,
                        pending_outbound: Vec::new(),
                        pending_inbound,
                        pending_group_fanouts,
                        pending_group_pairwise_payloads,
                        pending_group_sender_key_messages: legacy.pending_group_sender_key_messages,
                        pending_decrypted_deliveries: Vec::new(),
                        subscription_generation: 0,
                        last_backfill_attempt_secs: 0,
                    });
                }
            }
        }

        Self::from_seed(
            storage,
            owner_pubkey,
            local_owner,
            local_device,
            device_secret,
            seed_session_manager,
            seed_group_manager,
        )
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
            pending_decrypted_deliveries: Vec::new(),
            subscription_generation: 0,
            last_backfill_attempt_secs: 0,
        })
    }

    pub(super) fn debug_snapshot(&self) -> ProtocolEngineDebugSnapshot {
        ProtocolEngineDebugSnapshot {
            known_message_author_count: self.known_message_author_pubkeys().len(),
            pending_outbound_count: self.pending_outbound.len(),
            pending_inbound_count: self.pending_inbound.len(),
            pending_group_fanout_count: self.pending_group_fanouts.len(),
            pending_group_pairwise_payload_count: self.pending_group_pairwise_payloads.len(),
            pending_group_sender_key_message_count: self.pending_group_sender_key_messages.len(),
            pending_outbound_targets: self.queued_message_diagnostics(None),
            subscription_generation: self.subscription_generation,
            last_backfill_attempt_secs: self.last_backfill_attempt_secs,
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

    pub(super) fn message_session_debug_snapshots(
        &self,
        owner_pubkey: PublicKey,
    ) -> Vec<ProtocolMessageSessionDebugSnapshot> {
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .filter(|user| user.owner_pubkey == owner)
            .flat_map(|user| user.devices.into_iter())
            .flat_map(|device| {
                device
                    .active_session
                    .into_iter()
                    .chain(device.inactive_sessions)
                    .collect::<Vec<_>>()
            })
            .map(|state| {
                let mut tracked = HashSet::new();
                collect_expected_sender_pubkeys(&state, &mut tracked);
                let mut tracked_sender_pubkeys = tracked.into_iter().collect::<Vec<_>>();
                tracked_sender_pubkeys.sort_by_key(|pubkey| pubkey.to_hex());
                ProtocolMessageSessionDebugSnapshot {
                    has_receiving_capability: state.receiving_chain_key.is_some()
                        || state.their_current_nostr_public_key.is_some(),
                    state,
                    tracked_sender_pubkeys,
                }
            })
            .collect()
    }

    pub(super) fn active_session_count_for_owner(&self, owner_pubkey: PublicKey) -> usize {
        let owner = ndr_owner(owner_pubkey);
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .filter(|user| user.owner_pubkey == owner)
            .flat_map(|user| user.devices.into_iter())
            .filter(|device| device.active_session.is_some())
            .count()
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

    pub(super) fn has_queued_remote_message_work(&self, message_id: &str) -> bool {
        self.pending_outbound.iter().any(|pending| {
            pending.message_id == message_id
                && !self.pending_remote_target_hexes(pending).is_empty()
        })
    }

    pub(super) fn ingest_app_keys_snapshot(
        &mut self,
        owner_pubkey: PublicKey,
        app_keys: AppKeys,
        created_at: u64,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let session_checkpoint = self.session_manager.clone();
        let latest_checkpoint = self.latest_app_keys_created_at.clone();
        let owner_hex = owner_pubkey.to_hex();
        let latest = self
            .latest_app_keys_created_at
            .get(&owner_hex)
            .copied()
            .unwrap_or(0);
        if created_at < latest {
            return Ok(ProtocolRetryBatch::default());
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
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            self.latest_app_keys_created_at = latest_checkpoint;
            return Err(error);
        }
        self.retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
    }

    pub(super) fn observe_invite_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let session_checkpoint = self.session_manager.clone();
        let invite = parse_invite_event(event)?;
        let invite_owner = invite
            .inviter_owner_pubkey
            .unwrap_or_else(|| NdrOwnerPubkey::from_bytes(invite.inviter_device_pubkey.to_bytes()));
        if invite.inviter_device_pubkey != self.local_device {
            self.session_manager
                .observe_device_invite(invite_owner, invite)?;
        }
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            return Err(error);
        }
        self.retry_pending_protocol(NdrUnixSeconds(event.created_at.as_secs()))
    }

    pub(super) fn observe_invite_response_event(
        &mut self,
        event: &Event,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let Some(local_invite_recipient) = self
            .session_manager
            .snapshot()
            .local_invite
            .as_ref()
            .map(|invite| invite.inviter_ephemeral_public_key)
        else {
            return Ok(ProtocolRetryBatch::default());
        };
        let envelope = parse_invite_response_event(event)?;
        if envelope.recipient != local_invite_recipient {
            return Ok(ProtocolRetryBatch::default());
        }
        let session_checkpoint = self.session_manager.clone();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        let _ = self
            .session_manager
            .observe_invite_response(&mut ctx, &envelope)?;
        if let Err(error) = self.persist() {
            self.session_manager = session_checkpoint;
            return Err(error);
        }
        self.retry_pending_protocol(ctx.now)
    }

    pub(super) fn accept_invite(
        &mut self,
        invite: &Invite,
        owner_pubkey_hint: Option<PublicKey>,
    ) -> anyhow::Result<ProtocolAcceptInviteResult> {
        let invite_owner = owner_pubkey_hint
            .or_else(|| {
                invite
                    .inviter_owner_pubkey
                    .and_then(|owner| public_owner(owner).ok())
            })
            .unwrap_or_else(|| public_device(invite.inviter_device_pubkey).unwrap());
        let mut invite = invite.clone();
        invite.inviter_owner_pubkey = Some(ndr_owner(invite_owner));
        self.session_manager
            .observe_device_invite(ndr_owner(invite_owner), invite.clone())?;
        self.session_manager.observe_peer_roster(
            ndr_owner(invite_owner),
            DeviceRoster::new(
                NdrUnixSeconds(unix_now().get()),
                vec![AuthorizedDevice::new(
                    invite.inviter_device_pubkey,
                    invite.created_at,
                )],
            ),
        );
        let mut effects = Vec::new();
        if invite.purpose.as_deref() == Some("link") {
            let now = unix_now();
            let expires_at = now.get().saturating_add(60);
            let typing = pairwise_codec::typing_event(
                self.owner_pubkey,
                pairwise_codec::EncodeOptions::new(now.get(), current_unix_millis())
                    .with_expiration(expires_at),
            )?;
            let result =
                self.send_direct_unsigned_event(invite_owner, &invite_owner.to_hex(), typing, now)?;
            effects.extend(result.effects);
        } else {
            self.persist()?;
        }
        Ok(ProtocolAcceptInviteResult {
            owner_pubkey: invite_owner,
            inviter_device_pubkey: public_device(invite.inviter_device_pubkey)?,
            device_id: public_device(invite.inviter_device_pubkey)?.to_hex(),
            effects,
        })
    }

    pub(super) fn import_session_state(
        &mut self,
        peer_pubkey: PublicKey,
        device_id: Option<String>,
        state: SessionState,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let device_pubkey = device_id
            .as_deref()
            .and_then(|value| PublicKey::parse(value).ok())
            .map(ndr_device)
            .unwrap_or_else(|| ndr_device(peer_pubkey));
        self.session_manager.import_session_state(
            ndr_owner(peer_pubkey),
            device_pubkey,
            state,
            NdrUnixSeconds(now.get()),
        );
        self.persist()?;
        self.retry_pending_protocol(NdrUnixSeconds(now.get()))
    }

    pub(super) fn create_group(
        &mut self,
        name: String,
        member_owners: Vec<PublicKey>,
        now: UnixSeconds,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(now.get()), &mut rng);
        let result = self.group_manager.create_group_with_protocol(
            &mut self.session_manager,
            &mut ctx,
            name,
            member_owners.into_iter().map(ndr_owner).collect(),
            GroupProtocol::sender_key_v1(),
        )?;
        let mut output = self.protocol_group_send_from_prepared(&result.prepared, None)?;
        output.snapshot = Some(result.group);
        self.persist()?;
        Ok(output)
    }

    pub(super) fn update_group_name(
        &mut self,
        group_id: &str,
        name: String,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
        let prepared =
            self.group_manager
                .update_name(&mut self.session_manager, &mut ctx, group_id, name)?;
        let mut output = self.protocol_group_send_from_prepared(&prepared, None)?;
        output.snapshot = self.group_manager.group(group_id);
        self.persist()?;
        Ok(output)
    }

    pub(super) fn add_group_members(
        &mut self,
        group_id: &str,
        members: Vec<PublicKey>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
        let prepared = self.group_manager.add_members(
            &mut self.session_manager,
            &mut ctx,
            group_id,
            members.into_iter().map(ndr_owner).collect(),
        )?;
        let mut output = self.protocol_group_send_from_prepared(&prepared, None)?;
        output.snapshot = self.group_manager.group(group_id);
        self.persist()?;
        Ok(output)
    }

    pub(super) fn remove_group_member(
        &mut self,
        group_id: &str,
        member: PublicKey,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
        let prepared = self.group_manager.remove_members(
            &mut self.session_manager,
            &mut ctx,
            group_id,
            vec![ndr_owner(member)],
        )?;
        let mut output = self.protocol_group_send_from_prepared(&prepared, None)?;
        output.snapshot = self.group_manager.group(group_id);
        self.persist()?;
        Ok(output)
    }

    pub(super) fn set_group_admin(
        &mut self,
        group_id: &str,
        member: PublicKey,
        is_admin: bool,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
        let prepared = if is_admin {
            self.group_manager.add_admins(
                &mut self.session_manager,
                &mut ctx,
                group_id,
                vec![ndr_owner(member)],
            )?
        } else {
            self.group_manager.remove_admins(
                &mut self.session_manager,
                &mut ctx,
                group_id,
                vec![ndr_owner(member)],
            )?
        };
        let mut output = self.protocol_group_send_from_prepared(&prepared, None)?;
        output.snapshot = self.group_manager.group(group_id);
        self.persist()?;
        Ok(output)
    }

    pub(super) fn send_group_payload(
        &mut self,
        group_id: &str,
        payload: Vec<u8>,
        inner_event_id: Option<String>,
    ) -> anyhow::Result<ProtocolGroupSendResult> {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(unix_now().get()), &mut rng);
        let prepared = self.group_manager.send_message(
            &mut self.session_manager,
            &mut ctx,
            group_id,
            payload,
        )?;
        let message_id = inner_event_id.clone();
        let mut output = self.protocol_group_send_from_prepared(&prepared, inner_event_id)?;
        output.snapshot = self.group_manager.group(group_id);
        output.message_id = message_id;
        self.persist()?;
        Ok(output)
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
        effects.extend(protocol_effects_from_prepared(
            &remote,
            Some(message_id.clone()),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_prepared(
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
        effects.extend(protocol_effects_from_prepared(
            &remote,
            inner_event_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_prepared(
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
            self.queue_pending_inbound_direct_event(event.clone(), event.created_at.as_secs())?;
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
        self.record_pending_decrypted_delivery(decrypted.clone(), event.created_at.as_secs());
        self.persist()?;
        Ok(Some(decrypted))
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
        if result.events.is_empty() {
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
        let sender_owner = ndr_owner(from_owner_pubkey);
        let sender_device = from_sender_device_pubkey.map(ndr_device);
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
                    consumed: true,
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
                    Ok(ProtocolGroupIncomingResult {
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
            let remote_targets = self
                .remaining_remote_targets(recipient_owner, &pending.delivered_remote_device_hexes);
            let local_targets =
                self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes);

            if remote_targets.is_empty() && local_targets.is_empty() {
                let queued_targets = self.pending_target_hexes(&pending);
                if pending.waits_for_remote_protocol_state() && !queued_targets.is_empty() {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    still_pending.push(pending.clone());
                    results.push(ProtocolRetryResult {
                        message_id: pending.message_id.clone(),
                        chat_id: pending.chat_id.clone(),
                        event_ids: Vec::new(),
                        effects: Vec::new(),
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
            match self.decrypt_direct_message_event(&pending.event)? {
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

    fn decrypt_direct_message_event(
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

    fn retry_pending_group_inputs(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<ProtocolGroupIncomingResult> {
        let mut result = ProtocolGroupIncomingResult::default();
        result.consumed = false;

        let pairwise = std::mem::take(&mut self.pending_group_pairwise_payloads);
        let mut still_pairwise = Vec::new();
        for mut pending in pairwise {
            if pending.next_retry_at_secs > now.get() {
                still_pairwise.push(pending);
                continue;
            }
            let outcome = match pending.sender_device {
                Some(device_pubkey) => self.group_manager.handle_pairwise_payload(
                    pending.sender_owner,
                    device_pubkey,
                    &pending.payload,
                ),
                None => self
                    .group_manager
                    .handle_incoming(pending.sender_owner, &pending.payload),
            };
            match outcome {
                Ok(Some(event)) => result.events.push(event),
                Ok(None) => {}
                Err(_) => {
                    pending.next_retry_at_secs = now.get().saturating_add(PENDING_RETRY_DELAY_SECS);
                    still_pairwise.push(pending);
                }
            }
        }
        self.pending_group_pairwise_payloads = still_pairwise;

        let sender_keys = std::mem::take(&mut self.pending_group_sender_key_messages);
        let mut still_sender_keys = Vec::new();
        for parsed in sender_keys {
            let Some(message) = self.group_sender_key_message_from_parsed(&parsed) else {
                still_sender_keys.push(parsed);
                continue;
            };
            let outcome = self.handle_group_sender_key_message(message)?;
            if outcome.events.is_empty() {
                still_sender_keys.push(parsed);
            }
            result.events.extend(outcome.events);
            result.effects.extend(outcome.effects);
        }
        self.pending_group_sender_key_messages = still_sender_keys;
        if !result.events.is_empty() || !result.effects.is_empty() {
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
            &prepared.remote,
            inner_event_id.clone(),
            &mut event_ids,
        )?);
        effects.extend(protocol_effects_from_group_prepared_publish(
            &prepared.local_sibling,
            inner_event_id,
            &mut event_ids,
        )?);
        let mut queued_targets = self.queued_group_targets();
        queued_targets.sort();
        queued_targets.dedup();
        Ok(ProtocolGroupSendResult {
            event_ids,
            effects,
            queued_targets,
            ..Default::default()
        })
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
        targets.sort();
        targets.dedup();
        targets
    }

    fn queue_pending_inbound_direct_event(
        &mut self,
        event: Event,
        now_secs: u64,
    ) -> anyhow::Result<()> {
        let event_id = event.id.to_string();
        if !self
            .pending_inbound
            .iter()
            .any(|pending| pending.event.id.to_string() == event_id)
        {
            self.pending_inbound.push(ProtocolPendingInbound {
                event,
                created_at_secs: now_secs,
                next_retry_at_secs: now_secs.saturating_add(PENDING_RETRY_DELAY_SECS),
            });
            self.persist()?;
        }
        Ok(())
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
        if !self.pending_group_pairwise_payloads.contains(&pending) {
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
        let result = self
            .group_manager
            .handle_sender_key_message(message.clone())?;
        match result {
            GroupSenderKeyHandleResult::Event(event) => {
                self.persist()?;
                Ok(ProtocolGroupIncomingResult {
                    events: vec![event],
                    consumed: true,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::PendingDistribution { .. }
            | GroupSenderKeyHandleResult::PendingRevision { .. } => {
                Ok(ProtocolGroupIncomingResult {
                    consumed: true,
                    ..Default::default()
                })
            }
            GroupSenderKeyHandleResult::Ignored => Ok(ProtocolGroupIncomingResult::default()),
        }
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

    fn has_roster_for_owner(&self, owner: NdrOwnerPubkey) -> bool {
        self.session_manager
            .snapshot()
            .users
            .into_iter()
            .find(|user| user.owner_pubkey == owner)
            .and_then(|user| user.roster)
            .is_some_and(|roster| !roster.devices().is_empty())
    }

    fn pending_target_hexes(&self, pending: &ProtocolPendingOutbound) -> Vec<String> {
        let mut targets = self.pending_remote_target_hexes(pending);
        for target in self.remaining_local_sibling_targets(&pending.delivered_local_device_hexes) {
            targets.push(target.to_hex());
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

fn protocol_effects_from_prepared(
    prepared: &PreparedSend,
    inner_event_id: Option<String>,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut effects = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        effects.push(ProtocolEffect::PublishSigned(event));
    }
    for delivery in &prepared.deliveries {
        let event = message_event(&delivery.envelope)?;
        event_ids.push(event.id.to_string());
        effects.push(ProtocolEffect::PublishSignedForInnerEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: Some(public_owner(delivery.owner_pubkey)?.to_hex()),
            target_device_id: Some(public_device(delivery.device_pubkey)?.to_hex()),
        });
    }
    Ok(effects)
}

fn protocol_effects_from_group_prepared_publish(
    prepared: &GroupPreparedPublish,
    inner_event_id: Option<String>,
    event_ids: &mut Vec<String>,
) -> anyhow::Result<Vec<ProtocolEffect>> {
    let mut effects = Vec::new();
    for response in &prepared.invite_responses {
        let event = invite_response_event(response)?;
        effects.push(ProtocolEffect::PublishSigned(event));
    }
    for delivery in &prepared.deliveries {
        let event = message_event(&delivery.envelope)?;
        event_ids.push(event.id.to_string());
        effects.push(ProtocolEffect::PublishSignedForInnerEvent {
            event,
            inner_event_id: inner_event_id.clone(),
            target_owner_pubkey_hex: Some(public_owner(delivery.owner_pubkey)?.to_hex()),
            target_device_id: Some(public_device(delivery.device_pubkey)?.to_hex()),
        });
    }
    for sender_key_message in &prepared.sender_key_messages {
        let event = group_sender_key_message_event(sender_key_message)?;
        event_ids.push(event.id.to_string());
        effects.push(ProtocolEffect::PublishSigned(event));
    }
    Ok(effects)
}

fn group_publish_from_prepared_send(
    prepared: PreparedSend,
    fanout: GroupPendingFanout,
) -> GroupPreparedPublish {
    let pending_fanouts = if prepared.relay_gaps.is_empty() {
        Vec::new()
    } else {
        vec![fanout]
    };
    GroupPreparedPublish {
        deliveries: prepared.deliveries,
        invite_responses: prepared.invite_responses,
        sender_key_messages: Vec::new(),
        relay_gaps: prepared.relay_gaps,
        pending_fanouts,
    }
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
