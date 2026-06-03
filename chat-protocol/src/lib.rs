mod nearby;
mod protocol_engine;
mod storage;

use nostr::{Alphabet, SingleLetterTag, UnsignedEvent};
use nostr::{Event, Filter, Keys, Kind, PublicKey, Timestamp};
use nostr_double_ratchet::{
    AuthorizedDevice, DevicePubkey as NdrDevicePubkey, DeviceRoster, DomainError,
    Error as NdrError, GroupIncomingEvent, GroupManagerSnapshot, GroupPairwiseCommand,
    GroupPayloadCodec, GroupPendingFanout, GroupPreparedPublish, GroupPreparedSend, GroupProtocol,
    GroupSenderKeyHandleResult, GroupSenderKeyMessage, GroupSnapshot, Invite, MessageEnvelope,
    OwnerPubkey as NdrOwnerPubkey, PreparedSend, ProtocolContext, RelayGap, SenderKeyRepairRequest,
    SessionManager, SessionManagerSnapshot, SessionState, UnixSeconds as NdrUnixSeconds,
};
use nostr_double_ratchet_nostr::{
    group_sender_key_message_event, invite_response_event, message_event,
    parse_group_sender_key_message_event, parse_group_sender_key_message_event_unchecked,
    parse_invite_event, parse_invite_response_event, parse_message_event, JsonGroupPayloadCodecV1,
    NostrGroupManager,
};
use nostr_double_ratchet_pairwise_codec as pairwise_codec;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub use nearby::{
    decode_nearby_envelope_frame, decode_nearby_envelope_json, decode_nearby_frame_json,
    encode_nearby_envelope_frame, encode_nearby_envelope_json, encode_nearby_frame_json,
    nearby_frame_body_len_from_header, read_nearby_frame, NearbyEnvelope, NearbyFrameAssembler,
    NearbyInventoryItem, NEARBY_ENVELOPE_VERSION, NEARBY_FRAME_HEADER_BYTES,
    NEARBY_MAX_FRAME_BODY_BYTES,
};
pub use nostr_double_ratchet_nostr::{
    is_app_keys_event, AppKeys, APP_KEYS_EVENT_KIND, CHAT_MESSAGE_KIND, CHAT_SETTINGS_KIND,
    GROUP_SENDER_KEY_MESSAGE_KIND, INVITE_EVENT_KIND, INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND,
    REACTION_KIND, RECEIPT_KIND,
};
pub use protocol_engine::*;
pub use storage::{
    DebouncedFileStorage, FileStorageAdapter, InMemoryStorage, SqliteStorageAdapter,
    StorageAdapter, StorageError, StorageResult,
};

const DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS: u64 = 30 * 24 * 60 * 60;
const DEVICE_INVITE_DISCOVERY_LIMIT: usize = 256;
const NDR_APP_KEYS_D_TAG: &str = "double-ratchet/app-keys";
const NDR_INVITES_L_TAG: &str = "double-ratchet/invites";
pub const PROTOCOL_SENDER_KEY_REPAIR_RETRY_DELAYS_SECS: [u64; 5] = [10, 30, 60, 60, 60];

fn protocol_sender_key_repair_retry_delay_secs(sent_request_count: u32) -> u64 {
    let index = sent_request_count
        .saturating_sub(1)
        .min((PROTOCOL_SENDER_KEY_REPAIR_RETRY_DELAYS_SECS.len() - 1) as u32)
        as usize;
    PROTOCOL_SENDER_KEY_REPAIR_RETRY_DELAYS_SECS[index]
}

fn protocol_sender_key_repair_next_retry_at(
    now: NdrUnixSeconds,
    sent_request_count: u32,
) -> NdrUnixSeconds {
    NdrUnixSeconds(
        now.get()
            .saturating_add(protocol_sender_key_repair_retry_delay_secs(
                sent_request_count,
            )),
    )
}

pub type SharedConnection = Arc<Mutex<rusqlite::Connection>>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnixSeconds(pub u64);

impl UnixSeconds {
    pub fn get(self) -> u64 {
        self.0
    }
}

fn unix_now() -> UnixSeconds {
    UnixSeconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}
