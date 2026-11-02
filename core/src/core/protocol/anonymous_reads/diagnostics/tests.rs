use super::*;

#[test]
fn private_read_diagnostics_count_completed_and_cancelled_opens_without_identifiers() {
    let counters = Counters::default();
    OpenAttempt {
        counters: &counters,
        completed: false,
    }
    .completed(Some(rustls::HandshakeKind::Full));
    OpenAttempt {
        counters: &counters,
        completed: false,
    }
    .completed(Some(rustls::HandshakeKind::Resumed));
    OpenAttempt {
        counters: &counters,
        completed: false,
    }
    .completed(None);
    drop(OpenAttempt {
        counters: &counters,
        completed: false,
    });
    let snapshot = counters.snapshot();
    assert_eq!(snapshot.disposable_opens, 3);
    assert_eq!(snapshot.failed_opens, 1);
    assert_eq!(snapshot.tls_full, 1);
    assert_eq!(snapshot.tls_resumed, 1);
    let json = serde_json::to_value(snapshot).unwrap();
    let fields = json.as_object().unwrap();
    assert_eq!(fields.len(), 4);
    assert!(fields.values().all(|value| value.as_u64().is_some()));
}
