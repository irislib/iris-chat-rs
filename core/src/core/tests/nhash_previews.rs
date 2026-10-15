#[test]
fn nhash_previews_hide_valid_references_in_encrypted_push_cached_push_and_chat_lists() {
    let mut farm = GroupScaleFarm::new(&[1, 1]);
    farm.pump();
    farm.devices[0].core.create_group("Preview test", &[]);
    let group_id = farm.devices[0].core.groups.keys().next().unwrap().clone();
    farm.add_members(&group_id, 1..2);
    farm.retry();
    let chat = group_chat_id(&group_id);
    let hash = hashtree_core::nhash_encode(&[1; 32]).unwrap();
    farm.devices[1].core.active_chat_id = None;
    farm.devices[1].core.rebuild_persist_and_emit_state();
    for body in [
        hash.clone(),
        format!("Caption {hash}"),
        format!("htree://{hash} nhash://{hash}"),
        format!("[Download]({hash})"),
        format!("{hash}/photo.png"),
        format!("Caption {hash}/photo.png"),
        format!("↩ Someone: {hash}\n\nReply {hash}"),
        format!("Forwarded:\n\n{hash}"),
    ] {
        let before = farm.devices[1].core.state.chat_list.clone();
        farm.devices[0].core.send_message(&chat, &body, None);
        let authors = farm.devices[0]
            .core
            .protocol_engine
            .as_ref()
            .unwrap()
            .known_group_sender_event_pubkeys();
        let event = sorted_pending_events_for_test(&farm.devices[0].core)
            .into_iter()
            .find(|event| authors.contains(&event.pubkey))
            .unwrap();
        let payload = serde_json::json!({"event":event}).to_string();
        let resolve = |receiver: &GroupScaleDevice| {
            decrypt_mobile_push_notification(
                receiver._dir.path().to_string_lossy().into_owned(),
                farm.owners[1].public_key().to_hex(),
                receiver.keys.secret_key().to_secret_hex(),
                payload.clone(),
            )
        };
        let preview = resolve(&farm.devices[1]);
        assert!(preview.should_show);
        assert!(
            !preview.body.contains(&hash),
            "decrypted preview leaked {body}"
        );
        if body.starts_with("Caption") {
            assert!(preview.body.contains("Caption"));
        }
        farm.pump();
        farm.devices[1].core.persist_best_effort_inner();
        let preview = decrypt_mobile_push_notification(
            farm.devices[1]._dir.path().to_string_lossy().into_owned(),
            farm.owners[1].public_key().to_hex(),
            farm.devices[1].keys.secret_key().to_secret_hex(),
            payload,
        );
        assert!(preview.should_show);
        assert!(
            !preview.body.contains(&hash),
            "cached preview leaked {body}"
        );
        if body.starts_with("Caption") {
            assert!(preview.body.contains("Caption"));
        }
        let receiver = &farm.devices[1].core;
        let thread = receiver
            .state
            .chat_list
            .iter()
            .find(|item| item.chat_id == chat)
            .unwrap();
        assert!(!thread
            .last_message_preview
            .as_deref()
            .unwrap()
            .contains(&hash));
        let candidate = notifications::decide_notifications(
            &before,
            &receiver.state.chat_list,
            &receiver.preferences,
            false,
            None,
        );
        assert_eq!(candidate.len(), 1);
        assert!(!candidate[0].body.contains(&hash));
        if !body.contains("/photo.png") {
            assert_eq!(
                receiver.threads[&chat].messages.last().unwrap().body,
                body,
                "presentation never rewrites saved body"
            );
        }
    }
}
