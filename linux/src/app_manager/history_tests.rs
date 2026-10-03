use super::*;

pub fn verify_ui(manager: Rc<AppManager>) {
    let rx = manager.update_rx();
    let wait = |ready: &dyn Fn() -> bool| {
        let until = Instant::now() + Duration::from_secs(60);
        while !ready() {
            while let Ok(update) = rx.try_recv() {
                manager.apply_update(update);
            }
            let context = glib::MainContext::default();
            while context.pending() {
                context.iteration(false);
            }
            assert!(Instant::now() < until, "History fixture timed out");
            thread::sleep(Duration::from_millis(2));
        }
    };
    manager.dispatch(AppAction::CreateAccount {
        name: "History fixture".into(),
    });
    wait(&|| manager.current_state().account.is_some());
    let own = manager.current_state().account.unwrap().public_key_hex;
    manager.dispatch(AppAction::CreateChat {
        peer_input: own.clone(),
    });
    wait(&|| {
        manager
            .current_state()
            .current_chat
            .as_ref()
            .is_some_and(|chat| chat.chat_id == own)
    });
    for i in 0..240 {
        manager.dispatch(AppAction::SendMessage {
            chat_id: own.clone(),
            text: format!("History fixture {i:03}"),
        });
    }
    wait(&|| {
        manager
            .current_state()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len()
            == 240
    });
    let all = manager.current_state().current_chat.unwrap().messages;
    manager.dispatch(AppAction::UpdateScreenStack { stack: vec![] });
    manager.dispatch(AppAction::OpenChat {
        chat_id: own.clone(),
    });
    wait(&|| {
        manager
            .current_state()
            .current_chat
            .as_ref()
            .is_some_and(|chat| chat.messages.len() == 80)
    });
    assert!(manager.load_older_messages(&own));
    assert!(
        !manager.load_older_messages(&own),
        "One concurrent page per route"
    );
    wait(&|| {
        manager
            .current_state()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len()
            == 160
    });
    assert!(manager.load_older_messages(&own));
    wait(&|| {
        manager
            .current_state()
            .current_chat
            .as_ref()
            .unwrap()
            .messages
            .len()
            == 240
    });
    assert!(manager.load_older_messages(&own));
    wait(&|| manager.history_paging.borrow().loading_before.is_none());
    assert!(
        !manager.load_older_messages(&own),
        "No repeated query after the beginning"
    );

    let previous = manager.current_state();
    for fresh_messages in [all[161..].to_vec(), all[160..239].to_vec(), vec![]] {
        let mut fresh = previous.clone();
        let chat = fresh.current_chat.as_mut().unwrap();
        chat.messages = fresh_messages.clone();
        chat.draft.clear();
        chat.display_name = "Fresh title".into();
        preserve_page(
            &previous,
            &mut fresh,
            Some(&all[160..].iter().map(|m| m.id.clone()).collect()),
        );
        let expected: Vec<_> = all[..160].iter().chain(&fresh_messages).cloned().collect();
        let chat = fresh.current_chat.unwrap();
        assert_eq!(
            chat.messages, expected,
            "Deleted recent first/last/all remain deleted"
        );
        assert_eq!(chat.display_name, "Fresh title");
        assert!(chat.draft.is_empty());
    }
    let mut page = all[..81].to_vec();
    page[0].expires_at_secs = Some(1);
    let mut current = all[80..82].to_vec();
    current[0].body = "Fresh edit".into();
    current[0].reactions = vec![iris_chat_core::MessageReactionSnapshot {
        emoji: "👍".into(),
        count: 1,
        reacted_by_me: true,
    }];
    let merged = merge_messages(&page, &current);
    assert_eq!(merged.len(), 81);
    assert_eq!(merged[79], current[0]);
    assert_eq!(
        merged.iter().map(|m| &m.id).collect::<HashSet<_>>().len(),
        merged.len()
    );
    // Same-second bursts preserve database order instead of sorting random IDs.
    for message in &mut page {
        message.created_at_secs = 1;
    }
    for message in &mut current {
        message.created_at_secs = 1;
    }
    assert_eq!(merge_messages(&page, &current).last(), current.last());

    let mut fresh = previous.clone();
    fresh.current_chat.as_mut().unwrap().messages = vec![all[81].clone()];
    fresh.current_chat.as_mut().unwrap().messages[0].body = "Newest edit".into();
    fresh.current_chat.as_mut().unwrap().display_name = "Newest metadata".into();
    let mut late_page = previous.current_chat.clone().unwrap();
    late_page.messages = all[79..82].to_vec();
    let mut request = Paging {
        read_ids: all[80..82].iter().map(|m| m.id.clone()).collect(),
        ..Default::default()
    };
    assert!(request.complete(0, &own, all[82].id.clone(), Some(late_page), &mut fresh));
    let chat = fresh.current_chat.unwrap();
    assert_eq!(
        chat.messages.iter().map(|m| &m.id).collect::<Vec<_>>(),
        vec![&all[79].id, &all[81].id]
    );
    assert_eq!(chat.messages[1].body, "Newest edit");
    assert_eq!(chat.display_name, "Newest metadata");

    let generation = manager.history_paging.borrow().generation;
    let before = all[160].id.clone();
    let mut old_page = previous.current_chat.clone().unwrap();
    old_page.messages = all[..80].to_vec();
    manager.dispatch(AppAction::PushScreen {
        screen: Screen::GroupDetails {
            group_id: "synthetic".into(),
        },
    });
    manager.dispatch(AppAction::OpenChat {
        chat_id: own.clone(),
    });
    let mut state = manager.current_state();
    let mut paging = manager.history_paging.borrow_mut();
    paging.loading_before = Some("new-request".into());
    assert!(!paging.complete(generation, &own, before, Some(old_page), &mut state));
    assert_eq!(
        paging.loading_before.as_deref(),
        Some("new-request"),
        "Stale completion cannot clear a newer flight"
    );
    drop(paging);
    let mut changed_account = state.clone();
    changed_account.account.as_mut().unwrap().public_key_hex = "different".into();
    manager
        .history_paging
        .borrow_mut()
        .update_scope(&changed_account);
    assert_ne!(manager.history_paging.borrow().generation, generation);
    manager.ffi.shutdown();
    println!("PASS: GTK actual 240-message SQLite paging, authoritative deletes, overlap edits/expiry, stable ties and route/account cancellation");
}
