use super::*;

pub(super) struct LoggedInState {
    pub(super) owner_pubkey: OwnerPubkey,
    pub(super) owner_keys: Option<Keys>,
    pub(super) device_keys: Keys,
    pub(super) client: Client,
    pub(super) relay_urls: Vec<RelayUrl>,
    pub(super) session_manager: SessionManager,
    pub(super) group_manager: GroupManager,
    pub(super) authorization_state: LocalAuthorizationState,
}

#[derive(Clone)]
pub(super) struct ThreadRecord {
    pub(super) chat_id: String,
    pub(super) unread_count: u64,
    pub(super) updated_at_secs: u64,
    pub(super) messages: Vec<ChatMessageSnapshot>,
}

impl ThreadRecord {
    pub(super) fn insert_message_sorted(&mut self, message: ChatMessageSnapshot) {
        let position = self
            .messages
            .partition_point(|existing| message_order_key(existing) <= message_order_key(&message));
        self.messages.insert(position, message);
    }
}

fn message_order_key(message: &ChatMessageSnapshot) -> (u64, u64, &str) {
    (
        message.created_at_secs,
        message.id.parse::<u64>().unwrap_or(u64::MAX),
        message.id.as_str(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(super) enum PendingInbound {
    Envelope {
        envelope: MessageEnvelope,
        #[serde(default)]
        expires_at_secs: Option<u64>,
    },
    Decrypted {
        sender_owner_hex: String,
        payload: Vec<u8>,
        created_at_secs: u64,
        #[serde(default)]
        expires_at_secs: Option<u64>,
    },
}

impl PendingInbound {
    pub(super) fn envelope(envelope: MessageEnvelope, expires_at_secs: Option<u64>) -> Self {
        Self::Envelope {
            envelope,
            expires_at_secs,
        }
    }

    pub(super) fn decrypted(
        sender_owner: OwnerPubkey,
        payload: Vec<u8>,
        created_at_secs: u64,
        expires_at_secs: Option<u64>,
    ) -> Self {
        Self::Decrypted {
            sender_owner_hex: sender_owner.to_string(),
            payload,
            created_at_secs,
            expires_at_secs,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PreparedPublishBatch {
    #[serde(default)]
    pub(super) invite_events: Vec<Event>,
    #[serde(default)]
    pub(super) message_events: Vec<Event>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum OutboundPublishMode {
    FirstContactStaged,
    OrdinaryFirstAck,
    #[default]
    WaitForPeer,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PendingOutbound {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) expires_at_secs: Option<u64>,
    #[serde(default)]
    pub(super) prepared_publish: Option<PreparedPublishBatch>,
    #[serde(default)]
    pub(super) publish_mode: OutboundPublishMode,
    #[serde(default)]
    pub(super) reason: PendingSendReason,
    #[serde(default)]
    pub(super) next_retry_at_secs: u64,
    #[serde(default)]
    pub(super) in_flight: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum PendingGroupControlKind {
    Create {
        name: String,
        member_owner_hexes: Vec<String>,
    },
    Rename {
        name: String,
    },
    AddMembers {
        member_owner_hexes: Vec<String>,
    },
    RemoveMember {
        owner_pubkey_hex: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PendingGroupControl {
    pub(super) operation_id: String,
    pub(super) group_id: String,
    pub(super) target_owner_hexes: Vec<String>,
    #[serde(default)]
    pub(super) prepared_publish: Option<PreparedPublishBatch>,
    #[serde(default)]
    pub(super) reason: PendingSendReason,
    #[serde(default)]
    pub(super) next_retry_at_secs: u64,
    #[serde(default)]
    pub(super) in_flight: bool,
    pub(super) kind: PendingGroupControlKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppDirectMessagePayload {
    pub(super) version: u8,
    pub(super) chat_id: String,
    #[serde(default)]
    pub(super) message_id: Option<String>,
    pub(super) body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppGroupMessagePayload {
    pub(super) version: u8,
    #[serde(default)]
    pub(super) message_id: Option<String>,
    pub(super) body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct AppControlPayload {
    pub(super) version: u8,
    #[serde(rename = "type")]
    pub(super) control_type: AppControlType,
    #[serde(default)]
    pub(super) chat_id: Option<String>,
    #[serde(default)]
    pub(super) message_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum AppControlType {
    Typing,
    Delivered,
    Seen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AppPayload {
    DirectMessage(AppDirectMessagePayload),
    GroupMessage(AppGroupMessagePayload),
    Control(AppControlPayload),
    LegacyText(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct OwnerProfileRecord {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) picture: Option<String>,
    #[serde(default)]
    pub(super) updated_at_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct NostrProfileMetadata {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) picture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RoutedChatMessage {
    pub(super) chat_id: String,
    pub(super) message_id: Option<String>,
    pub(super) body: String,
    pub(super) is_outgoing: bool,
    pub(super) author: Option<String>,
    pub(super) expires_at_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TypingIndicatorRecord {
    pub(super) chat_id: String,
    pub(super) author_owner_hex: String,
    pub(super) expires_at_secs: u64,
    pub(super) last_event_secs: u64,
}

#[derive(Clone, Debug)]
pub(super) struct RecentHandshakePeer {
    pub(super) owner_hex: String,
    pub(super) device_hex: String,
    pub(super) observed_at_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocalAuthorizationState {
    Authorized,
    AwaitingApproval,
    Revoked,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum PendingSendReason {
    #[default]
    MissingRoster,
    MissingDeviceInvite,
    PublishingFirstContact,
    PublishRetry,
}

#[derive(Debug, Clone)]
pub(super) struct StagedOutboundSend {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) invite_events: Vec<Event>,
    pub(super) message_events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolSubscriptionPlan {
    pub(crate) roster_authors: Vec<String>,
    pub(crate) invite_authors: Vec<String>,
    pub(crate) invite_response_recipient: Option<String>,
    pub(crate) message_authors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProtocolSubscriptionRuntime {
    pub(super) current_plan: Option<ProtocolSubscriptionPlan>,
    pub(super) applying_plan: Option<ProtocolSubscriptionPlan>,
    pub(super) refresh_in_flight: bool,
    pub(super) refresh_dirty: bool,
    pub(super) force_refresh_dirty: bool,
    pub(super) refresh_token: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct DebugEventCounters {
    pub(super) roster_events: u64,
    pub(super) invite_events: u64,
    pub(super) invite_response_events: u64,
    pub(super) message_events: u64,
    pub(super) other_events: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct DebugLogEntry {
    pub(super) timestamp_secs: u64,
    pub(super) category: String,
    pub(super) detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimeDebugSnapshot {
    pub(super) generated_at_secs: u64,
    pub(super) local_owner_pubkey_hex: Option<String>,
    pub(super) local_device_pubkey_hex: Option<String>,
    pub(super) authorization_state: Option<String>,
    pub(super) active_chat_id: Option<String>,
    pub(super) current_protocol_plan: Option<RuntimeProtocolPlanDebug>,
    pub(super) tracked_owner_hexes: Vec<String>,
    pub(super) known_users: Vec<RuntimeKnownUserDebug>,
    pub(super) pending_outbound: Vec<RuntimePendingOutboundDebug>,
    pub(super) pending_group_controls: Vec<RuntimePendingGroupControlDebug>,
    pub(super) recent_handshake_peers: Vec<RuntimeRecentHandshakeDebug>,
    pub(super) event_counts: DebugEventCounters,
    pub(super) recent_log: Vec<DebugLogEntry>,
    pub(super) toast: Option<String>,
    pub(super) current_chat_list: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct SupportBuildMetadata {
    pub(super) app_version: String,
    pub(super) build_channel: String,
    pub(super) git_sha: String,
    pub(super) build_timestamp_utc: String,
    pub(super) relay_set_id: String,
    pub(super) trusted_test_build: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct SupportBundle {
    pub(super) generated_at_secs: u64,
    pub(super) build: SupportBuildMetadata,
    pub(super) relay_urls: Vec<String>,
    pub(super) authorization_state: Option<String>,
    pub(super) active_chat_id: Option<String>,
    pub(super) current_screen: String,
    pub(super) chat_count: usize,
    pub(super) direct_chat_count: usize,
    pub(super) group_chat_count: usize,
    pub(super) unread_chat_count: usize,
    pub(super) pending_outbound: Vec<RuntimePendingOutboundDebug>,
    pub(super) pending_group_controls: Vec<RuntimePendingGroupControlDebug>,
    pub(super) protocol: Option<RuntimeProtocolPlanDebug>,
    pub(super) tracked_owner_hexes: Vec<String>,
    pub(super) known_users: Vec<RuntimeKnownUserDebug>,
    pub(super) recent_handshake_peers: Vec<RuntimeRecentHandshakeDebug>,
    pub(super) event_counts: DebugEventCounters,
    pub(super) recent_log: Vec<DebugLogEntry>,
    pub(super) current_chat_list: Vec<String>,
    pub(super) latest_toast: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimeProtocolPlanDebug {
    pub(super) roster_authors: Vec<String>,
    pub(super) invite_authors: Vec<String>,
    pub(super) invite_response_recipient: Option<String>,
    pub(super) message_authors: Vec<String>,
    pub(super) refresh_in_flight: bool,
    pub(super) refresh_dirty: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimeKnownUserDebug {
    pub(super) owner_pubkey_hex: String,
    pub(super) has_roster: bool,
    pub(super) roster_device_count: usize,
    pub(super) device_count: usize,
    pub(super) authorized_device_count: usize,
    pub(super) active_session_device_count: usize,
    pub(super) inactive_session_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimePendingOutboundDebug {
    pub(super) message_id: String,
    pub(super) chat_id: String,
    pub(super) reason: String,
    pub(super) publish_mode: String,
    pub(super) in_flight: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimePendingGroupControlDebug {
    pub(super) operation_id: String,
    pub(super) group_id: String,
    pub(super) target_owner_hexes: Vec<String>,
    pub(super) reason: String,
    pub(super) in_flight: bool,
    pub(super) kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RuntimeRecentHandshakeDebug {
    pub(super) owner_hex: String,
    pub(super) device_hex: String,
    pub(super) observed_at_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PersistedState {
    pub(super) version: u32,
    #[serde(alias = "active_peer_hex")]
    pub(super) active_chat_id: Option<String>,
    pub(super) next_message_id: u64,
    pub(super) session_manager: Option<SessionManagerSnapshot>,
    #[serde(default)]
    pub(super) group_manager: Option<GroupManagerSnapshot>,
    #[serde(default)]
    pub(super) owner_profiles: BTreeMap<String, OwnerProfileRecord>,
    #[serde(default)]
    pub(super) preferences: PersistedPreferences,
    #[serde(default)]
    pub(super) chat_message_ttl_seconds: BTreeMap<String, u64>,
    pub(super) threads: Vec<PersistedThread>,
    #[serde(default)]
    pub(super) pending_inbound: Vec<PendingInbound>,
    #[serde(default)]
    pub(super) pending_outbound: Vec<PendingOutbound>,
    #[serde(default)]
    pub(super) pending_group_controls: Vec<PendingGroupControl>,
    #[serde(default)]
    pub(super) seen_event_ids: Vec<String>,
    #[serde(default)]
    pub(super) authorization_state: Option<PersistedAuthorizationState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedPreferences {
    #[serde(default = "default_true")]
    pub(super) send_typing_indicators: bool,
    #[serde(default = "default_true")]
    pub(super) send_read_receipts: bool,
    #[serde(default = "default_true")]
    pub(super) desktop_notifications_enabled: bool,
    #[serde(default)]
    pub(super) startup_at_login_enabled: bool,
    #[serde(default = "default_nostr_relay_urls")]
    pub(super) nostr_relay_urls: Vec<String>,
    #[serde(default = "default_true")]
    pub(super) image_proxy_enabled: bool,
    #[serde(default = "default_image_proxy_url")]
    pub(super) image_proxy_url: String,
    #[serde(default = "default_image_proxy_key_hex")]
    pub(super) image_proxy_key_hex: String,
    #[serde(default = "default_image_proxy_salt_hex")]
    pub(super) image_proxy_salt_hex: String,
}

impl Default for PersistedPreferences {
    fn default() -> Self {
        Self {
            send_typing_indicators: true,
            send_read_receipts: true,
            desktop_notifications_enabled: true,
            startup_at_login_enabled: false,
            nostr_relay_urls: default_nostr_relay_urls(),
            image_proxy_enabled: true,
            image_proxy_url: default_image_proxy_url(),
            image_proxy_key_hex: default_image_proxy_key_hex(),
            image_proxy_salt_hex: default_image_proxy_salt_hex(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_nostr_relay_urls() -> Vec<String> {
    configured_relays()
}

fn default_image_proxy_url() -> String {
    crate::image_proxy::DEFAULT_IMAGE_PROXY_URL.to_string()
}

fn default_image_proxy_key_hex() -> String {
    crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX.to_string()
}

fn default_image_proxy_salt_hex() -> String {
    crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX.to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PersistedThread {
    #[serde(alias = "peer_hex")]
    pub(super) chat_id: String,
    pub(super) unread_count: u64,
    #[serde(default)]
    pub(super) updated_at_secs: u64,
    pub(super) messages: Vec<PersistedMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PersistedMessage {
    pub(super) id: String,
    #[serde(alias = "peer_input")]
    pub(super) chat_id: String,
    pub(super) author: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) attachments: Vec<MessageAttachmentSnapshot>,
    #[serde(default)]
    pub(super) reactions: Vec<MessageReactionSnapshot>,
    pub(super) is_outgoing: bool,
    pub(super) created_at_secs: u64,
    #[serde(default)]
    pub(super) expires_at_secs: Option<u64>,
    pub(super) delivery: PersistedDeliveryState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum PersistedDeliveryState {
    Queued,
    Pending,
    Sent,
    Received,
    Seen,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum PersistedAuthorizationState {
    Authorized,
    AwaitingApproval,
    Revoked,
}

impl From<PersistedDeliveryState> for DeliveryState {
    fn from(value: PersistedDeliveryState) -> Self {
        match value {
            PersistedDeliveryState::Queued => DeliveryState::Queued,
            PersistedDeliveryState::Pending => DeliveryState::Pending,
            PersistedDeliveryState::Sent => DeliveryState::Sent,
            PersistedDeliveryState::Received => DeliveryState::Received,
            PersistedDeliveryState::Seen => DeliveryState::Seen,
            PersistedDeliveryState::Failed => DeliveryState::Failed,
        }
    }
}

impl From<&DeliveryState> for PersistedDeliveryState {
    fn from(value: &DeliveryState) -> Self {
        match value {
            DeliveryState::Queued => Self::Queued,
            DeliveryState::Pending => Self::Pending,
            DeliveryState::Sent => Self::Sent,
            DeliveryState::Received => Self::Received,
            DeliveryState::Seen => Self::Seen,
            DeliveryState::Failed => Self::Failed,
        }
    }
}

impl From<LocalAuthorizationState> for PersistedAuthorizationState {
    fn from(value: LocalAuthorizationState) -> Self {
        match value {
            LocalAuthorizationState::Authorized => Self::Authorized,
            LocalAuthorizationState::AwaitingApproval => Self::AwaitingApproval,
            LocalAuthorizationState::Revoked => Self::Revoked,
        }
    }
}

impl From<PersistedAuthorizationState> for LocalAuthorizationState {
    fn from(value: PersistedAuthorizationState) -> Self {
        match value {
            PersistedAuthorizationState::Authorized => Self::Authorized,
            PersistedAuthorizationState::AwaitingApproval => Self::AwaitingApproval,
            PersistedAuthorizationState::Revoked => Self::Revoked,
        }
    }
}
