use super::*;
mod partitions;
mod transfer;
use nostr_pubsub_reconcile::{Filter, Limits, Record, Session};
use partitions::HistoryPartition;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

const MAX_RECORDS: usize = 100_000;
const MAX_SESSIONS: usize = 64;
const MAX_TOTAL_RECORDS: usize = 200_000;
const FRAME_BYTES: usize = 16_384;
const TTL: Duration = Duration::from_secs(120);

#[derive(Default)]
pub(super) struct HistoryState {
    #[cfg(test)]
    record_limit: Option<usize>,
    pub(super) agreed: BTreeMap<String, u64>,
    pub(super) typed: BTreeSet<String>,
    private_events: BTreeSet<String>,
    state_ready: BTreeSet<String>,
    deferred_history: BTreeMap<String, u64>,
    partitions: BTreeMap<(String, RecordScope), HistoryPartition>,
    restart: BTreeMap<(String, RecordScope), u64>,
    sessions: BTreeMap<(String, String), HistorySession>,
}

struct HistorySession {
    engine: Session,
    filter: Filter,
    records: BTreeMap<String, HistoryRecordRef>,
    scope: RecordScope,
    prefix: String,
    pending_records: BTreeMap<String, DeviceSyncRecord>,
    initiator: bool,
    complete: bool,
    missing: VecDeque<String>,
    seen_missing: BTreeSet<String>,
    suppressed: BTreeSet<String>,
    requested: BTreeSet<String>,
    started: Instant,
    initial: bool,
    received: BTreeSet<String>,
    withheld: bool,
    batch_imported: u64,
    imported_original: bool,
    private_events: bool,
    deferred_mutation: bool,
}

#[derive(Clone, Copy)]
struct HistoryMutationAccess {
    supported: bool,
    private_events: bool,
    target_since: u64,
}

#[derive(Clone)]
enum HistoryRecordRef {
    Message(DeviceSyncCursor),
    Typed {
        id: [u8; 32],
        timestamp: u64,
        locator: RecordLocator,
    },
}
impl HistoryRecordRef {
    fn timestamp(&self) -> u64 {
        match self {
            Self::Message(cursor) => cursor.created_at,
            Self::Typed { timestamp, .. } => *timestamp,
        }
    }
    fn id(&self) -> [u8; 32] {
        match self {
            Self::Message(cursor) => record_id(&cursor.chat_id, &cursor.id),
            Self::Typed { id, .. } => *id,
        }
    }
    fn load(&self, core: &AppCore) -> Option<DeviceSyncRecord> {
        match self {
            Self::Message(cursor) => messages::load_history_message(core, cursor)
                .map(|message| DeviceSyncRecord::Message { message }),
            Self::Typed { id, locator, .. } => core
                .load_sync_record(locator)
                .filter(|record| record.id() == *id && core.sync_record_allowed(record)),
        }
    }
}

pub(super) fn record_id(chat_id: &str, id: &str) -> [u8; 32] {
    Sha256::digest(serde_json::json!([chat_id, id]).to_string().as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str, max: usize) -> Option<Vec<u8>> {
    if value.len() > max * 2
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && unhex(id, 32).is_some()
}

impl HistorySession {
    fn snapshot(
        core: &AppCore,
        filter: Filter,
        initiator: bool,
        capacity: usize,
        scope: RecordScope,
        prefix: &str,
        mutation_access: HistoryMutationAccess,
    ) -> Option<Self> {
        let mut records = BTreeMap::new();
        let mut cursor = None;
        if scope != RecordScope::State {
            loop {
                let (messages, next) =
                    collect_device_sync_messages(core, filter.since, cursor.as_ref(), 256);
                let mut ended = false;
                for message in messages {
                    if message.created_at > filter.until {
                        ended = true;
                        break;
                    }
                    if !messages::history_message_allowed(core, &message) {
                        continue;
                    }
                    let id = hex(&record_id(&message.chat_id, &message.id));
                    if !id.starts_with(prefix) {
                        continue;
                    }
                    records.insert(
                        id,
                        HistoryRecordRef::Message(DeviceSyncCursor::from(&message)),
                    );
                    if records.len() > core.device_history_record_limit().min(capacity) {
                        return None;
                    }
                }
                if ended || next.is_none() {
                    break;
                }
                cursor = next;
            }
        }
        let mut add = |record: DeviceSyncRecord| {
            if (mutation_access.supported
                || !matches!(record, DeviceSyncRecord::MessageMutation { .. }))
                && (mutation_access.private_events
                    || !matches!(record, DeviceSyncRecord::PrivateBlock { .. }))
                && record.scope() == scope
                && filter.contains(record.timestamp())
                && hex(&record.id()).starts_with(prefix)
                && core.sync_record_allowed(&record)
                && core.history_mutation_target_allowed(&record, mutation_access.target_since)
            {
                if let Some(locator) = record.locator() {
                    records.insert(
                        hex(&record.id()),
                        HistoryRecordRef::Typed {
                            id: record.id(),
                            timestamp: record.timestamp(),
                            locator,
                        },
                    );
                }
            }
            records.len() <= core.device_history_record_limit().min(capacity)
        };
        if scope == RecordScope::State {
            for group in core.build_device_sync_snapshot(0, false).groups {
                if !add(DeviceSyncRecord::Group { group }) {
                    return None;
                }
            }
        }
        core.visit_sync_records(&mut add).ok()?;
        if records.len() > core.device_history_record_limit().min(capacity) {
            return None;
        }
        let engine = Session::new(
            records.values().map(|cursor| Record {
                id: cursor.id(),
                timestamp: cursor.timestamp(),
            }),
            filter,
            Limits {
                max_records: MAX_RECORDS,
                max_frame_bytes: FRAME_BYTES,
                max_rounds: 256,
            },
        )
        .ok()?;
        Some(Self {
            engine,
            filter,
            records,
            scope,
            prefix: prefix.to_string(),
            pending_records: BTreeMap::new(),
            initiator,
            complete: false,
            missing: VecDeque::new(),
            seen_missing: BTreeSet::new(),
            suppressed: if initiator {
                core.app_store
                    .deleted_message_ids(MAX_RECORDS)
                    .ok()?
                    .into_iter()
                    .map(|(chat, id)| hex(&record_id(&chat, &id)))
                    .collect()
            } else {
                BTreeSet::new()
            },
            requested: BTreeSet::new(),
            started: Instant::now(),
            initial: false,
            received: BTreeSet::new(),
            withheld: false,
            batch_imported: 0,
            imported_original: false,
            private_events: mutation_access.private_events,
            deferred_mutation: false,
        })
    }

    fn next_need(&mut self, session: &str) -> Option<DeviceSyncPacket> {
        if !self.requested.is_empty() || self.missing.is_empty() {
            return None;
        }
        let ids = self
            .missing
            .drain(..self.missing.len().min(32))
            .collect::<Vec<_>>();
        self.requested.extend(ids.iter().cloned());
        Some(DeviceSyncPacket::HistoryNeed {
            v: 1,
            session: session.to_string(),
            ids,
        })
    }

    fn finished(&self) -> bool {
        self.initiator && self.complete && self.missing.is_empty() && self.requested.is_empty()
    }
}

impl AppCore {
    fn device_history_record_limit(&self) -> usize {
        #[cfg(test)]
        if let Some(limit) = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.history.record_limit)
        {
            return limit;
        }
        MAX_RECORDS
    }
    #[cfg(test)]
    pub(in crate::core) fn set_device_history_record_limit_for_test(&mut self, limit: usize) {
        self.device_sync.as_mut().unwrap().history.record_limit = Some(limit.max(1));
    }
    pub(in crate::core) fn clear_device_history(&mut self, peer: &str) {
        let cancelled = self.device_sync.as_ref().map_or_else(Vec::new, |runtime| {
            runtime
                .history
                .sessions
                .keys()
                .filter(|(source, _)| source == peer)
                .map(|(_, session)| DeviceSyncPacket::HistoryDone {
                    v: 1,
                    session: session.clone(),
                })
                .collect::<Vec<_>>()
        });
        // A metadata refresh can interrupt an advertised inventory. Close its
        // remote half before the new PageEnd starts replacement reconciliation.
        if !cancelled.is_empty() {
            self.send_history_packets(peer, cancelled);
        }
        if let Some(runtime) = &mut self.device_sync {
            runtime
                .history
                .sessions
                .retain(|(source, _), _| source != peer);
            runtime.history.agreed.remove(peer);
            runtime.history.typed.remove(peer);
            runtime.history.private_events.remove(peer);
            runtime.history.state_ready.remove(peer);
            runtime.history.deferred_history.remove(peer);
            runtime
                .history
                .partitions
                .retain(|(source, _), _| source != peer);
            runtime
                .history
                .restart
                .retain(|(source, _), _| source != peer);
        }
        if self
            .device_history_transfer(peer)
            .is_some_and(|record| !record.outbound && !record.complete && record.since == 0)
        {
            self.update_device_history_progress(peer, crate::DeviceHistorySyncPhase::Waiting, None);
        }
    }

    pub(super) fn negotiate_device_history(
        &mut self,
        peer: &str,
        roster_at: u64,
        page: Option<&DeviceSyncPage>,
        since: Option<u64>,
        record_capability: Option<u8>,
    ) {
        if page.is_none() {
            self.clear_device_history(peer);
        }
        self.negotiate_device_records(peer, record_capability);
        if record_capability != Some(1) {
            return;
        }
        let Some(local) = self.device_sync_roster_at() else {
            return;
        };
        let Some(peer_join) = self.device_sync_peer_since(peer) else {
            return;
        };
        let granted = self.device_history_send_since(peer);
        let floor = granted.unwrap_or(local.max(roster_at).max(peer_join));
        let agreed = floor.max(since.unwrap_or(floor));
        if agreed > unix_now().get() {
            return;
        }
        if let Some(runtime) = &mut self.device_sync {
            if runtime.history.agreed.len() < MAX_SESSIONS
                || runtime.history.agreed.contains_key(peer)
            {
                runtime.history.agreed.insert(peer.to_string(), agreed);
            }
        }
    }

    pub(super) fn negotiate_device_records(&mut self, peer: &str, capability: Option<u8>) {
        if capability == Some(1) && self.device_sync_peer_is_authorized(peer) {
            if let Some(runtime) = &mut self.device_sync {
                if runtime.history.typed.len() < MAX_SESSIONS {
                    runtime.history.typed.insert(peer.to_string());
                }
            }
        }
    }
    pub(super) fn negotiate_private_events(&mut self, peer: &str, capability: Option<u8>) {
        if capability == Some(1) && self.device_sync_peer_is_authorized(peer) {
            if let Some(runtime) = &mut self.device_sync {
                if runtime.history.private_events.len() < MAX_SESSIONS {
                    runtime.history.private_events.insert(peer.to_string());
                }
            }
        }
    }
    pub(super) fn private_events_supported(&self, peer: &str) -> bool {
        self.device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.private_events.contains(peer))
    }
    pub(super) fn start_device_state(&mut self, peer: &str) {
        if let Some(runtime) = &mut self.device_sync {
            runtime.history.state_ready.remove(peer);
        }
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.typed.contains(peer))
        {
            self.start_device_reconcile(peer, 0, RecordScope::State);
        }
    }
    pub(super) fn start_device_history(&mut self, peer: &str, agreed_since: u64) {
        if self.private_events_supported(peer) {
            if let Some(runtime) = &mut self.device_sync {
                if !runtime.history.state_ready.contains(peer) {
                    runtime
                        .history
                        .deferred_history
                        .insert(peer.to_string(), agreed_since);
                    return;
                }
            }
        }
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.typed.contains(peer))
        {
            self.start_device_reconcile(peer, agreed_since, RecordScope::History);
        }
    }
    fn start_device_reconcile(&mut self, peer: &str, agreed_since: u64, scope: RecordScope) {
        let Some(local) = self.device_sync_roster_at() else {
            return;
        };
        let mut filter = Filter {
            since: self
                .device_history_receive_since(peer)
                .unwrap_or(local)
                .max(agreed_since),
            until: unix_now().get(),
        };
        let initial = self.device_history_transfer(peer).filter(|record| {
            scope != RecordScope::State
                && !record.outbound
                && record.policy_known
                && !record.complete
                && record.since == 0
                && filter.since < record.link_at
        });
        if let Some(record) = &initial {
            filter.until = record.link_at.saturating_sub(1);
        }
        if scope == RecordScope::State {
            filter = Filter { since: 0, until: 0 };
        }
        if filter.since > filter.until {
            return;
        }
        let Some(runtime) = &mut self.device_sync else {
            return;
        };
        runtime
            .history
            .sessions
            .retain(|_, state| state.started.elapsed() < TTL);
        if runtime
            .history
            .sessions
            .iter()
            .any(|((source, _), state)| source == peer && state.initiator && state.scope == scope)
        {
            runtime
                .history
                .restart
                .insert((peer.to_string(), scope), agreed_since);
            return;
        }
        runtime.history.partitions.insert(
            (peer.to_string(), scope),
            HistoryPartition::new(filter, initial.map(|record| record.link_id)),
        );
        self.advance_device_history_partition(peer, scope);
    }

    pub(super) fn send_history_packets(&self, peer: &str, packets: Vec<DeviceSyncPacket>) -> bool {
        if !self.device_sync_peer_is_authorized(peer) {
            return false;
        }
        let Some((tcp, peer)) = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.tcp.as_ref().zip(fips_peer_from_hex(peer)))
        else {
            return false;
        };
        let Ok(records) = packets
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()
        else {
            return false;
        };
        if records
            .iter()
            .any(|bytes| bytes.len() > DEVICE_SYNC_MAX_PACKET_BYTES)
        {
            return false;
        }
        tcp.send_batch(peer, records)
    }

    pub(super) fn handle_device_history(&mut self, peer: &str, packet: DeviceSyncPacket) {
        match packet {
            DeviceSyncPacket::HistoryPolicy {
                v: 1,
                link_at,
                since,
                link_id,
            } => {
                self.handle_device_history_policy(peer, link_at, since, link_id);
                return;
            }
            DeviceSyncPacket::HistoryComplete {
                v: 1,
                link_at,
                link_id,
            } => {
                self.handle_device_history_complete(peer, link_at, link_id);
                return;
            }
            _ => {}
        }
        let session = match &packet {
            DeviceSyncPacket::HistoryOpen { v: 1, session, .. }
            | DeviceSyncPacket::HistoryFrame { v: 1, session, .. }
            | DeviceSyncPacket::HistoryNeed { v: 1, session, .. }
            | DeviceSyncPacket::HistoryRecords { v: 1, session, .. }
            | DeviceSyncPacket::HistoryOverflow { v: 1, session }
            | DeviceSyncPacket::HistoryDone { v: 1, session } => session.clone(),
            _ => return,
        };
        if session.len() != 32 || unhex(&session, 16).is_none() {
            return;
        }
        let key = (peer.to_string(), session.clone());
        let current_peer_floor = self.device_history_send_since(peer).or_else(|| {
            self.device_sync_peer_since(peer)
                .zip(self.device_sync_roster_at())
                .map(|(peer, local)| peer.max(local))
        });
        let peer_join = self.device_sync_peer_since(peer).unwrap_or(u64::MAX);
        let initial_permission = self
            .device_history_transfer(peer)
            .filter(|record| record.outbound && !record.complete && record.since == 0);
        let Some(runtime) = &mut self.device_sync else {
            return;
        };
        runtime
            .history
            .sessions
            .retain(|_, state| state.started.elapsed() < TTL);
        if let DeviceSyncPacket::HistoryOpen {
            message_mutations,
            private_events,
            since,
            until,
            frame,
            link_id,
            scope,
            prefix,
            ..
        } = packet
        {
            if !runtime.history.typed.contains(peer) {
                return;
            }
            let prefix = prefix.unwrap_or_default();
            if !partitions::valid_prefix(&prefix) {
                return;
            }
            let state_scope = scope == RecordScope::State;
            if state_scope && (since != 0 || until != 0 || link_id.is_some()) {
                return;
            }
            if !state_scope
                && since < peer_join
                && initial_permission.as_ref().is_none_or(|record| {
                    link_id.as_ref() != Some(&record.link_id) || until >= record.link_at
                })
            {
                return;
            }
            let Some(agreed) = runtime.history.agreed.get(peer).copied() else {
                return;
            };
            if (!state_scope
                && (current_peer_floor.is_none_or(|floor| since < floor) || since < agreed))
                || since > until
                || until > unix_now().get().saturating_add(300)
                || runtime.history.sessions.contains_key(&key)
            {
                return;
            }
            if runtime.history.sessions.iter().any(|((source, _), state)| {
                source == peer && !state.initiator && state.scope == scope
            }) || runtime.history.sessions.len() >= MAX_SESSIONS
            {
                return;
            }
            let Some(frame) = unhex(&frame, FRAME_BYTES) else {
                return;
            };
            let capacity = MAX_TOTAL_RECORDS.saturating_sub(
                runtime
                    .history
                    .sessions
                    .values()
                    .map(|state| state.records.len())
                    .sum::<usize>(),
            );
            let Some(mut state) = HistorySession::snapshot(
                self,
                Filter { since, until },
                false,
                capacity,
                scope,
                &prefix,
                HistoryMutationAccess {
                    supported: message_mutations == Some(1),
                    private_events: private_events == Some(1)
                        && self.private_events_supported(peer),
                    target_since: self.device_history_mutation_target_since(peer),
                },
            ) else {
                self.send_history_packets(
                    peer,
                    vec![DeviceSyncPacket::HistoryOverflow { v: 1, session }],
                );
                return;
            };
            state.initial = !state_scope && since < peer_join;
            let Ok(response) = state.engine.respond(&frame) else {
                return;
            };
            if self.send_history_packets(
                peer,
                vec![DeviceSyncPacket::HistoryFrame {
                    v: 1,
                    session: session.clone(),
                    frame: hex(&response),
                }],
            ) {
                if let Some(runtime) = &mut self.device_sync {
                    runtime.history.sessions.insert(key, state);
                }
            }
            return;
        }
        let Some(mut state) = runtime.history.sessions.remove(&key) else {
            return;
        };
        if matches!(packet, DeviceSyncPacket::HistoryOverflow { .. }) {
            if state.initiator && state.seen_missing.is_empty() && !state.complete {
                self.split_device_history_partition(peer, &state);
            } else if state.initiator {
                self.cancel_device_history_partition(peer, state.scope);
            }
            return;
        }
        let mut outgoing = Vec::new();
        let accepted = match packet {
            DeviceSyncPacket::HistoryFrame { frame, .. } => match unhex(&frame, FRAME_BYTES) {
                Some(frame) if state.initiator => match state.engine.reconcile(&frame) {
                    Ok(step) => {
                        if step
                            .need
                            .iter()
                            .any(|id| !hex(id).starts_with(&state.prefix))
                        {
                            self.cancel_device_history_partition(peer, state.scope);
                            return;
                        }
                        for id in step.need {
                            let id = hex(&id);
                            if !state.suppressed.contains(&id)
                                && state.seen_missing.insert(id.clone())
                            {
                                state.missing.push_back(id);
                            }
                        }
                        if state.seen_missing.len() > MAX_RECORDS {
                            false
                        } else {
                            state.complete = step.next.is_none();
                            if let Some(next) = step.next {
                                outgoing.push(DeviceSyncPacket::HistoryFrame {
                                    v: 1,
                                    session: session.clone(),
                                    frame: hex(&next),
                                });
                            }
                            true
                        }
                    }
                    Err(_) => false,
                },
                Some(frame) => match state.engine.respond(&frame) {
                    Ok(response) => {
                        outgoing.push(DeviceSyncPacket::HistoryFrame {
                            v: 1,
                            session: session.clone(),
                            frame: hex(&response),
                        });
                        true
                    }
                    Err(_) => false,
                },
                None => false,
            },
            DeviceSyncPacket::HistoryNeed { ids, .. } if !state.initiator => {
                if ids.is_empty()
                    || ids.len() > 32
                    || ids.iter().any(|id| {
                        !valid_id(id)
                            || !state.records.contains_key(id)
                            || state.seen_missing.contains(id)
                    })
                {
                    false
                } else {
                    state.seen_missing.extend(ids.iter().cloned());
                    for id in &ids {
                        let Some(cursor) = state.records.get(id) else {
                            continue;
                        };
                        if let Some(mut record) = cursor.load(self).filter(|record| {
                            self.history_mutation_target_allowed(
                                record,
                                self.device_history_mutation_target_since(peer),
                            ) && state.filter.contains(record.timestamp())
                                && (state.scope == RecordScope::State
                                    || current_peer_floor
                                        .is_some_and(|floor| record.timestamp() >= floor))
                        }) {
                            if state.initial {
                                if let DeviceSyncRecord::Message { message } = &mut record {
                                    self.attach_legacy_sync_reactions(message);
                                }
                            }
                            outgoing.push(DeviceSyncPacket::HistoryRecords {
                                v: 1,
                                session: session.clone(),
                                records: vec![record],
                                requested: Vec::new(),
                            });
                        }
                    }
                    outgoing.push(DeviceSyncPacket::HistoryRecords {
                        v: 1,
                        session: session.clone(),
                        records: Vec::new(),
                        requested: ids,
                    });
                    true
                }
            }
            DeviceSyncPacket::HistoryRecords {
                records, requested, ..
            } if state.initiator => {
                self.ingest_device_history_records(peer, &mut state, records, requested)
            }
            DeviceSyncPacket::HistoryDone { .. } => {
                if state.initiator {
                    self.cancel_device_history_partition(peer, state.scope);
                }
                return;
            }
            _ => false,
        };
        if !accepted {
            if state.initiator {
                self.cancel_device_history_partition(peer, state.scope);
            }
            self.send_history_packets(peer, vec![DeviceSyncPacket::HistoryDone { v: 1, session }]);
            return;
        }
        if let Some(need) = state.next_need(&session) {
            outgoing.push(need);
        }
        let finished = state.finished();
        if finished {
            outgoing.push(DeviceSyncPacket::HistoryDone { v: 1, session });
        }
        if !outgoing.is_empty() && !self.send_history_packets(peer, outgoing) {
            if state.initiator {
                self.cancel_device_history_partition(peer, state.scope);
            }
            return;
        }
        if finished {
            self.finish_device_history_partition(peer, state);
        } else if self.device_sync_peer_is_authorized(peer) {
            if let Some(runtime) = &mut self.device_sync {
                runtime.history.sessions.insert(key, state);
            }
        }
    }
}

impl HistoryState {
    #[cfg(test)]
    pub(super) fn session_count(&self) -> usize {
        self.sessions.len()
    }
}
