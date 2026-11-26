mod direct_messages;
mod protocol_engine;
mod storage;

use nostr::UnsignedEvent;
use nostr::{Event, Keys, PublicKey};
#[cfg(test)]
use nostr::{Kind, Timestamp};
use nostr_double_ratchet::{
    group_sender_key_message_event, invite_response_event, is_group_roster_fact_event,
    parse_group_roster_fact_event, parse_group_sender_key_message_event,
    parse_group_sender_key_message_event_unchecked, project_group_roster_fact_events,
    GroupEventManager, JsonGroupPayloadCodecV1,
};
use nostr_double_ratchet::{
    AuthorizedDevice, Delivery, DevicePubkey as NdrDevicePubkey, DeviceRoster, DomainError,
    Error as NdrError, GroupIncomingEvent, GroupManagerSnapshot, GroupPairwiseCommand,
    GroupPayloadCodec, GroupPendingFanout, GroupPreparedPublish, GroupPreparedSend, GroupProtocol,
    GroupSenderKeyHandleResult, GroupSenderKeyMessage, GroupSnapshot, MessageEnvelope,
    OwnerPubkey as NdrOwnerPubkey, PreparedSend, ProtocolContext, SenderKeyRepairRequest,
    SessionManager, UserRecordSnapshot,
};
use nostr_double_ratchet_pairwise_codec as pairwise_codec;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub use direct_messages::{
    DirectChatSnapshot, DirectInviteAcceptanceOutcome, DirectMessageCommand, DirectMessageDelivery,
    DirectMessageService, DirectMessageSnapshot, DirectThreadSnapshot,
};
pub use nostr_double_ratchet::{
    build_group_roster_fact_filter, build_protocol_discovery_filters, group_roster_unsigned_event,
    invite_unsigned_event, invite_url, is_app_keys_event, parse_invite_event,
    parse_invite_response_event, parse_invite_url, parse_message_event, AppKeys, DeviceEntry,
    GroupRosterFact, APP_KEYS_EVENT_KIND, CHAT_MESSAGE_KIND, CHAT_SETTINGS_KIND,
    GROUP_ROSTER_FACT_KIND, GROUP_ROSTER_FACT_SCHEMA, GROUP_ROSTER_FACT_TYPE,
    GROUP_SENDER_KEY_MESSAGE_KIND, INVITE_EVENT_KIND, INVITE_LIST_LABEL, INVITE_RESPONSE_KIND,
    MESSAGE_EVENT_KIND, REACTION_KIND, RECEIPT_KIND,
};
pub use nostr_double_ratchet::{
    Invite, SessionManagerSnapshot, SessionState, UnixSeconds as NdrUnixSeconds,
};
pub use protocol_engine::*;
pub use storage::{
    DebouncedFileStorage, FileStorageAdapter, InMemoryStorage, SqliteStorageAdapter,
    StorageAdapter, StorageCheckpoint, StorageCheckpointBody, StorageError, StorageResult,
};

const GROUP_ROSTER_FACT_EVENT_HISTORY_LIMIT: usize = 256;
pub const PROTOCOL_SENDER_KEY_REPAIR_RETRY_DELAYS_SECS: [u64; 5] = [10, 30, 60, 60, 60];
const PROTOCOL_SENDER_KEY_REPAIR_EXTENDED_RETRY_DELAYS_SECS: [u64; 8] =
    [10, 30, 60, 60, 60, 120, 180, 300];
const PROTOCOL_SENDER_KEY_REPAIR_UNCHANGED_RETRY_DELAYS_SECS: [u64; 3] = [300, 900, 3_600];
const PROTOCOL_SENDER_KEY_REPAIR_RESPONSE_RETRY_DELAYS_SECS: [u64; 5] = [10, 30, 60, 120, 300];
const PROTOCOL_SENDER_KEY_REPAIR_REVISION_RESPONSE_RETRY_DELAYS_SECS: [u64; 5] =
    [10, 30, 60, 60, 60];

fn protocol_sender_key_repair_retry_delay_secs(sent_request_count: u32) -> u64 {
    let index = sent_request_count
        .saturating_sub(1)
        .min((PROTOCOL_SENDER_KEY_REPAIR_EXTENDED_RETRY_DELAYS_SECS.len() - 1) as u32)
        as usize;
    PROTOCOL_SENDER_KEY_REPAIR_EXTENDED_RETRY_DELAYS_SECS[index]
}

fn protocol_sender_key_repair_retry_delay_secs_for(
    sent_request_count: u32,
    unchanged_response_rounds: u32,
) -> u64 {
    let base = protocol_sender_key_repair_retry_delay_secs(sent_request_count);
    if unchanged_response_rounds < 3 {
        return base;
    }
    let unchanged_index = unchanged_response_rounds
        .saturating_sub(3)
        .min((PROTOCOL_SENDER_KEY_REPAIR_UNCHANGED_RETRY_DELAYS_SECS.len() - 1) as u32)
        as usize;
    base.max(PROTOCOL_SENDER_KEY_REPAIR_UNCHANGED_RETRY_DELAYS_SECS[unchanged_index])
}

#[cfg(test)]
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

fn protocol_sender_key_repair_next_retry_at_for(
    now: NdrUnixSeconds,
    sent_request_count: u32,
    unchanged_response_rounds: u32,
) -> NdrUnixSeconds {
    NdrUnixSeconds(
        now.get()
            .saturating_add(protocol_sender_key_repair_retry_delay_secs_for(
                sent_request_count,
                unchanged_response_rounds,
            )),
    )
}

fn protocol_sender_key_repair_response_next_retry_at(
    now: NdrUnixSeconds,
    response_count: u32,
    revision_only: bool,
) -> NdrUnixSeconds {
    let delays = if revision_only {
        &PROTOCOL_SENDER_KEY_REPAIR_REVISION_RESPONSE_RETRY_DELAYS_SECS
    } else {
        &PROTOCOL_SENDER_KEY_REPAIR_RESPONSE_RETRY_DELAYS_SECS
    };
    let index = response_count
        .saturating_sub(1)
        .min((delays.len() - 1) as u32) as usize;
    NdrUnixSeconds(now.get().saturating_add(delays[index]))
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
