use iris_chat_core::{
    build_large_test_app_state, build_large_test_search_result, ChatKind, Screen,
};

#[test]
fn large_app_state_fixture_is_deterministic_and_populated() {
    let first = build_large_test_app_state(40, 12, 250);
    let second = build_large_test_app_state(40, 12, 250);

    assert_eq!(first, second);
    assert_eq!(first.chat_list.len(), 52);
    assert_eq!(
        first
            .chat_list
            .iter()
            .filter(|chat| matches!(chat.kind, ChatKind::Direct))
            .count(),
        40
    );
    assert_eq!(
        first
            .chat_list
            .iter()
            .filter(|chat| matches!(chat.kind, ChatKind::Group))
            .count(),
        12
    );
    assert_eq!(
        first
            .current_chat
            .as_ref()
            .expect("current chat")
            .messages
            .len(),
        250
    );
    assert!(matches!(first.router.default_screen, Screen::ChatList));
    assert!(first.account.is_some());
}

#[test]
fn large_search_fixture_preserves_all_requested_sections() {
    let result = build_large_test_search_result("needle".to_string(), 11, 25, 9, 120);

    assert_eq!(result.query, "needle");
    assert_eq!(result.people.len(), 11);
    assert_eq!(result.contacts.len(), 25);
    assert_eq!(result.groups.len(), 9);
    assert_eq!(result.messages.len(), 120);
    assert!(result
        .contacts
        .iter()
        .all(|chat| chat.display_name.to_lowercase().contains("needle")));
    assert!(result
        .groups
        .iter()
        .all(|chat| chat.display_name.to_lowercase().contains("needle")));
    assert!(result
        .messages
        .iter()
        .all(|message| message.body.contains("needle")));
}

#[test]
fn large_fixtures_cap_unbounded_counts() {
    let state = build_large_test_app_state(u32::MAX, u32::MAX, u32::MAX);
    let current_chat = state.current_chat.expect("current chat");

    assert_eq!(state.chat_list.len(), 4_000);
    assert_eq!(current_chat.messages.len(), 10_000);

    let search =
        build_large_test_search_result(String::new(), u32::MAX, u32::MAX, u32::MAX, u32::MAX);
    assert_eq!(search.query, "needle");
    assert_eq!(search.people.len(), 2_000);
    assert_eq!(search.contacts.len(), 2_000);
    assert_eq!(search.groups.len(), 2_000);
    assert_eq!(search.messages.len(), 10_000);
}
