const PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
const PROTOCOL_ENGINE_STATE_VERSION: u32 = 1;
const LOCAL_SIBLING_PROTOCOL: &str = "ndr-local-sibling-copy";
const PENDING_RETRY_DELAY_SECS: u64 = 2;
const SENDER_KEY_REPAIR_RETRY_DELAY_SECS: u64 = 30;
const LOCAL_SIBLING_ROSTER_PROBE_TTL_SECS: u64 = 120;

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
    pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    #[serde(default)]
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    #[serde(default)]
    subscription_generation: u64,
    #[serde(default)]
    last_backfill_attempt_secs: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct ProtocolPublishEvent {
    pub(super) event: Event,
    pub(super) inner_event_id: Option<String>,
    pub(super) target_owner_pubkey_hex: Option<String>,
    pub(super) target_device_id: Option<String>,
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
    PublishStagedFirstContact {
        bootstrap: Vec<ProtocolPublishEvent>,
        payload: Vec<ProtocolPublishEvent>,
    },
    FetchProtocolState {
        filters: Vec<Filter>,
        reason: &'static str,
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
    #[serde(default)]
    probe_local_sibling_roster: bool,
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
    #[serde(default)]
    event_id: String,
    #[serde(default)]
    envelope: Option<MessageEnvelope>,
    #[serde(default)]
    sender_message_pubkey_hex: Option<String>,
    #[serde(default)]
    resolved_owner_pubkey_hex: Option<String>,
    #[serde(default)]
    claimed_owner_pubkey_hex: Option<String>,
    #[serde(default)]
    metadata_verified: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProtocolPendingInboundTestDebug {
    pub(super) event_id: String,
    pub(super) sender_message_pubkey_hex: Option<String>,
    pub(super) claimed_owner_pubkey_hex: Option<String>,
    pub(super) has_envelope: bool,
    pub(super) metadata_verified: bool,
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
struct ProtocolPendingGroupSenderKeyRepair {
    group_id: String,
    sender_event_pubkey_hex: String,
    key_id: u32,
    message_number: u32,
    #[serde(default)]
    required_revision: Option<u64>,
    created_at_secs: u64,
    last_requested_at_secs: u64,
    request_count: u32,
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

#[derive(Clone, Debug)]
struct KnownMessageAuthorCache {
    pubkeys: Vec<PublicKey>,
    pubkey_set: HashSet<PublicKey>,
    hexes: HashSet<String>,
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
    pub(super) queued_targets: Vec<String>,
    pub(super) consumed: bool,
    pub(super) pending: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolRetryBatch {
    pub(super) direct_results: Vec<ProtocolRetryResult>,
    pub(super) group_result: ProtocolGroupIncomingResult,
    pub(super) direct_messages: Vec<ProtocolDecryptedMessage>,
    pub(super) effects: Vec<ProtocolEffect>,
}

impl ProtocolRetryBatch {
    pub(super) fn is_empty(&self) -> bool {
        self.direct_results.is_empty()
            && self.group_result.events.is_empty()
            && self.group_result.effects.is_empty()
            && self.group_result.queued_targets.is_empty()
            && self.direct_messages.is_empty()
            && self.effects.is_empty()
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ProtocolDeviceOwnerHint {
    pub(super) owner: PublicKey,
    pub(super) verified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtocolSenderOwnerResolution {
    Verified {
        owner: NdrOwnerPubkey,
    },
    PendingOwnerClaim {
        storage_owner: NdrOwnerPubkey,
        claimed_owner: NdrOwnerPubkey,
        sender_device: NdrDevicePubkey,
    },
    ProvisionalDeviceOwner {
        owner: NdrOwnerPubkey,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProtocolSenderDeviceRecord {
    storage_owner: NdrOwnerPubkey,
    device_pubkey: NdrDevicePubkey,
    claimed_owner_pubkey: Option<NdrOwnerPubkey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProtocolPendingInboundMetadata {
    event_id: String,
    envelope: Option<MessageEnvelope>,
    sender_message_pubkey_hex: Option<String>,
    resolved_owner_pubkey_hex: Option<String>,
    claimed_owner_pubkey_hex: Option<String>,
    metadata_verified: bool,
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
    pub(super) pending_group_sender_key_repair_count: usize,
    pub(super) pending_group_sender_key_repair_last_requested_at_secs: u64,
    pub(super) pending_outbound_targets: Vec<String>,
    #[serde(default)]
    pub(super) pending_outbound_details: Vec<ProtocolPendingOutboundDebug>,
    #[serde(default)]
    pub(super) pending_group_fanout_targets: Vec<String>,
    pub(super) subscription_generation: u64,
    pub(super) last_backfill_attempt_secs: u64,
    pub(super) latest_app_keys_owner_count: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct ProtocolPendingOutboundDebug {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) recipient_owner_hex: String,
    pub(super) reason: String,
    pub(super) probe_local_sibling_roster: bool,
    pub(super) delivered_remote_device_hexes: Vec<String>,
    pub(super) delivered_local_device_hexes: Vec<String>,
    pub(super) remaining_remote_targets: Vec<String>,
    pub(super) remaining_local_sibling_targets: Vec<String>,
    pub(super) queued_targets: Vec<String>,
    pub(super) next_retry_at_secs: u64,
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
    pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    known_message_author_cache: std::cell::RefCell<Option<KnownMessageAuthorCache>>,
    #[cfg(test)]
    known_message_author_cache_build_count: std::cell::Cell<u64>,
    subscription_generation: u64,
    last_backfill_attempt_secs: u64,
}

#[derive(Clone)]
struct ProtocolEngineCheckpoint {
    session_manager: SessionManager,
    group_manager: NostrGroupManager,
    latest_app_keys_created_at: BTreeMap<String, u64>,
    pending_outbound: Vec<ProtocolPendingOutbound>,
    pending_inbound: Vec<ProtocolPendingInbound>,
    pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    subscription_generation: u64,
    last_backfill_attempt_secs: u64,
}
