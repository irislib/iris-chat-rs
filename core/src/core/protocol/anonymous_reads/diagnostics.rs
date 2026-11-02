//! Process-local counts only, read by existing local support/debug exports.
//! Worker fields cover PrivateDiscoveryClient, including unconfigured workers.
//! An initial sweep is the first complete attempt over all relays of an adopted
//! configuration, not proof every relay succeeded. Reconfiguration cancels an
//! unfinished initial sweep and starts another; repeat sweeps count completion.
//! Socket/TLS fields cover all anonymous_reads users, including cold/FIPS reads.
//! Successful opens count WebSocket establishment; failed opens include timeout
//! and cancellation before that point. TLS counts describe successful Rustls
//! opens; plain sockets contribute only to open counts.
//! Relaxed snapshots can straddle transitions; compare quiescent observations
//! only within the same process. No keys, URLs, filters or ticket state is stored.

use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::core) struct Snapshot {
    pub active_scopes: u64,
    pub initial_sweeps_pending: u64,
    pub initial_sweeps_completed: u64,
    pub initial_sweeps_cancelled: u64,
    pub repeat_sweeps: u64,
    pub refresh_requests: u64,
    pub config_resets: u64,
    pub retained_reuses: u64,
    pub retained_opens: u64,
    pub disposable_opens: u64,
    pub failed_opens: u64,
    pub tls_full: u64,
    pub tls_resumed: u64,
}

#[derive(Default)]
struct Counters {
    active_scopes: AtomicU64,
    initial_sweeps_pending: AtomicU64,
    initial_sweeps_completed: AtomicU64,
    initial_sweeps_cancelled: AtomicU64,
    repeat_sweeps: AtomicU64,
    refresh_requests: AtomicU64,
    config_resets: AtomicU64,
    retained_reuses: AtomicU64,
    retained_opens: AtomicU64,
    disposable_opens: AtomicU64,
    failed_opens: AtomicU64,
    tls_full: AtomicU64,
    tls_resumed: AtomicU64,
}

static COUNTERS: LazyLock<Counters> = LazyLock::new(Counters::default);

impl Counters {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            active_scopes: self.active_scopes.load(Ordering::Relaxed),
            initial_sweeps_pending: self.initial_sweeps_pending.load(Ordering::Relaxed),
            initial_sweeps_completed: self.initial_sweeps_completed.load(Ordering::Relaxed),
            initial_sweeps_cancelled: self.initial_sweeps_cancelled.load(Ordering::Relaxed),
            repeat_sweeps: self.repeat_sweeps.load(Ordering::Relaxed),
            refresh_requests: self.refresh_requests.load(Ordering::Relaxed),
            config_resets: self.config_resets.load(Ordering::Relaxed),
            retained_reuses: self.retained_reuses.load(Ordering::Relaxed),
            retained_opens: self.retained_opens.load(Ordering::Relaxed),
            disposable_opens: self.disposable_opens.load(Ordering::Relaxed),
            failed_opens: self.failed_opens.load(Ordering::Relaxed),
            tls_full: self.tls_full.load(Ordering::Relaxed),
            tls_resumed: self.tls_resumed.load(Ordering::Relaxed),
        }
    }
}

pub(in crate::core) fn snapshot() -> Snapshot {
    COUNTERS.snapshot()
}

pub(super) fn retained_reused() {
    COUNTERS.retained_reuses.fetch_add(1, Ordering::Relaxed);
}

pub(in crate::core) fn refresh_requested() {
    COUNTERS.refresh_requests.fetch_add(1, Ordering::Relaxed);
}

pub(in crate::core) struct Scope<'a> {
    counters: &'a Counters,
    configured: bool,
    initial_pending: bool,
}

impl Scope<'static> {
    pub(in crate::core) fn new() -> Self {
        Self::with_counters(&COUNTERS)
    }
}

impl<'a> Scope<'a> {
    fn with_counters(counters: &'a Counters) -> Self {
        counters.active_scopes.fetch_add(1, Ordering::Relaxed);
        Self {
            counters,
            configured: false,
            initial_pending: false,
        }
    }

    pub(in crate::core) fn configured(&mut self) {
        self.cancel_initial();
        if self.configured {
            self.counters.config_resets.fetch_add(1, Ordering::Relaxed);
        }
        self.configured = true;
        self.initial_pending = true;
        self.counters
            .initial_sweeps_pending
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(in crate::core) fn sweep_completed(&mut self) {
        if self.initial_pending {
            self.initial_pending = false;
            self.counters
                .initial_sweeps_pending
                .fetch_sub(1, Ordering::Relaxed);
            self.counters
                .initial_sweeps_completed
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.repeat_sweeps.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn cancel_initial(&mut self) {
        if self.initial_pending {
            self.initial_pending = false;
            self.counters
                .initial_sweeps_pending
                .fetch_sub(1, Ordering::Relaxed);
            self.counters
                .initial_sweeps_cancelled
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.cancel_initial();
        self.counters.active_scopes.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) struct OpenAttempt<'a> {
    counters: &'a Counters,
    retained: bool,
    completed: bool,
}

impl OpenAttempt<'static> {
    pub(super) fn new(retained: bool) -> Self {
        Self::with_counters(&COUNTERS, retained)
    }
}

impl<'a> OpenAttempt<'a> {
    fn with_counters(counters: &'a Counters, retained: bool) -> Self {
        Self {
            counters,
            retained,
            completed: false,
        }
    }

    pub(super) fn completed(mut self, handshake: Option<rustls::HandshakeKind>) {
        self.completed = true;
        let opens = if self.retained {
            &self.counters.retained_opens
        } else {
            &self.counters.disposable_opens
        };
        opens.fetch_add(1, Ordering::Relaxed);
        match handshake {
            Some(
                rustls::HandshakeKind::Full | rustls::HandshakeKind::FullWithHelloRetryRequest,
            ) => {
                self.counters.tls_full.fetch_add(1, Ordering::Relaxed);
            }
            Some(rustls::HandshakeKind::Resumed) => {
                self.counters.tls_resumed.fetch_add(1, Ordering::Relaxed);
            }
            None => {}
        }
    }
}

impl Drop for OpenAttempt<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.counters.failed_opens.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;
