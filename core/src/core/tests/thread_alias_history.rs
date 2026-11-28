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
