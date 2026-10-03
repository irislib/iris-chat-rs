#[derive(Clone)]
struct ReceiptBatchStorage {
    inner: CountingStorage,
    updates: flume::Receiver<AppUpdate>,
    observed_chat: Arc<std::sync::Mutex<Option<String>>>,
    seen_visible_before_checkpoint: Arc<std::sync::atomic::AtomicBool>,
}

impl StorageAdapter for ReceiptBatchStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        if let Some(chat_id) = self.observed_chat.lock().unwrap().take() {
            for update in self.updates.try_iter() {
                if let AppUpdate::FullState(state) = update {
                    if state.current_chat.as_ref().is_some_and(|chat| {
                        chat.chat_id == chat_id
                            && !chat.messages.is_empty()
                            && chat
                                .messages
                                .iter()
                                .all(|message| matches!(message.delivery, DeliveryState::Seen))
                    }) {
                        self.seen_visible_before_checkpoint
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
        }
        self.inner.put(key, value)
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
}

enum ReceiptBatchTrigger {
    VisibleMessages,
    OpenChatFinalize,
}

fn group_receipt_batch_scenario(trigger: ReceiptBatchTrigger, nested: bool) -> (usize, bool) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _directory) =
        logged_in_test_core_with_updates("group-receipt-batch", &owner, &device);
    let storage = Arc::new(ReceiptBatchStorage {
        inner: CountingStorage::new(),
        updates: updates.clone(),
        observed_chat: Arc::new(std::sync::Mutex::new(None)),
        seen_visible_before_checkpoint: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let mut engine = test_protocol_engine_with_storage(&owner, &device, storage.clone());
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let authors = (0..8).map(|_| Keys::generate()).collect::<Vec<_>>();
    for author in &authors {
        let author_device = Keys::generate();
        observe_peer_device_invite_for_test(&mut engine, author, &author_device, 2);
        assert_eq!(
            engine.direct_send_readiness(author.public_key()),
            DirectSendReadiness::Ready
        );
    }
    let created = engine
        .create_group(
            "Receipt batch".into(),
            authors.iter().map(Keys::public_key).collect(),
            UnixSeconds(3),
        )
        .unwrap();
    let group = created.snapshot.unwrap();
    let chat_id = group_chat_id(&group.group_id);
    core.apply_group_roster_snapshot(group, 3);
    core.protocol_engine = Some(engine);
    core.preferences.send_read_receipts = true;
    let message_ids = authors
        .iter()
        .enumerate()
        .map(|(index, author)| {
            let id = format!("receipt-message-{index}");
            core.push_incoming_message_from(
                &chat_id,
                Some(id.clone()),
                format!("Message from author {index}"),
                10 + index as u64,
                None,
                None,
                Some(author.public_key().to_hex()),
                Some(format!("receipt-outer-{index}")),
            );
            id
        })
        .collect::<Vec<_>>();
    core.persist_best_effort();
    core.handle_messages(vec![CoreMsg::Action(AppAction::OpenChat {
        chat_id: chat_id.clone(),
    })]);
    updates.try_iter().for_each(drop);
    core.pending_relay_publishes.clear();
    let writes_before = storage.inner.put_count();
    *storage.observed_chat.lock().unwrap() = Some(chat_id.clone());
    if nested {
        core.enter_batch();
    }
    let message = match trigger {
        ReceiptBatchTrigger::VisibleMessages => CoreMsg::Action(AppAction::MarkMessagesSeen {
            chat_id: chat_id.clone(),
            message_ids,
        }),
        ReceiptBatchTrigger::OpenChatFinalize => {
            CoreMsg::Internal(Box::new(InternalEvent::OpenChatFinalize {
                chat_id: chat_id.clone(),
            }))
        }
    };
    core.handle_messages(vec![message]);
    if nested {
        assert_eq!(storage.inner.put_count(), writes_before);
        assert!(
            updates.is_empty(),
            "nested batches must keep updates coalesced"
        );
        core.exit_batch();
    }

    assert_eq!(protocol_send_log_count(&core, "receipt"), authors.len());
    assert!(core.pending_outgoing_receipts.is_empty());
    assert_eq!(
        pending_events_with_kind(&core, MESSAGE_EVENT_KIND).len(),
        authors.len(),
        "each author still receives its private receipt"
    );
    assert!(core
        .app_store
        .load_chat_read_states()
        .unwrap()
        .contains_key(&chat_id));
    (
        storage.inner.put_count() - writes_before,
        storage
            .seen_visible_before_checkpoint
            .load(std::sync::atomic::Ordering::SeqCst),
    )
}

#[test]
fn group_receipt_batch_has_one_protocol_checkpoint() {
    assert_eq!(
        group_receipt_batch_scenario(ReceiptBatchTrigger::VisibleMessages, false).0,
        1,
        "a group page must not rewrite the complete protocol backlog for every author"
    );
}

#[test]
fn group_receipt_batch_shows_durable_read_state_before_protocol_checkpoint() {
    assert!(
        group_receipt_batch_scenario(ReceiptBatchTrigger::VisibleMessages, false).1,
        "local read state must be visible before the expensive protocol checkpoint"
    );
}

#[test]
fn nested_group_receipt_batch_waits_for_outer_commit() {
    assert_eq!(
        group_receipt_batch_scenario(ReceiptBatchTrigger::VisibleMessages, true),
        (1, true)
    );
}

#[test]
fn open_chat_receipt_batch_projects_read_state_before_one_protocol_checkpoint() {
    assert_eq!(
        group_receipt_batch_scenario(ReceiptBatchTrigger::OpenChatFinalize, false),
        (1, true)
    );
}
