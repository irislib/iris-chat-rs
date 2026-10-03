use super::*;
use nostr_pubsub_reconcile::{Filter, Limits, Record, Session};
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
    pub(super) agreed: BTreeMap<String, u64>,
    pub(super) typed: BTreeSet<String>,
    restart: BTreeMap<(String, Option<RecordScope>), u64>,
    sessions: BTreeMap<(String, String), HistorySession>,
}

struct HistorySession {
    engine: Session,
    filter: Filter,
    records: BTreeMap<String, HistoryRecordRef>,
    scope: Option<RecordScope>,
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
    pending_messages: BTreeMap<String, DeviceSyncMessage>,
}

#[derive(Clone)]
enum HistoryRecordRef {
    Message(DeviceSyncCursor),
    Typed(DeviceSyncRecord),
}
impl HistoryRecordRef {
    fn timestamp(&self) -> u64 {
        match self {
            Self::Message(cursor) => cursor.created_at,
            Self::Typed(record) => record.timestamp(),
        }
    }
    fn id(&self) -> [u8; 32] {
        match self {
            Self::Message(cursor) => record_id(&cursor.chat_id, &cursor.id),
            Self::Typed(record) => record.id(),
        }
    }
    fn load(&self, core: &AppCore) -> Option<DeviceSyncRecord> {
        match self {
            Self::Message(cursor) => messages::load_history_message(core, cursor)
                .map(|message| DeviceSyncRecord::Message { message }),
            Self::Typed(record) => core.sync_record_allowed(record).then(|| record.clone()),
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
        scope: Option<RecordScope>,
    ) -> Option<Self> {
        let mut records = BTreeMap::new();
        let mut cursor = None;
        while scope != Some(RecordScope::State) {
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
                records.insert(
                    id,
                    HistoryRecordRef::Message(DeviceSyncCursor::from(&message)),
                );
                if records.len() > MAX_RECORDS.min(capacity) {
                    return None;
                }
            }
            if ended || next.is_none() {
                break;
            }
            cursor = next;
        }
        if let Some(scope) = scope {
            let extra = if scope == RecordScope::State {
                core.sync_state_records()
            } else {
                core.stored_sync_records()
            };
            for record in extra {
                if record.scope() == scope
                    && filter.contains(record.timestamp())
                    && core.sync_record_allowed(&record)
                {
                    records.insert(hex(&record.id()), HistoryRecordRef::Typed(record));
                    if records.len() > MAX_RECORDS.min(capacity) {
                        return None;
                    }
                }
            }
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
            pending_messages: BTreeMap::new(),
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
    pub(in crate::core) fn clear_device_history(&mut self, peer: &str) {
        if let Some(runtime) = &mut self.device_sync {
            runtime
                .history
                .sessions
                .retain(|(source, _), _| source != peer);
            runtime.history.agreed.remove(peer);
            runtime.history.typed.remove(peer);
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
        capability: Option<u8>,
        since: Option<u64>,
        record_capability: Option<u8>,
    ) {
        if page.is_none() {
            self.clear_device_history(peer);
        }
        self.negotiate_device_records(peer, record_capability);
        if capability != Some(1) {
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
    pub(super) fn start_device_state(&mut self, peer: &str) {
        if self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.typed.contains(peer))
        {
            self.start_device_reconcile(peer, 0, Some(RecordScope::State));
        }
    }
    pub(super) fn start_device_history(&mut self, peer: &str, agreed_since: u64) {
        let scope = self
            .device_sync
            .as_ref()
            .is_some_and(|runtime| runtime.history.typed.contains(peer))
            .then_some(RecordScope::History);
        self.start_device_reconcile(peer, agreed_since, scope);
    }
    fn start_device_reconcile(
        &mut self,
        peer: &str,
        agreed_since: u64,
        scope: Option<RecordScope>,
    ) {
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
            scope != Some(RecordScope::State)
                && !record.outbound
                && record.policy_known
                && !record.complete
                && record.since == 0
                && filter.since < record.link_at
        });
        if let Some(record) = &initial {
            filter.until = record.link_at.saturating_sub(1);
        }
        if scope == Some(RecordScope::State) {
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
        if runtime.history.sessions.len() >= MAX_SESSIONS {
            return;
        }
        let capacity = MAX_TOTAL_RECORDS.saturating_sub(
            runtime
                .history
                .sessions
                .values()
                .map(|state| state.records.len())
                .sum::<usize>(),
        );
        let Some(mut state) = HistorySession::snapshot(self, filter, true, capacity, scope) else {
            if scope.is_some() {
                if initial.is_some() {
                    self.update_device_history_progress(
                        peer,
                        crate::DeviceHistorySyncPhase::Waiting,
                        None,
                    );
                }
                // Message-only pagination cannot acknowledge missing typed controls.
                return;
            }
            if initial.is_some() {
                self.begin_device_history_fallback(peer);
            }
            // Oversized windows use bounded cursor pages with the exact private link operation.
            self.request_device_sync_snapshot(peer, Some(DeviceSyncPage::Messages { after: None }));
            return;
        };
        state.initial = initial.is_some();
        if state.initial {
            self.update_device_history_progress(
                peer,
                crate::DeviceHistorySyncPhase::Discovering,
                None,
            );
        }
        let Ok(frame) = state.engine.initiate() else {
            return;
        };
        let session = hex(&rand::random::<[u8; 16]>());
        let packet = DeviceSyncPacket::HistoryOpen {
            v: 1,
            session: session.clone(),
            scope,
            since: filter.since,
            link_id: initial.map(|record| record.link_id),
            until: filter.until,
            frame: hex(&frame),
        };
        if self.send_history_packets(peer, vec![packet]) {
            if let Some(runtime) = &mut self.device_sync {
                runtime
                    .history
                    .sessions
                    .insert((peer.to_string(), session), state);
            }
        }
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
        tcp.send_batch(peer, records)
    }

    pub(super) fn handle_device_history(&mut self, peer: &str, packet: DeviceSyncPacket) {
        match packet {
            DeviceSyncPacket::HistoryPageEnd {
                v: 1,
                link_at,
                link_id,
            } => {
                self.finish_device_history_fallback(peer, link_at, &link_id);
                return;
            }
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
            | DeviceSyncPacket::HistoryMessages { v: 1, session, .. }
            | DeviceSyncPacket::HistoryRecords { v: 1, session, .. }
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
            since,
            until,
            frame,
            link_id,
            scope,
            ..
        } = packet
        {
            if scope.is_some() && !runtime.history.typed.contains(peer) {
                return;
            }
            let state_scope = scope == Some(RecordScope::State);
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
            let Some(mut state) =
                HistorySession::snapshot(self, Filter { since, until }, false, capacity, scope)
            else {
                self.send_history_packets(
                    peer,
                    vec![DeviceSyncPacket::HistoryDone { v: 1, session }],
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
        let mut outgoing = Vec::new();
        let packet = match packet {
            DeviceSyncPacket::HistoryRecords {
                records, requested, ..
            } if state.initiator => {
                if state.scope.is_none()
                    || records.len() > 32
                    || records.iter().any(|record| {
                        Some(record.scope()) != state.scope
                            || !state.filter.contains(record.timestamp())
                            || !state.requested.contains(&hex(&record.id()))
                    })
                {
                    return;
                }
                for record in records {
                    state.pending_records.insert(hex(&record.id()), record);
                }
                let mut messages = Vec::new();
                if !requested.is_empty() {
                    if requested.len() != state.requested.len()
                        || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()
                        || requested.iter().any(|id| !state.requested.contains(id))
                    {
                        return;
                    }
                    let records = std::mem::take(&mut state.pending_records);
                    self.enter_batch();
                    for (hash, record) in records {
                        if let DeviceSyncRecord::Message { message } = record {
                            messages.push(message);
                        } else {
                            if self.apply_sync_record(record) {
                                state.received.insert(hash);
                            } else {
                                state.withheld = true;
                            }
                        }
                    }
                    self.rebuild_state();
                    self.emit_state();
                    self.exit_batch();
                }
                DeviceSyncPacket::HistoryMessages {
                    v: 1,
                    session: session.clone(),
                    messages,
                    requested,
                }
            }
            packet => packet,
        };
        let accepted = match packet {
            DeviceSyncPacket::HistoryFrame { frame, .. } => match unhex(&frame, FRAME_BYTES) {
                Some(frame) if state.initiator => match state.engine.reconcile(&frame) {
                    Ok(step) => {
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
                            state.filter.contains(record.timestamp())
                                && (state.scope == Some(RecordScope::State)
                                    || current_peer_floor
                                        .is_some_and(|floor| record.timestamp() >= floor))
                        }) {
                            if state.initial {
                                if let DeviceSyncRecord::Message { message } = &mut record {
                                    self.attach_legacy_sync_reactions(message);
                                }
                            }
                            if state.scope.is_some() {
                                outgoing.push(DeviceSyncPacket::HistoryRecords {
                                    v: 1,
                                    session: session.clone(),
                                    records: vec![record],
                                    requested: Vec::new(),
                                });
                            } else if let DeviceSyncRecord::Message { message } = record {
                                outgoing.push(DeviceSyncPacket::HistoryMessages {
                                    v: 1,
                                    session: session.clone(),
                                    messages: vec![message],
                                    requested: Vec::new(),
                                });
                            }
                        }
                    }
                    outgoing.push(if state.scope.is_some() {
                        DeviceSyncPacket::HistoryRecords {
                            v: 1,
                            session: session.clone(),
                            records: Vec::new(),
                            requested: ids,
                        }
                    } else {
                        DeviceSyncPacket::HistoryMessages {
                            v: 1,
                            session: session.clone(),
                            messages: Vec::new(),
                            requested: ids,
                        }
                    });
                    true
                }
            }
            DeviceSyncPacket::HistoryMessages {
                messages,
                requested,
                ..
            } if state.initiator => {
                if messages.len() > 32
                    || requested.len() > 32
                    || (!requested.is_empty()
                        && (requested.len() != state.requested.len()
                            || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()))
                    || requested.iter().any(|id| !state.requested.contains(id))
                    || messages.iter().any(|message| {
                        !state.filter.contains(message.created_at)
                            || !state
                                .requested
                                .contains(&hex(&record_id(&message.chat_id, &message.id)))
                    })
                {
                    false
                } else {
                    for mut message in messages {
                        if !state.initial {
                            message.legacy_reactions = None;
                        }
                        state
                            .pending_messages
                            .insert(hex(&record_id(&message.chat_id, &message.id)), message);
                    }
                    if requested.is_empty() {
                        if let Some(runtime) = &mut self.device_sync {
                            runtime.history.sessions.insert(key, state);
                        }
                        return;
                    }
                    let messages = std::mem::take(&mut state.pending_messages)
                        .into_values()
                        .collect::<Vec<_>>();
                    let incoming = messages
                        .iter()
                        .map(|message| {
                            (
                                message.chat_id.clone(),
                                message.id.clone(),
                                hex(&record_id(&message.chat_id, &message.id)),
                                self.app_store
                                    .message_exists_or_deleted(
                                        &message.chat_id,
                                        Some(&message.id),
                                        None,
                                    )
                                    .unwrap_or(true),
                            )
                        })
                        .collect::<Vec<_>>();
                    self.enter_batch();
                    self.apply_device_sync_snapshot(
                        DeviceSyncSnapshot {
                            roster_at: state.filter.since,
                            messages,
                            ..DeviceSyncSnapshot::default()
                        },
                        self.device_history_receive_since(peer),
                    );
                    self.exit_batch();
                    self.persist_best_effort_inner();
                    let durable = incoming.iter().all(|(chat, id, _, _)| {
                        self.app_store
                            .message_exists_or_deleted(chat, Some(id), None)
                            .unwrap_or(false)
                    });
                    if !durable {
                        state.withheld = true;
                    }
                    for (_, _, hash, existed) in incoming {
                        if durable && state.received.insert(hash) && !existed {
                            state.batch_imported += 1;
                        }
                    }
                    if !requested.is_empty() {
                        if requested.iter().any(|id| !state.received.contains(id)) {
                            state.withheld = true;
                        }
                        for id in requested {
                            state.requested.remove(&id);
                            state.received.remove(&id);
                        }
                        if state.initial {
                            if !self.add_device_history_imported(peer, state.batch_imported) {
                                state.withheld = true;
                            }
                            let imported = self
                                .device_history_transfer(peer)
                                .map_or(0, |record| record.imported);
                            self.update_device_history_progress(
                                peer,
                                crate::DeviceHistorySyncPhase::Transferring,
                                state
                                    .complete
                                    .then_some(imported + state.missing.len() as u64)
                                    .filter(|_| state.scope.is_none()),
                            );
                        }
                        state.batch_imported = 0;
                    }
                    true
                }
            }
            DeviceSyncPacket::HistoryDone { .. } => {
                if state.initiator && !state.finished() {
                    if state.initial {
                        self.begin_device_history_fallback(peer);
                    }
                    self.request_device_sync_snapshot(
                        peer,
                        Some(DeviceSyncPage::Messages { after: None }),
                    );
                }
                return;
            }
            _ => false,
        };
        if !accepted {
            self.send_history_packets(peer, vec![DeviceSyncPacket::HistoryDone { v: 1, session }]);
            return;
        }
        if let Some(need) = state.next_need(&session) {
            outgoing.push(need);
        }
        let finished = state.finished();
        let scope = state.scope;
        let resume_future = finished && state.initial;
        if resume_future {
            if !state.withheld {
                if let Some(done) = self.complete_device_history_import(peer) {
                    outgoing.push(done);
                }
            } else {
                self.update_device_history_progress(
                    peer,
                    crate::DeviceHistorySyncPhase::Waiting,
                    None,
                );
            }
        }
        if finished {
            outgoing.push(DeviceSyncPacket::HistoryDone { v: 1, session });
        }
        if (outgoing.is_empty() || self.send_history_packets(peer, outgoing))
            && !finished
            && self.device_sync_peer_is_authorized(peer)
        {
            if let Some(runtime) = &mut self.device_sync {
                runtime.history.sessions.insert(key, state);
            }
        }
        if finished {
            if let Some(since) = self
                .device_sync
                .as_mut()
                .and_then(|runtime| runtime.history.restart.remove(&(peer.to_string(), scope)))
            {
                self.start_device_reconcile(peer, since, scope);
            }
        }
        if let Some(record) = resume_future
            .then(|| self.device_history_transfer(peer))
            .flatten()
        {
            self.start_device_history(peer, record.link_at);
        }
    }
}

impl HistoryState {
    #[cfg(test)]
    pub(super) fn session_count(&self) -> usize {
        self.sessions.len()
    }
}
