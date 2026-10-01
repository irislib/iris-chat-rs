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
            AppUpdate::CallMedia { .. } => "call-media".to_string(),
            AppUpdate::NearbyPublishedEvent { .. } => "nearby".to_string(),
            AppUpdate::NearbyPeersChanged { .. } => "nearby-peers".to_string(),
            AppUpdate::SignerLoginSignEvent { .. } => "signer".to_string(),
        })
        .collect::<Vec<_>>();

    assert_eq!(order, vec!["persist", "state:3", "nearby"]);
}

#[test]
fn core_supervisor_recovers_after_batch_panic() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let app = new_ffi_app_inner(temp_dir.path().to_string_lossy().to_string());

    // A fresh test process first warms the social graph. Wait for the worker
    // before measuring recovery so startup under load cannot race the assertion.
    let (ready_tx, ready_rx) = flume::bounded(1);
    app.foreground_tx
        .send(CoreMsg::CorePerfCounters(ready_tx))
        .expect("send readiness request");
    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("core worker ready");

    app.foreground_tx
        .send(CoreMsg::PanicForTest)
        .expect("send test panic");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
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
