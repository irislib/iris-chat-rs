use super::*;
use futures_util::FutureExt;

#[test]
fn discovery_diagnostics_scope_reconfiguration_preserves_accounting() {
    let counters = Counters::default();
    let mut scope = Scope::with_counters(&counters);
    assert_eq!(counters.snapshot().active_scopes, 1);
    assert_eq!(counters.snapshot().initial_sweeps_pending, 0);
    scope.configured();
    assert_eq!(counters.snapshot().initial_sweeps_pending, 1);
    scope.configured();
    let changed = counters.snapshot();
    assert_eq!(changed.initial_sweeps_cancelled, 1);
    assert_eq!(changed.initial_sweeps_pending, 1);
    assert_eq!(changed.config_resets, 1);
    scope.sweep_completed();
    scope.sweep_completed();
    let completed = counters.snapshot();
    assert_eq!(completed.initial_sweeps_pending, 0);
    assert_eq!(completed.initial_sweeps_completed, 1);
    assert_eq!(completed.repeat_sweeps, 1);
    scope.configured();
    drop(scope);
    let stopped = counters.snapshot();
    assert_eq!(stopped.active_scopes, 0);
    assert_eq!(stopped.initial_sweeps_pending, 0);
    assert_eq!(stopped.initial_sweeps_completed, 1);
    assert_eq!(stopped.initial_sweeps_cancelled, 2);
    assert_eq!(stopped.config_resets, 2);
}

#[test]
fn discovery_diagnostics_cancelled_futures_release_scope_and_open_accounting() {
    let counters = Counters::default();
    let scope = Scope::with_counters(&counters);
    let unpolled = async move {
        let _scope = scope;
        std::future::pending::<()>().await;
    };
    drop(unpolled);
    assert_eq!(counters.snapshot(), Snapshot::default());

    let scope = Scope::with_counters(&counters);
    let mut pending = Box::pin(async {
        let mut scope = scope;
        scope.configured();
        let _opening = OpenAttempt::with_counters(&counters, true);
        std::future::pending::<()>().await;
    });
    assert!(pending.as_mut().now_or_never().is_none());
    assert_eq!(counters.snapshot().initial_sweeps_pending, 1);
    assert_eq!(counters.snapshot().failed_opens, 0);
    drop(pending);
    let cancelled = counters.snapshot();
    assert_eq!(cancelled.active_scopes, 0);
    assert_eq!(cancelled.initial_sweeps_pending, 0);
    assert_eq!(cancelled.initial_sweeps_cancelled, 1);
    assert_eq!(cancelled.initial_sweeps_completed, 0);
    assert_eq!(cancelled.failed_opens, 1);
}

#[test]
fn discovery_diagnostics_open_results_count_once_and_serialize_only_counts() {
    let counters = Counters::default();
    OpenAttempt::with_counters(&counters, true).completed(Some(rustls::HandshakeKind::Full));
    OpenAttempt::with_counters(&counters, false)
        .completed(Some(rustls::HandshakeKind::FullWithHelloRetryRequest));
    OpenAttempt::with_counters(&counters, false).completed(Some(rustls::HandshakeKind::Resumed));
    OpenAttempt::with_counters(&counters, true).completed(None);
    fn failed(counters: &Counters) -> Result<(), ()> {
        let _opening = OpenAttempt::with_counters(counters, false);
        Err(())
    }
    assert!(failed(&counters).is_err());
    let snapshot = counters.snapshot();
    assert_eq!(snapshot.retained_opens, 2);
    assert_eq!(snapshot.disposable_opens, 2);
    assert_eq!(snapshot.failed_opens, 1);
    assert_eq!(snapshot.tls_full, 2);
    assert_eq!(snapshot.tls_resumed, 1);
    let json = serde_json::to_value(snapshot).unwrap();
    let fields = json.as_object().unwrap();
    assert_eq!(fields.len(), 13);
    assert!(fields.values().all(|value| value.as_u64().is_some()));
}
