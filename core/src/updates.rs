use crate::actions::AppAction;
use crate::state::AppState;
use flume::Sender;
use nostr_sdk::prelude::{Event, RelayStatus};

#[derive(uniffi::Enum, Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AppUpdate {
    FullState(AppState),
    PersistAccountBundle {
        rev: u64,
        owner_nsec: Option<String>,
        owner_pubkey_hex: String,
        device_nsec: String,
    },
    NearbyPublishedEvent {
        event_id: String,
        kind: u32,
        created_at_secs: u64,
        event_json: String,
    },
}

#[derive(Debug)]
pub(crate) enum CoreMsg {
    Action(AppAction),
    Internal(Box<InternalEvent>),
    BuildNearbyPresenceEvent {
        peer_id: String,
        my_nonce: String,
        their_nonce: String,
        profile_event_id: String,
        reply_tx: Sender<String>,
    },
    ExportSupportBundle(Sender<String>),
    Shutdown(Option<Sender<()>>),
}

#[derive(Debug)]
pub(crate) enum InternalEvent {
    RelayEvent(Event),
    NearbyEvent {
        event: Event,
        transport: String,
    },
    FetchTrackedPeerCatchUp,
    ProtocolSubscriptionLivenessCheck {
        token: u64,
    },
    PollPendingDeviceInvites {
        token: u64,
    },
    PruneExpiredMessages {
        token: u64,
    },
    FetchCatchUpEvents(Vec<Event>),
    RelayStatusChanged {
        relay_url: String,
        status: RelayStatus,
    },
    RelayConnectionChecked {
        reason: String,
    },
    DebugLog {
        category: String,
        detail: String,
    },
    TypingIndicatorExpired {
        chat_id: String,
        author: String,
    },
    RelayPublishFinished {
        event_id: String,
        message_id: Option<String>,
        chat_id: Option<String>,
        success: bool,
        relay_urls: Vec<String>,
        detail: String,
    },
    AttachmentUploadFinished {
        chat_id: String,
        result: Result<String, String>,
    },
    GroupPictureUploadFinished {
        group_id: String,
        result: Result<String, String>,
    },
    ProfilePictureUploadFinished {
        result: Result<String, String>,
    },
    SyncComplete,
}
