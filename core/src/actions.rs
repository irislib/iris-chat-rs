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
    AppForegrounded,
    Logout,
    CreateChat {
        peer_input: String,
    },
    CreateGroup {
        name: String,
        member_inputs: Vec<String>,
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
    SetStartupAtLoginEnabled {
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
    MarkMessagesSeen {
        chat_id: String,
        message_ids: Vec<String>,
    },
    DeleteLocalMessage {
        chat_id: String,
        message_id: String,
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
}
