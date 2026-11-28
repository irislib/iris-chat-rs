use super::thread_alias_history::reopen_alias_core;
use super::*;

fn incoming(core: &mut AppCore, sender: &Keys, id: &str, source: &str, at: u64) {
    core.apply_runtime_text_message(
        sender.public_key(),
        None,
        format!("body {id}"),
        at,
        None,
        Some(id.to_string()),
        Some(source.to_string()),
    );
}

fn prove_alias(core: &mut AppCore, peer: &Keys, peer_device: &Keys) {
    let proof = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)])
        .get_event(peer.public_key())
        .sign_with_keys(peer)
        .unwrap();
    core.apply_app_keys_event(&proof).unwrap();
    core.persist_app_state().unwrap();
}

#[test]
fn late_alias_migration_respects_deleted_owner_cutoff_after_cold_reopen() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let unrelated = Keys::generate();
    let target = peer.public_key().to_hex();
    let alias = peer_device.public_key().to_hex();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("alias-chat-deletion", &owner, &device);
    let old = unix_now().get() - 10;
    incoming(&mut core, &peer, "owner-old", "owner-source", old);
    core.delete_chat(&target);
    let cutoff = core.chat_deletions[&target];
    incoming(
        &mut core,
        &peer_device,
        "alias-old",
        "alias-old-source",
        old,
    );
    incoming(
        &mut core,
        &peer_device,
        "alias-new",
        "alias-new-source",
        cutoff + 1,
    );
    incoming(&mut core, &unrelated, "alias-old", "alias-old-source", old);
    core.persist_app_state().unwrap();
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    assert_eq!(core.threads[&alias].messages.len(), 1);
    prove_alias(&mut core, &peer, &peer_device);
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    core.open_chat(&target);
    core.open_chat_finalize(&target);
    let stored = core.app_store.load_recent_messages(&target, 80).unwrap();
    let ids = stored.iter().map(|m| m.id.as_str()).collect::<Vec<_>>();
    assert_eq!(
        ids,
        ["alias-new"],
        "whole-chat cutoff must exclude migrated older history"
    );
    assert_eq!(
        core.threads[&target]
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["alias-new"]
    );
    assert!(core.app_store.load_thread(&alias, 80).unwrap().is_none());
    assert_eq!(core.chat_deletions[&target], cutoff);
    assert!(core
        .app_store
        .message_exists(&unrelated.public_key().to_hex(), Some("alias-old"), None)
        .unwrap());
    incoming(&mut core, &peer, "old-replay", "replay-source", old);
    assert!(!core
        .app_store
        .message_exists(&target, Some("old-replay"), None)
        .unwrap());
    core.shutdown();
}

#[test]
fn alias_individual_deletions_reject_resolved_owner_id_and_source_replays() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let unrelated = Keys::generate();
    let target = peer.public_key().to_hex();
    let alias = peer_device.public_key().to_hex();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("alias-message-deletion", &owner, &device);
    let at = unix_now().get() - 10;
    incoming(&mut core, &peer_device, "removed", "removed-source", at);
    incoming(
        &mut core,
        &peer_device,
        "survivor",
        "survivor-source",
        at + 1,
    );
    core.delete_local_message(&alias, "removed");
    incoming(&mut core, &unrelated, "removed", "removed-source", at);
    assert!(core
        .app_store
        .message_was_locally_deleted(&alias, Some("removed"), Some("removed-source"))
        .unwrap());
    core.persist_app_state().unwrap();
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    prove_alias(&mut core, &peer, &peer_device);
    core.shutdown();
    drop(core);
    let mut core = reopen_alias_core(&owner, &device, directory.path());
    // Reinstall the signed authorization after restart. These are the actual
    // authenticated receive projection entry points with the resolved owner.
    prove_alias(&mut core, &peer, &peer_device);
    incoming(&mut core, &peer, "removed", "fresh-outer-source", at);
    incoming(&mut core, &peer, "fresh-inner-id", "removed-source", at);
    core.persist_app_state().unwrap();
    let messages = core.app_store.load_recent_messages(&target, 80).unwrap();
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["survivor"],
        "canonical replay must retain both ID and source-event deletion guards"
    );
    assert!(core
        .app_store
        .message_was_locally_deleted(&target, Some("removed"), None)
        .unwrap());
    assert!(core
        .app_store
        .message_was_locally_deleted(&target, None, Some("removed-source"))
        .unwrap());
    assert!(core
        .app_store
        .message_exists(&unrelated.public_key().to_hex(), Some("removed"), None)
        .unwrap());
    assert!(!core
        .app_store
        .message_was_locally_deleted(
            &unrelated.public_key().to_hex(),
            Some("removed"),
            Some("removed-source")
        )
        .unwrap());
    core.shutdown();
}
