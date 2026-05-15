use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    CreateAccount,
    RestoreAccount,
    AddDevice,
    ChatList,
    NewChat,
    NewGroup,
    CreateInvite,
    JoinInvite,
    Settings,
    Chat { chat_id: String },
    GroupDetails { group_id: String },
    DeviceRoster,
    AwaitingDeviceApproval,
    DeviceRevoked,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Router {
    pub default_screen: Screen,
    pub screen_stack: Vec<Screen>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Eq)]
pub struct BusyState {
    pub creating_account: bool,
    pub restoring_session: bool,
    pub linking_device: bool,
    pub creating_chat: bool,
    pub creating_group: bool,
    pub sending_message: bool,
    pub updating_roster: bool,
    pub updating_group: bool,
    pub creating_invite: bool,
    pub accepting_invite: bool,
    pub syncing_network: bool,
    pub uploading_attachment: bool,
    pub upload_progress: Option<UploadProgress>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadProgress {
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PreferencesSnapshot {
    pub send_typing_indicators: bool,
    pub send_read_receipts: bool,
    pub desktop_notifications_enabled: bool,
    pub invite_acceptance_notifications_enabled: bool,
    pub startup_at_login_enabled: bool,
    pub nearby_enabled: bool,
    pub nearby_bluetooth_enabled: bool,
    pub nearby_lan_enabled: bool,
    /// Whether the on-device nearby mailbag actively reads and writes
    /// store-and-forward records. When off, the local bag is left
    /// alone (so contents survive a toggle off → on cycle) but no new
    /// events are stored and the broadcast/replay path skips it.
    pub nearby_mailbag_enabled: bool,
    pub nostr_relay_urls: Vec<String>,
    pub image_proxy_enabled: bool,
    pub image_proxy_url: String,
    pub image_proxy_key_hex: String,
    pub image_proxy_salt_hex: String,
    pub muted_chat_ids: Vec<String>,
    pub pinned_chat_ids: Vec<String>,
    /// Owner pubkeys (hex) the local user has blocked. Blocking drops
    /// the peer from the nostr relay subscription, the mobile push
    /// subscription, and hides their thread from the chat list; any
    /// further messages from them are also discarded at ingest.
    pub blocked_owner_pubkeys: Vec<String>,
    /// Owner pubkeys (hex) for which the user has accepted a message
    /// request (Signal-style "whitelist"). A direct chat is treated as
    /// a `is_request` thread until the peer is in this set; sending an
    /// outgoing message implicitly adds them.
    pub accepted_owner_pubkeys: Vec<String>,
    pub debug_logging_enabled: bool,
    pub accept_unknown_direct_messages: bool,
    /// User-configurable notification server URL. Empty string means
    /// "use the platform default" (notifications.iris.to in release,
    /// notifications-sandbox.iris.to in debug). When non-empty, the
    /// shells should pass this as the override to
    /// `build_mobile_push_*_subscription_request`.
    pub mobile_push_server_url: String,
}

impl Default for PreferencesSnapshot {
    fn default() -> Self {
        Self {
            send_typing_indicators: false,
            send_read_receipts: true,
            desktop_notifications_enabled: true,
            invite_acceptance_notifications_enabled: true,
            startup_at_login_enabled: true,
            nearby_enabled: true,
            nearby_bluetooth_enabled: false,
            nearby_lan_enabled: false,
            nearby_mailbag_enabled: true,
            nostr_relay_urls: crate::core::configured_relays(),
            image_proxy_enabled: true,
            image_proxy_url: crate::image_proxy::DEFAULT_IMAGE_PROXY_URL.to_string(),
            image_proxy_key_hex: crate::image_proxy::DEFAULT_IMAGE_PROXY_KEY_HEX.to_string(),
            image_proxy_salt_hex: crate::image_proxy::DEFAULT_IMAGE_PROXY_SALT_HEX.to_string(),
            muted_chat_ids: Vec::new(),
            pinned_chat_ids: Vec::new(),
            blocked_owner_pubkeys: Vec::new(),
            accepted_owner_pubkeys: Vec::new(),
            debug_logging_enabled: false,
            accept_unknown_direct_messages: true,
            mobile_push_server_url: String::new(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct OutgoingAttachment {
    pub file_path: String,
    pub filename: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AttachmentDownloadResult {
    pub data_base64: Option<String>,
    pub error: Option<String>,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum DeviceAuthorizationState {
    Authorized,
    AwaitingApproval,
    Revoked,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub public_key_hex: String,
    pub npub: String,
    pub display_name: String,
    pub picture_url: Option<String>,
    pub device_public_key_hex: String,
    pub device_npub: String,
    pub has_owner_signing_authority: bool,
    pub authorization_state: DeviceAuthorizationState,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DeviceEntrySnapshot {
    pub device_pubkey_hex: String,
    pub device_npub: String,
    pub is_current_device: bool,
    pub is_authorized: bool,
    pub is_stale: bool,
    pub added_at_secs: Option<u64>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DeviceRosterSnapshot {
    pub owner_public_key_hex: String,
    pub owner_npub: String,
    pub current_device_public_key_hex: String,
    pub current_device_npub: String,
    pub can_manage_devices: bool,
    pub authorization_state: DeviceAuthorizationState,
    pub devices: Vec<DeviceEntrySnapshot>,
}

#[derive(uniffi::Enum, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeliveryState {
    Queued,
    Pending,
    Sent,
    Received,
    Seen,
    Failed,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum ChatKind {
    Direct,
    Group,
}

#[derive(uniffi::Enum, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatMessageKind {
    User,
    System,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageAttachmentSnapshot {
    pub nhash: String,
    pub filename: String,
    pub filename_encoded: String,
    pub htree_url: String,
    pub is_image: bool,
    pub is_video: bool,
    pub is_audio: bool,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageReactionSnapshot {
    pub emoji: String,
    pub count: u64,
    pub reacted_by_me: bool,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageReactor {
    /// Hex-encoded pubkey of the user who reacted.
    pub author: String,
    /// Emoji content of their current (latest) reaction. Empty means unreacted.
    pub emoji: String,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRecipientDeliverySnapshot {
    /// Hex-encoded owner/user pubkey.
    pub owner_pubkey_hex: String,
    pub delivery: DeliveryState,
    pub updated_at_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct MessageDeliveryTraceSnapshot {
    pub outer_event_ids: Vec<String>,
    pub pending_relay_event_ids: Vec<String>,
    pub queued_protocol_targets: Vec<String>,
    pub target_device_ids: Vec<String>,
    pub transport_channels: Vec<String>,
    pub last_transport_error: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ChatMessageSnapshot {
    pub id: String,
    pub chat_id: String,
    pub kind: ChatMessageKind,
    pub author: String,
    pub body: String,
    pub attachments: Vec<MessageAttachmentSnapshot>,
    pub reactions: Vec<MessageReactionSnapshot>,
    pub reactors: Vec<MessageReactor>,
    pub is_outgoing: bool,
    pub created_at_secs: u64,
    pub expires_at_secs: Option<u64>,
    pub delivery: DeliveryState,
    pub recipient_deliveries: Vec<MessageRecipientDeliverySnapshot>,
    pub delivery_trace: MessageDeliveryTraceSnapshot,
    /// Hex ID of the outer relay event that carried this rumor. The
    /// notification extension joins on this to find a body the
    /// foreground app already decrypted, so it can render a real
    /// preview instead of "New activity". `None` for messages that
    /// didn't come over the wire (system notices, locally-composed
    /// outgoing rumors).
    pub source_event_id: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct TypingIndicatorSnapshot {
    pub chat_id: String,
    pub display_name: String,
    pub expires_at_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ChatThreadSnapshot {
    pub chat_id: String,
    pub kind: ChatKind,
    pub display_name: String,
    pub subtitle: Option<String>,
    pub picture_url: Option<String>,
    pub member_count: u64,
    pub last_message_preview: Option<String>,
    pub last_message_at_secs: Option<u64>,
    pub last_message_is_outgoing: Option<bool>,
    pub last_message_delivery: Option<DeliveryState>,
    pub unread_count: u64,
    pub is_typing: bool,
    pub is_muted: bool,
    pub is_pinned: bool,
    /// Unsent composer text the user typed in this thread, persisted
    /// across navigation / suspend / relaunch. Chat list rows render
    /// "Draft: …" in place of the last message preview when this is
    /// non-empty (Signal pattern).
    pub draft: String,
    /// True when this is a direct chat where the user has not yet
    /// replied to a stranger — i.e. incoming messages exist but no
    /// outgoing ones do. Shells render an Accept / Delete / Block
    /// gate (Signal-style "message request"), suppress read /
    /// delivery / typing receipts, and treat the conversation as
    /// untrusted until the user accepts.
    pub is_request: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct CurrentChatSnapshot {
    pub chat_id: String,
    pub kind: ChatKind,
    pub display_name: String,
    pub subtitle: Option<String>,
    pub picture_url: Option<String>,
    pub group_id: Option<String>,
    pub member_count: u64,
    pub message_ttl_seconds: Option<u64>,
    pub is_muted: bool,
    pub messages: Vec<ChatMessageSnapshot>,
    pub typing_indicators: Vec<TypingIndicatorSnapshot>,
    /// Same persisted draft text exposed on `ChatThreadSnapshot`. The
    /// chat screen pre-fills its composer with this on first appear.
    pub draft: String,
    /// Mirrors `ChatThreadSnapshot::is_request`. Chat screens replace
    /// the composer with an Accept / Delete / Block gate when set.
    pub is_request: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct GroupMemberSnapshot {
    pub owner_pubkey_hex: String,
    pub display_name: String,
    pub npub: String,
    pub is_admin: bool,
    pub is_creator: bool,
    pub is_local_owner: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct GroupDetailsSnapshot {
    pub group_id: String,
    pub name: String,
    pub picture_url: Option<String>,
    pub created_by_display_name: String,
    pub created_by_npub: String,
    pub can_manage: bool,
    pub is_muted: bool,
    pub revision: u64,
    pub members: Vec<GroupMemberSnapshot>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Eq)]
pub struct MutualGroupsSnapshot {
    pub groups: Vec<ChatThreadSnapshot>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct RelayConnectionSnapshot {
    pub url: String,
    pub status: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct NetworkStatusSnapshot {
    pub relay_set_id: String,
    pub relay_urls: Vec<String>,
    pub relay_connections: Vec<RelayConnectionSnapshot>,
    pub connected_relay_count: u64,
    pub all_relays_offline_since_secs: Option<u64>,
    pub syncing: bool,
    pub pending_outbound_count: u64,
    pub pending_group_control_count: u64,
    pub recent_event_count: u64,
    pub recent_log_count: u64,
    pub last_debug_category: Option<String>,
    pub last_debug_detail: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Eq)]
pub struct MobilePushSessionSnapshot {
    pub recipient_pubkey_hex: String,
    pub display_name: String,
    pub state_json: String,
    pub tracked_sender_pubkeys: Vec<String>,
    pub has_receiving_capability: bool,
}

#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Eq)]
pub struct MobilePushSyncSnapshot {
    pub owner_pubkey_hex: Option<String>,
    pub message_author_pubkeys: Vec<String>,
    pub invite_response_pubkeys: Vec<String>,
    pub sessions: Vec<MobilePushSessionSnapshot>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PeerProfileDebugSnapshot {
    pub owner_pubkey_hex: String,
    pub owner_npub: String,
    pub roster_device_count: u64,
    pub known_device_count: u64,
    pub active_session_count: u64,
    pub session_count: u64,
    pub receiving_session_count: u64,
    pub tracked_sender_count: u64,
    pub recent_handshake_device_count: u64,
    pub last_handshake_at_secs: Option<u64>,
    pub tracked_for_messages: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct MobilePushNotificationResolution {
    pub should_show: bool,
    pub title: String,
    pub body: String,
    pub payload_json: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct MobilePushSubscriptionRequest {
    pub method: String,
    pub url: String,
    pub authorization_header: String,
    pub body_json: Option<String>,
}

/// One row inside the "Messages" section of a search result. Shells
/// render this as a single conversation list row whose subtitle is the
/// matched body and whose title is the chat's display name (resolved
/// here so the UI doesn't have to look up the parent thread).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct MessageSearchHit {
    pub chat_id: String,
    pub message_id: String,
    pub chat_display_name: String,
    pub chat_picture_url: Option<String>,
    pub chat_kind: ChatKind,
    pub author_pubkey: String,
    pub body: String,
    pub is_outgoing: bool,
    pub created_at_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct SearchResultSnapshot {
    pub query: String,
    pub scope_chat_id: Option<String>,
    pub contacts: Vec<ChatThreadSnapshot>,
    pub groups: Vec<ChatThreadSnapshot>,
    pub messages: Vec<MessageSearchHit>,
    /// Inline shortcut row to show above the grouped sections when the
    /// query is itself an npub / nprofile / hex pubkey / invite URL.
    /// Shells render this as a single "Start chat with …" / "Accept
    /// invite from …" row that dispatches the carried action on tap.
    pub shortcut: Option<ChatInputShortcut>,
}

/// Classification of the free-text input typed into a chat-action
/// field — the search bar, the "New chat" paste field, the share-link
/// handler. Centralising the parsing here means the UI just looks at
/// the enum and dispatches; no platform has its own ad-hoc
/// `contains("://") && contains("#")` check.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum ChatInputShortcut {
    /// Input is a recognized owner pubkey (npub / nprofile / hex).
    /// `display` is the user-presentable short form (npub…).
    DirectPeer {
        peer_input: String,
        display: String,
        npub: String,
        pubkey_hex: String,
    },
    /// Input is a paste of an invite URL. `display` is a short label
    /// suitable for "Accept invite from …" rows.
    Invite {
        invite_input: String,
        display: String,
    },
}

impl SearchResultSnapshot {
    pub fn empty(query: String, scope_chat_id: Option<String>) -> Self {
        Self {
            query,
            scope_chat_id,
            contacts: Vec::new(),
            groups: Vec::new(),
            messages: Vec::new(),
            shortcut: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
            && self.groups.is_empty()
            && self.messages.is_empty()
            && self.shortcut.is_none()
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PublicInviteSnapshot {
    pub url: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct LinkDeviceSnapshot {
    pub url: String,
    pub device_input: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AppState {
    pub rev: u64,
    pub router: Router,
    pub account: Option<AccountSnapshot>,
    pub device_roster: Option<DeviceRosterSnapshot>,
    pub busy: BusyState,
    pub chat_list: Vec<ChatThreadSnapshot>,
    pub current_chat: Option<CurrentChatSnapshot>,
    pub group_details: Option<GroupDetailsSnapshot>,
    pub public_invite: Option<PublicInviteSnapshot>,
    pub link_device: Option<LinkDeviceSnapshot>,
    pub network_status: Option<NetworkStatusSnapshot>,
    pub mobile_push: MobilePushSyncSnapshot,
    pub preferences: PreferencesSnapshot,
    pub toast: Option<String>,
}

impl AppState {
    pub fn empty() -> Self {
        Self {
            rev: 0,
            router: Router {
                default_screen: Screen::Welcome,
                screen_stack: Vec::new(),
            },
            account: None,
            device_roster: None,
            busy: BusyState::default(),
            chat_list: Vec::new(),
            current_chat: None,
            group_details: None,
            public_invite: None,
            link_device: None,
            network_status: None,
            mobile_push: MobilePushSyncSnapshot::default(),
            preferences: PreferencesSnapshot::default(),
            toast: None,
        }
    }
}
