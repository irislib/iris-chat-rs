use crate::state::{OutgoingAttachment, Screen};

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppAction {
    CreateAccount {
        name: String,
    },
    UpdateProfileMetadata {
        name: String,
        picture_url: Option<String>,
    },
    RestoreSession {
        owner_nsec: String,
    },
    RestoreAccountBundle {
        owner_nsec: Option<String>,
        owner_pubkey_hex: String,
        device_nsec: String,
    },
    StartLinkedDevice {
        owner_input: String,
    },
    SetCurrentDeviceLabels {
        device_label: String,
        client_label: String,
    },
    AppForegrounded,
    Logout,
    CreateChat {
        peer_input: String,
    },
    CreateGroup {
        name: String,
        member_inputs: Vec<String>,
    },
    CreateGroupWithPicture {
        name: String,
        member_inputs: Vec<String>,
        picture_file_path: String,
        picture_filename: String,
    },
    CreatePublicInvite,
    AcceptInvite {
        invite_input: String,
    },
    OpenChat {
        chat_id: String,
    },
    SendMessage {
        chat_id: String,
        text: String,
    },
    SendDisappearingMessage {
        chat_id: String,
        text: String,
        expires_at_secs: u64,
    },
    SetChatMessageTtl {
        chat_id: String,
        ttl_seconds: Option<u64>,
    },
    SetChatMuted {
        chat_id: String,
        muted: bool,
    },
    SetChatPinned {
        chat_id: String,
        pinned: bool,
    },
    SetChatUnread {
        chat_id: String,
        unread: bool,
    },
    SendAttachment {
        chat_id: String,
        file_path: String,
        filename: String,
        caption: String,
    },
    SendAttachments {
        chat_id: String,
        attachments: Vec<OutgoingAttachment>,
        caption: String,
    },
    ToggleReaction {
        chat_id: String,
        message_id: String,
        emoji: String,
    },
    SendTyping {
        chat_id: String,
    },
    StopTyping {
        chat_id: String,
    },
    SetTypingIndicatorsEnabled {
        enabled: bool,
    },
    SetReadReceiptsEnabled {
        enabled: bool,
    },
    SetDesktopNotificationsEnabled {
        enabled: bool,
    },
    SetInviteAcceptanceNotificationsEnabled {
        enabled: bool,
    },
    SetStartupAtLoginEnabled {
        enabled: bool,
    },
    SetNearbyEnabled {
        enabled: bool,
    },
    SetNearbyBluetoothEnabled {
        enabled: bool,
    },
    SetNearbyLanEnabled {
        enabled: bool,
    },
    SetDebugLoggingEnabled {
        enabled: bool,
    },
    SetAcceptUnknownDirectMessages {
        enabled: bool,
    },
    /// Block / unblock a peer owner (Signal-style global blocklist).
    /// When blocked, the core drops the peer from both the nostr
    /// relay subscription and the mobile push subscription, refuses
    /// outgoing sends, and hides their thread / discards their
    /// incoming messages.
    SetUserBlocked {
        owner_pubkey_hex: String,
        blocked: bool,
    },
    /// Mark a direct chat's peer as accepted (Signal whitelist). The
    /// projection's `is_request` flag flips to false. Sending the
    /// first outgoing message also implicitly accepts.
    SetMessageRequestAccepted {
        chat_id: String,
    },
    /// Pause / resume the nearby mailbag's store-and-forward writer
    /// and reader. The bag's existing contents survive the toggle so
    /// the user can flip it back on without losing what was queued;
    /// wiping is a separate, shell-local "Empty mailbag" action that
    /// targets the platform's nearby service directly.
    SetNearbyMailbagEnabled {
        enabled: bool,
    },
    AddNostrRelay {
        relay_url: String,
    },
    UpdateNostrRelay {
        old_relay_url: String,
        new_relay_url: String,
    },
    RemoveNostrRelay {
        relay_url: String,
    },
    SetNostrRelays {
        relay_urls: Vec<String>,
    },
    ResetNostrRelays,
    SetImageProxyEnabled {
        enabled: bool,
    },
    SetImageProxyUrl {
        url: String,
    },
    SetImageProxyKeyHex {
        key_hex: String,
    },
    SetImageProxySaltHex {
        salt_hex: String,
    },
    ResetImageProxySettings,
    SetMobilePushServerUrl {
        url: String,
    },
    ResetMobilePushServerUrl,
    IngestMobilePushPayload {
        payload_json: String,
    },
    MarkMessagesSeen {
        chat_id: String,
        message_ids: Vec<String>,
    },
    SendReceipt {
        chat_id: String,
        receipt_type: String,
        message_ids: Vec<String>,
    },
    DeleteLocalMessage {
        chat_id: String,
        message_id: String,
    },
    DeleteChat {
        chat_id: String,
    },
    UpdateGroupName {
        group_id: String,
        name: String,
    },
    UpdateGroupPicture {
        group_id: String,
        file_path: String,
        filename: String,
    },
    AddGroupMembers {
        group_id: String,
        member_inputs: Vec<String>,
    },
    SetGroupAdmin {
        group_id: String,
        owner_pubkey_hex: String,
        is_admin: bool,
    },
    RemoveGroupMember {
        group_id: String,
        owner_pubkey_hex: String,
    },
    UploadProfilePicture {
        file_path: String,
    },
    AddAuthorizedDevice {
        device_input: String,
    },
    RemoveAuthorizedDevice {
        device_pubkey_hex: String,
    },
    AcknowledgeRevokedDevice,
    PushScreen {
        screen: Screen,
    },
    UpdateScreenStack {
        stack: Vec<Screen>,
    },
    // Pop the top of the navigation stack. Replaces "UI reads the
    // stack, computes the pop, dispatches UpdateScreenStack with the
    // new array" — the core owns the screen stack, the UI just signals
    // the intent. Appended at the end of the enum so adding it doesn't
    // shift existing variants' uniffi tags on still-stale bindings.
    NavigateBack,
    /// Persist the unsent composer text for a chat. Shells dispatch
    /// this on composer change (debounced) and on send-or-leave so
    /// the draft survives navigation, app suspend, and relaunch —
    /// same shape as Signal's `updateWithDraft`. Empty string clears.
    SetChatDraft {
        chat_id: String,
        text: String,
    },
    /// Publish a blank owner metadata event so public profile name/photo are
    /// cleared before the shell removes local keys and data.
    DeleteProfileMetadata,
}
