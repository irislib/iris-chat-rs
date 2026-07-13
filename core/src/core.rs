use crate::actions::AppAction;
use crate::state::{
    AccountSnapshot, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot,
    ChatParticipantSnapshot, ChatThreadSnapshot, CurrentChatSnapshot, DeliveryState,
    DeviceAuthorizationState, DeviceEntrySnapshot, DeviceRosterSnapshot, GroupDetailsSnapshot,
    GroupMemberSnapshot, LinkDeviceSnapshot, MessageAttachmentSnapshot,
    MessageDeliveryTraceSnapshot, MessageReactionSnapshot, MessageReactor,
    MessageRecipientDeliverySnapshot, MobilePushNotificationResolution,
    MobilePushSubscriptionRequest, MobilePushSyncSnapshot, NetworkStatusSnapshot,
    OutgoingAttachment, PeerProfileDebugSnapshot, PreferencesSnapshot, PublicInviteSnapshot,
    RelayConnectionSnapshot, Router, Screen, TypingIndicatorSnapshot,
};
use crate::updates::{AppUpdate, CoreMsg, InternalEvent, RelayPublishDrainResult};
use flume::Sender;
use iris_chat_protocol::*;
use nostr::{Alphabet, EventBuilder, SingleLetterTag, UnsignedEvent};
use nostr_double_ratchet::{
    apply_app_keys_snapshot_with_required_device, build_app_keys_device_authorization_filter,
    build_protocol_discovery_filters, deterministic_link_invite_for_device, is_app_keys_event,
    is_group_roster_fact_event, resolve_app_keys_owner_for_device, AppKeys, DeviceEntry,
    APP_KEYS_EVENT_KIND, CHAT_MESSAGE_KIND, CHAT_SETTINGS_KIND, GROUP_ROSTER_FACT_KIND,
    GROUP_SENDER_KEY_MESSAGE_KIND, INVITE_EVENT_KIND, INVITE_LIST_LABEL, INVITE_RESPONSE_KIND,
    MESSAGE_EVENT_KIND, REACTION_KIND, RECEIPT_KIND, TYPING_KIND,
};
use nostr_double_ratchet::{
    parse_group_sender_key_message_event, parse_group_sender_key_message_event_unchecked,
};
use nostr_double_ratchet::{
    GroupIncomingEvent, GroupSnapshot, Invite, OwnerPubkey as NdrOwnerPubkey,
    SessionManagerSnapshot, SessionState, UnixSeconds as NdrUnixSeconds,
};
use nostr_double_ratchet_pairwise_codec as pairwise_codec;
use nostr_identity::{
    approve_nostr_identity_device_approval_bootstrap,
    build_nostr_identity_device_approval_receipt_event,
    build_nostr_identity_roster_op_event_with_client_nonce,
    create_nostr_identity_device_approval_request, encode_nostr_identity_device_approval_bootstrap,
    nostr_identity_device_approval_bootstrap, parse_nostr_identity_device_approval_bootstrap,
    parse_nostr_identity_device_approval_receipt_event_for_bootstrap,
    ApproveNostrIdentityDeviceApprovalBootstrapOptions,
    CreateNostrIdentityDeviceApprovalRequestOptions, NostrIdentityDeviceApprovalBootstrap,
    NostrIdentityDeviceApprovalReceipt, NostrIdentityId, NostrIdentityRosterOp, FACT_OP_KIND,
    NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
};
use nostr_sdk::prelude::{
    Client, Event, Filter, Keys, Kind, PublicKey, RelayNotification, RelayPoolNotification,
    RelayStatus, RelayUrl, SubscribeOptions, SubscriptionId, Timestamp, ToBech32,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, sleep_until, Duration, Instant};

#[cfg(test)]
use nostr_double_ratchet::{
    invite_response_event, message_event, parse_invite_event, parse_message_event,
    GroupEventManager,
};
#[cfg(test)]
use nostr_double_ratchet::{
    AuthorizedDevice, DevicePubkey as NdrDevicePubkey, DeviceRoster, GroupManagerSnapshot,
    ProtocolContext, SessionManager,
};
#[cfg(test)]
use rand::rngs::OsRng;

mod account;
mod account_app_keys;
mod attachment_upload;
mod attachments;
mod chat_reactions;
mod chat_receipts;
mod chat_settings;
mod chat_typing;
mod chats;
mod config;
mod device_approval;
mod device_sync;
mod groups;
mod identity;
mod invites;
mod lifecycle;
mod message_expiry;
mod mobile_push;
mod model;
mod nearby;
pub(crate) mod notifications;
mod payloads;
mod persistence;
mod profile;
mod profile_helpers;
mod projection;
mod protocol;
mod protocol_filters;
mod publish_helpers;
mod publishing;
mod publishing_fact_events;
mod publishing_identity;
mod relay;
mod routing;
mod storage;
mod support;
#[cfg(test)]
mod tests;

pub(crate) const NEARBY_PRESENCE_KIND: u16 = 22242;
pub(super) const APPCORE_PROTOCOL_LABEL: &str = "appcore-protocol";
pub(super) const LOCAL_INVITE_PUBLISH_LABEL: &str = "invite";
pub(super) const PENDING_RELAY_CONTROL_PUBLISH_MAX_ROWS: usize = 2_048;

type OwnerPubkey = PublicKey;
type DevicePubkey = PublicKey;

use account_app_keys::next_app_keys_created_at;
use account_app_keys::{
    canonical_known_app_keys_snapshot, known_app_keys_from_ndr, known_app_keys_to_ndr,
    preserve_known_app_key_labels,
};
use attachment_upload::upload_profile_picture_to_hashtree;
use attachments::*;
use config::*;
pub(crate) use config::{
    app_version_string, build_summary, configured_relays, relay_set_id, trusted_test_build_flag,
};
use device_sync::*;
use identity::*;
pub(crate) use identity::{normalize_peer_input_for_display, parse_peer_input};
pub(crate) use mobile_push::{
    build_mobile_push_create_subscription_request, build_mobile_push_delete_subscription_request,
    build_mobile_push_list_subscriptions_request, build_mobile_push_update_subscription_request,
    decrypt_mobile_push_notification, mobile_push_stored_subscription_id_key,
    resolve_mobile_push_notification, resolve_mobile_push_server_url,
};
pub(crate) use model::ProtocolSubscriptionPlan;
use model::*;
use payloads::*;
use profile_helpers::*;
use protocol_filters::*;
use publish_helpers::*;
use storage::{open_database, AppStore, DataDirLock, SqliteStorageAdapter};
pub(crate) use storage::{search_messages_fts, PersistedMessageSearchHit, SharedConnection};

pub(crate) fn chat_snapshot_from_state_and_db(
    state: &AppState,
    shared_db: Option<&SharedConnection>,
    chat_id: &str,
    limit: usize,
) -> Option<CurrentChatSnapshot> {
    let chat_id = chat_id.trim();
    if chat_id.is_empty() || state.account.is_none() {
        return None;
    }
    if let Some(current) = state
        .current_chat
        .as_ref()
        .filter(|chat| chat.chat_id == chat_id)
    {
        return Some(current.clone());
    }

    build_chat_snapshot_with_messages(
        state,
        shared_db,
        chat_id,
        ChatPageRequest::Latest {
            limit: limit.max(1),
        },
    )
}

pub(crate) fn chat_snapshot_before_from_state_and_db(
    state: &AppState,
    shared_db: Option<&SharedConnection>,
    chat_id: &str,
    before_message_id: &str,
    limit: usize,
) -> Option<CurrentChatSnapshot> {
    build_chat_snapshot_with_messages(
        state,
        shared_db,
        chat_id,
        ChatPageRequest::Before {
            before_message_id,
            limit: limit.max(1),
        },
    )
}

pub(crate) fn chat_snapshot_around_message_from_state_and_db(
    state: &AppState,
    shared_db: Option<&SharedConnection>,
    chat_id: &str,
    message_id: &str,
    before_limit: usize,
    after_limit: usize,
) -> Option<CurrentChatSnapshot> {
    build_chat_snapshot_with_messages(
        state,
        shared_db,
        chat_id,
        ChatPageRequest::Around {
            message_id,
            before_limit,
            after_limit,
        },
    )
}

enum ChatPageRequest<'a> {
    Latest {
        limit: usize,
    },
    Before {
        before_message_id: &'a str,
        limit: usize,
    },
    Around {
        message_id: &'a str,
        before_limit: usize,
        after_limit: usize,
    },
}

fn build_chat_snapshot_with_messages(
    state: &AppState,
    shared_db: Option<&SharedConnection>,
    chat_id: &str,
    request: ChatPageRequest<'_>,
) -> Option<CurrentChatSnapshot> {
    let chat_id = chat_id.trim();
    if chat_id.is_empty() || state.account.is_none() {
        return None;
    }
    let thread = state.chat_list.iter().find(|chat| chat.chat_id == chat_id);
    let mut messages = load_chat_messages(shared_db, chat_id, request)?;
    let group_id = group_id_from_chat_id(chat_id);
    let kind = thread
        .map(|thread| thread.kind.clone())
        .unwrap_or_else(|| chat_kind_for_id(chat_id));
    let participants = state
        .current_chat
        .as_ref()
        .filter(|chat| chat.chat_id == chat_id)
        .map(|chat| chat.participants.clone())
        .unwrap_or_else(|| fallback_chat_participants(state, thread, chat_id, &kind));
    messages = messages
        .into_iter()
        .map(|message| {
            decorate_chat_page_message(
                message,
                chat_id,
                &kind,
                state.account.as_ref(),
                &participants,
            )
        })
        .collect();
    Some(CurrentChatSnapshot {
        chat_id: chat_id.to_string(),
        kind,
        display_name: thread
            .map(|thread| thread.display_name.clone())
            .unwrap_or_else(|| fallback_chat_title(chat_id)),
        nickname: thread.and_then(|thread| thread.nickname.clone()),
        profile_name: thread.and_then(|thread| thread.profile_name.clone()),
        subtitle: thread
            .and_then(|thread| thread.subtitle.clone())
            .or_else(|| group_id.as_ref().map(|_| "Group".to_string())),
        picture_url: thread.and_then(|thread| thread.picture_url.clone()),
        about: thread.and_then(|thread| thread.about.clone()),
        group_id,
        member_count: thread.map(|thread| thread.member_count).unwrap_or(0),
        message_ttl_seconds: None,
        is_muted: thread.map(|thread| thread.is_muted).unwrap_or(false),
        participants,
        messages,
        typing_indicators: Vec::new(),
        draft: thread
            .map(|thread| thread.draft.clone())
            .unwrap_or_default(),
        is_request: thread.map(|thread| thread.is_request).unwrap_or(false),
    })
}

fn load_chat_messages(
    shared_db: Option<&SharedConnection>,
    chat_id: &str,
    request: ChatPageRequest<'_>,
) -> Option<Vec<ChatMessageSnapshot>> {
    let Some(shared) = shared_db else {
        return match request {
            ChatPageRequest::Latest { .. } => Some(Vec::new()),
            ChatPageRequest::Before { .. } | ChatPageRequest::Around { .. } => None,
        };
    };
    let Ok(conn) = shared.try_lock() else {
        return match request {
            ChatPageRequest::Latest { .. } => Some(Vec::new()),
            ChatPageRequest::Before { .. } | ChatPageRequest::Around { .. } => None,
        };
    };
    let result = match request {
        ChatPageRequest::Latest { limit } => storage::load_recent_messages(&conn, chat_id, limit),
        ChatPageRequest::Before {
            before_message_id,
            limit,
        } => storage::load_messages_before(&conn, chat_id, before_message_id, limit),
        ChatPageRequest::Around {
            message_id,
            before_limit,
            after_limit,
        } => storage::load_messages_around(&conn, chat_id, message_id, before_limit, after_limit),
    };
    let Ok(messages) = result else {
        return match request {
            ChatPageRequest::Latest { .. } => Some(Vec::new()),
            ChatPageRequest::Before { .. } | ChatPageRequest::Around { .. } => None,
        };
    };
    Some(
        messages
            .iter()
            .map(chats::chat_message_from_persisted)
            .collect(),
    )
}

fn fallback_chat_participants(
    state: &AppState,
    thread: Option<&ChatThreadSnapshot>,
    chat_id: &str,
    kind: &ChatKind,
) -> Vec<ChatParticipantSnapshot> {
    let mut participants = Vec::new();
    if let Some(account) = state.account.as_ref() {
        participants.push(ChatParticipantSnapshot {
            owner_pubkey_hex: account.public_key_hex.clone(),
            display_name: account.display_name.trim().to_string(),
            picture_url: account.picture_url.clone(),
            is_local_owner: true,
        });
    }
    if matches!(kind, ChatKind::Direct)
        && state
            .account
            .as_ref()
            .is_none_or(|account| account.public_key_hex != chat_id)
    {
        participants.push(ChatParticipantSnapshot {
            owner_pubkey_hex: chat_id.to_string(),
            display_name: thread
                .map(|thread| thread.display_name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| profile::fallback_profile_name_for_identity(chat_id)),
            picture_url: thread.and_then(|thread| thread.picture_url.clone()),
            is_local_owner: false,
        });
    }
    participants
}

fn decorate_chat_page_message(
    mut message: ChatMessageSnapshot,
    chat_id: &str,
    kind: &ChatKind,
    account: Option<&AccountSnapshot>,
    participants: &[ChatParticipantSnapshot],
) -> ChatMessageSnapshot {
    let author_owner = message.author_owner_pubkey_hex.clone().or_else(|| {
        if matches!(message.kind, ChatMessageKind::System) {
            None
        } else if message.is_outgoing {
            account.map(|account| account.public_key_hex.clone())
        } else if matches!(kind, ChatKind::Direct) {
            Some(chat_id.to_string())
        } else {
            None
        }
    });
    if let Some(owner) = author_owner {
        if let Some(participant) = participant_for_owner(participants, &owner) {
            message.author = participant.display_name.clone();
            message.author_picture_url = participant.picture_url.clone();
        }
        message.author_owner_pubkey_hex = Some(owner);
    }
    message.recipient_deliveries = message
        .recipient_deliveries
        .into_iter()
        .map(|delivery| decorate_chat_page_delivery(delivery, participants))
        .collect();
    message.reactors = message
        .reactors
        .into_iter()
        .map(|reactor| decorate_chat_page_reactor(reactor, participants))
        .collect();
    message
}

fn decorate_chat_page_delivery(
    mut delivery: MessageRecipientDeliverySnapshot,
    participants: &[ChatParticipantSnapshot],
) -> MessageRecipientDeliverySnapshot {
    if let Some(participant) = participant_for_owner(participants, &delivery.owner_pubkey_hex) {
        delivery.display_name = participant.display_name.clone();
        delivery.picture_url = participant.picture_url.clone();
    } else if delivery.display_name.trim().is_empty() {
        delivery.display_name =
            profile::fallback_profile_name_for_identity(&delivery.owner_pubkey_hex);
    }
    delivery
}

fn decorate_chat_page_reactor(
    mut reactor: MessageReactor,
    participants: &[ChatParticipantSnapshot],
) -> MessageReactor {
    if let Some(participant) = participant_for_owner(participants, &reactor.author) {
        reactor.display_name = participant.display_name.clone();
        reactor.picture_url = participant.picture_url.clone();
    } else if reactor.display_name.trim().is_empty() {
        reactor.display_name = profile::fallback_profile_name_for_identity(&reactor.author);
    }
    reactor
}

fn participant_for_owner<'a>(
    participants: &'a [ChatParticipantSnapshot],
    owner_pubkey_hex: &str,
) -> Option<&'a ChatParticipantSnapshot> {
    participants
        .iter()
        .find(|participant| participant.owner_pubkey_hex == owner_pubkey_hex)
}

fn group_id_from_chat_id(chat_id: &str) -> Option<String> {
    chat_id
        .strip_prefix("group:")
        .filter(|group_id| !group_id.trim().is_empty())
        .map(ToString::to_string)
}

fn fallback_chat_title(chat_id: &str) -> String {
    if is_group_chat_id(chat_id) {
        return "Group".to_string();
    }
    let trimmed = chat_id.trim();
    let boundary = trimmed
        .char_indices()
        .map(|(index, _)| index)
        .nth(12)
        .unwrap_or(trimmed.len());
    if boundary < trimmed.len() {
        format!("{}...", &trimmed[..boundary])
    } else {
        trimmed.to_string()
    }
}

pub struct AppCore {
    update_tx: Sender<AppUpdate>,
    core_sender: Sender<CoreMsg>,
    priority_sender: Sender<CoreMsg>,
    shared_state: Arc<RwLock<AppState>>,
    runtime: tokio::runtime::Runtime,
    data_dir: PathBuf,
    state: AppState,
    logged_in: Option<LoggedInState>,
    protocol_engine: Option<ProtocolEngine>,
    pending_linked_device: Option<PendingLinkedDeviceState>,
    device_approval_relay_urls: Vec<RelayUrl>,
    private_chat_invites: BTreeMap<String, Invite>,
    threads: BTreeMap<String, ThreadRecord>,
    active_chat_id: Option<String>,
    screen_stack: Vec<Screen>,
    next_message_id: u64,
    owner_profiles: BTreeMap<String, OwnerProfileRecord>,
    profile_metadata_fetch_inflight: HashSet<String>,
    app_keys: BTreeMap<String, KnownAppKeys>,
    groups: BTreeMap<String, GroupSnapshot>,
    group_pictures: BTreeMap<String, String>,
    typing_indicators: BTreeMap<String, TypingIndicatorRecord>,
    /// Monotonic per-chat ceiling on `last_event_secs` we'll accept
    /// for incoming typing events. Bumped to the wire-clock
    /// timestamp of every message that lands in the thread. Defends
    /// against peer clients (notably iris-chat web) that don't send
    /// a stop-typing event when the user hits send: a stray typing
    /// rumor with the same wire-second as the message — or a
    /// re-delivery from a different device — never re-arms the
    /// indicator once we've already seen the message. Not persisted;
    /// rebuilt from `threads.messages.last()` on session start.
    typing_floor_secs: BTreeMap<String, u64>,
    chat_message_ttl_seconds: BTreeMap<String, u64>,
    preferences: PreferencesSnapshot,
    recent_handshake_peers: BTreeMap<String, RecentHandshakePeer>,
    seen_event_ids: HashSet<String>,
    seen_event_order: VecDeque<String>,
    device_invite_poll_token: u64,
    message_expiry_token: u64,
    protocol_reconnect_token: u64,
    protocol_liveness_token: u64,
    defer_owner_app_keys_publish: bool,
    current_device_labels: Option<CurrentDeviceLabels>,
    protocol_subscription_runtime: ProtocolSubscriptionRuntime,
    relay_transport_runtime: RelayTransportRuntime,
    relay_status_watch_urls: HashSet<String>,
    relay_status_watch_generation: u64,
    relay_status_by_url: BTreeMap<String, RelayStatus>,
    relay_connected_count: u64,
    all_relays_offline_since_secs: Option<u64>,
    device_sync: Option<DeviceSyncRuntime>,
    pending_relay_publishes: BTreeMap<String, PendingRelayPublish>,
    pending_relay_publish_inflight: HashSet<String>,
    pending_decrypted_delivery_acks: HashSet<String>,
    event_transport_channels: BTreeMap<String, String>,
    pending_mobile_push_events: VecDeque<Event>,
    debug_log: VecDeque<DebugLogEntry>,
    debug_event_counters: DebugEventCounters,
    debug_snapshot_write_generation: u64,
    debug_snapshot_write_inflight: bool,
    debug_snapshot_write_dirty: bool,
    /// Wall-clock millis of the last debug-snapshot file write. The
    /// snapshot is purely a test harness fixture (only `core/tests`,
    /// iOS InteropHarnessTests, and the Android RealRelayHarnessTest
    /// read it) — never read in production. We throttle to one
    /// rebuild per `DEBUG_SNAPSHOT_MIN_INTERVAL_MS` so a busy chat
    /// can't rebuild a full SessionManager clone × N known users on
    /// every relay event (the macOS CPU loop and the
    /// sluggish-over-time regression both traced back here).
    debug_snapshot_last_built_at_ms: u64,
    /// Cumulative call count of `build_runtime_debug_snapshot`. Read
    /// by `core_perf_counters()` so the release gate can budget core
    /// hot-loop work, not just FFI surface traffic.
    debug_snapshot_build_count: u64,
    /// Reentrancy guard: while > 0, `rebuild_state` / `emit_state` /
    /// `persist_best_effort` only set the matching dirty flag. The outermost
    /// `exit_batch()` call performs a single rebuild + persist + emit so a
    /// catch-up burst of N events triggers one UI re-render instead of N.
    batch_depth: u32,
    batch_dirty_state: bool,
    batch_dirty_persist: bool,
    /// Outgoing read-receipts queued during the current batch. Each
    /// `send_receipt` call inside an `enter_batch()/exit_batch()` scope
    /// pushes its message ids here keyed by `(chat_id, receipt_type)`,
    /// and the outermost `exit_batch()` flushes them as one relay event
    /// per (chat, type). Without this a 10-message catch-up would emit
    /// 10 separate `delivered` events; with it, one event with 10 e-tags.
    pending_outgoing_receipts: BTreeMap<(String, String), Vec<String>>,
    /// Incoming `delivered` receipts wait briefly before being sent. If the
    /// user opens the chat immediately, the `seen` receipt cancels them so we
    /// don't emit both receipt types for the same message in the same breath.
    pending_delivered_receipts: BTreeMap<(String, String), Instant>,
    pending_delivered_receipt_flush_due_at: Option<Instant>,
    pending_delivered_receipt_token: u64,
    /// Last `AppState` we successfully pushed across the FFI boundary, kept
    /// so `emit_state_inner` can skip pushes that don't change anything
    /// user-visible (a full `AppState` JNI marshal + Compose recomposition
    /// is ~400-1000 ms on Android debug).
    last_emitted_state: Option<AppState>,
    /// SQLite-backed durable storage for app state and NDR ratchet
    /// state. See `core/storage/`.
    app_store: AppStore,
    /// Process-wide writer/runtime guard for this data directory.
    /// Read-only helpers deliberately skip it so notification previews
    /// can inspect SQLite without racing the ratchet state.
    _data_dir_lock: DataDirLock,
    /// Cached `MobilePushSyncSnapshot`. Computing it walks every NDR
    /// session state and runs `serde_json::to_string` on each — that
    /// was ~440 ms per `rebuild_state`, dominating tap-to-render. The
    /// inputs change rarely (only when we accept an invite, pair a new
    /// device, or rotate the ratchet), so we cache the snapshot and
    /// only recompute when `mobile_push_dirty` is set.
    cached_mobile_push: MobilePushSyncSnapshot,
    mobile_push_dirty: bool,
    /// Set during `prepare_for_suspend` and cleared on `AppForegrounded`.
    /// While suspended, `handle_internal` drops queued background work
    /// (relay events, retries, polls, etc.) instead of writing to SQLite.
    /// Why: iOS terminates suspended apps that are mid-SQLite-write with
    /// RUNNINGBOARD 0xdead10cc, and the FFI message queue can hold relay
    /// events that arrived just before the scene phase change.
    suspended: bool,
}

async fn connect_client_with_timeout(client: &Client, timeout: Duration) {
    client.connect().await;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if connected_relay_count_for_client(client).await > 0 {
            return;
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn connected_relay_count_for_client(client: &Client) -> usize {
    client
        .relays()
        .await
        .values()
        .filter(|relay| relay.status() == RelayStatus::Connected)
        .count()
}
