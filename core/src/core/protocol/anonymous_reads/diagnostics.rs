//! Counts for private cold reads only. Public discovery uses the SDK relay pool.
//! No keys, URLs, filters or authentication state are exported.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::core) struct Snapshot {
    pub disposable_opens: u64,
    pub failed_opens: u64,
    pub tls_full: u64,
    pub tls_resumed: u64,
}

#[derive(Default)]
struct Counters {
    disposable_opens: AtomicU64,
    failed_opens: AtomicU64,
    tls_full: AtomicU64,
    tls_resumed: AtomicU64,
}
static COUNTERS: Counters = Counters {
    disposable_opens: AtomicU64::new(0),
    failed_opens: AtomicU64::new(0),
    tls_full: AtomicU64::new(0),
    tls_resumed: AtomicU64::new(0),
};
impl Counters {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
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

pub(super) struct OpenAttempt<'a> {
    counters: &'a Counters,
    completed: bool,
}
impl OpenAttempt<'static> {
    pub(super) fn new() -> Self {
        Self {
            counters: &COUNTERS,
            completed: false,
        }
    }
}
impl OpenAttempt<'_> {
    pub(super) fn completed(mut self, handshake: Option<rustls::HandshakeKind>) {
        self.completed = true;
        self.counters
            .disposable_opens
            .fetch_add(1, Ordering::Relaxed);
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
