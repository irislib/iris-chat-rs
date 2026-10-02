#[derive(Clone)]
struct SendDisplayStorage {
    inner: InMemoryStorage,
    updates: flume::Receiver<AppUpdate>,
    expected: Arc<std::sync::Mutex<Option<String>>>,
    observed: Arc<std::sync::Mutex<Vec<String>>>,
    first_save: Arc<std::sync::atomic::AtomicBool>,
    fail: bool,
}
impl StorageAdapter for SendDisplayStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }
    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        if self.expected.lock().unwrap().is_some()
            && self
                .first_save
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            let expected = self.expected.lock().unwrap().clone().unwrap();
            for update in self.updates.try_iter() {
                if let AppUpdate::FullState(state) = update {
                    if let Some(chat) = state.current_chat {
                        for message in chat.messages {
                            if message.body == expected
                                && message.is_outgoing
                                && matches!(
                                    message.delivery,
                                    DeliveryState::Queued | DeliveryState::Pending
                                )
                            {
                                self.observed.lock().unwrap().push(message.id);
                            }
                        }
                    }
                }
            }
            // Exercise the real synchronous encryption/checkpoint path while
            // the UI update receiver is free to render an already-queued bubble.
            std::thread::sleep(Duration::from_millis(120));
            if self.fail {
                return Err(StorageError::new("injected checkpoint failure"));
            }
        }
        self.inner.put(key, value)
    }
}

fn send_display_scenario(group: bool, fail: bool, batched: bool) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("send-display", &owner, &device);
    let storage = Arc::new(SendDisplayStorage {
        inner: InMemoryStorage::new(),
        updates: updates.clone(),
        expected: Arc::new(std::sync::Mutex::new(None)),
        observed: Arc::new(std::sync::Mutex::new(Vec::new())),
        first_save: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        fail,
    });
    let mut engine = test_protocol_engine_with_storage(&owner, &device, storage.clone());
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let chat_id = if group {
        let peers = (0..16).map(|_| Keys::generate().public_key()).collect();
        let created = engine
            .create_group("17 members".into(), peers, UnixSeconds(3))
            .unwrap();
        let snapshot = created.snapshot.unwrap();
        let chat_id = group_chat_id(&snapshot.group_id);
        core.apply_group_roster_snapshot(snapshot, 3);
        chat_id
    } else {
        let peer = Keys::generate();
        let peer_device = Keys::generate();
        observe_peer_appkeys_for_test(&mut engine, &peer, &[peer_device.public_key()], 1);
        observe_peer_device_invite_for_test(&mut engine, &peer, &peer_device, 2);
        assert_eq!(
            engine.direct_send_readiness(peer.public_key()),
            DirectSendReadiness::Ready
        );
        peer.public_key().to_hex()
    };
    core.protocol_engine = Some(engine);
    let body = "visible before encryption and checkpoint";
    let _: Vec<_> = updates.try_iter().collect();
    *storage.expected.lock().unwrap() = Some(body.into());
    if batched {
        core.handle_messages(vec![CoreMsg::Action(AppAction::SendMessage {
            chat_id: chat_id.clone(),
            text: body.into(),
        })]);
    } else {
        core.send_message(&chat_id, body, None);
    }
    assert!(
        !storage.first_save.load(std::sync::atomic::Ordering::SeqCst),
        "real protocol checkpoint must be exercised"
    );
    let observed = storage.observed.lock().unwrap();
    assert!(
        !observed.is_empty(),
        "a pending bubble must be emitted before slow protocol persistence"
    );
    let messages: Vec<_> = core.threads[&chat_id]
        .messages
        .iter()
        .filter(|m| m.body == body)
        .collect();
    assert_eq!(messages.len(), 1, "one stable row throughout delivery");
    assert!(observed.iter().all(|id| id == &messages[0].id));
    if fail {
        assert_eq!(messages[0].delivery, DeliveryState::Failed);
    }
    assert!(
        core.app_store
            .message_exists(&chat_id, Some(&messages[0].id), None)
            .unwrap(),
        "final delivery state remains durable"
    );
}

#[test]
fn direct_pending_bubble_is_emitted_before_protocol_checkpoint() {
    send_display_scenario(false, false, false);
}
#[test]
fn group_pending_bubble_is_emitted_before_protocol_checkpoint() {
    send_display_scenario(true, false, false);
}
#[test]
fn group_pending_bubble_survives_checkpoint_failure_as_failed() {
    send_display_scenario(true, true, false);
}

#[test]
fn batched_direct_pending_bubble_precedes_checkpoint() {
    send_display_scenario(false, false, true);
}
#[test]
fn batched_group_pending_bubble_precedes_checkpoint() {
    send_display_scenario(true, false, true);
}
