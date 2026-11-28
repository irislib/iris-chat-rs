use super::*;
use crate::core::storage::open_database;

fn save(store: &mut AppStore, threads: &BTreeMap<String, ThreadRecord>) -> anyhow::Result<()> {
    store.save_state(&SaveSnapshot {
        active_chat_id: None,
        next_message_id: 1,
        authorization_state: None,
        preferences: &PreferencesSnapshot::default(),
        owner_profiles: &BTreeMap::new(),
        chat_message_ttl_seconds: &BTreeMap::new(),
        app_keys: &BTreeMap::new(),
        groups: &BTreeMap::new(),
        group_pictures: &BTreeMap::new(),
        threads,
        seen_event_order: &VecDeque::new(),
    })
}

fn message(id: usize) -> ChatMessageSnapshot {
    ChatMessageSnapshot {
        id: format!("m{id}"),
        chat_id: "chat".into(),
        kind: ChatMessageKind::User,
        author: "alice".into(),
        author_owner_pubkey_hex: None,
        author_picture_url: None,
        body: format!("history {id}"),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: id as u64 + 1,
        expires_at_secs: None,
        delivery: DeliveryState::Received,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: None,
        edit_history: Vec::new(),
        deleted_for_everyone: false,
        system_notice_owner_pubkey_hex: None,
        direct_transfer: None,
        call: None,
    }
}

fn threads() -> BTreeMap<String, ThreadRecord> {
    [(
        "chat".into(),
        ThreadRecord {
            chat_id: "chat".into(),
            unread_count: 128,
            updated_at_secs: 128,
            messages: (0..128).map(message).collect(),
            draft: String::new(),
        },
    )]
    .into_iter()
    .collect()
}

#[test]
fn fingerprints_follow_loaded_pages_and_temporary_receipt_rows() {
    let directory = tempfile::TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(directory.path()).unwrap());
    let mut threads = threads();
    save(&mut store, &threads).unwrap();
    let full_thread = threads["chat"].clone();
    threads.get_mut("chat").unwrap().messages = vec![message(127)];

    // Dropping a page changes no rows or header; even the no-write save must
    // release its fingerprints without deleting stored history.
    save(&mut store, &threads).unwrap();
    assert_eq!(
        store.cache.threads["chat"].messages.len(),
        1,
        "old page fingerprints retained"
    );
    assert_eq!(store.load_recent_messages("chat", 200).unwrap().len(), 128);

    let loaded = store.load_recent_messages("chat", 200).unwrap();
    store.seed_loaded_messages("chat", &loaded);
    save(&mut store, &threads).unwrap();
    assert_eq!(store.cache.threads["chat"].messages.len(), 1);

    // Receipt handling may temporarily use older rows outside the live page.
    let mut seen = message(0);
    seen.delivery = DeliveryState::Seen;
    store
        .save_message_receipts("chat", &[seen], Some(&full_thread))
        .unwrap();
    save(&mut store, &threads).unwrap();
    assert_eq!(store.cache.threads["chat"].messages.len(), 1);
    let restored = store.load_recent_messages("chat", 200).unwrap();
    assert_eq!(restored.len(), 128);
    assert!(matches!(
        restored.iter().find(|m| m.id == "m0").unwrap().delivery,
        PersistedDeliveryState::Seen
    ));
    let changes = store.shared().lock().unwrap().total_changes();
    save(&mut store, &threads).unwrap();
    assert_eq!(store.shared().lock().unwrap().total_changes(), changes);
}

#[test]
fn intentional_snapshot_deletion_retries_after_transaction_failure() {
    let directory = tempfile::TempDir::new().unwrap();
    let mut store = AppStore::new(open_database(directory.path()).unwrap());
    save(&mut store, &threads()).unwrap();
    store.shared().lock().unwrap().execute_batch(
        "CREATE TEMP TRIGGER reject_delete BEFORE DELETE ON threads BEGIN SELECT RAISE(ABORT, 'review fault'); END;"
    ).unwrap();
    assert!(save(&mut store, &BTreeMap::new()).is_err());
    assert_eq!(store.cache.threads["chat"].messages.len(), 128);
    assert_eq!(store.load_recent_messages("chat", 200).unwrap().len(), 128);
    store
        .shared()
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_delete;")
        .unwrap();
    save(&mut store, &BTreeMap::new()).unwrap();
    assert!(!store.cache.threads.contains_key("chat"));
    assert!(store.load_thread("chat", 200).unwrap().is_none());
}
