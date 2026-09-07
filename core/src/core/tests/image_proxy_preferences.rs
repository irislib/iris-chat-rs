#[test]
fn image_proxy_fallback_opt_in_persists_and_reset_disables_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().to_string_lossy().to_string();
    let make_core = || {
        AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            path.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        )
    };
    let mut core = make_core();
    assert!(!core.state.preferences.image_proxy_fallback_enabled);
    core.handle_action(AppAction::SetImageProxyFallbackEnabled { enabled: true });
    assert!(core.state.preferences.image_proxy_fallback_enabled);
    drop(core);

    let mut core = make_core();
    assert!(core.state.preferences.image_proxy_fallback_enabled);
    core.handle_action(AppAction::SetImageProxyFallbackEnabled { enabled: false });
    drop(core);
    let mut core = make_core();
    assert!(!core.state.preferences.image_proxy_fallback_enabled);
    core.handle_action(AppAction::SetImageProxyFallbackEnabled { enabled: true });
    core.handle_action(AppAction::ResetImageProxySettings);
    assert!(!core.state.preferences.image_proxy_fallback_enabled);
    drop(core);
    assert!(!make_core().state.preferences.image_proxy_fallback_enabled);
}

#[test]
fn legacy_serialized_preferences_do_not_enable_image_proxy_fallback() {
    let preferences: PersistedPreferences = serde_json::from_str("{}").unwrap();
    assert!(!preferences.image_proxy_fallback_enabled);
}
