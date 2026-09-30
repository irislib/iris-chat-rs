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

#[test]
fn timed_mute_sync_preserves_deadline_unmute_and_rejects_stale_or_foreign_devices() {
    let mut pair = chat_read_sync_pair("timed-mute-sync");
    let chat_id = Keys::generate().public_key().to_hex();
    let until = unix_now().get() + 3600;
    pair.a.handle_action(AppAction::SetChatMuteUntil {
        chat_id: chat_id.clone(),
        until_secs: until,
    });
    let stale = pair.a.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.b, &Keys::generate(), &stale);
    assert!(!pair.b.is_chat_muted(&chat_id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    assert!(
        pair.b.is_chat_muted(&chat_id),
        "linked device must receive the mute"
    );
    assert_eq!(pair.b.preferences.timed_chat_mutes[0].until_secs, until);
    pair.b.handle_action(AppAction::SetChatMuted {
        chat_id: chat_id.clone(),
        muted: false,
    });
    sync_chat_reads(&pair.b, &mut pair.a, &pair.b_device, false);
    assert!(!pair.a.is_chat_muted(&chat_id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    assert!(
        !pair.b.is_chat_muted(&chat_id),
        "stale mute cannot undo unmute"
    );
    pair.a.persist_best_effort();
    drop(pair.a);
    let mut restarted = logged_in_test_core_at_data_dir(
        &pair.owner,
        &pair.a_device,
        pair.a_dir.path().to_string_lossy().into(),
    );
    restarted.load_persisted().unwrap();
    configure_test_device_sync_profile(
        &mut restarted,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
        None,
    );
    deliver_chat_read_packets(&mut restarted, &pair.b_device, &stale);
    assert!(
        !restarted.is_chat_muted(&chat_id),
        "unmute survives restart"
    );
}

#[test]
fn timed_mute_sync_covers_group_forever_mutes_and_expired_deadlines() {
    let mut pair = chat_read_sync_pair("group-mute-sync");
    let group = "group:muted-friends".to_string();
    pair.a.handle_action(AppAction::SetChatMuted {
        chat_id: group.clone(),
        muted: true,
    });
    sync_chat_reads(&pair.a, &mut pair.b, &pair.a_device, false);
    assert!(pair.b.is_chat_muted(&group));
    pair.a.handle_action(AppAction::SetChatMuteUntil {
        chat_id: group.clone(),
        until_secs: unix_now().get() + 60,
    });
    let mut packet: serde_json::Value =
        serde_json::from_slice(&pair.a.build_device_sync_packets_for_test(100, false)[0]).unwrap();
    packet["chatMutes"][0]["untilSecs"] = serde_json::json!(unix_now().get() - 1);
    pair.b.handle_device_sync_packet(
        &pair.a_device.public_key().to_hex(),
        DEVICE_SYNC_PORT,
        &serde_json::to_vec(&packet).unwrap(),
    );
    assert!(
        !pair.b.is_chat_muted(&group),
        "an expired timed mute replaces an older forever mute"
    );
}

#[test]
fn timed_mute_control_is_encrypted_to_siblings_and_replays_without_chat_messages() {
    let mut pair = chat_read_sync_pair("mute-ratchet");
    install_two_way_local_sibling_state_for_test(
        &mut pair.a,
        &mut pair.b,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
    );
    pair.a.pending_relay_publishes.clear();
    pair.b.pending_relay_publishes.clear();
    let chat_id = Keys::generate().public_key().to_hex();
    let until = unix_now().get() + 3600;
    pair.a.handle_action(AppAction::SetChatMuteUntil {
        chat_id: chat_id.clone(),
        until_secs: until,
    });
    let encrypted = sorted_pending_events_for_test(&pair.a);
    assert!(!encrypted.is_empty());
    assert!(encrypted
        .iter()
        .all(|event| !event.content.contains(&chat_id) && !event.content.contains("chat-mute")));
    // Delivery can happen after the receiver has been offline.
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    assert!(pair.b.is_chat_muted(&chat_id));
    assert_eq!(pair.b.preferences.timed_chat_mutes[0].until_secs, until);
    assert!(
        pair.b.threads.is_empty(),
        "control must not create a chat or bubble"
    );
    pair.b.handle_action(AppAction::SetChatMuted {
        chat_id: chat_id.clone(),
        muted: false,
    });
    deliver_pending_relay_events_for_test(&pair.b, &mut pair.a);
    assert!(!pair.a.is_chat_muted(&chat_id));
    for event in encrypted {
        pair.b.handle_relay_event(event);
    }
    assert!(!pair.b.is_chat_muted(&chat_id));
    assert!(pair.a.threads.is_empty());
}

#[test]
fn timed_mute_control_rejects_peer_spoof_and_removed_sibling() {
    let mut pair = chat_read_sync_pair("mute-control-auth");
    let stranger = Keys::generate();
    let chat_id = stranger.public_key().to_hex();
    let value = serde_json::json!({ "type": "chat-mute", "v": 1, "mute": {
        "chatId": chat_id, "untilSecs": 0, "updatedAtMs": unix_now_ms(),
    }})
    .to_string();
    for (sender, device) in [
        (stranger.public_key(), stranger.public_key()),
        (pair.owner.public_key(), stranger.public_key()),
    ] {
        let (rumor, _) = runtime_rumor_json(
            sender,
            chat_mute_sync::CHAT_MUTE_KIND,
            &value,
            unix_now().get(),
            vec![],
        );
        assert!(pair.b.apply_decrypted_runtime_message_with_metadata(
            sender,
            Some(device),
            Some(pair.owner.public_key()),
            rumor,
            None,
            unix_now().get()
        ));
        assert!(!pair.b.is_chat_muted(&chat_id));
        assert!(pair.b.threads.is_empty());
    }
}

#[test]
fn timed_mute_newly_linked_device_inherits_legacy_and_versioned_mutes() {
    let mut pair = chat_read_sync_pair("mute-new-device");
    let legacy = Keys::generate().public_key().to_hex();
    let timed = Keys::generate().public_key().to_hex();
    pair.a.preferences.muted_chat_ids.push(legacy.clone());
    pair.a.set_chat_mute_until(&timed, unix_now().get() + 3600);
    // A link time newer than both settings must not filter out preferences.
    let packets = pair
        .a
        .build_device_sync_packets_for_test(unix_now().get() + 10, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets);
    assert!(pair.b.is_chat_muted(&legacy));
    assert!(pair.b.is_chat_muted(&timed));
    assert_eq!(
        pair.b.preferences.timed_chat_mutes,
        pair.a.preferences.timed_chat_mutes
    );
    pair.b.set_chat_muted(&legacy, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets);
    assert!(!pair.b.is_chat_muted(&legacy));
}

#[test]
fn chat_pin_sync_survives_unpin_restart_and_new_device_catchup() {
    let mut pair = chat_read_sync_pair("chat-pin-sync");
    let chat_id = Keys::generate().public_key().to_hex();
    pair.a.handle_action(AppAction::SetChatPinned { chat_id: chat_id.clone(), pinned: true });
    let stale = pair.a.build_device_sync_packets_for_test(100, false);
    deliver_chat_read_packets(&mut pair.b, &Keys::generate(), &stale);
    assert!(!pair.b.is_chat_pinned(&chat_id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    assert!(pair.b.is_chat_pinned(&chat_id), "a sibling must receive the pin even without chat history");
    pair.b.handle_action(AppAction::SetChatPinned { chat_id: chat_id.clone(), pinned: false });
    sync_chat_reads(&pair.b, &mut pair.a, &pair.b_device, false);
    assert!(!pair.a.is_chat_pinned(&chat_id));
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &stale);
    assert!(!pair.b.is_chat_pinned(&chat_id), "stale pin must not undo unpin");
    pair.a.persist_best_effort();
    drop(pair.a);
    let mut restarted = logged_in_test_core_at_data_dir(&pair.owner, &pair.a_device, pair.a_dir.path().to_string_lossy().into());
    restarted.load_persisted().unwrap();
    assert!(!restarted.is_chat_pinned(&chat_id));
}

#[test]
fn chat_pin_control_is_encrypted_to_siblings_and_replays_without_chat_messages() {
    let mut pair = chat_read_sync_pair("pin-ratchet");
    install_two_way_local_sibling_state_for_test(
        &mut pair.a,
        &mut pair.b,
        &pair.owner,
        &pair.a_device,
        &pair.b_device,
    );
    pair.a.pending_relay_publishes.clear();
    pair.b.pending_relay_publishes.clear();
    let chat_id = Keys::generate().public_key().to_hex();
    pair.a.handle_action(AppAction::SetChatPinned {
        chat_id: chat_id.clone(),
        pinned: true,
    });
    let encrypted = sorted_pending_events_for_test(&pair.a);
    assert!(!encrypted.is_empty());
    assert!(encrypted
        .iter()
        .all(|event| !event.content.contains(&chat_id) && !event.content.contains("chat-pin")));
    // Delivery can happen after the receiver has been offline.
    deliver_pending_relay_events_for_test(&pair.a, &mut pair.b);
    assert!(pair.b.is_chat_pinned(&chat_id));
    assert!(
        pair.b.threads.is_empty(),
        "control must not create a chat or bubble"
    );
    pair.b.handle_action(AppAction::SetChatPinned {
        chat_id: chat_id.clone(),
        pinned: false,
    });
    deliver_pending_relay_events_for_test(&pair.b, &mut pair.a);
    assert!(!pair.a.is_chat_pinned(&chat_id));
    for event in encrypted {
        pair.b.handle_relay_event(event);
    }
    assert!(!pair.b.is_chat_pinned(&chat_id));
    assert!(pair.a.threads.is_empty());
}

#[test]
fn chat_pin_control_rejects_peer_spoof_and_removed_sibling() {
    let mut pair = chat_read_sync_pair("pin-control-auth");
    let stranger = Keys::generate();
    let chat_id = stranger.public_key().to_hex();
    let value = serde_json::json!({ "type": "chat-pin", "v": 1, "pin": {
        "chatId": chat_id, "pinned": true, "updatedAtMs": unix_now_ms(),
    }})
    .to_string();
    for (sender, device) in [
        (stranger.public_key(), stranger.public_key()),
        (pair.owner.public_key(), stranger.public_key()),
    ] {
        let (rumor, _) = runtime_rumor_json(
            sender,
            chat_pin_sync::CHAT_PIN_KIND,
            &value,
            unix_now().get(),
            vec![],
        );
        assert!(pair.b.apply_decrypted_runtime_message_with_metadata(
            sender,
            Some(device),
            Some(pair.owner.public_key()),
            rumor,
            None,
            unix_now().get()
        ));
        assert!(!pair.b.is_chat_pinned(&chat_id));
        assert!(pair.b.threads.is_empty());
    }
}


#[test]
fn chat_pin_new_device_inherits_legacy_pins_before_link_time() {
    let mut pair = chat_read_sync_pair("pin-new-device");
    let legacy = Keys::generate().public_key().to_hex();
    pair.a.preferences.pinned_chat_ids.push(legacy.clone());
    pair.a.set_chat_pinned("group:weekend", true);
    let packets = pair.a.build_device_sync_packets_for_test(unix_now().get() + 10, false);
    deliver_chat_read_packets(&mut pair.b, &pair.a_device, &packets);
    assert!(pair.b.is_chat_pinned(&legacy));
    assert!(pair.b.is_chat_pinned("group:weekend"));
}
