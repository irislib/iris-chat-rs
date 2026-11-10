//! Small groups of [`AppCore`] bookkeeping fields that always travel together.

use super::*;

/// Generation counters for scheduled timers. A timer event carries the token
/// it was armed with and is ignored once the matching counter has moved on.
#[derive(Default)]
pub(super) struct TimerTokens {
    pub(super) device_invite_poll: u64,
    pub(super) pending_device_link_poll: u64,
    pub(super) message_expiry: u64,
    pub(super) chat_mute_expiry: u64,
    pub(super) protocol_reconnect: u64,
    pub(super) protocol_liveness: u64,
}

/// Connection status of the relays currently being watched.
#[derive(Default)]
pub(super) struct RelayStatusState {
    pub(super) watch_urls: HashSet<String>,
    pub(super) watch_generation: u64,
    pub(super) by_url: BTreeMap<String, RelayStatus>,
}

/// Bookkeeping for the debug-snapshot file.
#[derive(Default)]
pub(super) struct DebugSnapshotState {
    pub(super) write_generation: u64,
    pub(super) write_inflight: bool,
    pub(super) write_dirty: bool,
    /// Wall-clock millis of the last debug-snapshot file write. The
    /// snapshot is purely a test harness fixture (only `core/tests`,
    /// iOS InteropHarnessTests, and the Android RealRelayHarnessTest
    /// read it) — never read in production. We throttle to one
    /// rebuild per `DEBUG_SNAPSHOT_MIN_INTERVAL_MS` so a busy chat
    /// can't rebuild a full SessionManager clone × N known users on
    /// every relay event (the macOS CPU loop and the
    /// sluggish-over-time regression both traced back here).
    pub(super) last_built_at_ms: u64,
    /// Cumulative call count of `build_runtime_debug_snapshot`. Read
    /// by `core_perf_counters()` so the release gate can budget core
    /// hot-loop work, not just FFI surface traffic.
    pub(super) build_count: u64,
}

/// Reentrancy guard: while `depth > 0`, `rebuild_state` / `emit_state` /
/// `persist_best_effort` only set the matching dirty flag. The outermost
/// `exit_batch()` call performs a single rebuild + persist + emit so a
/// catch-up burst of N events triggers one UI re-render instead of N.
#[derive(Default)]
pub(super) struct BatchState {
    pub(super) depth: u32,
    pub(super) dirty_state: bool,
    pub(super) dirty_persist: bool,
}
