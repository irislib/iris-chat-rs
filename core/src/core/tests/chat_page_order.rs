#[test]
fn open_chat_merges_same_second_cached_and_stored_order() {
    for cached_start in [0, 239, 240] {
        let owner = Keys::generate();
        let device = Keys::generate();
        let peer = Keys::generate();
        let mut core = logged_in_test_core("open-chat-anchored-page", &owner, &device);
        let chat_id = peer.public_key().to_hex();
        let time = 1_777_159_500;
        let messages: Vec<_> = (0..240)
            .map(|i| {
                test_chat_message(
                    &chat_id,
                    &format!("row-{i:03}"),
                    &format!("message {i}"),
                    time,
                    true,
                )
            })
            .collect();
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 0,
                updated_at_secs: time,
                messages: messages.clone(),
                draft: String::new(),
            },
        );
        core.persist_best_effort();
        core.threads.get_mut(&chat_id).unwrap().messages = messages[cached_start..].to_vec();
        if cached_start == 240 {
            core.threads
                .get_mut(&chat_id)
                .unwrap()
                .messages
                .push(test_chat_message(
                    &chat_id,
                    "unsaved-only",
                    "Unsaved without anchor",
                    time,
                    true,
                ));
        }
        core.open_chat(&chat_id);
        let current = core.state.current_chat.as_ref().unwrap();
        let mut expected = messages[cached_start.min(160)..]
            .iter()
            .map(|m| m.id.clone())
            .collect::<Vec<_>>();
        if cached_start == 240 {
            expected.push("unsaved-only".into());
        }
        assert_eq!(current.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), expected,
            "cached_start={cached_start}: preserve stored order and cached history across shared IDs");
        if cached_start == 240 {
            core.threads
                .get_mut(&chat_id)
                .unwrap()
                .messages
                .retain(|m| m.id != "unsaved-only");
        }

        // A fresh DB-only row must precede a same-second unsaved live tail.
        // Overlap content remains authoritative even when the cached copy is stale.
        core.threads
            .get_mut(&chat_id)
            .unwrap()
            .messages
            .iter_mut()
            .find(|m| m.id == "row-239")
            .unwrap()
            .body = "Fresh DB edit".into();
        core.persist_best_effort();
        core.threads
            .get_mut(&chat_id)
            .unwrap()
            .messages
            .iter_mut()
            .find(|m| m.id == "row-239")
            .unwrap()
            .body = "Stale cached value".into();
        let stored = test_chat_message(&chat_id, "row-240", "New stored row", time, false);
        core.app_store
            .upsert_notification_preview_message(&chat_id, 0, time, &stored)
            .unwrap();
        let live = test_chat_message(&chat_id, "row-241", "Unsaved live tail", time, true);
        core.threads.get_mut(&chat_id).unwrap().messages.push(live);
        core.open_chat(&chat_id);
        let current = core.state.current_chat.as_ref().unwrap();
        let expected_start = cached_start.min(160);
        assert_eq!(
            current
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
            (expected_start..242)
                .map(|i| format!("row-{i:03}"))
                .collect::<Vec<_>>(),
            "stored-only rows and unseen live tail retain same-second order"
        );
        assert_eq!(
            current
                .messages
                .iter()
                .find(|m| m.id == "row-239")
                .unwrap()
                .body,
            "Fresh DB edit"
        );
        assert_eq!(current.messages.last().unwrap().body, "Unsaved live tail");
    }
}
