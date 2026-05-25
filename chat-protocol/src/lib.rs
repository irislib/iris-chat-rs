mod protocol_engine;
mod storage;

use nostr::{Alphabet, SingleLetterTag, UnsignedEvent};
use nostr_double_ratchet::{
    sender_key_repair_default_next_retry_at, AuthorizedDevice, DevicePubkey as NdrDevicePubkey,
    DeviceRoster, DomainError, Error as NdrError, GroupIncomingEvent, GroupManagerSnapshot,
    GroupPendingFanout, GroupPreparedPublish, GroupPreparedSend, GroupProtocol,
    GroupSenderKeyHandleResult, GroupSenderKeyMessage, GroupSnapshot, Invite, MessageEnvelope,
    OwnerPubkey as NdrOwnerPubkey, PreparedSend, ProtocolContext, RelayGap, SenderKeyRepairRequest,
    SessionManager, SessionManagerSnapshot, SessionState, UnixSeconds as NdrUnixSeconds,
};
use nostr_double_ratchet_nostr::{
    group_sender_key_message_event, invite_response_event, message_event,
    parse_group_sender_key_message_event, parse_group_sender_key_message_event_unchecked,
    parse_invite_event, parse_invite_response_event, parse_message_event, AppKeys,
    NostrGroupManager, APP_KEYS_EVENT_KIND, INVITE_EVENT_KIND,
};
use nostr_double_ratchet_pairwise_codec as pairwise_codec;
use nostr_double_ratchet_runtime::StorageAdapter;
use nostr_sdk::prelude::{Event, Filter, Keys, Kind, PublicKey, Timestamp};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub use protocol_engine::*;
pub use storage::SqliteStorageAdapter;

const DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS: u64 = 30 * 24 * 60 * 60;
const DEVICE_INVITE_DISCOVERY_LIMIT: usize = 256;
const NDR_APP_KEYS_D_TAG: &str = "double-ratchet/app-keys";
const NDR_INVITES_L_TAG: &str = "double-ratchet/invites";

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
