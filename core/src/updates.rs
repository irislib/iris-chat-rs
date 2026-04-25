use crate::actions::AppAction;
use crate::core::ProtocolSubscriptionPlan;
use crate::state::AppState;
use flume::Sender;
use nostr_sdk::prelude::Event;

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
}

#[derive(Debug)]
pub(crate) enum CoreMsg {
    Action(AppAction),
    Internal(Box<InternalEvent>),
    ExportSupportBundle(Sender<String>),
    Shutdown(Option<Sender<()>>),
}

#[derive(Debug)]
pub(crate) enum InternalEvent {
    RelayEvent(Event),
    RetryPendingOutbound,
    FetchTrackedPeerCatchUp,
    PollPendingDeviceInvites {
        token: u64,
    },
    FetchCatchUpEvents(Vec<Event>),
    FetchPendingDeviceInvites(Vec<Event>),
    DebugLog {
        category: String,
        detail: String,
    },
    TypingIndicatorExpired {
        chat_id: String,
        author: String,
    },
    PublishFinished {
        message_id: String,
        chat_id: String,
        success: bool,
    },
    AttachmentUploadFinished {
        chat_id: String,
        result: Result<String, String>,
    },
    GroupControlPublishFinished {
        operation_id: String,
        success: bool,
    },
    ProtocolSubscriptionRefreshCompleted {
        token: u64,
        applied: bool,
        plan: Option<ProtocolSubscriptionPlan>,
    },
    SyncComplete,
}
