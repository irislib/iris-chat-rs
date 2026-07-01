//! Per-FFI-method call budgets — a release-gate canary for "the
//! shell started re-entering the core in a hot loop" regressions.
//!
//! Each scenario drives `FfiApp` through a realistic UX flow that a
//! sane shell should run, then reads `perf_counters()` and asserts
//! the call counts stay within their expected envelope. When a
//! refactor accidentally re-introduces something like the iOS
//! chat-list search re-running on every SwiftUI body re-eval, the
//! `search` counter blows past its budget and this test goes red
//! before the build even gets to a device.
//!
//! Budgets are tuned loose (≥ observed value at write time) so we
//! catch order-of-magnitude regressions, not noise. Tighten when an
//! optimisation reliably lowers a count; loosen when a feature
//! genuinely needs more calls and the headroom check is still
//! catching real regressions.
//!
//! These tests intentionally only count calls that cross the FFI
//! boundary — that's the line where shells could spam us without
//! the test framework noticing. Internal core counters (rebuild
//! state, persist, …) are out of scope here; they'd need their own
//! per-operation atomics and a different scenario shape.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iris_chat_core::{AppAction, AppReconciler, AppState, AppUpdate, FfiApp};
use tempfile::TempDir;

/// Cold start: create an account. The shell only calls `state()`
/// once at boot, then leans on the reconciler callback for
/// updates. Dispatch count covers the one `CreateAccount` action.
#[test]
fn cold_start_account_creation_stays_under_budget() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    let baseline = app.perf_counters();

    let _ = app.state();
    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());
    let after = app.perf_counters();

    let delta = countDelta(&baseline, &after);
    assert!(
        delta.state <= 2,
        "FFI state() polling regression: {delta:?}"
    );
    assert_eq!(delta.dispatch, 1, "{delta:?}");
    assert_eq!(delta.search, 0, "{delta:?}");
    assert_eq!(delta.peer_profile_debug, 0, "{delta:?}");
}

/// A shell that opens a chat then idles must not start polling
/// `state()` / re-firing `search()` / etc. Even after a stream of
/// internal state pushes (the chat-list search bug pattern), the
/// FFI counters should sit at the small fixed set from setup.
#[test]
fn idle_post_open_chat_emits_no_extra_ffi_calls() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());

    let bob_npub = ensure_account(&TempDir::new().unwrap(), "Bob");
    app.dispatch(AppAction::CreateChat {
        peer_input: bob_npub,
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.current_chat.is_some());

    let before = app.perf_counters();
    // Idle the way a real shell does: hand control back to the
    // reconciler thread and wait for state pushes to settle. 50ms
    // is plenty to catch any periodic FFI poller — anything that
    // re-enters the core ≥20Hz would tick at least once in this
    // window.
    std::thread::sleep(Duration::from_millis(50));
    let after = app.perf_counters();

    let delta = countDelta(&before, &after);
    assert_eq!(
        delta.state, 0,
        "FFI state() polled while shell idle: {delta:?}",
    );
    assert_eq!(
        delta.search, 0,
        "FFI search() fired while shell idle (chat-list search regression): {delta:?}",
    );
    assert_eq!(
        delta.dispatch, 0,
        "FFI dispatch() fired with no user input: {delta:?}",
    );
}

/// Typing a 5-character query into the search field must fire
/// search() exactly five times — once per keystroke. The
/// regression we're guarding against is the iOS body-re-eval bug
/// where receiving a relay event in the background re-ran the
/// search, multiplying the call count by the number of state
/// pushes.
#[test]
fn search_keystrokes_fire_one_search_call_each() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());

    let before = app.perf_counters();
    for prefix_len in 1..=5 {
        let query: String = "hello".chars().take(prefix_len).collect();
        let _ = app.search(query, None, 20);
    }
    let after = app.perf_counters();

    let delta = countDelta(&before, &after);
    assert_eq!(
        delta.search, 5,
        "expected one search per keystroke; got {delta:?}",
    );
}

/// The debug runtime snapshot is a test-only fixture (only this
/// repo's harness tests read it); production never does. It used
/// to rebuild on every relay event + protocol persist — a
/// SessionManager clone × N known users + a JSON disk write — which
/// pinned the macOS app at ~28% CPU after extended use and
/// surfaced as "UI gets sluggish after going back and forth
/// between chats". The fix throttles rebuilds to
/// `DEBUG_SNAPSHOT_MIN_INTERVAL_MS` (5 s) and only does the work
/// in debug builds / under `IRIS_RUNTIME_DEBUG_SNAPSHOT=1`. This
/// asserts the throttle: sending five messages back-to-back must
/// not produce five rebuilds.
///
/// Budget: 2 rebuilds covers the initial post-account build plus
/// a single throttle-window rebuild during the burst. Anything
/// approaching one-per-event means the throttle regressed.
#[test]
fn debug_snapshot_rebuilds_stay_under_budget_during_message_burst() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());

    app.dispatch(AppAction::CreateGroup {
        name: "Debug Burst".to_string(),
        member_inputs: Vec::new(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.current_chat.is_some());
    let chat_id = inbox.snapshot().current_chat.unwrap().chat_id;

    let before = app.core_perf_counters();
    for index in 0..5 {
        app.dispatch(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text: format!("burst {index}"),
        });
    }
    inbox.wait_until(Duration::from_secs(5), |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len() >= 5)
            .unwrap_or(false)
    });
    let after = app.core_perf_counters();

    let rebuilds = after.debug_snapshot_builds - before.debug_snapshot_builds;
    assert!(
        rebuilds <= 2,
        "debug_snapshot rebuilt {rebuilds} times for 5 messages; throttle regressed (was per-event before the fix). before={before:?} after={after:?}",
    );
}

/// Receiving messages must not pull anything new across the FFI
/// boundary — relay events flow into the core via the internal
/// CoreMsg channel, not through FFI entry points. The shell sees
/// the result via the reconciler callback (not counted) and reads
/// it from the next `state()` only when it actively needs it.
#[test]
fn receiving_messages_does_not_call_ffi() {
    let dir = TempDir::new().unwrap();
    let app = FfiApp::new(
        dir.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: "Alice".to_string(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.account.is_some());

    app.dispatch(AppAction::CreateGroup {
        name: "Receive Budget".to_string(),
        member_inputs: Vec::new(),
    });
    inbox.wait_until(Duration::from_secs(5), |state| state.current_chat.is_some());
    let chat_id = inbox.snapshot().current_chat.unwrap().chat_id;

    let before = app.perf_counters();
    // Five outgoing sends are the closest thing the in-process
    // test fixture gives us to "five state pushes that update the
    // chat list" — each one fires emit_state and triggers a
    // reconcile, mirroring the real "messages stream in over the
    // relay" path without needing an actual second client.
    for index in 0..5 {
        app.dispatch(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text: format!("ping {index}"),
        });
    }
    inbox.wait_until(Duration::from_secs(5), |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len() >= 5)
            .unwrap_or(false)
    });
    let after = app.perf_counters();

    let delta = countDelta(&before, &after);
    assert_eq!(
        delta.search, 0,
        "search() must not fire on message receipt: {delta:?}",
    );
    assert_eq!(
        delta.state, 0,
        "state() polling on message receipt: {delta:?}",
    );
    // Dispatch covers the five SendMessage actions, nothing more.
    assert_eq!(delta.dispatch, 5, "{delta:?}");
}

#[allow(non_snake_case)]
fn countDelta(
    before: &iris_chat_core::FfiPerfCountersSnapshot,
    after: &iris_chat_core::FfiPerfCountersSnapshot,
) -> iris_chat_core::FfiPerfCountersSnapshot {
    iris_chat_core::FfiPerfCountersSnapshot {
        state: after.state - before.state,
        dispatch: after.dispatch - before.dispatch,
        search: after.search - before.search,
        ingest_nearby_event_json: after.ingest_nearby_event_json - before.ingest_nearby_event_json,
        export_support_bundle_json: after.export_support_bundle_json
            - before.export_support_bundle_json,
        peer_profile_debug: after.peer_profile_debug - before.peer_profile_debug,
        mutual_groups: after.mutual_groups - before.mutual_groups,
        prepare_for_suspend: after.prepare_for_suspend - before.prepare_for_suspend,
    }
}

#[derive(Clone)]
struct ReconcilerInbox {
    state: Arc<Mutex<AppState>>,
}

impl ReconcilerInbox {
    fn install(app: &FfiApp) -> Self {
        let inbox = Self {
            state: Arc::new(Mutex::new(AppState::empty())),
        };
        let collector = Box::new(StateCollector {
            slot: inbox.state.clone(),
        });
        app.listen_for_updates(collector);
        inbox
    }

    fn wait_until<F>(&self, timeout: Duration, mut predicate: F)
    where
        F: FnMut(&AppState) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(guard) = self.state.lock() {
                if predicate(&guard) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("predicate never observed within {timeout:?}");
    }

    fn snapshot(&self) -> AppState {
        self.state.lock().unwrap().clone()
    }
}

struct StateCollector {
    slot: Arc<Mutex<AppState>>,
}

impl AppReconciler for StateCollector {
    fn reconcile(&self, update: AppUpdate) {
        if let AppUpdate::FullState(state) = update {
            if let Ok(mut guard) = self.slot.lock() {
                if state.rev >= guard.rev {
                    *guard = state;
                }
            }
        }
    }
}

fn ensure_account(temp: &TempDir, name: &str) -> String {
    let app = FfiApp::new(
        temp.path().to_string_lossy().to_string(),
        String::new(),
        "test".to_string(),
    );
    let inbox = ReconcilerInbox::install(&app);
    app.dispatch(AppAction::CreateAccount {
        name: name.to_string(),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            panic!("account creation timeout for {name}");
        }
        if let Some(account) = inbox.state.lock().unwrap().account.clone() {
            return account.npub;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
