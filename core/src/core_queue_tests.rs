use super::*;

fn background_msg(index: usize) -> CoreMsg {
    CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
        category: "test.background".to_string(),
        detail: index.to_string(),
    }))
}

#[test]
fn foreground_queue_preempts_background_backlog() {
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }
    foreground_tx
        .send(CoreMsg::Action(AppAction::NavigateBack))
        .unwrap();

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert!(matches!(
        batch.first(),
        Some(CoreMsg::Action(AppAction::NavigateBack))
    ));
    assert!(
        batch.iter().all(is_foreground_core_msg),
        "foreground work should not be bundled behind background backlog"
    );
}

#[test]
fn foreground_internal_preempts_background_backlog() {
    let (foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }
    foreground_tx
        .send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "test.priority".to_string(),
            detail: "priority".to_string(),
        })))
        .unwrap();

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert!(matches!(
        batch.first(),
        Some(CoreMsg::Internal(event))
            if matches!(
                event.as_ref(),
                InternalEvent::DebugLog { detail, .. } if detail == "priority"
            )
    ));
}

#[test]
fn background_queue_drains_in_bounded_chunks() {
    let (_foreground_tx, foreground_rx) = flume::unbounded();
    let (background_tx, background_rx) = flume::unbounded();
    for index in 0..100 {
        background_tx.send(background_msg(index)).unwrap();
    }

    let batch = recv_core_batch(&foreground_rx, &background_rx).unwrap();

    assert_eq!(batch.len(), CORE_BACKGROUND_BATCH_LIMIT);
    assert!(batch.iter().all(|msg| !is_foreground_core_msg(msg)));
}

#[test]
fn route_chat_snapshot_uses_chat_list_without_core_queue() {
    let state = build_large_test_app_state(80, 20, 1_200);
    let chat_id = state.chat_list[10].chat_id.clone();

    let snapshot =
        crate::core::chat_snapshot_from_state_and_db(&state, None, &chat_id, 80).unwrap();

    assert_eq!(snapshot.chat_id, chat_id);
    assert_eq!(snapshot.display_name, state.chat_list[10].display_name);
    assert!(snapshot.messages.is_empty());
}

#[test]
fn route_chat_snapshot_requires_account() {
    let mut state = build_large_test_app_state(80, 20, 1_200);
    state.account = None;
    let chat_id = state.chat_list[10].chat_id.clone();

    assert!(crate::core::chat_snapshot_from_state_and_db(&state, None, &chat_id, 80).is_none());
}

#[test]
fn search_preserves_thread_results_with_large_active_history() {
    let app = ffi_app_failure(String::new());
    let mut state = build_large_test_app_state(80, 20, 1_200);
    state.chat_list[10].nickname = Some("Needle contact".to_string());
    state.chat_list[85].about = Some("Needle group".to_string());
    let expected_contact = state.chat_list[10].clone();
    let expected_group = state.chat_list[85].clone();
    let chat_id = expected_contact.chat_id.clone();
    *app.shared_state.write().unwrap() = state;

    let results = app.search("needle".to_string(), None, 80);

    assert_eq!(results.contacts, vec![expected_contact]);
    assert_eq!(results.groups, vec![expected_group]);
    assert!(results.people.is_empty());
    assert!(results.messages.is_empty());

    let scoped = app.search("needle".to_string(), Some(chat_id.clone()), 80);
    assert_eq!(scoped.scope_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(scoped.contacts.is_empty());
    assert!(scoped.groups.is_empty());
    assert_eq!(
        app.shared_state
            .read()
            .unwrap()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len(),
        1_200,
    );
}
