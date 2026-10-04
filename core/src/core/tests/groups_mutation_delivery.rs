#[test]
fn message_mutations_group_storage_failures_keep_authenticated_delivery_retryable() {
    for projection_failure in [false, true] {
        let mut devices = sender_key_matrix_devices(2);
        let recipient = devices[1].owner.public_key();
        let created = devices[0]
            .engine
            .create_group("Retry edits".into(), vec![recipient], unix_now())
            .unwrap();
        let group = created.snapshot.unwrap();
        deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
        for sender in 0..2 {
            let peer = devices[1 - sender].owner.public_key();
            let sent = devices[sender]
                .engine
                .send_direct_text(peer, "mutation-warmup", "hello", None, unix_now())
                .unwrap();
            deliver_protocol_effects_to_engine(&mut devices[1 - sender].engine, &sent.effects);
        }
        let mut directories = Vec::new();
        let cores = devices
            .into_iter()
            .map(|device| {
                let (mut core, _, directory) = logged_in_test_core_with_updates(
                    "group-mutation-retry",
                    &device.owner,
                    &device.device,
                );
                directories.push(directory);
                core.protocol_engine = Some(device.engine);
                core.apply_group_decrypted_event(GroupIncomingEvent::MetadataUpdated(
                    group.clone(),
                ));
                core
            })
            .collect::<Vec<_>>();
        let mut cores: [AppCore; 2] = cores.try_into().ok().unwrap();
        let chat = group_chat_id(&group.group_id);
        cores[0].send_message(&chat, "original", None);
        deliver_group_reaction_test_events(&mut cores);
        cores[1].retry_protocol_engine_pending_work("group-mutation-original");
        let target = cores[1].threads[&chat]
            .messages
            .iter()
            .find(|message| message.body == "original")
            .unwrap()
            .id
            .clone();
        let shared = cores[1].app_store.shared();
        let trigger = if projection_failure {
            "CREATE TEMP TRIGGER reject_group_mutation BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT, 'injected projection failure'); END"
        } else {
            "CREATE TEMP TRIGGER reject_group_mutation BEFORE INSERT ON app_meta WHEN NEW.key LIKE 'iris-chat-sync-record-v1:%' BEGIN SELECT RAISE(ABORT, 'injected record failure'); END"
        };
        shared.lock().unwrap().execute_batch(trigger).unwrap();
        cores[0].send_group_event(
            &chat,
            MESSAGE_EDIT_KIND,
            "recovered edit",
            vec![vec!["e".into(), target.clone()]],
            None,
        );
        deliver_group_reaction_test_events(&mut cores);
        assert_eq!(
            cores[1].message_for_mutation(&chat, &target).unwrap().body,
            "original"
        );
        let pending = cores[1]
            .protocol_engine
            .as_mut()
            .unwrap()
            .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
            .unwrap();
        assert!(pending.direct_messages.iter().any(|delivery| delivery
            .event_id
            .as_deref()
            .is_some_and(|id| id.starts_with("group-delivery:"))));
        cores[1].process_protocol_engine_retry_batch("group-mutation-failed-retry", pending);
        assert_eq!(
            cores[1].message_for_mutation(&chat, &target).unwrap().body,
            "original"
        );
        shared
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_group_mutation")
            .unwrap();
        cores[1].retry_protocol_engine_pending_work("group-mutation-recovered");
        let message = cores[1].message_for_mutation(&chat, &target).unwrap();
        assert_eq!(message.body, "recovered edit");
        assert_eq!(message.edit_history.len(), 2);
        assert!(cores[1]
            .protocol_engine
            .as_mut()
            .unwrap()
            .retry_pending_protocol(NdrUnixSeconds(unix_now().get()))
            .unwrap()
            .direct_messages
            .is_empty());
    }
}
