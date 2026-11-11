use super::*;

pub(super) const PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";
pub(super) const PROTOCOL_ENGINE_STATE_VERSION: u32 = 1;
pub(super) const PROTOCOL_APP_KEYS_PROVENANCE_VERSION: u8 = 1;
pub(super) const PROTOCOL_GROUP_INGRESS_PROVENANCE_VERSION: u8 = 1;
pub(super) const LEGACY_GROUP_INGRESS_QUARANTINE_KEY: &str = "appcore/quarantine/group-ingress-v0";
pub(super) const LOCAL_SIBLING_PROTOCOL: &str = "ndr-local-sibling-copy";
pub(super) const PENDING_RETRY_DELAY_SECS: u64 = 2;
pub(super) const MAX_UNTRACKED_PENDING_INBOUND: usize = 256;
pub(super) const PENDING_GROUP_FANOUT_RETRY_BATCH_SIZE: usize = 64;
pub(super) const PROCESSED_GROUP_SENDER_KEY_LIMIT: usize = 16_384;
pub(super) const ANSWERED_GROUP_SENDER_KEY_REPAIR_LIMIT: usize = 512;

pub(super) fn group_chat_id(group_id: &str) -> String {
    format!("group:{group_id}")
}

/// Temporary capacity refusal. Callers must not mark this event as delivered
/// or permanently seen: a later handshake can make its author admissible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingInboundAdmissionError;

impl std::fmt::Display for PendingInboundAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Pending first-contact message queue is full; retry after a session is available",
        )
    }
}

impl std::error::Error for PendingInboundAdmissionError {}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ProtocolEnginePersistedState {
    pub(super) version: u32,
    // record_protocol_checkpoint checks this prefix so it can assemble cached
    // queue records separately from the smaller session and retry state.
    #[serde(default)]
    pub(super) pending_group_sender_key_messages: PendingGroupMessages,
    #[serde(default)]
    pub(super) pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    #[serde(default)]
    pub(super) pending_inbound: Vec<ProtocolPendingInbound>,
    pub(super) session_manager: SessionManagerSnapshot,
    pub(super) group_manager: GroupManagerSnapshot,
    /// Owners whose current roster was installed through trusted AppKeys ingestion.
    ///
    /// This defaults to empty so v1 states written before roster provenance was
    /// persisted can be loaded and quarantined instead of treating synthetic
    /// invite-owner rosters as authoritative.
    #[serde(default)]
    pub(super) verified_app_keys_owners: BTreeSet<NdrOwnerPubkey>,
    /// Nonzero only for states written after roster-provenance quarantine was
    /// introduced. Legacy booleans are ignored because vulnerable releases
    /// could synthesize them from an owner hint.
    #[serde(default)]
    pub(super) app_keys_provenance_version: u8,
    /// Earlier releases admitted raw network ciphertext to group recovery.
    /// Only new states have exclusively authenticated pairwise group ingress.
    #[serde(default)]
    pub(super) group_ingress_provenance_version: u8,
    /// Exact signed evidence used for invite owner/device authorization.
    /// Reconstructed device lists and legacy booleans are never sufficient.
    #[serde(default)]
    pub(super) invite_owner_app_keys_evidence: BTreeMap<NdrOwnerPubkey, ProtocolAppKeysEvidence>,
    #[serde(default)]
    pub(super) processed_private_invite_response_ids: Vec<String>,
    #[serde(default)]
    pub(super) group_invite_uses: BTreeMap<String, BTreeSet<NdrOwnerPubkey>>,
    #[serde(default)]
    pub(super) ready_group_publications: BTreeMap<String, ProtocolReadyGroupPublish>,
    #[serde(default)]
    pub(super) pending_local_sibling_sends: Vec<ProtocolPendingLocalSiblingSend>,
    #[serde(default)]
    pub(super) pending_remote_sends: Vec<ProtocolPendingRemoteSend>,
    #[serde(default)]
    pub(super) pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    #[serde(default)]
    pub(super) pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    #[serde(default)]
    pub(super) processed_group_sender_key_messages: ProtocolGroupReplayCache,
    #[serde(default)]
    pub(super) answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    #[serde(default)]
    pub(super) pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    #[serde(default)]
    pub(super) group_roster_fact_histories: BTreeMap<String, GroupRosterFactHistory>,
    #[serde(default)]
    pub(super) subscription_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ProtocolAppKeysEvidence {
    Verified(Box<Event>),
    Ambiguous { created_at_secs: u64 },
}

/// What a publish effect carries, recorded by the engine that produced it.
/// It travels beside the event (effect, queue, outbox row) and never on the
/// wire: message events name no recipient, so a pairwise envelope and shared
/// group ciphertext cannot be told apart, or routed, from their tags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolCarrier {
    /// Invite responses, and anything queued before carriers were recorded.
    #[default]
    Unmarked,
    /// A recipient-specific ratchet envelope for exactly this device.
    Pairwise { device: PublicKey },
    /// Shared group sender-key ciphertext: a local intermediate that only ever
    /// travels inside pairwise envelopes. Never hand it to a network carrier.
    SharedGroup,
}

impl ProtocolCarrier {
    pub fn pairwise_device(self) -> Option<PublicKey> {
        match self {
            Self::Pairwise { device } => Some(device),
            Self::Unmarked | Self::SharedGroup => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolPublish {
    /// Original plaintext timestamp, preserved when sealing happens on a later retry.
    pub authored_at_secs: Option<u64>,
    pub event: Event,
    pub chat_id: String,
    pub inner_event_id: Option<String>,
    #[serde(default)]
    pub carrier: ProtocolCarrier,
}

impl ProtocolPublish {
    /// Whether any network carrier may take this event. Message events need
    /// the engine's own pairwise marker and must not name a recipient, which
    /// also retires envelopes sealed by versions that still tagged one.
    pub fn is_publishable(&self) -> bool {
        self.event.kind.as_u16() as u32 != MESSAGE_EVENT_KIND
            || (self.carrier.pairwise_device().is_some()
                && self.event.tags.public_keys().next().is_none())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProtocolReadyGroupPublish {
    pub(super) publish: ProtocolPublish,
    #[serde(default)]
    pub(super) wake_recipient: Option<NdrOwnerPubkey>,
}

#[derive(Clone, Debug)]
pub enum ProtocolEffect {
    Publish(ProtocolPublish),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProtocolPendingInbound {
    #[serde(flatten)]
    pub(super) body: CheckpointInbound,
    pub(super) created_at_secs: u64,
    pub(super) next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProtocolPendingInboundBody {
    pub(super) event: Event,
    #[serde(default)]
    pub(super) event_id: String,
    #[serde(default)]
    pub(super) envelope: Option<MessageEnvelope>,
    #[serde(default)]
    pub(super) sender_message_pubkey_hex: Option<String>,
    #[serde(default)]
    pub(super) resolved_owner_pubkey_hex: Option<String>,
    #[serde(default)]
    pub(super) claimed_owner_pubkey_hex: Option<String>,
    #[serde(default)]
    pub(super) metadata_verified: bool,
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
pub(super) struct ProtocolPendingGroupFanout {
    pub(super) group_id: String,
    pub(super) fanout: CheckpointGroupFanout,
    // None until the roster is known (also for legacy persisted queues).
    #[serde(default)]
    pub(super) remaining_devices: Option<BTreeSet<NdrDevicePubkey>>,
    pub(super) inner_event_id: Option<String>,
    pub(super) created_at_secs: u64,
    pub(super) next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ProtocolPendingGroupPairwisePayload {
    pub(super) sender_owner: NdrOwnerPubkey,
    pub(super) sender_device: Option<NdrDevicePubkey>,
    pub(super) payload: Vec<u8>,
    pub(super) created_at_secs: u64,
    pub(super) next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ProtocolPendingGroupSenderKeyRepair {
    pub(super) group_id: String,
    pub(super) sender_event_pubkey_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) key_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message_number: Option<u32>,
    #[serde(default)]
    pub(super) required_revision: Option<u64>,
    pub(super) created_at_secs: u64,
    pub(super) last_requested_at_secs: u64,
    pub(super) request_count: u32,
    pub(super) next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ProtocolAnsweredGroupSenderKeyRepair {
    pub(super) requester_owner_hex: String,
    pub(super) group_id: String,
    pub(super) sender_event_pubkey_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) key_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message_number: Option<u32>,
    #[serde(default)]
    pub(super) required_revision: Option<u64>,
    pub(super) request_created_at_secs: u64,
    pub(super) last_responded_at_secs: u64,
    pub(super) response_count: u32,
    pub(super) next_response_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ProtocolPendingDecryptedDelivery {
    // In-memory receive completion only; rejected content is never retained.
    #[serde(skip)]
    pub(super) discarded: bool,
    pub(super) sender: PublicKey,
    pub(super) sender_device: Option<PublicKey>,
    pub(super) conversation_owner: Option<PublicKey>,
    pub(super) content: String,
    pub(super) event_id: Option<String>,
    pub(super) created_at_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct GroupRosterFactHistory {
    pub(super) events: Vec<Event>,
}

#[derive(Clone, Debug)]
pub(super) struct KnownMessageAuthorCache {
    pub(super) pubkeys: Vec<PublicKey>,
    pub(super) pubkey_set: HashSet<PublicKey>,
    pub(super) usable_session_owners: HashSet<PublicKey>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolDirectSendResult {
    pub message_id: String,
    pub event_ids: Vec<String>,
    pub effects: Vec<ProtocolEffect>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolGroupSendResult {
    pub snapshot: Option<GroupSnapshot>,
    pub message_id: Option<String>,
    pub event_ids: Vec<String>,
    pub effects: Vec<ProtocolEffect>,
    pub has_pending_deliveries: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolGroupIncomingResult {
    /// Prefix of `events` decrypted from this exact outer event. Later entries
    /// can be recovered queued messages and must not inherit its source ID.
    pub source_event_count: usize,
    pub events: Vec<GroupIncomingEvent>,
    pub effects: Vec<ProtocolEffect>,
    pub consumed: bool,
    pub pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProtocolRetryBatch {
    pub group_result: ProtocolGroupIncomingResult,
    pub direct_messages: Vec<ProtocolDecryptedMessage>,
    pub effects: Vec<ProtocolEffect>,
}

impl ProtocolRetryBatch {
    pub fn is_empty(&self) -> bool {
        self.group_result.events.is_empty()
            && self.group_result.effects.is_empty()
            && self.direct_messages.is_empty()
            && self.effects.is_empty()
    }
}

/// Whether authenticated discovery changed protocol state. An empty retry
/// batch alone cannot establish this: authorization can change without sending
/// or decrypting a message.
#[derive(Clone, Debug, Default)]
pub struct ProtocolMetadataIngestionResult {
    pub changed: bool,
    pub retry_batch: ProtocolRetryBatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectSendReadiness {
    Ready,
    MissingLocalAppKeys,
    MissingPeerAppKeys,
    MissingPeerInviteOrSession,
    MissingLocalSiblingInviteOrSession,
}

impl DirectSendReadiness {
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Clone, Debug)]
pub struct ProtocolDeviceRosterIngestionResult {
    pub owner_pubkey: PublicKey,
    pub app_keys: AppKeys,
    pub created_at: u64,
    pub retry_batch: ProtocolRetryBatch,
}

#[derive(Clone, Debug)]
pub struct ProtocolGroupRosterIngestionResult {
    pub snapshot: Option<GroupSnapshot>,
    pub retry_batch: ProtocolRetryBatch,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ProtocolAcceptInviteResult {
    pub owner_pubkey: PublicKey,
    pub inviter_device_pubkey: PublicKey,
    pub device_id: String,
    pub effects: Vec<ProtocolEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolAcceptInviteBlock {
    MissingOwnerRoster {
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
    },
    UnauthorizedDevice {
        owner_pubkey: PublicKey,
        device_pubkey: PublicKey,
    },
}

#[derive(Clone, Debug)]
pub enum ProtocolAcceptInviteOutcome {
    Accepted(ProtocolAcceptInviteResult),
    Blocked(ProtocolAcceptInviteBlock),
}

#[derive(Clone, Debug)]
pub enum ProtocolInviteSessionImportOutcome {
    Imported(ProtocolRetryBatch),
    AlreadyImported,
    Blocked(ProtocolAcceptInviteBlock),
}

#[derive(Clone, Debug)]
pub struct ProtocolDecryptedMessage {
    /// Original signed outer-event time, retained across durable delivery retries.
    pub created_at_secs: u64,
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
pub(super) enum ProtocolSenderOwnerResolution {
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
pub(super) struct ProtocolSenderDeviceRecord {
    pub(super) storage_owner: NdrOwnerPubkey,
    pub(super) device_pubkey: NdrDevicePubkey,
    pub(super) claimed_owner_pubkey: Option<NdrOwnerPubkey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProtocolPendingInboundMetadata {
    pub(super) event_id: String,
    pub(super) envelope: Option<MessageEnvelope>,
    pub(super) sender_message_pubkey_hex: Option<String>,
    pub(super) resolved_owner_pubkey_hex: Option<String>,
    pub(super) claimed_owner_pubkey_hex: Option<String>,
    pub(super) metadata_verified: bool,
}

impl From<ProtocolPendingDecryptedDelivery> for ProtocolDecryptedMessage {
    fn from(pending: ProtocolPendingDecryptedDelivery) -> Self {
        Self {
            created_at_secs: pending.created_at_secs,
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
    #[serde(default)]
    pub pending_group_fanout_targets: Vec<String>,
    pub subscription_generation: u64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ProtocolMessageSessionDebugSnapshot {
    pub state: SessionState,
    pub tracked_sender_pubkeys: Vec<PublicKey>,
    pub has_receiving_capability: bool,
}

#[derive(Clone, Copy)]
pub(super) struct PendingInboundRetryDeadline {
    pub(super) queue_len: usize,
    pub(super) next_retry_at_secs: Option<u64>,
}

#[derive(Clone, Copy)]
pub(super) struct PendingGroupFanoutRetryDeadline {
    pub(super) queue_len: usize,
    pub(super) next_retry_at_secs: Option<u64>,
}

pub struct ProtocolEngine {
    pub(super) owner_pubkey: PublicKey,
    pub(super) local_owner: NdrOwnerPubkey,
    pub(super) local_device: NdrDevicePubkey,
    pub(super) local_device_secret: [u8; 32],
    pub(super) storage: Arc<dyn StorageAdapter>,
    pub(super) session_manager: SessionManager,
    pub(super) group_manager: GroupEventManager,
    pub(super) pending_inbound: Vec<ProtocolPendingInbound>,
    /// Runtime-only scheduling hint. Unknown after restoration or queue/deadline mutation;
    /// never copied into a durable or rollback checkpoint.
    pub(super) pending_inbound_retry_deadline: std::cell::Cell<Option<PendingInboundRetryDeadline>>,
    pub(super) failed_inbound_receives: BTreeMap<nostr::EventId, FailedInboundReceive>,
    pub(super) pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    /// Runtime-only hint, populated after finding no due fanouts. Queue changes
    /// and restoration discard it; durable and rollback checkpoints exclude it.
    pub(super) pending_group_fanout_retry_deadline:
        std::cell::Cell<Option<PendingGroupFanoutRetryDeadline>>,
    pub(super) ready_group_publications: BTreeMap<String, ProtocolReadyGroupPublish>,
    pub(super) pending_local_sibling_sends: Vec<ProtocolPendingLocalSiblingSend>,
    pub(super) held_direct_chats: BTreeSet<String>,
    pub(super) pending_remote_sends: Vec<ProtocolPendingRemoteSend>,
    pub(super) pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pub(super) pending_group_sender_key_messages: PendingGroupMessages,
    pub(super) pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    pub(super) processed_group_sender_key_messages: ProtocolGroupReplayCache,
    pub(super) group_sender_key_retry: std::cell::RefCell<ProtocolGroupSenderKeyRetry>,
    pub(super) group_retry_pacing: std::cell::RefCell<GroupRetryPacing>,
    pub(super) answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    pub(super) pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    pub(super) group_roster_fact_histories: BTreeMap<String, GroupRosterFactHistory>,
    pub(super) handshake_proof_cache: std::cell::RefCell<HandshakeProofCache>,
    pub(super) known_message_author_cache: std::cell::RefCell<Option<KnownMessageAuthorCache>>,
    pub(super) known_message_author_cache_build_count: std::cell::Cell<u64>,
    pub(super) verified_app_keys_owners: BTreeSet<NdrOwnerPubkey>,
    pub(super) invite_owner_app_keys_evidence: BTreeMap<NdrOwnerPubkey, ProtocolAppKeysEvidence>,
    pub(super) processed_private_invite_response_ids: Vec<String>,
    pub(super) group_invite_uses: BTreeMap<String, BTreeSet<NdrOwnerPubkey>>,
    pub(super) local_app_keys_observed: bool,
    /// Ephemeral proof that this runtime holds the owner's secret key. This
    /// permits outbound recovery while a restored AppKeys roster is still
    /// being fetched, without persisting or publishing a provisional roster.
    pub(super) local_owner_authenticated: bool,
    pub(super) subscription_generation: u64,
    /// While > 0, `persist()` only flips `batch_persist_dirty` instead of
    /// serializing+writing. AppCore wraps catch-up bursts and other
    /// multi-event entry points so an N-event burst issues one persist
    /// instead of N. The exclusive SQLite write under iOS DELETE-mode
    /// journaling can keep UI reads blocked on the connection mutex for
    /// hundreds of ms each — N of them stacked produced the multi-second
    /// foreground freeze.
    pub batch_depth: std::cell::Cell<u32>,
    pub batch_persist_dirty: std::cell::Cell<bool>,
    pub(super) checkpoint_layout: std::cell::RefCell<ProtocolCheckpointLayout>,
}

#[derive(Clone)]
pub(super) struct ProtocolEngineCheckpoint {
    pub(super) session_manager: SessionManager,
    pub(super) group_manager: GroupEventManager,
    pub(super) pending_inbound: Vec<ProtocolPendingInbound>,
    pub(super) pending_group_fanouts: Vec<ProtocolPendingGroupFanout>,
    pub(super) ready_group_publications: BTreeMap<String, ProtocolReadyGroupPublish>,
    pub(super) pending_local_sibling_sends: Vec<ProtocolPendingLocalSiblingSend>,
    pub(super) pending_remote_sends: Vec<ProtocolPendingRemoteSend>,
    pub(super) pending_group_pairwise_payloads: Vec<ProtocolPendingGroupPairwisePayload>,
    pub(super) pending_group_sender_key_messages: PendingGroupMessages,
    pub(super) pending_group_sender_key_repairs: Vec<ProtocolPendingGroupSenderKeyRepair>,
    pub(super) processed_group_sender_key_messages: ProtocolGroupReplayCache,
    pub(super) group_sender_key_retry: ProtocolGroupSenderKeyRetry,
    pub(super) answered_group_sender_key_repairs: Vec<ProtocolAnsweredGroupSenderKeyRepair>,
    pub(super) pending_decrypted_deliveries: Vec<ProtocolPendingDecryptedDelivery>,
    pub(super) group_roster_fact_histories: BTreeMap<String, GroupRosterFactHistory>,
    pub(super) verified_app_keys_owners: BTreeSet<NdrOwnerPubkey>,
    pub(super) invite_owner_app_keys_evidence: BTreeMap<NdrOwnerPubkey, ProtocolAppKeysEvidence>,
    pub(super) processed_private_invite_response_ids: Vec<String>,
    pub(super) group_invite_uses: BTreeMap<String, BTreeSet<NdrOwnerPubkey>>,
    pub(super) local_app_keys_observed: bool,
    pub(super) subscription_generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProtocolPendingLocalSiblingSend {
    // Message bodies are restricted to devices authorized when sent. None
    // preserves current-roster broadcast semantics for read-state metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) eligible_devices: Option<BTreeSet<NdrDevicePubkey>>,
    pub(super) chat_id: String,
    pub(super) payload: Vec<u8>,
    pub(super) message_id: String,
    pub(super) completed_devices: BTreeSet<NdrDevicePubkey>,
    pub(super) created_at_secs: u64,
    pub(super) next_retry_at_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProtocolPendingRemoteSend {
    pub(super) recipient_owner: NdrOwnerPubkey,
    pub(super) eligible_devices: BTreeSet<NdrDevicePubkey>,
    pub(super) completed_devices: BTreeSet<NdrDevicePubkey>,
    pub(super) chat_id: String,
    pub(super) payload: Vec<u8>,
    pub(super) inner_event_id: Option<String>,
    pub(super) message_id: String,
    pub(super) expires_at_secs: Option<u64>,
    pub(super) created_at_secs: u64,
    pub(super) next_retry_at_secs: u64,
}
