//! Pace failed TCP/TLS/WebSocket establishment across independent readers.
//! Only an exact server URL and a retry clock are shared. Query results, AUTH,
//! author keys, filters, sockets and TLS tickets remain in their original scopes.
use nostr::RelayUrl;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const MAX_SERVERS: usize = 64;
type Entry = Arc<Mutex<State>>;
static SERVERS: LazyLock<Mutex<BTreeMap<RelayUrl, Entry>>> = LazyLock::new(Mutex::default);

struct State {
    generation: u64,
    failures: u32,
    retry_at: Instant,
    probing: bool,
}

pub(super) fn clear() {
    SERVERS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

pub(super) struct Attempt {
    state: Option<Entry>,
    generation: u64,
    deadline: Instant,
    completed: bool,
}

impl Attempt {
    pub(super) fn begin(relay: &RelayUrl, deadline: Instant) -> anyhow::Result<Self> {
        let now = Instant::now();
        let entry = {
            let mut servers = SERVERS.lock().unwrap_or_else(|error| error.into_inner());
            if !servers.contains_key(relay) && servers.len() == MAX_SERVERS {
                // Do not evict an in-flight probe. An excess server can still
                // connect without caching, as with our bounded DNS cache.
                let idle = servers
                    .iter()
                    .find(|(_, entry)| Arc::strong_count(entry) == 1)
                    .map(|(url, _)| url.clone());
                if let Some(idle) = idle {
                    servers.remove(&idle);
                }
            }
            if servers.contains_key(relay) || servers.len() < MAX_SERVERS {
                Some(
                    servers
                        .entry(relay.clone())
                        .or_insert_with(|| {
                            Arc::new(Mutex::new(State {
                                generation: 0,
                                failures: 0,
                                retry_at: now,
                                probing: false,
                            }))
                        })
                        .clone(),
                )
            } else {
                None
            }
        };
        let mut generation = 0;
        if let Some(entry) = &entry {
            let mut state = entry.lock().unwrap_or_else(|error| error.into_inner());
            if state.failures > 0 {
                anyhow::ensure!(
                    now >= state.retry_at && !state.probing,
                    "anonymous server connection cooling down"
                );
                state.probing = true;
            }
            generation = state.generation;
        }
        Ok(Self {
            state: entry,
            generation,
            deadline,
            completed: false,
        })
    }

    fn finish(&self, success: bool) {
        let Some(entry) = &self.state else { return };
        let mut state = entry.lock().unwrap_or_else(|error| error.into_inner());
        // Concurrent failures from the same wave count once. Older successes
        // cannot clear a newer failure, nor can an old failure poison recovery.
        if state.generation != self.generation {
            return;
        }
        if success {
            if state.failures > 0 {
                state.generation = state.generation.wrapping_add(1);
                state.failures = 0;
            }
        } else {
            state.generation = state.generation.wrapping_add(1);
            state.failures = state.failures.saturating_add(1);
            let seconds = (5u64 << state.failures.saturating_sub(1).min(5)).min(120);
            state.retry_at = Instant::now() + Duration::from_secs(seconds);
        }
        state.probing = false;
    }

    pub(super) fn completed(mut self, success: bool) {
        self.finish(success);
        self.completed = true;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if Instant::now() >= self.deadline {
            // The enclosing read deadline cancels a stalled handshake.
            self.finish(false);
        } else if let Some(entry) = &self.state {
            let mut state = entry.lock().unwrap_or_else(|error| error.into_inner());
            if state.generation == self.generation {
                // Removing a reader before its deadline is not a server failure.
                state.probing = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_wave_and_cancelled_probe_preserve_recovery() {
        if super::super::tests::capacity_experiment_in_child("core::protocol::anonymous_reads::relay_backoff::tests::failure_wave_and_cancelled_probe_preserve_recovery") { return; }
        let url = RelayUrl::parse("wss://fixture.invalid/history").unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let first = Attempt::begin(&url, deadline).unwrap();
        let late_success = Attempt::begin(&url, deadline).unwrap();
        let late_failure = Attempt::begin(&url, deadline).unwrap();
        let entry = first.state.clone().unwrap();
        first.completed(false);
        late_success.completed(true);
        assert!(
            Attempt::begin(&url, deadline).is_err(),
            "older success must not undo cooldown"
        );
        entry.lock().unwrap().retry_at = Instant::now();
        let probe = Attempt::begin(&url, deadline).unwrap();
        assert!(
            Attempt::begin(&url, deadline).is_err(),
            "only one recovery probe"
        );
        drop(probe);
        Attempt::begin(&url, deadline).unwrap().completed(true);
        late_failure.completed(false);
        Attempt::begin(&url, deadline).unwrap().completed(true);
        assert_eq!(entry.lock().unwrap().failures, 0);
        let other = RelayUrl::parse("wss://fixture.invalid/other").unwrap();
        Attempt::begin(&url, deadline).unwrap().completed(false);
        Attempt::begin(&other, deadline).unwrap().completed(true);
        // Resetting network reachability must also detach old in-flight epochs.
        let old = Attempt::begin(&other, deadline).unwrap();
        clear();
        old.completed(false);
        Attempt::begin(&other, deadline).unwrap().completed(true);
    }

    #[test]
    fn only_expired_handshake_cancellation_marks_server_unavailable() {
        if super::super::tests::capacity_experiment_in_child("core::protocol::anonymous_reads::relay_backoff::tests::only_expired_handshake_cancellation_marks_server_unavailable") { return; }
        let url = RelayUrl::parse("wss://cancel.fixture.invalid").unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        drop(Attempt::begin(&url, deadline).unwrap());
        Attempt::begin(&url, deadline).unwrap().completed(true);
        drop(Attempt::begin(&url, Instant::now()).unwrap());
        assert!(Attempt::begin(&url, deadline).is_err());
    }
}
