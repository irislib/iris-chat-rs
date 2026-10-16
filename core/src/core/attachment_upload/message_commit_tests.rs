use super::*;

const PAYLOAD: &[u8] = b"new attachment after existing caption references";

fn logged_in_core(directory: &Path, owner: &nostr::Keys) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
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
    core
}

fn stage_after_caption(
    core: &mut AppCore,
    directory: &Path,
    repeated: bool,
) -> (Arc<AttachmentChunkStore>, String, Cid) {
    let store = core.ensure_attachment_service().unwrap();
    let path = directory.join("new.bin");
    std::fs::write(&path, PAYLOAD).unwrap();
    let session = store.begin(None).unwrap();
    let nhash = core
        .runtime
        .block_on(upload_file_to_store(&path, session.clone()))
        .unwrap();
    // No upload target until the test reopens the store without a worker.
    store
        .finish_batch(&[(session, nhash.clone())], &[])
        .unwrap();
    let decoded = nhash_decode(&nhash).unwrap();
    let caption = (0..32)
        .map(|index| {
            let existing = nhash_encode_full(&NHashData {
                hash: hashtree_core::sha256(&[if repeated { 0 } else { index }]),
                decrypt_key: Some([7; 32]),
            })
            .unwrap();
            format!("{existing}/old.bin")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format_attachment_links_message(&caption, &[(nhash, "new.bin".into())]);
    assert!(text.len() < 32 * 1024);
    assert_eq!(extract_message_attachments(&text).1.len(), 33);
    (
        store,
        text,
        Cid {
            hash: decoded.hash,
            key: decoded.decrypt_key,
        },
    )
}

fn assert_retained_and_replicable(directory: &Path, owner: &nostr::Keys, cid: &Cid) {
    let core = logged_in_core(directory, owner);
    let store =
        AttachmentChunkStore::open(&directory.join("attachments"), local_store::DEFAULT_BUDGET)
            .unwrap();
    store.bind_owner(&owner.public_key().to_hex()).unwrap();
    // Run the same reconciliation used before startup's worker is spawned.
    // Inspect eligibility directly so no remote upload or retry clock is involved.
    core.resolve_attachment_staging(&store, None).unwrap();
    let server = "http://127.0.0.1:9".to_string();
    store
        .configure_pending(std::slice::from_ref(&server))
        .unwrap();
    assert!(store.staged_references().unwrap().is_empty());
    assert_eq!(store.next_replica().unwrap(), Some((cid.hash, server)));
    assert!(core.runtime.block_on(store.delete(&cid.hash)).is_err());
    assert_eq!(
        core.runtime
            .block_on(read_hashtree_attachment(cid, store, MAX_ATTACHMENT_BYTES))
            .unwrap(),
        PAYLOAD
    );
}

#[test]
fn caption_references_do_not_hide_new_attachment_from_send_or_restart() {
    for repeated in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let mut core = logged_in_core(dir.path(), &owner);
        let (store, text, cid) = stage_after_caption(&mut core, dir.path(), repeated);
        let peer = nostr::Keys::generate().public_key().to_hex();

        core.handle_attachment_upload_finished(peer.clone(), Ok(text));
        let authored = core.threads[&peer].messages.last().unwrap();
        assert!(core
            .app_store
            .message_exists(&peer, Some(&authored.id), None)
            .unwrap());
        assert!(authored
            .attachments
            .iter()
            .any(|attachment| { nhash_decode(&attachment.nhash).unwrap().hash == cid.hash }));
        assert!(store.staged_references().unwrap().is_empty());
        assert!(core.runtime.block_on(store.delete(&cid.hash)).is_err());

        drop(store);
        drop(core);
        assert_retained_and_replicable(dir.path(), &owner, &cid);
    }
}

#[test]
fn caption_references_do_not_hide_new_attachment_at_message_commit_crash_boundary() {
    for repeated in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let mut core = logged_in_core(dir.path(), &owner);
        let (store, text, cid) = stage_after_caption(&mut core, dir.path(), repeated);
        let peer = nostr::Keys::generate().public_key().to_hex();
        let event = core
            .prepare_direct_text_event(&peer, &text, unix_now(), None)
            .unwrap();
        let id = event.id.unwrap().to_hex();
        assert!(core.bind_attachment_message(&peer, &id, &text).unwrap());
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
        assert_eq!(
            store.staged_references().unwrap(),
            vec![(cid.hash, Some(peer), Some(id))]
        );
        assert!(store.next_replica().unwrap().is_none());

        // Crash after the authored message is durable, before finishing staging.
        drop(store);
        drop(core);
        assert_retained_and_replicable(dir.path(), &owner, &cid);
    }
}

#[test]
fn rejected_message_releases_new_attachment_after_existing_caption_references() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = logged_in_core(dir.path(), &nostr::Keys::generate());
    let (store, text, cid) = stage_after_caption(&mut core, dir.path(), true);
    let peer = nostr::Keys::generate().public_key().to_hex();
    core.preferences.blocked_owner_pubkeys.push(peer.clone());
    core.handle_attachment_upload_finished(peer, Ok(text));
    assert!(store.staged_references().unwrap().is_empty());
    assert!(store.next_replica().unwrap().is_none());
    assert!(core.runtime.block_on(store.delete(&cid.hash)).unwrap());
}
