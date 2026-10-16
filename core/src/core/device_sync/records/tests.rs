use super::*;
use base64::Engine;

#[test]
fn chat_read_records_are_exact_idempotent_and_durable() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let chat = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        crate::core::tests::logged_in_test_core_with_updates("exact-read-record", &owner, &device);
    let read = MessageReadState {
        created_at_secs: 100,
        expires_at_secs: None,
    };
    let record = |id: &str| DeviceSyncRecord::MessageRead {
        chat_id: chat.clone(),
        message_id: id.into(),
        read: read.clone(),
    };
    let a = record("first");
    let b = record("second");
    assert_eq!(a.scope(), RecordScope::State);
    assert_eq!(a.timestamp(), 0);
    assert_ne!(a.id(), b.id());
    for incoming in [a.clone(), b.clone(), a.clone(), b] {
        assert!(core.apply_sync_record(incoming));
    }
    assert_eq!(core.app_store.message_read_page(None).unwrap().len(), 2);
    assert_eq!(
        core.load_sync_record(&a.locator().unwrap()),
        Some(a.clone())
    );
    assert_eq!(
        core.app_store
            .message_read_state(&chat, "unlisted")
            .unwrap(),
        None
    );
    assert!(!core.apply_sync_record(DeviceSyncRecord::MessageRead {
        chat_id: chat.clone(),
        message_id: "future".into(),
        read: MessageReadState {
            created_at_secs: unix_now().get() + 600,
            expires_at_secs: None
        },
    }));
    assert!(!core.apply_sync_record(DeviceSyncRecord::MessageRead {
        chat_id: "invalid".into(),
        message_id: "first".into(),
        read
    }));
    core.chat_deletions.insert(chat.clone(), 201);
    core.app_store
        .apply_chat_deletion(&chat, 201, false, false)
        .unwrap();
    assert!(!core.sync_record_allowed(&a));
    assert!(
        core.apply_sync_record(a),
        "an intentionally deleted target resolves stale requests"
    );
    assert!(core.app_store.message_read_page(None).unwrap().is_empty());
}

#[test]
fn chat_read_records_fill_missing_pending_expiry() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let chat = Keys::generate().public_key().to_hex();
    let (mut core, _, _dir) =
        crate::core::tests::logged_in_test_core_with_updates("read-expiry", &owner, &device);
    let legacy = DeviceSyncRecord::MessageRead {
        chat_id: chat.clone(),
        message_id: "pending".into(),
        read: MessageReadState {
            created_at_secs: 100,
            expires_at_secs: None,
        },
    };
    let expires_at = unix_now().get() + 60;
    let mut complete = legacy.clone();
    if let DeviceSyncRecord::MessageRead { read, .. } = &mut complete {
        read.expires_at_secs = Some(expires_at);
    }
    assert!(core.apply_sync_record(legacy.clone()));
    for i in 0..MAX_RECENT_READ_IDS {
        assert!(core.apply_sync_record(DeviceSyncRecord::MessageRead {
            chat_id: chat.clone(),
            message_id: format!("newer-{i}"),
            read: MessageReadState {
                created_at_secs: 101 + i as u64,
                expires_at_secs: None,
            },
        }));
    }
    assert!(!core.chat_read_states[&chat]
        .seen_messages
        .contains_key("pending"));
    assert!(core.apply_sync_record(complete.clone()));
    assert!(core.apply_sync_record(legacy));
    assert_eq!(
        core.load_sync_record(&complete.locator().unwrap()),
        Some(complete)
    );
    core.app_store.delete_expired_messages(expires_at).unwrap();
    assert!(core
        .app_store
        .message_read_state(&chat, "pending")
        .unwrap()
        .is_none());
}

#[test]
fn typed_record_ids_match_browser_fixture() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/deviceSyncRecords.json")).unwrap();
    for fixture in fixtures {
        let mut wire = fixture["record"].clone();
        if wire["type"] == "message" {
            wire["message"]["body"] = base64::engine::general_purpose::STANDARD
                .encode(wire["message"]["body"].as_str().unwrap())
                .into();
        }
        let record: DeviceSyncRecord = serde_json::from_value(wire).unwrap();
        let id = record
            .id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(id, fixture["id"], "{}", fixture["name"]);
        assert_eq!(record.timestamp(), fixture["timestamp"].as_u64().unwrap());
        if let DeviceSyncRecord::Profile { event } = record {
            event.verify().unwrap();
        }
    }
}

#[test]
fn typed_record_envelope_size_is_checked_with_real_wire_encoding() {
    let keys = Keys::generate();
    let event = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"name":"Contact", "about":"a".repeat(20_000)}).to_string(),
    )
    .sign_with_keys(&keys)
    .unwrap();
    assert!(DeviceSyncRecord::Profile { event }.fits_packet());
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/deviceSyncRecords.json")).unwrap();
    let mut group: DeviceSyncRecord = serde_json::from_value(
        fixtures
            .into_iter()
            .find(|fixture| fixture["record"]["type"] == "group")
            .unwrap()["record"]
            .clone(),
    )
    .unwrap();
    let DeviceSyncRecord::Group { group: body } = &mut group else {
        unreachable!()
    };
    body.members = vec![keys.public_key().to_hex(); 2_000];
    assert!(!group.fits_packet());
}

#[test]
fn obsolete_message_history_wire_routes_are_not_supported() {
    for value in [
        serde_json::json!({"v":1,"type":"historyMessages","session":"ab".repeat(16),"messages":[],"requested":[]}),
        serde_json::json!({"v":1,"type":"historyPageEnd","linkAt":100,"linkId":"ab".repeat(32)}),
        serde_json::json!({"v":1,"type":"request","rosterAt":100,"page":{"kind":"messages","after":null}}),
        serde_json::json!({"v":1,"type":"historyOpen","session":"ab".repeat(16),"since":100,"until":200,"frame":"00"}),
    ] {
        assert!(serde_json::from_value::<DeviceSyncPacket>(value).is_err());
    }
}

#[test]
fn request_acceptance_from_sibling_reaction_and_history_records_is_durable() {
    for reaction in [false, true] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let (mut core, _, _dir) = crate::core::tests::logged_in_test_core_with_updates(
            "engagement-records",
            &owner,
            &device,
        );
        crate::core::tests::configure_test_device_sync_profile(
            &mut core,
            &owner,
            &device,
            &Keys::generate(),
            None,
        );
        let peer = Keys::generate();
        let chat = peer.public_key().to_hex();
        core.apply_runtime_text_message(
            peer.public_key(),
            None,
            "Request".into(),
            200,
            None,
            Some("request".into()),
            None,
        );
        if reaction {
            assert!(core.apply_sync_record(DeviceSyncRecord::Reaction {
                reaction: DeviceSyncReaction {
                    chat_id: chat.clone(),
                    id: "reacted-on-sibling".into(),
                    author: owner.public_key().to_hex(),
                    created_at: 201,
                    created_at_ms: Some(201_123),
                    message_id: "request".into(),
                    emoji: "❤".into(),
                },
            }));
        } else {
            core.apply_device_sync_snapshot(
                DeviceSyncSnapshot {
                    messages: vec![DeviceSyncMessage {
                        chat_id: chat.clone(),
                        id: "replied-on-sibling".into(),
                        body: "Hello".into(),
                        author: owner.public_key().to_hex(),
                        created_at: 201,
                        expires_at: None,
                        legacy_reactions: None,
                    }],
                    ..Default::default()
                },
                Some(100),
            );
        }
        assert_eq!(core.request_decision(&chat).unwrap().decision, "accepted");
        assert!(!core.thread_is_message_request(&chat));
    }
}

#[test]
fn request_acceptance_typed_reaction_never_uses_a_later_target_timestamp() {
    let owner = Keys::generate();
    let (mut core, _, _dir) = crate::core::tests::logged_in_test_core_with_updates(
        "reaction-original-clock",
        &owner,
        &Keys::generate(),
    );
    let peer = Keys::generate();
    let chat = peer.public_key().to_hex();
    core.ensure_thread_record(&chat, 100);
    assert!(core.apply_sync_record(DeviceSyncRecord::Reaction {
        reaction: DeviceSyncReaction {
            chat_id: chat.clone(),
            id: "typed-reaction".into(),
            author: owner.public_key().to_hex(),
            created_at: 200,
            created_at_ms: None,
            message_id: "late-target".into(),
            emoji: "❤".into(),
        }
    }));
    let rejection = nostr_identity::build_private_setting(
        owner.public_key(),
        "requestDecision",
        &chat,
        &serde_json::json!({"chatId":chat,"decision":"rejected","updatedAtMs":250_000}),
    )
    .unwrap();
    assert!(core.merge_private_setting(&rejection));
    core.apply_runtime_text_message(
        peer.public_key(),
        None,
        "Late target".into(),
        300,
        None,
        Some("late-target".into()),
        None,
    );
    core.restore_device_sync_record_projection();
    assert_eq!(core.request_decision(&chat).unwrap().decision, "rejected");
    assert!(core.thread_is_message_request(&chat));
}
