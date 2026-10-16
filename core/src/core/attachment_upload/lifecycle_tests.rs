use super::*;

fn fresh_core(directory: &Path) -> AppCore {
    AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    )
}
fn login(core: &mut AppCore, owner: &nostr::Keys) {
    core.app_store.bind_account(owner.public_key()).unwrap();
    let device = nostr::Keys::generate();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: nostr_sdk::Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
}
fn stage(core: &mut AppCore, directory: &Path) -> (Arc<AttachmentChunkStore>, String, Cid) {
    let store = core.ensure_attachment_service().unwrap();
    let path = directory.join("fixture.bin");
    std::fs::write(&path, b"pending account-scoped attachment").unwrap();
    let session = store.begin(None).unwrap();
    let nhash = core
        .runtime
        .block_on(upload_file_to_store(&path, session.clone()))
        .unwrap();
    store
        .finish_batch(&[(session, nhash.clone())], &["http://127.0.0.1:9".into()])
        .unwrap();
    let decoded = nhash_decode(&nhash).unwrap();
    (
        store,
        nhash,
        Cid {
            hash: decoded.hash,
            key: decoded.decrypt_key,
        },
    )
}
fn message(nhash: &str) -> String {
    format_attachment_links_message("", &[(nhash.to_string(), "fixture.bin".into())])
}

#[test]
fn attachment_logout_before_restore_clears_unopened_owned_store() {
    let dir = tempfile::tempdir().unwrap();
    let alice = nostr::Keys::generate();
    let mut core = fresh_core(dir.path());
    login(&mut core, &alice);
    let (store, nhash, cid) = stage(&mut core, dir.path());
    let peer = nostr::Keys::generate().public_key().to_hex();
    core.handle_attachment_upload_finished(peer, Ok(message(&nhash)));
    assert!(store.staged_references().unwrap().is_empty());
    assert!(core.runtime.block_on(store.delete(&cid.hash)).is_err());
    drop(store);
    drop(core);
    let mut core = fresh_core(dir.path());
    assert!(core.attachment_service.is_none());
    core.logout(); // No account has been restored and no blob worker was opened.
    let bob = nostr::Keys::generate();
    login(&mut core, &bob);
    let store = core.ensure_attachment_service().unwrap();
    assert!(store.next_replica().unwrap().is_none());
    assert!(core
        .runtime
        .block_on(store.get(&cid.hash))
        .unwrap()
        .is_none());
}

#[test]
fn attachment_live_logout_invalidates_outstanding_store_and_rebinds_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = fresh_core(dir.path());
    login(&mut core, &nostr::Keys::generate());
    let (old_store, _, cid) = stage(&mut core, dir.path());
    let lease = old_store.lease(&cid.hash).unwrap();
    core.logout();
    assert!(core.runtime.block_on(old_store.get(&cid.hash)).is_err());
    login(&mut core, &nostr::Keys::generate());
    let new_store = core.ensure_attachment_service().unwrap();
    assert!(core
        .runtime
        .block_on(new_store.get(&cid.hash))
        .unwrap()
        .is_none());
    drop(lease);
    assert!(new_store.next_replica().unwrap().is_none());
}

#[test]
fn attachment_failed_logout_remains_fail_closed_for_the_next_account() {
    let dir = tempfile::tempdir().unwrap();
    let alice = nostr::Keys::generate();
    let mut core = fresh_core(dir.path());
    login(&mut core, &alice);
    let (store, _, _) = stage(&mut core, dir.path());
    drop(store);
    drop(core);
    let index = dir.path().join("attachments/index.sqlite3");
    let db = rusqlite::Connection::open(&index).unwrap();
    db.execute_batch("CREATE TRIGGER fail_reset BEFORE DELETE ON blocks BEGIN SELECT RAISE(FAIL,'fixture cleanup failure'); END;").unwrap();
    let mut core = fresh_core(dir.path());
    core.logout();
    let bob = nostr::Keys::generate();
    login(&mut core, &bob);
    assert!(core.ensure_attachment_service().is_err());
    assert!(core.attachment_service.is_none());
    let resetting: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='resetting')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(resetting);
    db.execute_batch("DROP TRIGGER fail_reset").unwrap();
    core.logout();
    login(&mut core, &bob);
    assert!(core
        .ensure_attachment_service()
        .unwrap()
        .next_replica()
        .unwrap()
        .is_none());
}

#[test]
fn attachment_account_switch_cannot_adopt_another_owners_pending_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = fresh_core(dir.path());
    let alice = nostr::Keys::generate();
    login(&mut core, &alice);
    let (store, _, _) = stage(&mut core, dir.path());
    drop(store);
    drop(core);
    // Model the original bug: private account binding got reset independently.
    let mut core = fresh_core(dir.path());
    core.app_store.clear().unwrap();
    login(&mut core, &nostr::Keys::generate());
    assert!(core.ensure_attachment_service().is_err());
    assert!(core.attachment_service.is_none());
}

#[test]
fn attachment_rejected_caption_block_and_authorization_release_only_unsent_stages() {
    for rejection in ["caption", "blocked", "authorization"] {
        let dir = tempfile::tempdir().unwrap();
        let mut core = fresh_core(dir.path());
        login(&mut core, &nostr::Keys::generate());
        let (store, nhash, cid) = stage(&mut core, dir.path());
        let peer = nostr::Keys::generate().public_key().to_hex();
        let mut text = message(&nhash);
        match rejection {
            "caption" => text = format!("{} {text}", "x".repeat(32 * 1024)),
            "blocked" => core.preferences.blocked_owner_pubkeys.push(peer.clone()),
            _ => {
                core.logged_in.as_mut().unwrap().authorization_state =
                    LocalAuthorizationState::Revoked
            }
        }
        core.handle_attachment_upload_finished(peer, Ok(text));
        assert!(store.staged_references().unwrap().is_empty(), "{rejection}");
        assert!(store.next_replica().unwrap().is_none(), "{rejection}");
        assert!(
            core.runtime.block_on(store.delete(&cid.hash)).unwrap(),
            "{rejection}"
        );
    }
}

#[test]
fn attachment_crash_recovery_resolves_exact_durable_message_boundary() {
    for boundary in ["before_binding", "before_message", "after_message"] {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let mut core = fresh_core(dir.path());
        login(&mut core, &owner);
        let (store, nhash, cid) = stage(&mut core, dir.path());
        let peer = nostr::Keys::generate().public_key().to_hex();
        let text = message(&nhash);
        let event = core
            .prepare_direct_text_event(&peer, &text, unix_now(), None)
            .unwrap();
        let id = event.id.unwrap().to_hex();
        if boundary != "before_binding" {
            assert!(core.bind_attachment_message(&peer, &id, &text).unwrap());
        }
        if boundary == "after_message" {
            let snapshot = core.push_outgoing_message_with_id(
                id.clone(),
                &peer,
                text,
                unix_now().get(),
                None,
                DeliveryState::Queued,
            );
            core.app_store
                .save_outgoing_event(&core.threads[&peer], &snapshot, &event)
                .unwrap();
        }
        assert_eq!(store.staged_references().unwrap().len(), 1);
        assert!(
            store.next_replica().unwrap().is_none(),
            "uncommitted attachments must not replicate"
        );
        drop(store);
        drop(core);
        let mut core = fresh_core(dir.path());
        login(&mut core, &owner);
        let store = core.ensure_attachment_service().unwrap();
        assert!(store.staged_references().unwrap().is_empty());
        if boundary == "after_message" {
            assert!(core.runtime.block_on(store.delete(&cid.hash)).is_err());
            assert_eq!(
                core.runtime
                    .block_on(read_hashtree_attachment(&cid, store, MAX_ATTACHMENT_BYTES))
                    .unwrap(),
                b"pending account-scoped attachment"
            );
        } else {
            assert!(core.runtime.block_on(store.delete(&cid.hash)).unwrap());
        }
    }
}

#[test]
fn attachment_group_message_is_durable_before_releasing_staging() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = fresh_core(dir.path());
    let owner = nostr::Keys::generate();
    login(&mut core, &owner);
    let (store, nhash, cid) = stage(&mut core, dir.path());
    let group_id = "attachment-staging-group";
    let group = GroupSnapshot {
        group_id: group_id.into(),
        protocol: nostr_double_ratchet::GroupProtocol::sender_key_v1(),
        name: "Fixture".into(),
        picture: None,
        about: None,
        created_by: nostr_double_ratchet::OwnerPubkey::from_bytes(owner.public_key().to_bytes()),
        members: vec![nostr_double_ratchet::OwnerPubkey::from_bytes(
            owner.public_key().to_bytes(),
        )],
        admins: vec![nostr_double_ratchet::OwnerPubkey::from_bytes(
            owner.public_key().to_bytes(),
        )],
        revision: 1,
        created_at: nostr_double_ratchet::UnixSeconds(1),
        updated_at: nostr_double_ratchet::UnixSeconds(1),
    };
    core.groups.insert(group_id.into(), group);
    let chat = group_chat_id(group_id);
    core.handle_attachment_upload_finished(chat.clone(), Ok(message(&nhash)));
    let authored = core.threads[&chat].messages.last().unwrap();
    assert!(core
        .app_store
        .message_exists(&chat, Some(&authored.id), None)
        .unwrap());
    assert!(store.staged_references().unwrap().is_empty());
    assert!(core.runtime.block_on(store.delete(&cid.hash)).is_err());
    drop(store);
    drop(core);
    let mut reopened = fresh_core(dir.path());
    login(&mut reopened, &owner);
    let store = reopened.ensure_attachment_service().unwrap();
    assert!(reopened.runtime.block_on(store.delete(&cid.hash)).is_err());
}
