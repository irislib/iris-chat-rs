use super::*;

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
