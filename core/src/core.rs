use crate::actions::AppAction;
use crate::state::{
    AccountSnapshot, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot, ChatThreadSnapshot,
    CurrentChatSnapshot, DeliveryState, DeviceAuthorizationState, DeviceEntrySnapshot,
    DeviceRosterSnapshot, GroupDetailsSnapshot, GroupMemberSnapshot, LinkDeviceSnapshot,
    MessageAttachmentSnapshot, MessageDeliveryTraceSnapshot, MessageReactionSnapshot,
    MessageReactor, MessageRecipientDeliverySnapshot, MobilePushNotificationResolution,
    MobilePushSubscriptionRequest, MobilePushSyncSnapshot, NetworkStatusSnapshot,
    OutgoingAttachment, PeerProfileDebugSnapshot, PreferencesSnapshot, PublicInviteSnapshot,
    RelayConnectionSnapshot, Router, Screen, TypingIndicatorSnapshot,
};
use crate::updates::{AppUpdate, CoreMsg, InternalEvent};
use flume::Sender;
use nostr::{EventBuilder, UnsignedEvent};
use nostr_double_ratchet::{
    AuthorizedDevice, DevicePubkey as NdrDevicePubkey, DeviceRoster, GroupIncomingEvent,
    GroupManagerSnapshot, GroupSnapshot, Invite, MessageEnvelope, OwnerPubkey as NdrOwnerPubkey,
    PreparedSend, ProtocolContext, RelayGap, SessionManager, SessionManagerSnapshot, SessionState,
    UnixSeconds as NdrUnixSeconds,
};
use nostr_double_ratchet_nostr::{
    apply_app_keys_snapshot_with_required_device, is_app_keys_event, AppKeys, DeviceEntry,
    NostrGroupManager, APP_KEYS_EVENT_KIND, CHAT_MESSAGE_KIND, CHAT_SETTINGS_KIND,
    GROUP_SENDER_KEY_MESSAGE_KIND, INVITE_EVENT_KIND, INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND,
    REACTION_KIND, RECEIPT_KIND, TYPING_KIND,
};
use nostr_double_ratchet_nostr::{
    invite_response_event, message_event, parse_invite_event, parse_invite_response_event,
    parse_message_event,
};
use nostr_double_ratchet_pairwise_codec as pairwise_codec;
use nostr_double_ratchet_runtime::{
    build_direct_message_backfill_filter, DirectMessageSubscriptionTracker,
    NdrProtocolBackfillOptions, NdrRuntime, RuntimeEffect, SendOptions, StorageAdapter,
};
use nostr_sdk::prelude::{
    Client, Event, Filter, Keys, Kind, PublicKey, RelayNotification, RelayPoolNotification,
    RelayStatus, RelayUrl, SubscriptionId, Timestamp, ToBech32,
};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};

mod account;
mod attachment_upload;
mod attachments;
mod chat_reactions;
mod chat_receipts;
mod chat_settings;
mod chat_typing;
mod chats;
mod config;
mod groups;
mod identity;
mod invites;
mod lifecycle;
mod message_expiry;
mod mobile_push;
mod model;
mod nearby;
mod payloads;
mod persistence;
mod profile;
mod profile_helpers;
mod projection;
mod protocol;
mod protocol_engine;
mod protocol_filters;
mod publish_helpers;
mod publishing;
mod relay;
mod routing;
mod storage;
mod support;
#[cfg(test)]
mod tests;

pub(crate) const NEARBY_PRESENCE_KIND: u16 = 22242;

type OwnerPubkey = PublicKey;
type DevicePubkey = PublicKey;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct UnixSeconds(u64);

impl UnixSeconds {
    pub(super) fn get(self) -> u64 {
        self.0
    }
}

use account::known_app_keys_from_ndr;
use account::known_app_keys_to_ndr;
use attachment_upload::upload_profile_picture_to_hashtree;
use attachments::*;
use config::*;
pub(crate) use config::{
    app_version_string, build_summary, configured_relays, relay_set_id, trusted_test_build_flag,
};
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
use protocol_engine::*;
use protocol_filters::*;
use publish_helpers::*;
use storage::{
    import_legacy_ndr_storage, open_database, AppStore, DataDirLock, SqliteStorageAdapter,
};

pub struct AppCore {
    update_tx: Sender<AppUpdate>,
    core_sender: Sender<CoreMsg>,
    shared_state: Arc<RwLock<AppState>>,
    runtime: tokio::runtime::Runtime,
    data_dir: PathBuf,
    state: AppState,
    logged_in: Option<LoggedInState>,
    protocol_engine: Option<ProtocolEngine>,
    pending_linked_device: Option<PendingLinkedDeviceState>,
    private_chat_invites: BTreeMap<String, Invite>,
    threads: BTreeMap<String, ThreadRecord>,
    active_chat_id: Option<String>,
    screen_stack: Vec<Screen>,
    next_message_id: u64,
    owner_profiles: BTreeMap<String, OwnerProfileRecord>,
    app_keys: BTreeMap<String, KnownAppKeys>,
    groups: BTreeMap<String, GroupSnapshot>,
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
    defer_owner_app_keys_publish: bool,
    protocol_subscription_runtime: ProtocolSubscriptionRuntime,
    direct_message_subscriptions: DirectMessageSubscriptionTracker,
    relay_status_watch_urls: HashSet<String>,
    relay_connected_count: u64,
    all_relays_offline_since_secs: Option<u64>,
    pending_relay_publishes: BTreeMap<String, PendingRelayPublish>,
    pending_relay_publish_inflight: HashSet<String>,
    pending_decrypted_delivery_acks: HashSet<String>,
    event_transport_channels: BTreeMap<String, String>,
    pending_mobile_push_events: VecDeque<Event>,
    debug_log: VecDeque<DebugLogEntry>,
    debug_event_counters: DebugEventCounters,
    /// Reentrancy guard: while > 0, `rebuild_state` / `emit_state` /
    /// `persist_best_effort` only set the matching dirty flag. The outermost
    /// `exit_batch()` call performs a single rebuild + persist + emit so a
    /// catch-up burst of N events triggers one UI re-render instead of N.
    batch_depth: u32,
    batch_dirty_state: bool,
    batch_dirty_persist: bool,
    /// Owners we've already passed through `ndr_runtime.setup_user(...)`.
    /// `setup_user` is idempotent at the subscription level, but the work
    /// it triggers in `sync_direct_message_subscriptions` (walking every
    /// session, JSON-serialising state) is ~300ms per call on Android
    /// debug. Skipping known owners turns a 5 s per-tap cost into < 50 ms.
    setup_user_done: HashSet<String>,
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
}

async fn connect_client_with_timeout(client: &Client, timeout: Duration) {
    client.connect().await;
    client.wait_for_connection(timeout).await;
}
