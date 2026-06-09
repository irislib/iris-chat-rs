const PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
const PROTOCOL_ENGINE_STATE_VERSION: u32 = 1;
const LOCAL_SIBLING_PROTOCOL: &str = "ndr-local-sibling-copy";
const PENDING_RETRY_DELAY_SECS: u64 = 2;
const LOCAL_SIBLING_ROSTER_PROBE_TTL_SECS: u64 = 120;
const DELIVERED_GROUP_SENDER_KEY_ACK_LIMIT: usize = 512;
const ANSWERED_GROUP_SENDER_KEY_REPAIR_LIMIT: usize = 512;

fn default_true() -> bool {
    true
}

fn group_chat_id(group_id: &str) -> String {
    format!("group:{group_id}")
}

#[derive(Debug, Serialize, Deserialize)]
struct ProtocolEnginePersistedState {
    version: u32,
    session_manager: SessionManagerSnapshot,
    group_manager: GroupManagerSnapshot,
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
    delivered_group_sender_key_acks: Vec<ProtocolDeliveredGroupSenderKeyAck>,
    #[serde(default)]
    answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    #[serde(default)]
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    #[serde(default)]
    subscription_generation: u64,
    #[serde(default)]
    last_backfill_attempt_secs: u64,
}

#[derive(Clone, Debug)]
pub struct ProtocolPublish {
    pub event: Event,
    pub chat_id: String,
    pub inner_event_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ProtocolEffect {
    Publish(ProtocolPublish),
    FetchProtocolState {
        filters: Vec<Filter>,
        reason: &'static str,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolPendingOutbound {
    pub message_id: String,
    pub chat_id: String,
    recipient_owner_hex: String,
    #[serde(default = "default_true")]
    send_remote: bool,
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
pub enum ProtocolPendingReason {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolPendingInboundTestDebug {
    pub event_id: String,
    pub sender_message_pubkey_hex: Option<String>,
    pub claimed_owner_pubkey_hex: Option<String>,
    pub has_envelope: bool,
    pub metadata_verified: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_number: Option<u32>,
    #[serde(default)]
    required_revision: Option<u64>,
    created_at_secs: u64,
    last_requested_at_secs: u64,
    request_count: u32,
    next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolDeliveredGroupSenderKeyAck {
    group_id: String,
    sender_event_pubkey_hex: String,
    created_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtocolAnsweredGroupSenderKeyRepair {
    requester_owner_hex: String,
    group_id: String,
    sender_event_pubkey_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_number: Option<u32>,
    #[serde(default)]
    required_revision: Option<u64>,
    request_created_at_secs: u64,
    last_responded_at_secs: u64,
    response_count: u32,
    next_response_at_secs: u64,
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
pub struct ProtocolDirectSendResult {
    pub message_id: String,
    pub event_ids: Vec<String>,
    pub effects: Vec<ProtocolEffect>,
    pub queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolRetryResult {
    pub message_id: String,
    pub chat_id: String,
    pub event_ids: Vec<String>,
    pub effects: Vec<ProtocolEffect>,
    pub queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolGroupSendResult {
    pub snapshot: Option<GroupSnapshot>,
    pub message_id: Option<String>,
    pub event_ids: Vec<String>,
    pub effects: Vec<ProtocolEffect>,
    pub queued_targets: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolGroupIncomingResult {
    pub events: Vec<GroupIncomingEvent>,
    pub effects: Vec<ProtocolEffect>,
    pub queued_targets: Vec<String>,
    pub consumed: bool,
    pub pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolRetryBatch {
    pub direct_results: Vec<ProtocolRetryResult>,
    pub group_result: ProtocolGroupIncomingResult,
    pub direct_messages: Vec<ProtocolDecryptedMessage>,
    pub effects: Vec<ProtocolEffect>,
}

impl ProtocolRetryBatch {
    pub fn is_empty(&self) -> bool {
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
pub struct ProtocolAcceptInviteResult {
    pub owner_pubkey: PublicKey,
    pub inviter_device_pubkey: PublicKey,
    pub device_id: String,
    pub effects: Vec<ProtocolEffect>,
}

#[derive(Clone, Debug)]
pub struct ProtocolDecryptedMessage {
    pub sender: PublicKey,
    pub sender_device: Option<PublicKey>,
    pub conversation_owner: Option<PublicKey>,
    pub content: String,
    pub event_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolDeviceOwnerHint {
    pub owner: PublicKey,
    pub verified: bool,
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
pub struct ProtocolEngineDebugSnapshot {
    pub known_message_author_count: usize,
    #[serde(default)]
    pub known_message_author_pubkeys: Vec<String>,
    #[serde(default)]
    pub known_group_sender_key_author_count: usize,
    #[serde(default)]
    pub known_group_sender_key_author_pubkeys: Vec<String>,
    pub pending_outbound_count: usize,
    pub pending_inbound_count: usize,
    pub pending_group_fanout_count: usize,
    pub pending_group_pairwise_payload_count: usize,
    pub pending_group_sender_key_message_count: usize,
    #[serde(default)]
    pub pending_group_sender_key_retry_count: usize,
    #[serde(default)]
    pub pending_group_sender_key_unmapped_count: usize,
    pub pending_group_sender_key_repair_count: usize,
    pub pending_group_sender_key_repair_last_requested_at_secs: u64,
    #[serde(default)]
    pub pending_group_sender_key_repair_next_retry_at_secs: u64,
    #[serde(default)]
    pub pending_group_sender_key_repair_max_request_count: u32,
    pub pending_outbound_targets: Vec<String>,
    #[serde(default)]
    pub pending_outbound_details: Vec<ProtocolPendingOutboundDebug>,
    #[serde(default)]
    pub pending_group_fanout_targets: Vec<String>,
    pub subscription_generation: u64,
    pub last_backfill_attempt_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProtocolPendingOutboundDebug {
    pub message_id: String,
    pub chat_id: String,
    pub recipient_owner_hex: String,
    pub reason: String,
    pub probe_local_sibling_roster: bool,
    pub delivered_remote_device_hexes: Vec<String>,
    pub delivered_local_device_hexes: Vec<String>,
    pub remaining_remote_targets: Vec<String>,
    pub remaining_local_sibling_targets: Vec<String>,
    pub queued_targets: Vec<String>,
    pub next_retry_at_secs: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ProtocolMessageSessionDebugSnapshot {
    pub state: SessionState,
    pub tracked_sender_pubkeys: Vec<PublicKey>,
    pub has_receiving_capability: bool,
}

pub struct ProtocolEngine {
    owner_pubkey: PublicKey,
    local_owner: NdrOwnerPubkey,
    local_device: NdrDevicePubkey,
    storage: Arc<dyn StorageAdapter>,
    session_manager: SessionManager,
    group_manager: NostrGroupManager,
    pending_outbound: Vec<ProtocolPendingOutbound>,
    pending_inbound: Vec<ProtocolPendingInbound>,
    pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    delivered_group_sender_key_acks: Vec<ProtocolDeliveredGroupSenderKeyAck>,
    answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    known_message_author_cache: std::cell::RefCell<Option<KnownMessageAuthorCache>>,
    known_message_author_cache_build_count: std::cell::Cell<u64>,
    local_app_keys_observed: bool,
    subscription_generation: u64,
    last_backfill_attempt_secs: u64,
    /// While > 0, `persist()` only flips `batch_persist_dirty` instead of
    /// serializing+writing. AppCore wraps catch-up bursts and other
    /// multi-event entry points so an N-event burst issues one persist
    /// instead of N. The exclusive SQLite write under iOS DELETE-mode
    /// journaling can keep UI reads blocked on the connection mutex for
    /// hundreds of ms each — N of them stacked produced the multi-second
    /// foreground freeze.
    pub batch_depth: std::cell::Cell<u32>,
    pub batch_persist_dirty: std::cell::Cell<bool>,
}

#[derive(Clone)]
struct ProtocolEngineCheckpoint {
    session_manager: SessionManager,
    group_manager: NostrGroupManager,
    pending_outbound: Vec<ProtocolPendingOutbound>,
    pending_inbound: Vec<ProtocolPendingInbound>,
    pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pending_group_sender_key_messages:
        Vec<nostr_double_ratchet_nostr::nostr_codec::ParsedGroupSenderKeyMessageEvent>,
    pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    delivered_group_sender_key_acks: Vec<ProtocolDeliveredGroupSenderKeyAck>,
    answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    subscription_generation: u64,
    last_backfill_attempt_secs: u64,
}
