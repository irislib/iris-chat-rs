use crate::actions::AppAction;
use crate::state::{
    AccountSnapshot, AppState, ChatKind, ChatMessageSnapshot, ChatThreadSnapshot,
    CurrentChatSnapshot, DeliveryState, DeviceAuthorizationState, DeviceEntrySnapshot,
    DeviceRosterSnapshot, GroupDetailsSnapshot, GroupMemberSnapshot, MessageAttachmentSnapshot,
    MessageReactionSnapshot, MobilePushNotificationResolution, MobilePushSessionSnapshot,
    MobilePushSubscriptionRequest, MobilePushSyncSnapshot, NetworkStatusSnapshot,
    OutgoingAttachment, PreferencesSnapshot, PublicInviteSnapshot, Router, Screen,
    TypingIndicatorSnapshot,
};
use crate::updates::{AppUpdate, CoreMsg, InternalEvent};
use flume::Sender;
use nostr::EventBuilder;
use nostr_double_ratchet::{
    DevicePubkey, DeviceRoster, DomainError, Error, GroupIncomingEvent, GroupManager,
    GroupManagerSnapshot, GroupSnapshot, Invite, MessageEnvelope, OwnerPubkey, ProtocolContext,
    RelayGap, RosterEditor, SessionManager, SessionManagerSnapshot, SessionState, UnixSeconds,
};
use nostr_double_ratchet_nostr::nostr as codec;
use nostr_sdk::prelude::{
    Client, Event, Filter, Keys, Kind, PublicKey, RelayPoolNotification, RelayUrl, SecretKey,
    SubscriptionId, Timestamp, ToBech32,
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
mod chats;
mod config;
mod groups;
mod identity;
mod invites;
mod lifecycle;
mod mobile_push;
mod model;
mod payloads;
mod persistence;
mod profile;
mod profile_helpers;
mod projection;
mod protocol;
mod protocol_filters;
mod publish_helpers;
mod publishing;
mod relay;
mod routing;
mod support;
#[cfg(test)]
mod tests;

use attachments::*;
use config::*;
pub(crate) use config::{build_summary, configured_relays, relay_set_id, trusted_test_build_flag};
use identity::*;
pub(crate) use identity::{normalize_peer_input_for_display, parse_peer_input};
pub(crate) use mobile_push::{
    build_mobile_push_create_subscription_request, build_mobile_push_delete_subscription_request,
    build_mobile_push_list_subscriptions_request, build_mobile_push_update_subscription_request,
    mobile_push_stored_subscription_id_key, resolve_mobile_push_notification,
    resolve_mobile_push_server_url,
};
pub(crate) use model::ProtocolSubscriptionPlan;
use model::*;
use payloads::*;
use profile_helpers::*;
use protocol_filters::*;
use publish_helpers::*;

pub struct AppCore {
    update_tx: Sender<AppUpdate>,
    core_sender: Sender<CoreMsg>,
    shared_state: Arc<RwLock<AppState>>,
    runtime: tokio::runtime::Runtime,
    data_dir: PathBuf,
    state: AppState,
    logged_in: Option<LoggedInState>,
    threads: BTreeMap<String, ThreadRecord>,
    active_chat_id: Option<String>,
    screen_stack: Vec<Screen>,
    next_message_id: u64,
    pending_inbound: Vec<PendingInbound>,
    pending_outbound: Vec<PendingOutbound>,
    pending_group_controls: Vec<PendingGroupControl>,
    owner_profiles: BTreeMap<String, OwnerProfileRecord>,
    typing_indicators: BTreeMap<String, TypingIndicatorRecord>,
    chat_message_ttl_seconds: BTreeMap<String, u64>,
    preferences: PreferencesSnapshot,
    recent_handshake_peers: BTreeMap<String, RecentHandshakePeer>,
    seen_event_ids: HashSet<String>,
    seen_event_order: VecDeque<String>,
    device_invite_poll_token: u64,
    protocol_subscription_runtime: ProtocolSubscriptionRuntime,
    debug_log: VecDeque<DebugLogEntry>,
    debug_event_counters: DebugEventCounters,
}
