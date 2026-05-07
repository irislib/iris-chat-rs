use crate::actions::AppAction;
use crate::state::{AppState, PeerProfileDebugSnapshot};
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
    PeerProfileDebug {
        owner_input: String,
        reply_tx: Sender<Option<PeerProfileDebugSnapshot>>,
    },
    PrepareForSuspend(Sender<()>),
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
        generation: u64,
    },
    ProtocolSubscriptionReconcileCompleted {
        generation: u64,
        token: u64,
        reason: String,
        relay_statuses: Vec<(String, RelayStatus)>,
        connected_before: u64,
        connected_after: u64,
        applied: u64,
        failed: u64,
    },
    RelayConnectionChecked {
        reason: String,
    },
    DebugSnapshotWriteFinished {
        generation: u64,
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
    RetryPendingRelayPublishes {
        reason: String,
    },
    AttachmentUploadFinished {
        chat_id: String,
        result: Result<String, String>,
    },
    ProfilePictureUploadFinished {
        result: Result<String, String>,
    },
    SyncComplete,
}
