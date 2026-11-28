use super::message_mutations::receive_test_mutation;
use super::*;

/// A second supported SQLite writer can add history while the live core has
/// not materialized that chat (notification/history arrival while suspended).
fn seed_db_only_history(core: &AppCore, chat: &str, now: u64) {
    let mut writer = AppStore::new(core.app_store.shared());
    for (id, body) in [("older", "older history"), ("target", "original body")] {
        let mut message = test_chat_message(chat, id, body, now, false);
        message.source_event_id = Some(id.to_string());
        writer.save_message_mutation_projection(&message).unwrap();
    }
    assert!(!core.threads.contains_key(chat));
}

#[test]
fn db_only_edit_survives_partial_snapshot_and_reopen() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("db-only-edit", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.persist_app_state().unwrap();
    seed_db_only_history(&core, &chat, now);

    receive_test_mutation(
        &mut core,
        &peer,
        &chat,
        "target",
        MESSAGE_EDIT_KIND,
        "edited body",
        now * 1000 + 1,
    );
    assert_eq!(
        core.message_for_mutation(&chat, "target").unwrap().body,
        "edited body"
    );
    core.persist_app_state().unwrap();

    let reader = AppStore::new(open_database(directory.path()).unwrap());
    let thread = reader
        .load_thread(&chat, 80)
        .unwrap()
        .expect("partial app snapshot must retain the DB-only thread");
    assert_eq!(
        thread.messages.len(),
        2,
        "saving an edit must preserve older history"
    );
    assert_eq!(
        thread
            .messages
            .iter()
            .find(|m| m.id == "target")
            .unwrap()
            .body,
        "edited body"
    );
    core.shutdown();
}

#[test]
fn db_only_receive_and_open_preserve_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("db-only-receive", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.persist_app_state().unwrap();
    seed_db_only_history(&core, &chat, now);

    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "new arrival".into(),
        now + 1,
        None,
        Some("new".into()),
        None,
    );
    core.persist_app_state().unwrap();
    core.open_chat(&chat);
    core.open_chat_finalize(&chat);
    core.persist_app_state().unwrap();
    let messages = core.app_store.load_recent_messages(&chat, 80).unwrap();
    assert_eq!(messages.len(), 3);
    assert!(messages
        .iter()
        .any(|m| m.id == "older" && m.body == "older history"));
    assert!(messages
        .iter()
        .any(|m| m.id == "new" && m.body == "new arrival"));
    core.shutdown();
}

#[test]
fn loaded_reactions_survive_large_unloaded_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("reaction-history-limit", &owner, &device);
    let chat = peer.public_key().to_hex();
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer.public_key(),
        Some(chat.clone()),
        "visible message".into(),
        now,
        None,
        Some("z-visible".into()),
        None,
    );
    core.persist_app_state().unwrap();
    let prefix = core.sync_record_prefix().unwrap();
    {
        let shared = core.app_store.shared();
        let conn = shared.lock().unwrap();
        // Valid older heads for messages outside the loaded page must not use
        // up a chat-wide query limit before its visible message is reached.
        conn.execute(
            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000)
             INSERT INTO app_meta(key,value)
             SELECT ?1||json_array('reaction',?2,printf('m%06d',x),?2),
                    json_object('type','reaction','reaction',json_object(
                        'chatId',?2,'id',printf('r%06d',x),'author',?2,'createdAt',?3,
                        'messageId',printf('m%06d',x),'emoji','👍')) FROM n",
            rusqlite::params![prefix, chat, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_meta(key,value) VALUES (
                ?1||json_array('reaction',?2,'z-visible',?2),
                json_object('type','reaction','reaction',json_object(
                    'chatId',?2,'id','visible-reaction','author',?2,'createdAt',?3,
                    'messageId','z-visible','emoji','👍')))",
            rusqlite::params![prefix, chat, now],
        )
        .unwrap();
    }
    core.restore_device_sync_chat_reactions(&chat);
    let visible = core.message_for_mutation(&chat, "z-visible").unwrap();
    assert!(
        visible
            .reactors
            .iter()
            .any(|r| r.author == chat && r.emoji == "👍"),
        "unloaded reaction history must not starve the visible page"
    );
    core.shutdown();
}
