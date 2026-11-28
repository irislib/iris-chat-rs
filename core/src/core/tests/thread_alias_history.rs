use super::*;

fn reopen_alias_core(owner: &Keys, device: &Keys, path: &std::path::Path) -> AppCore {
    let mut core =
        logged_in_test_core_at_data_dir(owner, device, path.to_string_lossy().into_owned());
    let persisted = core.load_persisted().unwrap().unwrap();
    core.next_message_id = persisted.next_message_id.max(1);
    core.threads = persisted
        .threads
        .into_iter()
        .map(|thread| {
            (
                thread.chat_id.clone(),
                ThreadRecord {
                    chat_id: thread.chat_id,
                    unread_count: thread.unread_count,
                    updated_at_secs: thread.updated_at_secs,
                    draft: thread.draft,
                    messages: thread
                        .messages
                        .iter()
                        .map(chats::chat_message_from_persisted)
                        .collect(),
                },
            )
        })
        .collect();
    core
}

fn verified_alias_history_after_restart(save_before_alias: bool) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let alias = peer_device.public_key().to_hex();
    let target = peer.public_key().to_hex();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("alias-history", &owner, &device);
    let now = unix_now().get();
    for (sender, id, body, offset) in [
        (&peer_device, "alias-old", "old alias history", 0),
        (&peer_device, "alias-middle", "middle alias history", 1),
        (&peer_device, "alias-latest", "latest alias history", 2),
        (&peer, "owner-old", "old owner history", 0),
        (&peer, "owner-latest", "latest owner history", 2),
    ] {
        core.apply_runtime_text_message(
            sender.public_key(),
            Some(sender.public_key().to_hex()),
            body.to_string(),
            now + offset,
            None,
            Some(id.to_string()),
            None,
        );
    }
    core.persist_app_state().unwrap();
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    assert_eq!(core.threads[&alias].messages.len(), 1);
    assert_eq!(core.threads[&target].messages.len(), 1);
    if save_before_alias {
        core.persist_app_state().unwrap();
    }

    // Exercise the normal verified owner proof, not only the private merge helper.
    let proof = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), now)])
        .get_event(peer.public_key())
        .sign_with_keys(&peer)
        .unwrap();
    assert!(core.apply_app_keys_event(&proof).unwrap());
    core.persist_app_state().unwrap();
    core.shutdown();
    drop(core);
    let mut reopened = reopen_alias_core(&owner, &device, directory.path());
    let mut history: Vec<_> = reopened
        .app_store
        .load_recent_messages(&target, 80)
        .unwrap()
        .into_iter()
        .map(|message| (message.id, message.body))
        .collect();
    history.sort();
    let mut expected: Vec<_> = [
        ("alias-old", "old alias history"),
        ("alias-middle", "middle alias history"),
        ("alias-latest", "latest alias history"),
        ("owner-old", "old owner history"),
        ("owner-latest", "latest owner history"),
    ]
    .into_iter()
    .map(|(id, body)| (id.to_string(), body.to_string()))
    .collect();
    expected.sort();
    let alias_survived = reopened.threads.contains_key(&alias)
        || reopened
            .app_store
            .load_thread(&alias, 80)
            .unwrap()
            .is_some();
    reopened.shutdown();
    assert_eq!((history, alias_survived), (expected, false),
        "verified alias migration must move unloaded history and erase the alias, save_before_alias={save_before_alias}");
}

#[test]
fn verified_alias_preserves_unloaded_history_before_first_snapshot() {
    verified_alias_history_after_restart(false);
}

#[test]
fn verified_alias_preserves_unloaded_history_after_first_snapshot() {
    verified_alias_history_after_restart(true);
}

#[test]
fn verified_alias_retries_identical_proof_after_transaction_failure() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let alias = peer_device.public_key().to_hex();
    let target = peer.public_key().to_hex();
    let (mut core, _, _directory) =
        logged_in_test_core_with_updates("alias-failure", &owner, &device);
    let now = unix_now().get();
    core.apply_runtime_text_message(
        peer_device.public_key(),
        Some(alias.clone()),
        "alias history".into(),
        now,
        None,
        Some("alias".into()),
        None,
    );
    core.persist_app_state().unwrap();
    let proof = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), now)])
        .get_event(peer.public_key())
        .sign_with_keys(&peer)
        .unwrap();
    let mut forged = proof.clone();
    forged.content.push_str("tampered");
    core.handle_relay_event(forged);
    assert!(core.threads.contains_key(&alias));
    assert!(!core.threads.contains_key(&target));
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TEMP TRIGGER reject_alias_delete BEFORE DELETE ON threads
         BEGIN SELECT RAISE(ABORT, 'alias transaction fault'); END;",
        )
        .unwrap();
    core.apply_app_keys_event(&proof).unwrap();
    assert!(
        core.threads.contains_key(&alias),
        "failed migration must preserve live source"
    );
    assert!(!core.threads.contains_key(&target));
    assert_eq!(
        core.app_store
            .load_recent_messages(&alias, 80)
            .unwrap()
            .len(),
        1
    );
    assert!(core.app_store.load_thread(&target, 80).unwrap().is_none());
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_alias_delete")
        .unwrap();
    core.apply_app_keys_event(&proof).unwrap();
    assert!(!core.threads.contains_key(&alias));
    assert_eq!(
        core.app_store
            .load_recent_messages(&target, 80)
            .unwrap()
            .len(),
        1
    );
    let unread = core.threads[&target].unread_count;
    core.apply_app_keys_event(&proof).unwrap();
    assert_eq!(core.threads[&target].unread_count, unread);
    assert!(core.app_store.load_thread(&alias, 80).unwrap().is_none());
    core.shutdown();
}

#[test]
fn verified_alias_deduplicates_against_unloaded_target_history() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let alias = peer_device.public_key().to_hex();
    let target = peer.public_key().to_hex();
    let (mut core, _, directory) = logged_in_test_core_with_updates("alias-dedup", &owner, &device);
    let now = unix_now().get();
    core.persist_app_state().unwrap();
    for (chat, id, event, body, at) in [
        (&target, "same-id", "target-id-event", "target id wins", now),
        (
            &target,
            "target-source",
            "same-source",
            "target source wins",
            now,
        ),
        (
            &target,
            "target-latest",
            "target-latest",
            "target latest",
            now + 2,
        ),
        (&alias, "same-id", "alias-id-event", "alias id loses", now),
        (
            &alias,
            "alias-unique",
            "alias-unique",
            "alias unique",
            now + 1,
        ),
        (
            &alias,
            "alias-source",
            "same-source",
            "alias source loses",
            now + 2,
        ),
    ] {
        let mut message = test_chat_message(chat, id, body, at, false);
        message.source_event_id = Some(event.to_string());
        core.app_store
            .save_message_mutation_projection(&message)
            .unwrap();
    }
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    assert_eq!(core.threads[&alias].messages[0].id, "alias-source");
    assert_eq!(core.threads[&target].messages[0].id, "target-latest");
    let proof = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), now)])
        .get_event(peer.public_key())
        .sign_with_keys(&peer)
        .unwrap();
    core.apply_app_keys_event(&proof).unwrap();
    core.persist_app_state().unwrap();
    let messages = core.app_store.load_recent_messages(&target, 80).unwrap();
    assert_eq!(messages.len(), 4);
    for (id, body) in [
        ("same-id", "target id wins"),
        ("target-source", "target source wins"),
        ("alias-unique", "alias unique"),
        ("target-latest", "target latest"),
    ] {
        assert_eq!(messages.iter().find(|m| m.id == id).unwrap().body, body);
    }
    assert!(core.threads[&target]
        .messages
        .iter()
        .all(|m| m.id != "alias-source"));
    assert!(core.app_store.load_thread(&alias, 80).unwrap().is_none());
    core.shutdown();
}
