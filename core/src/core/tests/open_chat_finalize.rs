fn open_chat_finalize_fixture() -> (
    AppCore,
    flume::Receiver<CoreMsg>,
    Arc<CountingStorage>,
    String,
    tempfile::TempDir,
) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let (mut core, _, directory) =
        logged_in_test_core_with_updates("open-chat-finalize", &owner, &device);
    core.app_store.bind_account(owner.public_key()).unwrap();
    core.preferences.nearby_enabled = false;
    core.preferences.nearby_bluetooth_enabled = false;
    core.preferences.nearby_lan_enabled = false;
    core.preferences.send_read_receipts = false;
    let storage = Arc::new(CountingStorage::new());
    install_test_protocol_engine(&mut core, &owner, &device, storage.clone(), None, None);
    let now = unix_now().get();
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)]),
            now,
        ),
    );
    // Session startup already publishes these durable identity artifacts.
    core.republish_local_identity_artifacts();
    assert!(core.pending_relay_publishes.values().any(|p| p.label == "app-keys"));
    assert!(core
        .pending_relay_publishes
        .values()
        .any(|p| p.label == LOCAL_INVITE_PUBLISH_LABEL));
    let chat_id = peer.public_key().to_hex();
    core.ensure_thread_record(&chat_id, now);
    core.persist_best_effort();
    let (sender, receiver) = flume::unbounded();
    core.core_sender = sender;
    (core, receiver, storage, chat_id, directory)
}

fn run_open_chat_finalize(core: &mut AppCore, receiver: &flume::Receiver<CoreMsg>, chat_id: &str) {
    core.handle_messages(vec![CoreMsg::Action(AppAction::OpenChat {
        chat_id: chat_id.to_owned(),
    })]);
    let finalize = receiver
        .try_iter()
        .find(|message| matches!(message, CoreMsg::Internal(event)
            if matches!(event.as_ref(), InternalEvent::OpenChatFinalize { chat_id: id } if id == chat_id)))
        .expect("opening a chat queues its follow-up");
    core.handle_messages(vec![finalize]);
}

#[test]
fn open_chat_finalize_preserves_identity_without_rewriting_protocol_checkpoint() {
    let (mut core, receiver, storage, chat_id, _directory) = open_chat_finalize_fixture();
    let before = storage.put_count();
    let pending = core.pending_relay_publishes.len();
    let (update_tx, updates) = flume::unbounded();
    core.update_tx = update_tx;

    run_open_chat_finalize(&mut core, &receiver, &chat_id);

    assert_eq!(storage.put_count(), before, "opening an unchanged chat must not checkpoint the global protocol state");
    assert_eq!(core.pending_relay_publishes.len(), pending);
    assert!(!updates.try_iter().any(|update| matches!(update, AppUpdate::NearbyPublishedEvent { .. })),
        "opening an existing chat must not republish unchanged identity artifacts");
    assert_eq!(core.state.current_chat.as_ref().unwrap().chat_id, chat_id);
    assert_eq!(core.app_store.load_state().unwrap().unwrap().active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(core.chat_read_states.contains_key(&chat_id));
    assert!(core.protocol_subscription_runtime.tracked_peer_catch_up_due_at.is_some());
}

#[test]
fn open_chat_finalize_schedules_catch_up_without_starting_global_fetch() {
    let (mut core, receiver, _storage, chat_id, _directory) = open_chat_finalize_fixture();
    let relay = crate::local_relay::TestRelay::start();
    core.logged_in.as_mut().unwrap().relay_urls = vec![RelayUrl::parse(relay.url()).unwrap()];
    core.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();

    run_open_chat_finalize(&mut core, &receiver, &chat_id);

    assert!(!core.protocol_subscription_runtime.protocol_fetch_in_flight,
        "opening a chat must not start an immediate global fetch ahead of queued foreground actions");
    assert!(core.protocol_subscription_runtime.desired_plan.as_ref().is_some_and(|plan|
        plan.roster_authors.contains(&chat_id)), "the opened peer must still be subscribed");
    assert!(core.protocol_subscription_runtime.tracked_peer_catch_up_due_at.is_some(),
        "coalesced catch-up must remain available for the opened peer");
}
