use crate::actions::AppAction;
use crate::state::{
    AccountSnapshot, AppState, ChatKind, ChatMessageKind, ChatMessageSnapshot, ChatThreadSnapshot,
    CurrentChatSnapshot, DeliveryState, DeviceAuthorizationState, DeviceEntrySnapshot,
    DeviceRosterSnapshot, GroupDetailsSnapshot, GroupMemberSnapshot, MessageAttachmentSnapshot,
    MessageReactionSnapshot, MobilePushNotificationResolution, MobilePushSessionSnapshot,
    MobilePushSubscriptionRequest, MobilePushSyncSnapshot, NetworkStatusSnapshot,
    OutgoingAttachment, PreferencesSnapshot, PublicInviteSnapshot, Router, Screen,
    TypingIndicatorSnapshot,
};
use crate::updates::{AppUpdate, CoreMsg, InternalEvent};
use flume::Sender;
use nostr::{EventBuilder, UnsignedEvent};
use nostr_double_ratchet::{
    add_group_admin, add_group_member, apply_metadata_update, build_direct_message_backfill_filter,
    is_app_keys_event, parse_group_metadata, remove_group_admin, remove_group_member,
    update_group_data, validate_metadata_creation, validate_metadata_update, AppKeys,
    CreateGroupOptions, DeviceEntry, DirectMessageSubscriptionTracker, FanoutGroupMetadataOptions,
    FileStorageAdapter, GroupData, GroupDecryptedEvent, GroupSendEvent, GroupUpdate, Invite,
    MetadataValidation, NdrRuntime, SendOptions, SessionManagerEvent, SessionState, StorageAdapter,
    APP_KEYS_EVENT_KIND, CHAT_MESSAGE_KIND, CHAT_SETTINGS_KIND, GROUP_METADATA_KIND,
    GROUP_SENDER_KEY_DISTRIBUTION_KIND, INVITE_EVENT_KIND, INVITE_RESPONSE_KIND,
    MESSAGE_EVENT_KIND, REACTION_KIND, RECEIPT_KIND, TYPING_KIND,
};
use nostr_sdk::prelude::{
    Client, Event, Filter, Keys, Kind, PublicKey, RelayPoolNotification, RelayUrl, SubscriptionId,
    Timestamp, ToBech32,
};
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

type OwnerPubkey = PublicKey;
type DevicePubkey = PublicKey;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct UnixSeconds(u64);

impl UnixSeconds {
    pub(super) fn get(self) -> u64 {
        self.0
    }
}

use account::{known_app_keys_from_ndr, known_app_keys_to_ndr};
use attachment_upload::{
    display_filename, upload_file_to_hashtree, upload_profile_picture_to_blossom,
};
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
    owner_profiles: BTreeMap<String, OwnerProfileRecord>,
    app_keys: BTreeMap<String, KnownAppKeys>,
    groups: BTreeMap<String, GroupData>,
    typing_indicators: BTreeMap<String, TypingIndicatorRecord>,
    chat_message_ttl_seconds: BTreeMap<String, u64>,
    preferences: PreferencesSnapshot,
    recent_handshake_peers: BTreeMap<String, RecentHandshakePeer>,
    seen_event_ids: HashSet<String>,
    seen_event_order: VecDeque<String>,
    device_invite_poll_token: u64,
    protocol_subscription_runtime: ProtocolSubscriptionRuntime,
    direct_message_subscriptions: DirectMessageSubscriptionTracker,
    debug_log: VecDeque<DebugLogEntry>,
    debug_event_counters: DebugEventCounters,
}
