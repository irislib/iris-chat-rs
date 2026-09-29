#[test]
fn timed_mute_actions_persist_and_unmute_only_the_selected_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let temp = tempfile::TempDir::new().unwrap();
    let mut core =
        logged_in_test_core_at_data_dir(&owner, &device, temp.path().to_string_lossy().into());
    let until = unix_now().get() + 3600;
    core.handle_action(AppAction::SetChatMuted {
        chat_id: other.clone(),
        muted: true,
    });
    core.handle_action(AppAction::SetChatMuteUntil {
        chat_id: peer.clone(),
        until_secs: until,
    });
    assert!(core.is_chat_muted(&peer));
    assert!(core.is_chat_muted(&other));
    let persisted = core.app_store.load_preferences_snapshot().unwrap().unwrap();
    let mut restored = PreferencesSnapshot::default();
    persistence::apply_persisted_preferences(&mut restored, &persisted);
    assert_eq!(
        restored.timed_chat_mutes,
        vec![ChatMuteDeadline {
            chat_id: peer.clone(),
            until_secs: until
        }]
    );
    assert_eq!(restored.muted_chat_ids, vec![other.clone()]);
    assert!(mobile_push::is_chat_muted_in(
        &core.app_store.shared().lock().unwrap(),
        &peer
    ));
    core.handle_action(AppAction::SetChatMuted {
        chat_id: peer.clone(),
        muted: false,
    });
    assert!(!core.is_chat_muted(&peer));
    assert!(core.is_chat_muted(&other));
    assert!(core.preferences.timed_chat_mutes.is_empty());
}

#[test]
fn timed_mute_expiry_is_inclusive_and_stale_timers_cannot_clear_a_new_mute() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("timed-mute-expiry", &owner, &device);
    core.handle_action(AppAction::SetChatMuteUntil {
        chat_id: peer.clone(),
        until_secs: unix_now().get() + 3600,
    });
    let old = core.chat_mute_expiry_token;
    core.handle_action(AppAction::SetChatMuteUntil {
        chat_id: peer.clone(),
        until_secs: unix_now().get() + 28800,
    });
    core.handle_chat_mute_expiry(old);
    assert!(core.is_chat_muted(&peer));
    let deadlines = vec![ChatMuteDeadline {
        chat_id: peer.clone(),
        until_secs: 100,
    }];
    assert!(chat_settings::chat_is_muted_at(&[], &deadlines, &peer, 99));
    assert!(!chat_settings::chat_is_muted_at(
        &[],
        &deadlines,
        &peer,
        100
    ));
    core.preferences.timed_chat_mutes[0].until_secs = unix_now().get();
    core.persist_best_effort();
    assert!(
        !mobile_push::is_chat_muted_in(&core.app_store.shared().lock().unwrap(), &peer),
        "background resolver checks deadline without foreground timer"
    );
    core.handle_chat_mute_expiry(core.chat_mute_expiry_token);
    assert!(core.preferences.timed_chat_mutes.is_empty());
}

#[test]
fn timed_mute_can_replace_indefinite_and_old_preferences_remain_indefinite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("timed-mute-legacy", &owner, &device);
    let legacy: PersistedPreferences =
        serde_json::from_value(serde_json::json!({ "muted_chat_ids": [peer.clone()] })).unwrap();
    persistence::apply_persisted_preferences(&mut core.preferences, &legacy);
    assert!(core.is_chat_muted(&peer));
    core.handle_action(AppAction::SetChatMuteUntil {
        chat_id: peer.clone(),
        until_secs: unix_now().get() + 3600,
    });
    assert!(core.preferences.muted_chat_ids.is_empty());
    core.handle_action(AppAction::SetChatMuted {
        chat_id: peer.clone(),
        muted: true,
    });
    assert!(core.preferences.timed_chat_mutes.is_empty());
    assert!(core.is_chat_muted(&peer));
}

#[test]
fn timed_mute_subscription_bounds_every_delayed_author_and_legacy_fallback_stays_silent() {
    let owner = Keys::generate();
    let author = Keys::generate().public_key().to_hex();
    let request = build_mobile_push_create_subscription_request(
        owner.secret_key().to_secret_hex(),
        "ios".into(),
        "test-token".into(),
        Some("test.app".into()),
        vec![],
        vec![],
        vec![],
        false,
        None,
        vec![MobilePushDelayedAuthor {
            author_pubkey: author.clone(),
            since_secs: 200,
        }],
    )
    .unwrap();
    let body: serde_json::Value =
        serde_json::from_str(request.body_json.as_deref().unwrap()).unwrap();
    assert_eq!(body["filter"]["since"], 200);
    assert!(body["filters"]
        .as_array()
        .unwrap()
        .iter()
        .all(|filter| filter["since"] == 200));
    let fallback = crate::mobile_push_request_without_timed_filters(request).unwrap();
    let body: serde_json::Value =
        serde_json::from_str(fallback.body_json.as_deref().unwrap()).unwrap();
    assert_eq!(body["filter"]["authors"], serde_json::json!([]));
    assert!(!fallback.body_json.unwrap().contains(&author));
}

#[test]
fn timed_mute_resumes_foreground_expiry_after_suspended_timer_was_dropped() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let mut core = logged_in_test_core("timed-mute-foreground", &owner, &device);
    core.handle_action(AppAction::SetChatMuteUntil {
        chat_id: peer.clone(),
        until_secs: unix_now().get() + 3600,
    });
    core.preferences.timed_chat_mutes[0].until_secs = unix_now().get();
    let old_token = core.chat_mute_expiry_token;
    core.suspended = true;
    core.handle_internal(InternalEvent::ExpireChatMutes { token: old_token });
    assert_eq!(core.preferences.timed_chat_mutes.len(), 1);
    core.handle_action(AppAction::AppForegrounded);
    assert!(core.preferences.timed_chat_mutes.is_empty());
    assert_ne!(core.chat_mute_expiry_token, old_token);
    assert!(!core.is_chat_muted(&peer));
}

#[test]
fn timed_mute_schema_migration_preserves_existing_indefinite_mutes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate().public_key().to_hex();
    let temp = tempfile::TempDir::new().unwrap();
    let data_dir = temp.path().to_string_lossy().to_string();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());
    core.handle_action(AppAction::SetChatMuted {
        chat_id: peer.clone(),
        muted: true,
    });
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch(
            "ALTER TABLE preferences DROP COLUMN timed_chat_mutes_json; PRAGMA user_version = 36;",
        )
        .unwrap();
    drop(core);
    let mut restored = logged_in_test_core_at_data_dir(&owner, &device, data_dir);
    let persisted = restored
        .app_store
        .load_preferences_snapshot()
        .unwrap()
        .unwrap();
    assert_eq!(persisted.muted_chat_ids, vec![peer]);
    assert!(persisted.timed_chat_mutes.is_empty());
}

#[test]
fn timed_mute_duplicate_restored_chat_deadlines_keep_the_longest_mute() {
    let mut deadlines = vec![
        ChatMuteDeadline {
            chat_id: "chat".into(),
            until_secs: 100,
        },
        ChatMuteDeadline {
            chat_id: "chat".into(),
            until_secs: 200,
        },
    ];
    chat_settings::normalize_timed_chat_mutes(&mut deadlines);
    assert_eq!(
        deadlines,
        vec![ChatMuteDeadline {
            chat_id: "chat".into(),
            until_secs: 200
        }]
    );
}
