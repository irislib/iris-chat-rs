mod message_mutation_cleanup {
    use super::*;
    use rusqlite::params;
    use std::collections::BTreeSet;

    fn store() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AppStore::new(open_database(dir.path()).unwrap());
        (dir, store)
    }

    fn message(store: &mut AppStore, chat: &str, id: &str, created: u64, expiry: Option<u64>) {
        let mut message = test_chat_message(chat, id, "original secret", created, false);
        message.source_event_id = Some(format!("source-{id}"));
        message.expires_at_secs = expiry;
        store.save_message_mutation_projection(&message).unwrap();
    }

    fn control(
        store: &AppStore,
        device: &str,
        kind: &str,
        chat: &str,
        target: &str,
        id: &str,
        created: u64,
        expiry: Option<u64>,
    ) -> String {
        let inner = if kind == "reaction" {
            "reaction"
        } else {
            "mutation"
        };
        let mut record = serde_json::json!({
            "chatId": chat, "id": id, "author": "author", "messageId": target,
            "createdAt": created, "createdAtMs": created * 1000,
            "operation": "edit", "content": "retained edit secret", "emoji": "👍"
        });
        if let Some(expiry) = expiry {
            record["expiresAt"] = expiry.into();
        }
        let value = serde_json::json!({ "type": kind, inner: record });
        let key = format!(
            "iris-chat-sync-record-v1:owner:{device}:{}",
            serde_json::json!([kind, chat, target, id])
        );
        meta(store, &key, &value.to_string());
        key
    }

    fn meta(store: &AppStore, key: &str, value: &str) {
        store
            .shared()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_meta(key,value) VALUES (?1,?2)",
                params![key, value],
            )
            .unwrap();
    }

    fn retained(store: &AppStore, key: &str) -> bool {
        store
            .shared()
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM app_meta WHERE key=?1)",
                [key],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn local_delete_purges_all_target_versions_and_reactions_across_device_prefixes() {
        let (_dir, mut store) = store();
        let chat = "group:quoted-\"-🌱";
        message(&mut store, chat, "target", 100, None);
        let removed = [
            control(
                &store,
                "a",
                "messageMutation",
                chat,
                "target",
                "edit-1",
                110,
                None,
            ),
            control(
                &store,
                "b",
                "messageMutation",
                chat,
                "source-target",
                "edit-2",
                120,
                None,
            ),
            control(
                &store,
                "b",
                "messageMutation",
                chat,
                "supplied-source",
                "edit-3",
                130,
                None,
            ),
            control(
                &store, "a", "reaction", chat, "target", "reaction", 115, None,
            ),
        ];
        let other_message = control(
            &store,
            "a",
            "messageMutation",
            chat,
            "other",
            "other-edit",
            120,
            None,
        );
        let other_chat = control(
            &store,
            "a",
            "messageMutation",
            "other-chat",
            "target",
            "edit",
            120,
            None,
        );
        let malformed = "iris-chat-sync-record-v1:owner:a:malformed";
        meta(&store, malformed, "not JSON");
        let wrong_shape = "iris-chat-sync-record-v1:owner:a:wrong-shape";
        meta(
            &store,
            wrong_shape,
            r#"{"type":"messageMutation","mutation":"not JSON"}"#,
        );
        meta(&store, "unrelated-setting", "private setting");

        store
            .delete_message_locally(chat, "target", Some("supplied-source"))
            .unwrap();

        for key in removed {
            assert!(!retained(&store, &key));
        }
        for key in [
            &other_message,
            &other_chat,
            malformed,
            wrong_shape,
            "unrelated-setting",
        ] {
            assert!(retained(&store, key));
        }
        assert!(!store.message_exists(chat, Some("target"), None).unwrap());
        assert!(store
            .message_was_locally_deleted(chat, Some("target"), None)
            .unwrap());
        assert!(store
            .search_messages_fts("secret", Some(chat), 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn remote_delete_purges_old_device_plaintext_but_keeps_all_deletion_heads() {
        let (_dir, mut store) = store();
        message(&mut store, "chat", "target", 100, None);
        let removed = [
            control(
                &store,
                "active",
                "messageMutation",
                "chat",
                "target",
                "edit",
                110,
                None,
            ),
            control(
                &store,
                "old-device",
                "messageMutation",
                "chat",
                "source-target",
                "old-edit",
                120,
                None,
            ),
            control(
                &store,
                "old-device",
                "reaction",
                "chat",
                "target",
                "reaction",
                130,
                None,
            ),
        ];
        let mut preserved = vec![control(
            &store,
            "old-device",
            "messageMutation",
            "other-chat",
            "target",
            "unrelated",
            110,
            None,
        )];
        for device in ["active", "old-device"] {
            let deletion = control(
                &store,
                device,
                "messageMutation",
                "chat",
                "target",
                "deletion",
                140,
                None,
            );
            store.shared().lock().unwrap().execute(
                "UPDATE app_meta SET value=json_set(value,'$.mutation.operation','delete','$.mutation.content','') WHERE key=?1",
                [&deletion],
            ).unwrap();
            preserved.push(deletion);
        }
        let malformed = "iris-chat-sync-record-v1:owner:old-device:malformed";
        meta(&store, malformed, "not JSON");
        let mut tombstone = test_chat_message("chat", "target", "", 100, false);
        tombstone.source_event_id = Some("source-target".into());
        tombstone.deleted_for_everyone = true;
        store.save_message_mutation_projection(&tombstone).unwrap();
        for key in removed {
            assert!(!retained(&store, &key));
        }
        for key in preserved {
            assert!(retained(&store, &key));
        }
        assert!(retained(&store, malformed));
        let stored = store.load_messages_around("chat", "target", 0, 0).unwrap();
        assert!(stored[0].deleted_for_everyone && stored[0].body.is_empty());
        assert!(store
            .search_messages_fts("secret", Some("chat"), 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn expiry_purges_target_versions_and_expired_deferred_controls_without_original() {
        let (_dir, mut store) = store();
        message(&mut store, "chat", "expired", 100, Some(200));
        message(&mut store, "chat", "future", 100, Some(201));
        let removed = [
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "expired",
                "edit",
                150,
                None,
            ),
            control(
                &store,
                "a",
                "reaction",
                "chat",
                "source-expired",
                "reaction",
                150,
                None,
            ),
            control(
                &store,
                "b",
                "messageMutation",
                "missing-chat",
                "missing",
                "deferred",
                150,
                Some(200),
            ),
        ];
        let preserved = [
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "future",
                "edit",
                150,
                None,
            ),
            control(
                &store,
                "a",
                "messageMutation",
                "missing-chat",
                "later",
                "later",
                150,
                Some(201),
            ),
            control(
                &store,
                "a",
                "messageMutation",
                "missing-chat",
                "untimed",
                "untimed",
                150,
                None,
            ),
        ];

        assert_eq!(store.delete_expired_messages(200).unwrap(), 1);
        for key in removed {
            assert!(!retained(&store, &key));
        }
        for key in preserved {
            assert!(retained(&store, &key));
        }
        assert!(store.message_exists("chat", Some("future"), None).unwrap());
        assert!(store
            .message_was_locally_deleted("chat", Some("expired"), None)
            .unwrap());
        // Deferred records must also expire when there are no message rows to delete.
        assert_eq!(store.delete_expired_messages(201).unwrap(), 1);
        let deferred = control(
            &store,
            "a",
            "messageMutation",
            "missing-chat",
            "last",
            "last",
            201,
            Some(202),
        );
        assert_eq!(store.delete_expired_messages(202).unwrap(), 0);
        assert!(!retained(&store, &deferred));
    }

    #[test]
    fn chat_delete_purges_old_target_edits_but_preserves_newer_messages() {
        let (_dir, mut store) = store();
        message(&mut store, "chat", "old", 100, None);
        message(&mut store, "chat", "new", 160, None);
        let removed = [
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "old",
                "late-edit",
                200,
                None,
            ),
            control(
                &store,
                "b",
                "reaction",
                "chat",
                "source-old",
                "reaction",
                210,
                None,
            ),
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "missing",
                "old-deferred",
                120,
                None,
            ),
        ];
        let preserved = [
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "new",
                "new-edit",
                170,
                None,
            ),
            control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "later-missing",
                "new-deferred",
                170,
                None,
            ),
            control(
                &store,
                "a",
                "messageMutation",
                "other-chat",
                "old",
                "other-edit",
                120,
                None,
            ),
        ];

        store.apply_chat_deletion("chat", 150, true, true).unwrap();

        for key in removed {
            assert!(!retained(&store, &key));
        }
        for key in &preserved {
            assert!(retained(&store, key));
        }
        assert!(!store.message_exists("chat", Some("old"), None).unwrap());
        assert!(store.message_exists("chat", Some("new"), None).unwrap());
        // Removing the thread cascades even newer physical rows; their edits
        // must follow those rows while deferred post-cutoff controls survive.
        store
            .apply_chat_deletion("chat", 150, false, false)
            .unwrap();
        assert!(!store.message_exists("chat", Some("new"), None).unwrap());
        assert!(!retained(&store, &preserved[0]));
        assert!(retained(&store, &preserved[1]));
    }

    #[test]
    fn deferred_control_arms_and_runs_expiry_timer_without_a_message_row() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, _dir) =
            logged_in_test_core_with_updates("mutation-cleanup-timer", &owner, &device);
        let (sender, receiver) = flume::unbounded();
        core.core_sender = sender;
        let now = unix_now().get();
        let expiry = now + 2;
        let chat = peer.public_key().to_hex();
        assert!(core.capture_device_sync_control(
            &chat,
            "deferred-expiring-edit",
            &chat,
            now,
            MESSAGE_EDIT_KIND,
            "short-lived secret",
            &[
                nostr::Tag::parse(["e", "missing-original"]).unwrap(),
                nostr::Tag::parse(["expiration", &expiry.to_string()]).unwrap(),
            ],
        ));
        assert_eq!(
            core.message_mutation_records(&chat, "missing-original")
                .len(),
            1
        );
        assert_eq!(
            core.app_store.next_message_expiration_after(now).unwrap(),
            Some(expiry)
        );
        assert!(!core
            .app_store
            .message_exists(&chat, Some("missing-original"), None)
            .unwrap());
        // Observe the actual timer event instead of sleeping and manually invoking cleanup.
        loop {
            let event = receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("scheduled expiry event");
            if let CoreMsg::Internal(event) = event {
                if let InternalEvent::PruneExpiredMessages { token } = *event {
                    assert_eq!(token, core.message_expiry_token);
                    core.handle_prune_expired_messages(token);
                    break;
                }
            }
        }
        assert!(core
            .message_mutation_records(&chat, "missing-original")
            .is_empty());
        assert_eq!(
            core.app_store.next_message_expiration_after(now).unwrap(),
            None
        );
    }

    #[test]
    fn account_restore_keeps_old_device_versions_when_new_edits_arrive() {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let (mut core, _, _dir) =
            logged_in_test_core_with_updates("mutation-cleanup-rotation", &owner, &device);
        let now = unix_now().get();
        let chat = peer.public_key().to_hex();
        let target = "rotation-original";
        core.apply_runtime_text_message(
            peer.public_key(),
            Some(chat.clone()),
            "original secret".into(),
            now,
            None,
            Some(target.into()),
            None,
        );
        let first = receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "first edit",
            now * 1000 + 10,
        );
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "second edit",
            now * 1000 + 20,
        );
        assert!(core.capture_device_sync_control(
            &chat,
            "old-reaction",
            &chat,
            now,
            REACTION_KIND,
            "👍",
            &[nostr::Tag::parse(["e", target]).unwrap()],
        ));
        let old_prefix = core.sync_record_prefix().unwrap();
        meta(
            &core.app_store,
            &format!("{old_prefix}malformed"),
            "{bad json",
        );
        core.persist_best_effort();
        // Simulate an interrupted projection save: startup must replay the old
        // device's durable heads, even though the new device has no heads yet.
        core.app_store.shared().lock().unwrap().execute(
            "UPDATE messages SET body='original secret', edit_history_json='[]' WHERE chat_id=?1 AND id=?2",
            params![chat, target],
        ).unwrap();
        // This supported sign-in path replaces the device key, but keeps the
        // account database and the controls stored under the previous key.
        core.restore_primary_session(&owner.secret_key().to_secret_hex());
        assert_ne!(
            core.logged_in.as_ref().unwrap().device_keys.public_key(),
            device.public_key()
        );
        assert_eq!(core.message_mutation_records(&chat, target).len(), 2);
        assert_eq!(
            core.message_for_mutation(&chat, target).unwrap().body,
            "second edit"
        );
        let exported = core.export_sync_record_values_for_test().unwrap();
        assert_eq!(
            exported
                .iter()
                .filter(|record| record["type"] == "messageMutation")
                .count(),
            2
        );
        assert!(!exported.iter().any(|record| record["type"] == "reaction"));
        // A repeated control stored under the new device must not duplicate a
        // history entry. Malformed unrelated metadata must not block reads.
        assert!(core.capture_device_sync_control(
            &chat,
            &first,
            &chat,
            now,
            MESSAGE_EDIT_KIND,
            "first edit",
            &[
                nostr::Tag::parse(["e", target]).unwrap(),
                nostr::Tag::parse(["ms", &(now * 1000 + 10).to_string()]).unwrap(),
            ],
        ));
        receive_test_mutation(
            &mut core,
            &peer,
            &chat,
            target,
            MESSAGE_EDIT_KIND,
            "third edit",
            now * 1000 + 30,
        );
        let message = core.message_for_mutation(&chat, target).unwrap();
        assert_eq!(message.body, "third edit");
        assert_eq!(
            message
                .edit_history
                .iter()
                .map(|version| version.body.as_str())
                .collect::<Vec<_>>(),
            vec!["original secret", "first edit", "second edit", "third edit"]
        );
        assert_eq!(core.message_mutation_records(&chat, target).len(), 3);
        assert!(core.capture_device_sync_control(
            &chat,
            "new-reaction",
            &chat,
            now,
            REACTION_KIND,
            "🌱",
            &[nostr::Tag::parse(["e", target]).unwrap()],
        ));
        let exported = core.export_sync_record_values_for_test().unwrap();
        let edits = exported
            .iter()
            .filter(|record| record["type"] == "messageMutation")
            .map(|record| record["mutation"]["id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(edits.len(), 3);
        let reactions = exported
            .iter()
            .filter(|record| record["type"] == "reaction")
            .collect::<Vec<_>>();
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0]["reaction"]["id"], "new-reaction");
    }

    #[test]
    fn all_cleanup_paths_roll_back_messages_and_markers_if_control_purge_fails() {
        for path in ["local", "expiry", "chat", "remote"] {
            let (_dir, mut store) = store();
            message(&mut store, "chat", "target", 100, Some(200));
            let key = control(
                &store,
                "a",
                "messageMutation",
                "chat",
                "target",
                "edit",
                120,
                None,
            );
            store.shared().lock().unwrap().execute_batch(
                "CREATE TRIGGER prevent_control_cleanup BEFORE DELETE ON app_meta
                 WHEN OLD.key >= 'iris-chat-sync-record-v1:' AND OLD.key < 'iris-chat-sync-record-v1;'
                 BEGIN SELECT RAISE(ABORT, 'simulated cleanup failure'); END;"
            ).unwrap();
            let result = match path {
                "local" => store.delete_message_locally("chat", "target", None),
                "expiry" => store.delete_expired_messages(200).map(|_| ()),
                "chat" => store.apply_chat_deletion("chat", 150, false, false),
                _ => {
                    let mut tombstone = test_chat_message("chat", "target", "", 100, false);
                    tombstone.deleted_for_everyone = true;
                    store.save_message_mutation_projection(&tombstone)
                }
            };
            assert!(result.is_err(), "{path} must propagate cleanup failure");
            assert!(store.message_exists("chat", Some("target"), None).unwrap());
            assert!(!store
                .message_was_locally_deleted("chat", Some("target"), None)
                .unwrap());
            assert!(store.load_chat_deletions().unwrap().is_empty());
            assert!(retained(&store, &key));
        }
    }
}
