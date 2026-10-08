use super::*;

const MAX_PENDING_PREFIXES: usize = 960;

pub(super) struct HistoryPartition {
    filter: Filter,
    link_id: Option<String>,
    pending: VecDeque<String>,
    withheld: bool,
    deferred_mutation: bool,
    imported_original: bool,
    retried_mutations: bool,
}
impl HistoryPartition {
    pub(super) fn new(filter: Filter, link_id: Option<String>) -> Self {
        Self {
            filter,
            link_id,
            pending: VecDeque::from([String::new()]),
            withheld: false,
            deferred_mutation: false,
            imported_original: false,
            retried_mutations: false,
        }
    }
    fn split(&mut self, prefix: &str) -> bool {
        if prefix.len() >= 64 || self.pending.len() + 16 > MAX_PENDING_PREFIXES {
            return false;
        }
        for digit in b"0123456789abcdef".iter().rev() {
            self.pending
                .push_front(format!("{prefix}{}", *digit as char));
        }
        true
    }
}
pub(super) fn valid_prefix(prefix: &str) -> bool {
    prefix.len() <= 64
        && prefix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl AppCore {
    pub(super) fn cancel_device_history_partition(&mut self, peer: &str, scope: RecordScope) {
        let initial = self
            .device_sync
            .as_mut()
            .and_then(|runtime| {
                runtime
                    .history
                    .partitions
                    .remove(&(peer.to_string(), scope))
            })
            .is_some_and(|plan| plan.link_id.is_some());
        if initial {
            self.update_device_history_progress(peer, crate::DeviceHistorySyncPhase::Waiting, None);
        }
    }
    pub(super) fn split_device_history_partition(&mut self, peer: &str, state: &HistorySession) {
        let split = self
            .device_sync
            .as_mut()
            .and_then(|runtime| {
                runtime
                    .history
                    .partitions
                    .get_mut(&(peer.to_string(), state.scope))
            })
            .is_some_and(|plan| plan.split(&state.prefix));
        if split {
            self.advance_device_history_partition(peer, state.scope);
        } else {
            self.cancel_device_history_partition(peer, state.scope);
        }
    }
    pub(super) fn advance_device_history_partition(&mut self, peer: &str, scope: RecordScope) {
        loop {
            if !self.device_sync_peer_is_authorized(peer) {
                self.cancel_device_history_partition(peer, scope);
                return;
            }
            let Some(runtime) = &mut self.device_sync else {
                return;
            };
            let capacity = MAX_TOTAL_RECORDS.saturating_sub(
                runtime
                    .history
                    .sessions
                    .values()
                    .map(|s| s.records.len())
                    .sum(),
            );
            if capacity == 0 || runtime.history.sessions.len() >= MAX_SESSIONS {
                self.cancel_device_history_partition(peer, scope);
                return;
            }
            let Some(plan) = runtime
                .history
                .partitions
                .get_mut(&(peer.to_string(), scope))
            else {
                return;
            };
            let Some(prefix) = plan.pending.pop_front() else {
                return;
            };
            let filter = plan.filter;
            let link_id = plan.link_id.clone();
            let Some(mut state) = HistorySession::snapshot(
                self,
                filter,
                true,
                capacity,
                scope,
                &prefix,
                HistoryMutationAccess {
                    supported: true,
                    private_events: self.private_events_supported(peer),
                    target_since: self.device_history_mutation_target_since(peer),
                },
            ) else {
                let split = self
                    .device_sync
                    .as_mut()
                    .and_then(|runtime| {
                        runtime
                            .history
                            .partitions
                            .get_mut(&(peer.to_string(), scope))
                    })
                    .is_some_and(|plan| plan.split(&prefix));
                if split {
                    continue;
                }
                self.cancel_device_history_partition(peer, scope);
                return;
            };
            state.initial = link_id.is_some();
            if state.initial {
                self.update_device_history_progress(
                    peer,
                    crate::DeviceHistorySyncPhase::Discovering,
                    None,
                );
            }
            let Ok(frame) = state.engine.initiate() else {
                self.cancel_device_history_partition(peer, scope);
                return;
            };
            let session = hex(&rand::random::<[u8; 16]>());
            let packet = DeviceSyncPacket::HistoryOpen {
                v: 1,
                message_mutations: Some(1),
                private_events: self.private_events_supported(peer).then_some(1),
                session: session.clone(),
                scope,
                prefix: (!prefix.is_empty()).then_some(prefix),
                since: filter.since,
                until: filter.until,
                link_id,
                frame: hex(&frame),
            };
            if self.send_history_packets(peer, vec![packet]) {
                if let Some(runtime) = &mut self.device_sync {
                    runtime
                        .history
                        .sessions
                        .insert((peer.to_string(), session), state);
                }
            } else {
                self.cancel_device_history_partition(peer, scope);
            }
            return;
        }
    }
    pub(super) fn finish_device_history_partition(&mut self, peer: &str, state: HistorySession) {
        let key = (peer.to_string(), state.scope);
        let Some(runtime) = &mut self.device_sync else {
            return;
        };
        let Some(plan) = runtime.history.partitions.get_mut(&key) else {
            return;
        };
        plan.withheld |= state.withheld;
        plan.deferred_mutation |= state.deferred_mutation;
        plan.imported_original |= state.imported_original;
        // A target can live in a later hash partition. Retry once after actual
        // original import progress; absent/invalid targets cannot create a loop.
        if plan.pending.is_empty()
            && plan.deferred_mutation
            && plan.imported_original
            && !plan.retried_mutations
        {
            plan.pending.push_back(String::new());
            plan.withheld = false;
            plan.deferred_mutation = false;
            plan.imported_original = false;
            plan.retried_mutations = true;
        }
        if !plan.pending.is_empty() {
            self.advance_device_history_partition(peer, state.scope);
            return;
        }
        let Some(plan) = runtime.history.partitions.remove(&key) else {
            return;
        };
        let restart = runtime.history.restart.remove(&key);
        if plan.link_id.is_some() {
            if plan.withheld {
                self.update_device_history_progress(
                    peer,
                    crate::DeviceHistorySyncPhase::Waiting,
                    None,
                );
            } else if let Some(done) = self.complete_device_history_import(peer) {
                self.send_history_packets(peer, vec![done]);
            }
        }
        if let Some(since) = restart {
            self.start_device_reconcile(peer, since, state.scope);
        }
        if state.scope == RecordScope::State && !plan.withheld && restart.is_none() {
            let deferred = self.device_sync.as_mut().and_then(|runtime| {
                runtime.history.state_ready.insert(peer.to_string());
                runtime.history.deferred_history.remove(peer)
            });
            if let Some(since) = deferred {
                self.start_device_history(peer, since);
            }
        }
        if plan.link_id.is_some() {
            if let Some(record) = self.device_history_transfer(peer) {
                self.start_device_history(peer, record.link_at);
            }
        }
    }
}
