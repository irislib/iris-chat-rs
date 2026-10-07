use super::*;

#[test]
fn background_diagnostics_do_not_republish_history_and_foreground_refreshes_them() {
    let (update_tx, update_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let data_dir = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        data_dir.path().to_string_lossy().into_owned(),
        shared_state,
    );
    core.state = crate::build_large_test_app_state(16, 8, 200);
    core.state.network_status = Some(core.build_network_status_snapshot());
    core.emit_state();
    update_rx.try_recv().unwrap();
    core.handle_action(AppAction::AppBackgrounded);
    for index in 1..=20 {
        let network = core.state.network_status.as_mut().unwrap();
        network.recent_event_count = index;
        network.recent_log_count = index;
        network.last_debug_category = Some("network".into());
        network.last_debug_detail = Some(format!("status {index}"));
        core.emit_state();
    }
    assert!(
        update_rx.try_recv().is_err(),
        "debug-only changes must not serialize chat history while hidden"
    );
    core.handle_action(AppAction::AppForegrounded);
    let AppUpdate::FullState(latest) = update_rx.try_recv().unwrap() else {
        panic!("foreground snapshot");
    };
    assert_eq!(latest.network_status.unwrap().recent_event_count, 20);
    assert!(update_rx.try_recv().is_err());

    core.handle_action(AppAction::AppBackgrounded);
    core.state
        .network_status
        .as_mut()
        .unwrap()
        .connected_relay_count += 1;
    core.emit_state();
    assert!(
        matches!(update_rx.try_recv(), Ok(AppUpdate::FullState(_))),
        "connection changes remain immediate"
    );
    core.state.current_chat.as_mut().unwrap().messages[0].body = "Changed while hidden".into();
    core.emit_state();
    assert!(
        matches!(update_rx.try_recv(), Ok(AppUpdate::FullState(_))),
        "messages remain immediate"
    );
}

#[test]
fn state_publication_preserves_large_history_and_suppresses_duplicates() {
    let (update_tx, update_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    let data_dir = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        data_dir.path().to_string_lossy().into_owned(),
        shared_state.clone(),
    );
    core.state = crate::build_large_test_app_state(80, 20, 1_200);

    core.emit_state();
    let AppUpdate::FullState(first) = update_rx.try_recv().unwrap() else {
        panic!("expected full state");
    };
    assert_eq!(first, core.state);
    assert_eq!(*shared_state.read().unwrap(), first);

    core.emit_state();
    assert!(update_rx.try_recv().is_err());
    assert_eq!(core.state.rev, first.rev);

    core.state.current_chat.as_mut().unwrap().messages[0].body = "Edited".to_string();
    core.emit_state();
    let AppUpdate::FullState(edited) = update_rx.try_recv().unwrap() else {
        panic!("expected changed full state");
    };
    assert_eq!(edited.rev, first.rev + 1);
    assert_eq!(edited, core.state);
    assert_eq!(*shared_state.read().unwrap(), edited);
    assert_ne!(edited.current_chat, first.current_chat);
}
