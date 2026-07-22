mod actions;
mod core;
mod desktop_nearby;
mod desktop_update;
mod emoji;
mod fips_ble_ffi;
pub mod image_proxy;
pub mod local_relay;
pub mod perflog;
mod qr;
mod state;
mod test_fixtures;
#[doc(hidden)]
pub mod update_announcements;
pub mod update_policy;
mod updates;

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use std::{panic, panic::AssertUnwindSafe};

use flume::{Receiver, Sender};

pub use actions::AppAction;
#[cfg(feature = "stack-fixture")]
#[doc(hidden)]
pub use core::download_hashtree_attachment;
pub use desktop_nearby::*;
pub use desktop_update::*;
pub use emoji::*;
pub use fips_ble_ffi::*;
pub use qr::*;
pub use state::*;
pub use test_fixtures::*;
pub use update_policy::UpdateAutoCheckPolicy;
pub use updates::*;

use crate::core::AppCore;

uniffi::setup_scaffolding!();

pub(crate) const CORE_RESTART_TOAST: &str = "Iris needs restart. Copy support bundle in Settings.";
const SUPPORT_BUNDLE_REPLY_TIMEOUT: Duration = Duration::from_secs(8);

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum FipsBleCommand {
    Listen {
        request_id: u64,
        preferred_psm: u16,
    },
    StopListening,
    StartAdvertising {
        request_id: u64,
        bootstrap: Vec<u8>,
    },
    StopAdvertising {
        request_id: u64,
    },
    StartScanning {
        request_id: u64,
    },
    StopScanning,
    Connect {
        request_id: u64,
        peer_token: String,
        psm: u16,
    },
    Write {
        request_id: u64,
        connection_id: u64,
        bytes: Vec<u8>,
    },
    Close {
        connection_id: u64,
    },
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum FipsBleEvent {
    Listening {
        request_id: u64,
        psm: u16,
    },
    AdvertisingStarted {
        request_id: u64,
    },
    AdvertisingStopped {
        request_id: u64,
    },
    ScanningStarted {
        request_id: u64,
    },
    PeerDiscovered {
        peer_token: String,
        bootstrap: Vec<u8>,
    },
    Connected {
        request_id: u64,
        connection_id: u64,
        peer_token: String,
        send_segment_mtu: u16,
        receive_segment_mtu: u16,
    },
    IncomingConnection {
        connection_id: u64,
        peer_token: String,
        send_segment_mtu: u16,
        receive_segment_mtu: u16,
    },
    BytesReceived {
        connection_id: u64,
        bytes: Vec<u8>,
    },
    WriteCompleted {
        request_id: u64,
    },
    Disconnected {
        connection_id: u64,
        reason: Option<String>,
    },
    Failed {
        request_id: u64,
        message: String,
    },
}

/// Per-FFI-method call counters that feed the release-gate budget
/// tests. Every `FfiApp::*` entry point bumps the matching atomic at
/// the top of the call so a misbehaving shell that re-enters the
/// core in a hot loop shows up as an obvious counter spike.
///
/// We track FFI surface area (not internal core counters) because
/// the categorical heat bugs we hit have all been "the shell
/// re-evaluated something and called us N times instead of once" —
/// e.g. the iOS chat-list search re-firing on every body re-eval.
/// Adding finer-grained counters inside the core is a future move
/// if needed; the FFI line is what we can confidently budget today
/// because each entry corresponds to one observable shell action.
#[derive(Default, Debug)]
pub(crate) struct FfiPerfCounters {
    pub state: AtomicU64,
    pub dispatch: AtomicU64,
    pub search: AtomicU64,
    pub export_support_bundle_json: AtomicU64,
    pub peer_profile_debug: AtomicU64,
    pub mutual_groups: AtomicU64,
    pub prepare_for_suspend: AtomicU64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq, Default)]
pub struct FfiPerfCountersSnapshot {
    pub state: u64,
    pub dispatch: u64,
    pub search: u64,
    pub export_support_bundle_json: u64,
    pub peer_profile_debug: u64,
    pub mutual_groups: u64,
    pub prepare_for_suspend: u64,
}

/// Core-internal hot-loop counters. FFI surface counters can only
/// catch shells that re-enter the core in a loop — the
/// build_runtime_debug_snapshot regression that caused the macOS CPU
/// loop was entirely internal (relay events fanned out through
/// `persist_best_effort_inner` → `persist_debug_snapshot_best_effort`
/// → full SessionManager clone × N known users). The release-gate
/// budget tests assert on this snapshot too so the next time core
/// work explodes per event, CI catches it before a device does.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq, Default)]
pub struct CorePerfCountersSnapshot {
    pub debug_snapshot_builds: u64,
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    foreground_tx: Sender<CoreMsg>,
    foreground_rx: Receiver<CoreMsg>,
    background_tx: Sender<CoreMsg>,
    background_rx: Receiver<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
    /// Shared SQLite handle used by direct read FFI calls. The core
    /// supervisor swaps this when it recreates `AppCore` after a panic.
    shared_db: Arc<RwLock<Option<crate::core::SharedConnection>>>,
    perf: FfiPerfCounters,
    queue_metrics: Arc<CoreQueueMetrics>,
    recovery: Arc<CoreRecoveryState>,
}

#[derive(Default, Debug)]
struct CoreQueueMetrics {
    foreground_processed: AtomicU64,
    background_processed: AtomicU64,
    batch_active: AtomicBool,
    last_batch_started_at_ms: AtomicU64,
    last_batch_finished_at_ms: AtomicU64,
    last_batch_size: AtomicU64,
    last_batch_foreground_count: AtomicU64,
    last_batch_background_count: AtomicU64,
}

impl CoreQueueMetrics {
    fn mark_batch_start(&self, size: u64, foreground: u64, background: u64) {
        self.last_batch_started_at_ms
            .store(crate::perflog::now_ms(), Ordering::Relaxed);
        self.last_batch_size.store(size, Ordering::Relaxed);
        self.last_batch_foreground_count
            .store(foreground, Ordering::Relaxed);
        self.last_batch_background_count
            .store(background, Ordering::Relaxed);
        self.batch_active.store(true, Ordering::Release);
    }

    fn mark_batch_finished(&self, foreground: u64, background: u64) {
        self.foreground_processed
            .fetch_add(foreground, Ordering::Relaxed);
        self.background_processed
            .fetch_add(background, Ordering::Relaxed);
        self.last_batch_finished_at_ms
            .store(crate::perflog::now_ms(), Ordering::Relaxed);
        self.batch_active.store(false, Ordering::Release);
    }
}

#[derive(Default, Debug)]
struct CoreRecoveryState {
    restore_action: RwLock<Option<AppAction>>,
    restart_count: AtomicU64,
    last_panic: RwLock<Option<String>>,
}

impl CoreRecoveryState {
    fn remember_action(&self, action: &AppAction) {
        match action {
            AppAction::RestoreSession { .. }
            | AppAction::RestoreAccountBundle { .. }
            | AppAction::RestorePendingDeviceLink { .. } => {
                self.set_restore_action(Some(action.clone()));
            }
            AppAction::Logout => self.set_restore_action(None),
            _ => {}
        }
    }

    fn remember_update(&self, update: &AppUpdate) {
        match update {
            AppUpdate::PersistAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
                ..
            } => self.set_restore_action(Some(AppAction::RestoreAccountBundle {
                owner_nsec: owner_nsec.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                device_nsec: device_nsec.clone(),
            })),
            AppUpdate::PersistPendingDeviceLink {
                device_nsec,
                approval_bootstrap_json,
            } => self.set_restore_action(Some(AppAction::RestorePendingDeviceLink {
                device_nsec: device_nsec.clone(),
                approval_bootstrap_json: approval_bootstrap_json.clone(),
            })),
            AppUpdate::ClearPendingDeviceLink => self.set_restore_action(None),
            _ => {}
        }
    }

    fn restore_action(&self) -> Option<AppAction> {
        match self.restore_action.read() {
            Ok(action) => action.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    fn mark_panic(&self, detail: String) -> u64 {
        match self.last_panic.write() {
            Ok(mut slot) => *slot = Some(detail),
            Err(poison) => *poison.into_inner() = Some(detail),
        }
        self.restart_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn restart_count(&self) -> u64 {
        self.restart_count.load(Ordering::Relaxed)
    }

    fn last_panic(&self) -> Option<String> {
        match self.last_panic.read() {
            Ok(slot) => slot.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    fn set_restore_action(&self, action: Option<AppAction>) {
        match self.restore_action.write() {
            Ok(mut slot) => *slot = action,
            Err(poison) => *poison.into_inner() = action,
        }
    }
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String, _keychain_group: String, _app_version: String) -> Arc<Self> {
        match panic::catch_unwind(AssertUnwindSafe(|| new_ffi_app_inner(data_dir))) {
            Ok(app) => app,
            Err(payload) => ffi_app_failure(format!(
                "Iris could not start: {}",
                panic_payload_to_string(payload)
            )),
        }
    }

    pub fn state(&self) -> AppState {
        self.perf.state.fetch_add(1, Ordering::Relaxed);
        ffi_or("ffiapp.state", ffi_failure_state(), || {
            match self.shared_state.read() {
                Ok(slot) => slot.clone(),
                Err(poison) => poison.into_inner().clone(),
            }
        })
    }

    pub fn dispatch(&self, action: AppAction) {
        self.perf.dispatch.fetch_add(1, Ordering::Relaxed);
        ffi_or("ffiapp.dispatch", (), || {
            crate::perflog!("ffi.dispatch action={:?}", std::mem::discriminant(&action));
            self.recovery.remember_action(&action);
            let _ = self.foreground_tx.send(CoreMsg::Action(action));
        })
    }

    /// Snapshot of the per-FFI-method call counts since `FfiApp` was
    /// created. Used by `tests/perf_budgets.rs` to assert that hot
    /// shell paths stay within their expected FFI traffic. Reading
    /// is best-effort `Relaxed` — call counts are advisory not
    /// transactional.
    pub fn perf_counters(&self) -> FfiPerfCountersSnapshot {
        FfiPerfCountersSnapshot {
            state: self.perf.state.load(Ordering::Relaxed),
            dispatch: self.perf.dispatch.load(Ordering::Relaxed),
            search: self.perf.search.load(Ordering::Relaxed),
            export_support_bundle_json: self
                .perf
                .export_support_bundle_json
                .load(Ordering::Relaxed),
            peer_profile_debug: self.perf.peer_profile_debug.load(Ordering::Relaxed),
            mutual_groups: self.perf.mutual_groups.load(Ordering::Relaxed),
            prepare_for_suspend: self.perf.prepare_for_suspend.load(Ordering::Relaxed),
        }
    }

    /// Snapshot of core-internal hot-loop counters. Used by
    /// `tests/perf_budgets.rs` to budget work that happens entirely
    /// inside the core thread — the FFI surface counters above can't
    /// see those. Default snapshot on timeout so a wedged core can't
    /// pin the test on a perpetual wait.
    pub fn core_perf_counters(&self) -> CorePerfCountersSnapshot {
        ffi_or(
            "ffiapp.core_perf_counters",
            CorePerfCountersSnapshot::default(),
            || {
                let (reply_tx, reply_rx) = flume::bounded(1);
                if self
                    .foreground_tx
                    .send(CoreMsg::CorePerfCounters(reply_tx))
                    .is_err()
                {
                    return CorePerfCountersSnapshot::default();
                }
                match reply_rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(snapshot) => CorePerfCountersSnapshot {
                        debug_snapshot_builds: snapshot.debug_snapshot_builds,
                    },
                    Err(_) => CorePerfCountersSnapshot::default(),
                }
            },
        )
    }

    /// Grouped Signal-style search: filters the in-memory chat list
    /// into contacts/groups by display name + subtitle + chat id, and
    /// runs the SQLite FTS5 index for the messages section. Optional
    /// `scope_chat_id` restricts message hits to a single thread (the
    /// "search in this chat" pill in the desktop sidebar). Returns an
    /// empty snapshot for empty / whitespace queries.
    pub fn search(
        &self,
        query: String,
        scope_chat_id: Option<String>,
        limit: u32,
    ) -> SearchResultSnapshot {
        self.perf.search.fetch_add(1, Ordering::Relaxed);
        ffi_or(
            "ffiapp.search",
            SearchResultSnapshot::empty(query.clone(), scope_chat_id.clone()),
            || {
                let trimmed = query.trim();
                if trimmed.is_empty() {
                    return SearchResultSnapshot::empty(query.clone(), scope_chat_id.clone());
                }
                let limit = limit.max(1) as usize;
                let state_snapshot = match self.shared_state.read() {
                    Ok(slot) => slot.clone(),
                    Err(poison) => poison.into_inner().clone(),
                };
                let (contacts, groups) = if scope_chat_id.is_some() {
                    (Vec::new(), Vec::new())
                } else {
                    filter_threads_for_search(&state_snapshot.chat_list, trimmed)
                };
                let shared_db = self.shared_db_snapshot();
                let messages = match shared_db.as_ref() {
                    Some(shared) => match shared.lock() {
                        Ok(conn) => crate::core::search_messages_fts(
                            &conn,
                            trimmed,
                            scope_chat_id.as_deref(),
                            limit,
                        )
                        .unwrap_or_default(),
                        Err(poison) => crate::core::search_messages_fts(
                            &poison.into_inner(),
                            trimmed,
                            scope_chat_id.as_deref(),
                            limit,
                        )
                        .unwrap_or_default(),
                    },
                    None => Vec::new(),
                };
                let enriched = enrich_message_hits(messages, &state_snapshot.chat_list);
                // The shortcut row only makes sense for global search.
                // Once the user has scoped to a single chat, an npub
                // paste should still search that chat's messages, not
                // jump out of the scope.
                let shortcut = if scope_chat_id.is_none() {
                    chat_input_shortcut(trimmed)
                } else {
                    None
                };
                SearchResultSnapshot {
                    query,
                    scope_chat_id,
                    contacts,
                    groups,
                    messages: enriched,
                    shortcut,
                }
            },
        )
    }

    /// Bounded chat projection for a route-selected chat. Unlike
    /// `OpenChat`, this is a direct read from the shared state/SQLite
    /// handle and never waits behind the core action queue. Shells use
    /// it as the first paint for chat screens; the core still receives
    /// `OpenChat` for unread clearing, subscriptions, and side effects.
    pub fn chat_snapshot(&self, chat_id: String, limit: u32) -> Option<CurrentChatSnapshot> {
        ffi_or("ffiapp.chat_snapshot", None, || {
            let state_snapshot = match self.shared_state.read() {
                Ok(slot) => slot.clone(),
                Err(poison) => poison.into_inner().clone(),
            };
            crate::core::chat_snapshot_from_state_and_db(
                &state_snapshot,
                self.shared_db_snapshot().as_ref(),
                &chat_id,
                limit.max(1) as usize,
            )
        })
    }

    pub fn chat_snapshot_before(
        &self,
        chat_id: String,
        before_message_id: String,
        limit: u32,
    ) -> Option<CurrentChatSnapshot> {
        ffi_or("ffiapp.chat_snapshot_before", None, || {
            let state_snapshot = match self.shared_state.read() {
                Ok(slot) => slot.clone(),
                Err(poison) => poison.into_inner().clone(),
            };
            crate::core::chat_snapshot_before_from_state_and_db(
                &state_snapshot,
                self.shared_db_snapshot().as_ref(),
                &chat_id,
                &before_message_id,
                limit.max(1) as usize,
            )
        })
    }

    pub fn chat_snapshot_around_message(
        &self,
        chat_id: String,
        message_id: String,
        before_limit: u32,
        after_limit: u32,
    ) -> Option<CurrentChatSnapshot> {
        ffi_or("ffiapp.chat_snapshot_around_message", None, || {
            let state_snapshot = match self.shared_state.read() {
                Ok(slot) => slot.clone(),
                Err(poison) => poison.into_inner().clone(),
            };
            crate::core::chat_snapshot_around_message_from_state_and_db(
                &state_snapshot,
                self.shared_db_snapshot().as_ref(),
                &chat_id,
                &message_id,
                before_limit as usize,
                after_limit as usize,
            )
        })
    }

    pub fn export_support_bundle_json(&self) -> String {
        self.perf
            .export_support_bundle_json
            .fetch_add(1, Ordering::Relaxed);
        ffi_or(
            "ffiapp.export_support_bundle_json",
            self.support_bundle_json_with_ffi_diagnostics("{}".to_string(), true),
            || {
                let (reply_tx, reply_rx) = flume::bounded(1);
                if self
                    .foreground_tx
                    .send(CoreMsg::ExportSupportBundle(reply_tx))
                    .is_err()
                {
                    return self.support_bundle_json_with_ffi_diagnostics("{}".to_string(), true);
                }
                match reply_rx.recv_timeout(SUPPORT_BUNDLE_REPLY_TIMEOUT) {
                    Ok(json) => self.support_bundle_json_with_ffi_diagnostics(json, false),
                    Err(_) => self.support_bundle_json_with_ffi_diagnostics("{}".to_string(), true),
                }
            },
        )
    }

    pub fn peer_profile_debug(&self, owner_input: String) -> Option<PeerProfileDebugSnapshot> {
        self.perf.peer_profile_debug.fetch_add(1, Ordering::Relaxed);
        ffi_or("ffiapp.peer_profile_debug", None, || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .foreground_tx
                .send(CoreMsg::PeerProfileDebug {
                    owner_input,
                    reply_tx,
                })
                .is_err()
            {
                return None;
            }
            reply_rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
        })
    }

    pub fn mutual_groups(&self, owner_input: String) -> MutualGroupsSnapshot {
        self.perf.mutual_groups.fetch_add(1, Ordering::Relaxed);
        ffi_or(
            "ffiapp.mutual_groups",
            MutualGroupsSnapshot::default(),
            || {
                let (reply_tx, reply_rx) = flume::bounded(1);
                if self
                    .foreground_tx
                    .send(CoreMsg::MutualGroups {
                        owner_input,
                        reply_tx,
                    })
                    .is_err()
                {
                    return MutualGroupsSnapshot::default();
                }
                reply_rx
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or_default()
            },
        )
    }

    pub fn prepare_for_suspend(&self) {
        self.perf
            .prepare_for_suspend
            .fetch_add(1, Ordering::Relaxed);
        ffi_or("ffiapp.prepare_for_suspend", (), || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .foreground_tx
                .send(CoreMsg::PrepareForSuspend(reply_tx))
                .is_err()
            {
                return;
            }
            let _ = reply_rx.recv_timeout(Duration::from_secs(2));
        })
    }

    pub fn shutdown(&self) {
        ffi_or("ffiapp.shutdown", (), || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .foreground_tx
                .send(CoreMsg::Shutdown(Some(reply_tx)))
                .is_err()
            {
                return;
            }
            let _ = reply_rx.recv_timeout(Duration::from_secs(2));
        })
    }

    fn support_bundle_json_with_ffi_diagnostics(
        &self,
        rust_json: String,
        core_support_bundle_timed_out: bool,
    ) -> String {
        let mut object = serde_json::from_str::<serde_json::Value>(&rust_json)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let now_ms = crate::perflog::now_ms();
        let last_started_at_ms = self
            .queue_metrics
            .last_batch_started_at_ms
            .load(Ordering::Relaxed);
        let last_finished_at_ms = self
            .queue_metrics
            .last_batch_finished_at_ms
            .load(Ordering::Relaxed);
        let batch_active = self.queue_metrics.batch_active.load(Ordering::Acquire);
        let active_batch_age_ms = if batch_active && last_started_at_ms > 0 {
            Some(now_ms.saturating_sub(last_started_at_ms))
        } else {
            None
        };
        let last_batch_started_ago_ms = if last_started_at_ms > 0 {
            Some(now_ms.saturating_sub(last_started_at_ms))
        } else {
            None
        };
        let last_batch_finished_ago_ms = if last_finished_at_ms > 0 {
            Some(now_ms.saturating_sub(last_finished_at_ms))
        } else {
            None
        };
        object.insert(
            "ffi_queue".to_string(),
            serde_json::json!({
                "core_support_bundle_timed_out": core_support_bundle_timed_out,
                "foreground_pending": self.foreground_rx.len(),
                "background_pending": self.background_rx.len(),
                "foreground_processed": self.queue_metrics.foreground_processed.load(Ordering::Relaxed),
                "background_processed": self.queue_metrics.background_processed.load(Ordering::Relaxed),
                "batch_active": batch_active,
                "active_batch_age_ms": active_batch_age_ms,
                "last_batch_started_ago_ms": last_batch_started_ago_ms,
                "last_batch_finished_ago_ms": last_batch_finished_ago_ms,
                "last_batch_size": self.queue_metrics.last_batch_size.load(Ordering::Relaxed),
                "last_batch_foreground_count": self.queue_metrics.last_batch_foreground_count.load(Ordering::Relaxed),
                "last_batch_background_count": self.queue_metrics.last_batch_background_count.load(Ordering::Relaxed),
                "core_restarts": self.recovery.restart_count(),
                "last_core_panic": self.recovery.last_panic(),
                "has_cached_restore_action": self.recovery.restore_action().is_some(),
            }),
        );
        serde_json::Value::Object(object).to_string()
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let update_rx = self.update_rx.clone();
        let recovery = self.recovery.clone();
        let spawn_result = thread::Builder::new()
            .name("iris-updates".to_string())
            .spawn(move || {
                // Drain queued updates and deliver the latest FullState only.
                // The shell side already discards FullStates with stale `rev`,
                // but the JNI marshal of an AppState is itself ~20-30 ms and
                // each push triggers a full Compose recomposition (~400 ms on
                // Android debug). When the core emits a tight burst of 3-4
                // updates (OpenChat → SyncComplete → FetchCatchUpEvents → …)
                // the UI keeps re-rendering for seconds even though only the
                // final state mattered.
                //
                // Secret persistence updates are side effects, not
                // a UI update, so we never collapse those — every one must run.
                while let Ok(first) = update_rx.recv() {
                    let mut latest_full_state: Option<AppUpdate> = None;
                    let mut before_full_state: Vec<AppUpdate> = Vec::new();
                    let mut after_full_state: Vec<AppUpdate> = Vec::new();
                    let process = |update: AppUpdate,
                                   latest: &mut Option<AppUpdate>,
                                   before: &mut Vec<AppUpdate>,
                                   after: &mut Vec<AppUpdate>| {
                        recovery.remember_update(&update);
                        enqueue_update_for_delivery(update, latest, before, after);
                    };
                    process(
                        first,
                        &mut latest_full_state,
                        &mut before_full_state,
                        &mut after_full_state,
                    );
                    while let Ok(next) = update_rx.try_recv() {
                        process(
                            next,
                            &mut latest_full_state,
                            &mut before_full_state,
                            &mut after_full_state,
                        );
                    }
                    for update in before_full_state
                        .into_iter()
                        .chain(latest_full_state)
                        .chain(after_full_state)
                    {
                        let kind = match &update {
                            AppUpdate::FullState(_) => "FullState",
                            AppUpdate::PersistAccountBundle { .. } => "PersistAccountBundle",
                            AppUpdate::PersistPendingDeviceLink { .. } => {
                                "PersistPendingDeviceLink"
                            }
                            AppUpdate::ClearPendingDeviceLink => "ClearPendingDeviceLink",
                            AppUpdate::NearbyPublishedEvent { .. } => "NearbyPublishedEvent",
                            AppUpdate::NearbyPeersChanged { .. } => "NearbyPeersChanged",
                        };
                        let t0 = crate::perflog::now_ms();
                        crate::perflog!("reconcile.start kind={kind}");
                        if panic::catch_unwind(AssertUnwindSafe(|| reconciler.reconcile(update)))
                            .is_err()
                        {
                            crate::perflog!("reconcile.failed kind={kind}");
                            continue;
                        }
                        crate::perflog!(
                            "reconcile.end kind={kind} elapsed_ms={}",
                            crate::perflog::now_ms().saturating_sub(t0)
                        );
                    }
                }
            });
        if let Err(error) = spawn_result {
            crate::perflog!("updates.spawn.failed error={error}");
            self.listening.store(false, Ordering::SeqCst);
        }
    }
}

impl FfiApp {
    fn shared_db_snapshot(&self) -> Option<crate::core::SharedConnection> {
        match self.shared_db.read() {
            Ok(slot) => slot.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }
}

fn new_ffi_app_inner(data_dir: String) -> Arc<FfiApp> {
    let (update_tx, update_rx) = flume::unbounded();
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let queue_metrics = Arc::new(CoreQueueMetrics::default());
    let recovery = Arc::new(CoreRecoveryState::default());
    let shared_db = Arc::new(RwLock::new(None));

    let update_tx_for_error = update_tx.clone();
    match AppCore::try_new_with_priority_sender(
        update_tx.clone(),
        background_tx.clone(),
        foreground_tx.clone(),
        data_dir.clone(),
        shared_state.clone(),
    ) {
        Ok(core) => {
            set_shared_db(&shared_db, Some(core.shared_db()));
            let spawn_result = spawn_core_supervisor(
                core,
                CoreSupervisor {
                    data_dir,
                    update_tx: update_tx.clone(),
                    core_sender: background_tx.clone(),
                    priority_sender: foreground_tx.clone(),
                    foreground_rx: foreground_rx.clone(),
                    background_rx: background_rx.clone(),
                    shared_state: shared_state.clone(),
                    shared_db: shared_db.clone(),
                    queue_metrics: queue_metrics.clone(),
                    recovery: recovery.clone(),
                },
            );
            if let Err(error) = spawn_result {
                publish_core_failure_state(
                    &shared_state,
                    &update_tx_for_error,
                    format!("Iris could not start: {error}"),
                );
            }
        }
        Err(error) => {
            publish_core_failure_state(&shared_state, &update_tx_for_error, error.to_string());
        }
    }

    Arc::new(FfiApp {
        foreground_tx,
        foreground_rx,
        background_tx,
        background_rx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db,
        perf: FfiPerfCounters::default(),
        queue_metrics,
        recovery,
    })
}

fn ffi_app_failure(message: String) -> Arc<FfiApp> {
    let (_update_tx, update_rx) = flume::unbounded();
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    let mut state = AppState::empty();
    state.toast = Some(message);
    state.rev = 1;
    let shared_state = Arc::new(RwLock::new(state));
    Arc::new(FfiApp {
        foreground_tx,
        foreground_rx,
        background_tx,
        background_rx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db: Arc::new(RwLock::new(None)),
        perf: FfiPerfCounters::default(),
        queue_metrics: Arc::new(CoreQueueMetrics::default()),
        recovery: Arc::new(CoreRecoveryState::default()),
    })
}

struct CoreSupervisor {
    data_dir: String,
    update_tx: Sender<AppUpdate>,
    core_sender: Sender<CoreMsg>,
    priority_sender: Sender<CoreMsg>,
    foreground_rx: Receiver<CoreMsg>,
    background_rx: Receiver<CoreMsg>,
    shared_state: Arc<RwLock<AppState>>,
    shared_db: Arc<RwLock<Option<crate::core::SharedConnection>>>,
    queue_metrics: Arc<CoreQueueMetrics>,
    recovery: Arc<CoreRecoveryState>,
}

fn spawn_core_supervisor(
    core: AppCore,
    supervisor: CoreSupervisor,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("iris-core".to_string())
        .spawn(move || {
            let mut core_slot = Some(core);
            // User actions and synchronous shell requests must not sit behind
            // relay/nearby backlog. The core keeps internal work on a
            // background queue and drains it in bounded chunks between
            // foreground batches.
            while let Ok(batch) =
                recv_core_batch(&supervisor.foreground_rx, &supervisor.background_rx)
            {
                let batch_size = batch.len();
                let foreground_count = batch
                    .iter()
                    .filter(|msg| is_foreground_core_msg(msg))
                    .count() as u64;
                let background_count = batch_size as u64 - foreground_count;
                supervisor.queue_metrics.mark_batch_start(
                    batch_size as u64,
                    foreground_count,
                    background_count,
                );
                let t0 = crate::perflog::now_ms();
                crate::perflog!("core.batch.start size={batch_size}");
                let result = match core_slot.as_mut() {
                    Some(core) => catch_core_batch(|| handle_core_batch_responsive(core, batch)),
                    None => break,
                };
                let elapsed_ms = crate::perflog::now_ms().saturating_sub(t0);
                supervisor
                    .queue_metrics
                    .mark_batch_finished(foreground_count, background_count);
                match result {
                    Ok(true) => {
                        crate::perflog!(
                            "core.batch.end size={batch_size} elapsed_ms={elapsed_ms}"
                        );
                    }
                    Ok(false) => {
                        crate::perflog!(
                            "core.batch.end size={batch_size} elapsed_ms={elapsed_ms} result=shutdown"
                        );
                        break;
                    }
                    Err(error) => {
                        if let Some(mut failed_core) = core_slot.take() {
                            failed_core.record_core_panic(error.clone());
                        }
                        crate::perflog!(
                            "core.batch.end size={batch_size} elapsed_ms={elapsed_ms} result=panic"
                        );
                        match recover_core_after_panic(&supervisor, error) {
                            Some(core) => core_slot = Some(core),
                            None => break,
                        }
                    }
                }
            }
        })
}

fn recover_core_after_panic(supervisor: &CoreSupervisor, detail: String) -> Option<AppCore> {
    let restart_count = supervisor.recovery.mark_panic(detail);
    crate::perflog!("core.supervisor.restart count={restart_count}");
    set_shared_db(&supervisor.shared_db, None);

    let mut core = match AppCore::try_new_with_priority_sender(
        supervisor.update_tx.clone(),
        supervisor.core_sender.clone(),
        supervisor.priority_sender.clone(),
        supervisor.data_dir.clone(),
        supervisor.shared_state.clone(),
    ) {
        Ok(core) => core,
        Err(error) => {
            crate::perflog!("core.supervisor.restart.failed count={restart_count} error={error}");
            publish_core_failure_state(
                &supervisor.shared_state,
                &supervisor.update_tx,
                CORE_RESTART_TOAST.to_string(),
            );
            return None;
        }
    };

    set_shared_db(&supervisor.shared_db, Some(core.shared_db()));
    if let Some(action) = supervisor.recovery.restore_action() {
        crate::perflog!(
            "core.supervisor.restore action={:?}",
            std::mem::discriminant(&action)
        );
        match catch_core_batch(|| core.handle_messages(vec![CoreMsg::Action(action)])) {
            Ok(true) => {}
            Ok(false) => {
                publish_core_failure_state(
                    &supervisor.shared_state,
                    &supervisor.update_tx,
                    CORE_RESTART_TOAST.to_string(),
                );
                return None;
            }
            Err(error) => {
                core.mark_core_panic(format!("core recovery restore panic: {error}"));
                return None;
            }
        }
    }

    crate::perflog!("core.supervisor.recovered count={restart_count}");
    Some(core)
}

fn set_shared_db(
    shared_db: &Arc<RwLock<Option<crate::core::SharedConnection>>>,
    value: Option<crate::core::SharedConnection>,
) {
    match shared_db.write() {
        Ok(mut slot) => *slot = value,
        Err(poison) => *poison.into_inner() = value,
    }
}

fn publish_core_failure_state(
    shared_state: &Arc<RwLock<AppState>>,
    update_tx: &Sender<AppUpdate>,
    message: String,
) {
    let mut state = match shared_state.read() {
        Ok(slot) => slot.clone(),
        Err(poison) => poison.into_inner().clone(),
    };
    state.toast = Some(message);
    state.rev = state.rev.saturating_add(1).max(1);
    match shared_state.write() {
        Ok(mut slot) => *slot = state.clone(),
        Err(poison) => *poison.into_inner() = state.clone(),
    }
    let _ = update_tx.send(AppUpdate::FullState(state));
}

const CORE_FOREGROUND_BATCH_LIMIT: usize = 64;
const CORE_BACKGROUND_BATCH_LIMIT: usize = 16;

fn recv_core_batch(
    foreground_rx: &Receiver<CoreMsg>,
    background_rx: &Receiver<CoreMsg>,
) -> Result<Vec<CoreMsg>, flume::RecvError> {
    if let Some(batch) = try_recv_core_batch(foreground_rx, background_rx) {
        return Ok(batch);
    }

    let (is_foreground, first) = flume::Selector::new()
        .recv(foreground_rx, |result| result.map(|msg| (true, msg)))
        .recv(background_rx, |result| result.map(|msg| (false, msg)))
        .wait()?;
    Ok(drain_core_batch_after_first(
        is_foreground,
        first,
        foreground_rx,
        background_rx,
    ))
}

fn try_recv_core_batch(
    foreground_rx: &Receiver<CoreMsg>,
    background_rx: &Receiver<CoreMsg>,
) -> Option<Vec<CoreMsg>> {
    if let Ok(first) = foreground_rx.try_recv() {
        return Some(drain_core_batch_after_first(
            true,
            first,
            foreground_rx,
            background_rx,
        ));
    }
    background_rx
        .try_recv()
        .ok()
        .map(|first| drain_core_batch_after_first(false, first, foreground_rx, background_rx))
}

fn drain_core_batch_after_first(
    is_foreground: bool,
    first: CoreMsg,
    foreground_rx: &Receiver<CoreMsg>,
    background_rx: &Receiver<CoreMsg>,
) -> Vec<CoreMsg> {
    let mut batch = Vec::with_capacity(if is_foreground {
        CORE_FOREGROUND_BATCH_LIMIT
    } else {
        CORE_BACKGROUND_BATCH_LIMIT
    });
    batch.push(first);
    drain_foreground_messages(&mut batch, foreground_rx);
    if batch.iter().any(is_foreground_core_msg) {
        return batch;
    }

    while batch.len() < CORE_BACKGROUND_BATCH_LIMIT {
        let Ok(next) = background_rx.try_recv() else {
            break;
        };
        batch.push(next);
        drain_foreground_messages(&mut batch, foreground_rx);
        if batch.iter().any(is_foreground_core_msg) {
            break;
        }
    }
    batch
}

fn drain_foreground_messages(batch: &mut Vec<CoreMsg>, foreground_rx: &Receiver<CoreMsg>) {
    while batch.len() < CORE_FOREGROUND_BATCH_LIMIT {
        let Ok(next) = foreground_rx.try_recv() else {
            break;
        };
        batch.push(next);
    }
}

fn handle_core_batch_responsive(core: &mut AppCore, messages: Vec<CoreMsg>) -> bool {
    if messages.len() <= 1 || !messages.iter().any(is_foreground_core_msg) {
        return core.handle_messages(messages);
    }

    let mut foreground = Vec::new();
    let mut background = Vec::new();
    for message in messages {
        if is_foreground_core_msg(&message) {
            foreground.push(message);
        } else {
            background.push(message);
        }
    }

    // Each foreground message runs in its own batch — the user gets immediate
    // feedback per action, but a single action that cascades into multiple
    // engine.persist() calls (e.g. send → retry_pending_protocol → another
    // persist) coalesces them into one SQLite write.
    for message in foreground {
        if !core.handle_messages(vec![message]) {
            return false;
        }
    }
    background.is_empty() || core.handle_messages(background)
}

fn catch_core_batch<F>(f: F) -> Result<bool, String>
where
    F: FnOnce() -> bool,
{
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(panic_payload_to_string)
}

fn ffi_or<T, F>(label: &'static str, fallback: T, f: F) -> T
where
    F: FnOnce() -> T,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            crate::perflog!(
                "ffi.panic label={label} detail={}",
                panic_payload_to_string(payload)
            );
            fallback
        }
    }
}

fn ffi_failure_state() -> AppState {
    let mut state = AppState::empty();
    state.toast = Some("Iris needs restart. Copy support bundle in Settings.".to_string());
    state
}

fn suppressed_mobile_push_resolution() -> MobilePushNotificationResolution {
    MobilePushNotificationResolution {
        should_show: false,
        title: String::new(),
        body: String::new(),
        payload_json: "{}".to_string(),
    }
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn is_foreground_core_msg(message: &CoreMsg) -> bool {
    !matches!(message, CoreMsg::Internal(_))
}

#[cfg(test)]
mod core_queue_tests;

fn filter_threads_for_search(
    chat_list: &[ChatThreadSnapshot],
    query: &str,
) -> (Vec<ChatThreadSnapshot>, Vec<ChatThreadSnapshot>) {
    let needle = query.to_lowercase();
    let mut contacts = Vec::new();
    let mut groups = Vec::new();
    for chat in chat_list {
        if !thread_matches_query(chat, &needle) {
            continue;
        }
        match chat.kind {
            ChatKind::Direct => contacts.push(chat.clone()),
            ChatKind::Group => groups.push(chat.clone()),
        }
    }
    (contacts, groups)
}

fn thread_matches_query(chat: &ChatThreadSnapshot, needle_lower: &str) -> bool {
    let candidates: [&str; 7] = [
        &chat.display_name,
        chat.nickname.as_deref().unwrap_or(""),
        chat.profile_name.as_deref().unwrap_or(""),
        chat.about.as_deref().unwrap_or(""),
        chat.subtitle.as_deref().unwrap_or(""),
        &chat.draft,
        &chat.chat_id,
    ];
    candidates
        .iter()
        .any(|field| field.to_lowercase().contains(needle_lower))
}

fn enrich_message_hits(
    hits: Vec<crate::core::PersistedMessageSearchHit>,
    chat_list: &[ChatThreadSnapshot],
) -> Vec<MessageSearchHit> {
    use std::collections::HashMap;
    let lookup: HashMap<&str, &ChatThreadSnapshot> = chat_list
        .iter()
        .map(|chat| (chat.chat_id.as_str(), chat))
        .collect();
    hits.into_iter()
        .map(|hit| {
            let parent = lookup.get(hit.chat_id.as_str());
            let display_name = parent
                .map(|chat| chat.display_name.clone())
                .unwrap_or_else(|| short_chat_label(&hit.chat_id));
            let picture_url = parent.and_then(|chat| chat.picture_url.clone());
            let kind = parent
                .map(|chat| chat.kind.clone())
                .unwrap_or(ChatKind::Direct);
            MessageSearchHit {
                chat_id: hit.chat_id,
                message_id: hit.message_id,
                chat_display_name: display_name,
                chat_picture_url: picture_url,
                chat_kind: kind,
                author_pubkey: hit.author,
                body: hit.body,
                is_outgoing: hit.is_outgoing,
                created_at_secs: hit.created_at_secs,
            }
        })
        .collect()
}

fn short_chat_label(chat_id: &str) -> String {
    let trimmed = chat_id.trim();
    if trimmed.len() > 12 {
        format!("{}…", &trimmed[..12])
    } else {
        trimmed.to_string()
    }
}

impl Drop for FfiApp {
    fn drop(&mut self) {
        let _ = self.foreground_tx.send(CoreMsg::Shutdown(None));
    }
}

#[uniffi::export]
pub fn normalize_peer_input(input: String) -> String {
    ffi_or("normalize_peer_input", String::new(), || {
        crate::core::normalize_peer_input_for_display(&input)
    })
}

#[uniffi::export]
pub fn is_valid_peer_input(input: String) -> bool {
    ffi_or("is_valid_peer_input", false, || {
        crate::core::parse_peer_input(&input).is_ok()
    })
}

/// Single source of truth for "is this typed text an npub or an
/// invite URL?". Used by the New Chat paste field, the chat-list
/// search bar, and the deep-link handler so all three branches agree
/// on what counts as a chat-opening shortcut. Returns `None` for
/// regular search-style text.
#[uniffi::export]
pub fn classify_chat_input(input: String) -> Option<ChatInputShortcut> {
    ffi_or("classify_chat_input", None, || chat_input_shortcut(&input))
}

fn chat_input_shortcut(raw: &str) -> Option<ChatInputShortcut> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("://") && lower.contains('#') {
        return Some(ChatInputShortcut::Invite {
            invite_input: trimmed.to_string(),
            display: short_invite_display(trimmed),
        });
    }
    if crate::core::parse_peer_input(trimmed).is_ok() {
        use nostr::nips::nip19::ToBech32;
        let normalized = crate::core::normalize_peer_input_for_display(trimmed);
        if let Ok(pubkey) = nostr::PublicKey::parse(&normalized) {
            let npub = pubkey.to_bech32().unwrap_or_else(|_| normalized.clone());
            let display = short_npub_display(&npub);
            return Some(ChatInputShortcut::DirectPeer {
                peer_input: normalized,
                display,
                npub,
                pubkey_hex: pubkey.to_hex(),
            });
        }
    }
    None
}

fn short_npub_display(npub: &str) -> String {
    if npub.len() > 16 {
        format!("{}…{}", &npub[..10], &npub[npub.len() - 4..])
    } else {
        npub.to_string()
    }
}

fn short_invite_display(invite: &str) -> String {
    // Strip the scheme + host so the row reads "iris.to/invite/…" not
    // a 120-char URL. We don't try to parse the bech32 payload; the
    // visible host is enough context for the user.
    let after_scheme = invite
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(invite);
    if after_scheme.len() > 32 {
        format!("{}…", &after_scheme[..32])
    } else {
        after_scheme.to_string()
    }
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its
/// canonical lowercase-hex form. The empty string is returned when the
/// input can't be parsed as a public key — callers expecting hex
/// downstream can short-circuit on that.
#[uniffi::export]
pub fn peer_input_to_hex(input: String) -> String {
    ffi_or("peer_input_to_hex", String::new(), || {
        let normalized = crate::core::normalize_peer_input_for_display(&input);
        match nostr::PublicKey::parse(&normalized) {
            Ok(pubkey) => pubkey.to_hex(),
            Err(_) => String::new(),
        }
    })
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its npub form.
/// Returns the original string when it can't be parsed as a public key.
#[uniffi::export]
pub fn peer_input_to_npub(input: String) -> String {
    ffi_or("peer_input_to_npub", String::new(), || {
        use nostr::nips::nip19::ToBech32;
        let normalized = crate::core::normalize_peer_input_for_display(&input);
        match nostr::PublicKey::parse(&normalized) {
            Ok(pubkey) => pubkey.to_bech32().unwrap_or(normalized),
            Err(_) => normalized,
        }
    })
}

#[uniffi::export]
pub fn build_summary() -> String {
    ffi_or("build_summary", String::new(), crate::core::build_summary)
}

#[uniffi::export]
pub fn relay_set_id() -> String {
    ffi_or("relay_set_id", String::new(), || {
        crate::core::relay_set_id().to_string()
    })
}

#[uniffi::export]
pub fn proxied_image_url(
    original_src: String,
    preferences: PreferencesSnapshot,
    width: Option<u32>,
    height: Option<u32>,
    square: bool,
) -> String {
    ffi_or("proxied_image_url", original_src.clone(), || {
        image_proxy::proxied_image_url(&original_src, &preferences, width, height, square)
    })
}

#[uniffi::export]
pub fn is_trusted_test_build() -> bool {
    ffi_or(
        "is_trusted_test_build",
        false,
        crate::core::trusted_test_build_flag,
    )
}

/// Marketing version baked in at build time from `IRIS_APP_VERSION_NAME`
/// (or `IRIS_APP_VERSION`), falling back to the crate semver. Use this
/// instead of `env!("CARGO_PKG_VERSION")` so UI/release artifacts agree
/// on a single version string.
#[uniffi::export]
pub fn app_version() -> String {
    crate::core::app_version_string().to_string()
}

#[uniffi::export]
pub fn resolve_mobile_push_notification_payload(
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    ffi_or(
        "resolve_mobile_push_notification_payload",
        suppressed_mobile_push_resolution(),
        || crate::core::resolve_mobile_push_notification(raw_payload_json),
    )
}

/// Decrypt a notification payload against the persisted double-ratchet
/// state under `data_dir`. Use from the FCM service (Android) or
/// Notification Service Extension (iOS) where there's no live `FfiApp`.
/// Falls back to the generic resolver when keys, payload, or storage
/// are unavailable so the user still gets *some* notification.
#[uniffi::export]
pub fn decrypt_mobile_push_notification_payload(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    ffi_or(
        "decrypt_mobile_push_notification_payload",
        suppressed_mobile_push_resolution(),
        || {
            crate::core::decrypt_mobile_push_notification(
                data_dir,
                owner_pubkey_hex,
                device_nsec,
                raw_payload_json,
            )
        },
    )
}

pub use crate::core::notifications::NotificationCandidate;

/// Compute the list of chats that should raise a notification given two
/// successive `AppState` chat-list snapshots. Single source of truth for
/// suppression — chat muted, chat open with app foregrounded, outgoing
/// last message, no unread increase, or global pref off. Use this from
/// Android, macOS, Linux, and Windows. iOS APNS uses a separate path
/// that cannot suppress until Apple grants the filtering entitlement.
#[uniffi::export]
pub fn decide_pending_notifications(
    previous_chats: Vec<ChatThreadSnapshot>,
    next_chats: Vec<ChatThreadSnapshot>,
    preferences: PreferencesSnapshot,
    app_foreground: bool,
    open_chat_id: Option<String>,
) -> Vec<NotificationCandidate> {
    crate::core::notifications::decide_notifications(
        &previous_chats,
        &next_chats,
        &preferences,
        app_foreground,
        open_chat_id.as_deref(),
    )
}

/// Pull the open chat id out of a `Router`, falling back to its default
/// screen. Shells normally just call this with `state.router` so they
/// don't each reimplement the `Screen::Chat { chat_id }` extraction.
#[uniffi::export]
pub fn router_open_chat_id(router: Router) -> Option<String> {
    crate::core::notifications::active_chat_id(&router)
}

#[uniffi::export]
pub fn resolve_mobile_push_subscription_server_url(
    platform_key: String,
    is_release: bool,
    override_url: Option<String>,
) -> String {
    ffi_or(
        "resolve_mobile_push_subscription_server_url",
        String::new(),
        || crate::core::resolve_mobile_push_server_url(platform_key, is_release, override_url),
    )
}

#[uniffi::export]
pub fn mobile_push_subscription_id_key(platform_key: String) -> String {
    ffi_or("mobile_push_subscription_id_key", String::new(), || {
        crate::core::mobile_push_stored_subscription_id_key(platform_key)
    })
}

#[uniffi::export]
pub fn build_mobile_push_list_subscriptions_request(
    owner_nsec: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or("build_mobile_push_list_subscriptions_request", None, || {
        crate::core::build_mobile_push_list_subscriptions_request(
            owner_nsec,
            platform_key,
            is_release,
            server_url_override,
        )
    })
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_mobile_push_create_subscription_request(
    owner_nsec: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_create_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_create_subscription_request(
                owner_nsec,
                platform_key,
                push_token,
                apns_topic,
                message_author_pubkeys,
                invite_response_pubkeys,
                is_release,
                server_url_override,
            )
        },
    )
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_mobile_push_update_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_update_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_update_subscription_request(
                owner_nsec,
                subscription_id,
                platform_key,
                push_token,
                apns_topic,
                message_author_pubkeys,
                invite_response_pubkeys,
                is_release,
                server_url_override,
            )
        },
    )
}

#[uniffi::export]
pub fn build_mobile_push_delete_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_delete_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_delete_subscription_request(
                owner_nsec,
                subscription_id,
                platform_key,
                is_release,
                server_url_override,
            )
        },
    )
}

#[cfg(test)]
mod ffi_hardening_tests {
    use super::*;

    #[test]
    fn ffi_guard_returns_fallback_after_panic() {
        let value = ffi_or("test.panic", 42, || -> i32 {
            panic!("ffi boom");
        });

        assert_eq!(value, 42);
    }

    #[test]
    fn core_batch_guard_converts_panic_to_error() {
        let result = catch_core_batch(|| -> bool {
            panic!("batch boom");
        });

        assert_eq!(result, Err("batch boom".to_string()));
    }

    #[test]
    fn core_batch_guard_preserves_success_result() {
        assert_eq!(catch_core_batch(|| true), Ok(true));
        assert_eq!(catch_core_batch(|| false), Ok(false));
    }

    #[test]
    fn recovery_state_tracks_restore_action_and_logout() {
        let recovery = CoreRecoveryState::default();
        recovery.remember_action(&AppAction::RestoreSession {
            owner_nsec: "secret".to_string(),
        });

        match recovery.restore_action() {
            Some(AppAction::RestoreSession { owner_nsec }) => assert_eq!(owner_nsec, "secret"),
            other => panic!("unexpected restore action: {other:?}"),
        }

        recovery.remember_action(&AppAction::Logout);
        assert!(recovery.restore_action().is_none());
    }

    #[test]
    fn recovery_state_tracks_persisted_account_bundle() {
        let recovery = CoreRecoveryState::default();
        recovery.remember_update(&AppUpdate::PersistAccountBundle {
            rev: 7,
            owner_nsec: None,
            owner_pubkey_hex: "owner".to_string(),
            device_nsec: "device-secret".to_string(),
        });

        match recovery.restore_action() {
            Some(AppAction::RestoreAccountBundle {
                owner_nsec,
                owner_pubkey_hex,
                device_nsec,
            }) => {
                assert_eq!(owner_nsec, None);
                assert_eq!(owner_pubkey_hex, "owner");
                assert_eq!(device_nsec, "device-secret");
            }
            other => panic!("unexpected restore action: {other:?}"),
        }
    }

    #[test]
    fn recovery_state_tracks_and_clears_pending_device_link() {
        let recovery = CoreRecoveryState::default();
        recovery.remember_update(&AppUpdate::PersistPendingDeviceLink {
            device_nsec: "device-secret".to_string(),
            approval_bootstrap_json: "{}".to_string(),
        });

        assert!(matches!(
            recovery.restore_action(),
            Some(AppAction::RestorePendingDeviceLink { .. })
        ));
        recovery.remember_update(&AppUpdate::ClearPendingDeviceLink);
        assert!(recovery.restore_action().is_none());
    }

    #[test]
    fn nearby_published_events_wait_behind_latest_state_in_drained_batch() {
        let mut latest_full_state = None;
        let mut before_full_state = Vec::new();
        let mut after_full_state = Vec::new();

        updates::enqueue_update_for_delivery(
            AppUpdate::NearbyPublishedEvent {
                event_id: "a".repeat(64),
                kind: 14,
                created_at_secs: 1,
                event_json: "{}".to_string(),
            },
            &mut latest_full_state,
            &mut before_full_state,
            &mut after_full_state,
        );
        let mut stale = AppState::empty();
        stale.rev = 1;
        updates::enqueue_update_for_delivery(
            AppUpdate::FullState(stale),
            &mut latest_full_state,
            &mut before_full_state,
            &mut after_full_state,
        );
        enqueue_update_for_delivery(
            AppUpdate::PersistAccountBundle {
                rev: 2,
                owner_nsec: None,
                owner_pubkey_hex: "owner".to_string(),
                device_nsec: "device".to_string(),
            },
            &mut latest_full_state,
            &mut before_full_state,
            &mut after_full_state,
        );
        let mut latest = AppState::empty();
        latest.rev = 3;
        enqueue_update_for_delivery(
            AppUpdate::FullState(latest),
            &mut latest_full_state,
            &mut before_full_state,
            &mut after_full_state,
        );

        let order = before_full_state
            .into_iter()
            .chain(latest_full_state)
            .chain(after_full_state)
            .map(|update| match update {
                AppUpdate::PersistAccountBundle { .. } => "persist".to_string(),
                AppUpdate::PersistPendingDeviceLink { .. } => "pending-link".to_string(),
                AppUpdate::ClearPendingDeviceLink => "clear-pending-link".to_string(),
                AppUpdate::FullState(state) => format!("state:{}", state.rev),
                AppUpdate::NearbyPublishedEvent { .. } => "nearby".to_string(),
                AppUpdate::NearbyPeersChanged { .. } => "nearby-peers".to_string(),
            })
            .collect::<Vec<_>>();

        assert_eq!(order, vec!["persist", "state:3", "nearby"]);
    }

    #[test]
    fn core_supervisor_recovers_after_batch_panic() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let app = new_ffi_app_inner(temp_dir.path().to_string_lossy().to_string());

        app.foreground_tx
            .send(CoreMsg::PanicForTest)
            .expect("send test panic");

        for _ in 0..40 {
            if app.recovery.restart_count() > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(app.recovery.restart_count(), 1);

        let (reply_tx, reply_rx) = flume::bounded(1);
        app.foreground_tx
            .send(CoreMsg::CorePerfCounters(reply_tx))
            .expect("send post-recovery request");
        assert!(reply_rx.recv_timeout(Duration::from_secs(2)).is_ok());

        app.shutdown();
    }
}
