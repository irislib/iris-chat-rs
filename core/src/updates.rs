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
        plan: Option<crate::core::ProtocolSubscriptionPlan>,
        success: bool,
        error: Option<String>,
        relay_statuses: Vec<(String, RelayStatus)>,
        connected_before: u64,
        connected_after: u64,
        filter_count: u64,
    },
    RelayTransportConnectionFinished {
        token: u64,
        reason: String,
        relay_statuses: Vec<(String, RelayStatus)>,
        connected_count: u64,
    },
    #[cfg(not(target_os = "ios"))]
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
    RelayPublishDrainFinished {
        token: u64,
        results: Vec<RelayPublishDrainResult>,
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
    // Heavy tail of `open_chat` — DB page load, identity republish,
    // persist, protocol refresh. Runs on the same event loop as a
    // queued follow-up so subsequent UI actions (back, switch chat)
    // can interleave between the screen flip and the load.
    OpenChatFinalize {
        chat_id: String,
    },
}

#[derive(Debug)]
pub(crate) struct RelayPublishDrainResult {
    pub(crate) event_id: String,
    pub(crate) message_id: Option<String>,
    pub(crate) chat_id: Option<String>,
    pub(crate) success: bool,
    pub(crate) relay_urls: Vec<String>,
    pub(crate) detail: String,
}
