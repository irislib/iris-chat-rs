use super::*;
use nostr::{EventBuilder, Keys, Kind};

fn fixture() -> (tempfile::TempDir, AppStore, ShareDraft) {
    let directory = tempfile::tempdir().unwrap();
    let mut store = AppStore::new(crate::core::storage::open_database(directory.path()).unwrap());
    let owner = Keys::generate().public_key();
    store.bind_account(owner).unwrap();
    let draft = ShareDraft {
        id: "share-1".into(),
        account_owner_pubkey_hex: owner.to_hex(),
        chat_ids: vec![
            Keys::generate().public_key().to_hex(),
            "group:friends".into(),
        ],
        text: "Shared link".into(),
        attachments: vec![],
    };
    (directory, store, draft)
}

fn authored(
    draft: &ShareDraft,
    chat: &str,
    text: &str,
) -> (UnsignedEvent, ThreadRecord, ChatMessageSnapshot) {
    let mut event = EventBuilder::new(Kind::from(14), text)
        .build(PublicKey::from_hex(&draft.account_owner_pubkey_hex).unwrap());
    let message = ChatMessageSnapshot {
        id: event.id().to_hex(),
        chat_id: chat.into(),
        body: text.into(),
        kind: ChatMessageKind::User,
        author: "You".into(),
        author_owner_pubkey_hex: Some(draft.account_owner_pubkey_hex.clone()),
        author_picture_url: None,
        is_outgoing: true,
        created_at_secs: event.created_at.as_secs(),
        expires_at_secs: None,
        delivery: DeliveryState::Queued,
        attachments: vec![],
        reactions: vec![],
        reactors: vec![],
        recipient_deliveries: vec![],
        delivery_trace: Default::default(),
        source_event_id: None,
        call: None,
        direct_transfer: None,
        system_notice_owner_pubkey_hex: None,
        edit_history: vec![],
        deleted_for_everyone: false,
    };
    let thread = ThreadRecord {
        chat_id: chat.into(),
        unread_count: 0,
        updated_at_secs: message.created_at_secs,
        draft: "Unrelated composer draft".into(),
        messages: vec![message.clone()],
    };
    (event, thread, message)
}

#[test]
fn duplicate_request_is_idempotent_but_changed_content_or_targets_is_rejected() {
    let (_directory, mut store, draft) = fixture();
    store.accept_share(&draft).unwrap();
    store.accept_share(&draft).unwrap();
    let mut changed = draft.clone();
    changed.text.push('!');
    assert!(store.accept_share(&changed).is_err());
    changed = draft.clone();
    changed.chat_ids.reverse();
    assert!(store.accept_share(&changed).is_err());
    let saved = store
        .shared_submission(&draft.id, &draft.account_owner_pubkey_hex)
        .unwrap();
    assert!(saved.draft.as_ref() == Some(&draft));
    assert_eq!(saved.recipients.len(), 2);
    assert!(saved.recipients.iter().all(|recipient| !recipient.queued));
}

#[test]
fn account_switch_and_unbound_database_cannot_adopt_an_old_share() {
    let (_directory, mut store, draft) = fixture();
    let mut other = draft.clone();
    other.account_owner_pubkey_hex = Keys::generate().public_key().to_hex();
    assert!(store.accept_share(&other).is_err());
    store.accept_share(&draft).unwrap();
    store.clear().unwrap();
    assert!(store.accept_share(&draft).is_err());
    store
        .bind_account(PublicKey::from_hex(&other.account_owner_pubkey_hex).unwrap())
        .unwrap();
    assert!(store.accept_share(&draft).is_err());
    assert!(store
        .shared_submission(&draft.id, &other.account_owner_pubkey_hex)
        .is_err());
}

#[test]
fn partial_fanout_restart_retains_exact_event_and_skips_completed_recipient() {
    let (directory, mut store, draft) = fixture();
    store.accept_share(&draft).unwrap();
    let first = authored(&draft, &draft.chat_ids[0], &draft.text);
    let second = authored(&draft, &draft.chat_ids[1], &draft.text);
    for (event, thread, _) in [&first, &second] {
        store
            .prepare_shared_event(&draft.id, &thread.chat_id, event)
            .unwrap();
    }
    assert!(store
        .save_shared_outgoing_event(&draft.id, &first.1, &first.2, &first.0)
        .unwrap());
    drop(store);
    let mut restored =
        AppStore::new(crate::core::storage::open_database(directory.path()).unwrap());
    let pending = restored
        .shared_submission(&draft.id, &draft.account_owner_pubkey_hex)
        .unwrap();
    assert!(pending.draft.is_some());
    assert!(pending.recipients[0].queued);
    assert!(pending.recipients[0].event.is_none());
    assert!(!pending.recipients[1].queued);
    assert_eq!(
        pending.recipients[1].event.as_ref().unwrap().id,
        second.0.id
    );
    assert!(!restored
        .save_shared_outgoing_event(&draft.id, &first.1, &first.2, &first.0)
        .unwrap());
    assert!(restored
        .save_shared_outgoing_event(&draft.id, &second.1, &second.2, &second.0)
        .unwrap());
    let completed = restored
        .shared_submission(&draft.id, &draft.account_owner_pubkey_hex)
        .unwrap();
    assert!(completed.draft.is_none() && completed.prepared_text.is_none());
    assert!(completed
        .recipients
        .iter()
        .all(|recipient| recipient.queued && recipient.event.is_none()));
    // Retained receipts still reject a reused request ID after its body is scrubbed.
    restored.accept_share(&draft).unwrap();
    let mut changed = draft.clone();
    changed.text.push('!');
    assert!(restored.accept_share(&changed).is_err());
}

#[test]
fn storage_failure_cannot_acknowledge_a_recipient_without_its_message() {
    let (_directory, mut store, draft) = fixture();
    store.accept_share(&draft).unwrap();
    let (event, thread, message) = authored(&draft, &draft.chat_ids[0], &draft.text);
    store
        .prepare_shared_event(&draft.id, &thread.chat_id, &event)
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_test_share BEFORE INSERT ON messages
        BEGIN SELECT RAISE(ABORT, 'simulated storage failure'); END;",
        )
        .unwrap();
    assert!(store
        .save_shared_outgoing_event(&draft.id, &thread, &message, &event)
        .is_err());
    let saved = store
        .shared_submission(&draft.id, &draft.account_owner_pubkey_hex)
        .unwrap();
    assert!(!saved.recipients[0].queued);
    assert_eq!(saved.recipients[0].event.as_ref().unwrap().id, event.id);
    assert!(!store
        .message_exists(&thread.chat_id, Some(&message.id), None)
        .unwrap());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_test_share;")
        .unwrap();
    assert!(store
        .save_shared_outgoing_event(&draft.id, &thread, &message, &event)
        .unwrap());
}

#[test]
fn prepared_attachment_text_survives_restart_and_cannot_be_replaced() {
    let (directory, mut store, mut draft) = fixture();
    draft.attachments.push(ShareAttachment {
        path: "photo.jpg".into(),
        filename: "Photo.jpg".into(),
    });
    store.accept_share(&draft).unwrap();
    let event = authored(&draft, &draft.chat_ids[0], "Prepared attachment link").0;
    assert!(store
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &event)
        .is_err());
    store
        .prepare_shared_text(&draft.id, &draft.account_owner_pubkey_hex, &event.content)
        .unwrap();
    drop(store);
    let mut restored =
        AppStore::new(crate::core::storage::open_database(directory.path()).unwrap());
    let pending = restored
        .shared_submission(&draft.id, &draft.account_owner_pubkey_hex)
        .unwrap();
    assert_eq!(
        pending.prepared_text.as_deref(),
        Some(event.content.as_str())
    );
    restored
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &event)
        .unwrap();
    assert!(restored
        .prepare_shared_text(&draft.id, &draft.account_owner_pubkey_hex, "Changed link")
        .is_err());
}

#[test]
fn a_recipient_cannot_gain_a_second_event_or_unapproved_destination() {
    let (_directory, mut store, draft) = fixture();
    store.accept_share(&draft).unwrap();
    let (event, _, _) = authored(&draft, &draft.chat_ids[0], &draft.text);
    store
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &event)
        .unwrap();
    store
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &event)
        .unwrap();
    let mut changed = event.clone();
    changed.created_at = nostr::Timestamp::from(changed.created_at.as_secs() + 1);
    changed.id = None;
    changed.id();
    assert!(store
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &changed)
        .is_err());
    assert!(store
        .prepare_shared_event(&draft.id, "unapproved-chat", &event)
        .is_err());
    let mut forged = event.clone();
    forged.content.push('!');
    assert!(store
        .prepare_shared_event(&draft.id, &draft.chat_ids[0], &forged)
        .is_err());
}

#[test]
fn malformed_staging_paths_and_duplicate_recipients_are_rejected() {
    let (_directory, mut store, mut draft) = fixture();
    draft.chat_ids.push(draft.chat_ids[0].clone());
    assert!(store.accept_share(&draft).is_err());
    draft.chat_ids.pop();
    for path in ["../outside", "/private/account", "photo/../../outside", ""] {
        draft.attachments = vec![ShareAttachment {
            path: path.into(),
            filename: "Photo".into(),
        }];
        assert!(store.accept_share(&draft).is_err());
    }
}

#[test]
fn completed_share_does_not_resurrect_a_deleted_message() {
    let (_directory, mut store, mut draft) = fixture();
    draft.chat_ids.truncate(1);
    store.accept_share(&draft).unwrap();
    let (event, thread, message) = authored(&draft, &draft.chat_ids[0], &draft.text);
    store
        .prepare_shared_event(&draft.id, &thread.chat_id, &event)
        .unwrap();
    store
        .save_shared_outgoing_event(&draft.id, &thread, &message, &event)
        .unwrap();
    store.delete_message(&thread.chat_id, &message.id).unwrap();
    store.accept_share(&draft).unwrap();
    assert!(!store
        .save_shared_outgoing_event(&draft.id, &thread, &message, &event)
        .unwrap());
    assert!(!store
        .message_exists(&thread.chat_id, Some(&message.id), None)
        .unwrap());
}
