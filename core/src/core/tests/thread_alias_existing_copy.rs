use super::thread_alias_deletion::prove_alias;
use super::thread_alias_history::reopen_alias_core;
use super::*;

fn receive(core: &mut AppCore, sender: &Keys, id: &str, source: &str, body: &str, at: u64) {
    core.apply_runtime_text_message(
        sender.public_key(),
        None,
        body.to_string(),
        at,
        None,
        Some(id.to_string()),
        Some(source.to_string()),
    );
}

fn assert_removed(core: &AppCore, target: &str, unrelated: &str, stage: &str) {
    let stored = core.app_store.load_recent_messages(target, 80).unwrap();
    let mut stored_ids = stored.iter().map(|m| m.id.as_str()).collect::<Vec<_>>();
    stored_ids.sort();
    let mut visible = core.threads[target]
        .messages
        .iter()
        .map(|m| m.id.as_str())
        .collect::<Vec<_>>();
    visible.sort();
    let scoped = core
        .app_store
        .search_messages_fts("stalealiasneedle", Some(target), 10)
        .unwrap();
    let global = core
        .app_store
        .search_messages_fts("stalealiasneedle", None, 10)
        .unwrap();
    assert_eq!(
        (
            stored_ids,
            visible,
            scoped.len(),
            global
                .iter()
                .map(|hit| hit.chat_id.as_str())
                .collect::<Vec<_>>()
        ),
        (
            vec!["alias-survivor", "owner-survivor"],
            vec!["alias-survivor", "owner-survivor"],
            0,
            vec![unrelated]
        ),
        "stale owner copy survived {stage}"
    );
    let shared = core.app_store.shared();
    let conn = shared.lock().unwrap();
    let count_controls = |chat: &str| {
        conn.query_row("SELECT count(*) FROM app_meta
        WHERE key LIKE 'iris-chat-sync-record-v1:%' AND CASE WHEN json_valid(value)
            THEN COALESCE(json_extract(value,'$.mutation.chatId'),json_extract(value,'$.reaction.chatId')) END=?1",
        [chat], |row| row.get::<_,i64>(0)).unwrap()
    };
    assert_eq!(
        count_controls(target),
        0,
        "removed copy retained controls at {stage}"
    );
    assert_eq!(
        count_controls(unrelated),
        2,
        "unrelated controls changed at {stage}"
    );
}

fn preexisting_owner_copy_is_removed(unloaded: bool, match_source: bool, fail_once: bool) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let other = Keys::generate();
    let alias = peer_device.public_key().to_hex();
    let target = peer.public_key().to_hex();
    let unrelated = other.public_key().to_hex();
    let copy_id = if match_source {
        "owner-copy"
    } else {
        "removed"
    };
    let copy_source = if match_source {
        "removed-source"
    } else {
        "owner-copy-source"
    };
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("alias-existing-copy", &owner, &device);
    let at = unix_now().get() - 10;
    receive(
        &mut core,
        &peer_device,
        "removed",
        "removed-source",
        "stalealiasneedle",
        at,
    );
    receive(
        &mut core,
        &peer_device,
        "alias-survivor",
        "alias-survivor-source",
        "alias survivor",
        at + 1,
    );
    core.delete_local_message(&alias, "removed");
    // The owner copy is already stored before device ownership is established.
    receive(
        &mut core,
        &peer,
        copy_id,
        copy_source,
        "stalealiasneedle",
        at,
    );
    receive(
        &mut core,
        &peer,
        "owner-survivor",
        "owner-survivor-source",
        "owner survivor",
        at + 2,
    );
    receive(
        &mut core,
        &other,
        copy_id,
        copy_source,
        "stalealiasneedle",
        at,
    );
    let prefix = core.sync_record_prefix().unwrap();
    for chat in [&target, &unrelated] {
        for (kind, inner) in [("messageMutation", "mutation"), ("reaction", "reaction")] {
            let key = format!(
                "{prefix}{}",
                serde_json::json!([kind, chat, copy_id, "old-control"])
            );
            let value = serde_json::json!({"type":kind,inner:{"chatId":chat,"messageId":copy_id,
                "id":"old-control","author":peer.public_key().to_hex(),"createdAt":at,
                "createdAtMs":at*1000,"operation":"edit","content":"erased edit body","emoji":"👍"}});
            core.app_store
                .shared()
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO app_meta(key,value) VALUES (?1,?2)",
                    rusqlite::params![key, value.to_string()],
                )
                .unwrap();
        }
    }
    core.persist_app_state().unwrap();
    if unloaded {
        core.shutdown();
        drop(core);
        core = reopen_alias_core(&owner, &device, directory.path());
        assert_eq!(core.threads[&target].messages.len(), 1);
    } else {
        assert_eq!(core.threads[&target].messages.len(), 2);
    }
    if fail_once {
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TEMP TRIGGER reject_alias_delete BEFORE DELETE ON threads
             BEGIN SELECT RAISE(ABORT, 'alias copy rollback fault'); END;",
            )
            .unwrap();
        prove_alias(&mut core, &peer, &peer_device);
        assert!(core.threads.contains_key(&alias));
        assert!(core.threads[&target]
            .messages
            .iter()
            .any(|m| m.id == copy_id));
        assert!(core
            .app_store
            .message_exists(&target, Some(copy_id), None)
            .unwrap());
        assert!(!core
            .app_store
            .message_was_locally_deleted(&target, Some(copy_id), Some(copy_source))
            .unwrap());
        assert_eq!(
            core.app_store
                .search_messages_fts("stalealiasneedle", Some(&target), 10)
                .unwrap()
                .len(),
            1
        );
        core.app_store
            .shared()
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_alias_delete")
            .unwrap();
    }
    prove_alias(&mut core, &peer, &peer_device);
    assert_removed(&core, &target, &unrelated, "immediate migration");
    assert_eq!(core.threads[&target].unread_count, 2);
    assert!(!core.threads.contains_key(&alias));
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    core.open_chat(&target);
    core.open_chat_finalize(&target);
    assert_removed(&core, &target, &unrelated, "cold reopen/read/search");
    prove_alias(&mut core, &peer, &peer_device);
    // The additional identifiers of the removed owner copy must also remain
    // deleted if the same rumor is re-encapsulated or arrives in legacy form.
    receive(
        &mut core,
        &peer,
        copy_id,
        "fresh-outer",
        "stalealiasneedle",
        at,
    );
    receive(
        &mut core,
        &peer,
        "fresh-inner",
        copy_source,
        "stalealiasneedle",
        at,
    );
    receive(
        &mut core,
        &peer,
        "removed",
        "another-outer",
        "stalealiasneedle",
        at,
    );
    core.persist_app_state().unwrap();
    assert_removed(&core, &target, &unrelated, "later authorized replay");
    assert!(core
        .app_store
        .message_was_locally_deleted(&target, Some(copy_id), None)
        .unwrap());
    assert!(core
        .app_store
        .message_was_locally_deleted(&target, None, Some(copy_source))
        .unwrap());
    assert!(!core
        .app_store
        .message_was_locally_deleted(&unrelated, Some(copy_id), Some(copy_source))
        .unwrap());
    core.shutdown();
}

#[test]
fn alias_deletion_removes_loaded_owner_copy_with_same_id() {
    preexisting_owner_copy_is_removed(false, false, false);
}
#[test]
fn alias_deletion_removes_loaded_owner_copy_with_same_source() {
    preexisting_owner_copy_is_removed(false, true, false);
}
#[test]
fn alias_deletion_removes_unloaded_owner_copy_with_same_id() {
    preexisting_owner_copy_is_removed(true, false, false);
}
#[test]
fn alias_deletion_removes_unloaded_owner_copy_with_same_source() {
    preexisting_owner_copy_is_removed(true, true, false);
}

#[test]
fn alias_deletion_of_existing_owner_copy_rolls_back_then_retries() {
    preexisting_owner_copy_is_removed(false, true, true);
}
