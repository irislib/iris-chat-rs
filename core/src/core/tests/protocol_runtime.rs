
#[derive(Clone)]
struct SwitchableFailStorage {
    inner: InMemoryStorage,
    fail_puts: Arc<std::sync::atomic::AtomicBool>,
}

impl SwitchableFailStorage {
    fn new() -> Self {
        Self {
            inner: InMemoryStorage::new(),
            fail_puts: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn set_fail_puts(&self, fail: bool) {
        self.fail_puts
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

impl StorageAdapter for SwitchableFailStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        if self.fail_puts.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StorageError::new("injected storage failure"));
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

#[derive(Clone)]
struct CountingStorage {
    inner: InMemoryStorage,
    put_count: Arc<std::sync::atomic::AtomicUsize>,
}

impl CountingStorage {
    fn new() -> Self {
        Self {
            inner: InMemoryStorage::new(),
            put_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn put_count(&self) -> usize {
        self.put_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl StorageAdapter for CountingStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        self.put_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.put(key, value)
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
}

fn protocol_plan_for_test(
    message_authors: Vec<PublicKey>,
    group_sender_key_authors: Vec<PublicKey>,
) -> ProtocolSubscriptionPlan {
    ProtocolSubscriptionPlan {
        runtime_subscriptions: vec!["ndr-protocol".to_string()],
        roster_authors: Vec::new(),
        invite_authors: Vec::new(),
        message_authors: message_authors
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect(),
        message_recipients: Vec::new(),
        group_roster_group_ids: Vec::new(),
        group_roster_authors: Vec::new(),
        group_sender_key_authors: group_sender_key_authors
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect(),
        invite_response_recipient: None,
    }
}

fn runtime_rumor_json(
    author: PublicKey,
    kind: u32,
    content: &str,
    created_at_secs: u64,
    tags: Vec<Vec<String>>,
) -> (String, String) {
    let tags = tags
        .into_iter()
        .map(|tag| nostr::Tag::parse(tag).expect("runtime rumor tag"))
        .collect::<Vec<_>>();
    let mut rumor = UnsignedEvent::new(
        author,
        Timestamp::from_secs(created_at_secs),
        Kind::Custom(kind as u16),
        tags,
        content.to_string(),
    );
    rumor.ensure_id();
    let id = rumor.id.as_ref().expect("runtime rumor id").to_string();
    (
        serde_json::to_string(&rumor).expect("runtime rumor json"),
        id,
    )
}

fn appcore_direct_message_event_for_test(
    receiver_engine: &mut ProtocolEngine,
    sender_keys: &Keys,
    body: &str,
    created_at_secs: u64,
) -> Event {
    appcore_direct_message_event_with_author_keys_for_test(
        receiver_engine,
        sender_keys,
        body,
        created_at_secs,
    )
    .0
}

fn appcore_direct_message_event_with_author_keys_for_test(
    receiver_engine: &mut ProtocolEngine,
    sender_keys: &Keys,
    body: &str,
    created_at_secs: u64,
) -> (Event, Keys) {
    let invite = receiver_engine
        .local_invite()
        .expect("receiver local invite");
    let (mut sender_session, response) = invite
        .accept_with_owner(
            sender_keys.public_key(),
            sender_keys.secret_key().to_secret_bytes(),
            Some(sender_keys.public_key().to_hex()),
            Some(sender_keys.public_key()),
        )
        .expect("sender accepts receiver invite");
    let response_event = invite_response_event(&response).expect("invite response event");
    receiver_engine
        .observe_invite_response_event(&response_event)
        .expect("receiver observes invite response");

    let (content, _) = runtime_rumor_json(
        sender_keys.public_key(),
        CHAT_MESSAGE_KIND,
        body,
        created_at_secs,
        Vec::new(),
    );
    let plan = sender_session
        .plan_send(content.as_bytes(), NdrUnixSeconds(created_at_secs))
        .expect("sender plans message");
    let sent = sender_session.apply_send(plan);
    let author_keys = Keys::new(
        nostr::SecretKey::from_slice(&sent.envelope.signer_secret_key)
            .expect("message event author secret key"),
    );
    (
        message_event(&sent.envelope).expect("message event"),
        author_keys,
    )
}

fn logged_in_test_core_with_updates(
    label: &str,
    owner: &Keys,
    device: &Keys,
) -> (AppCore, flume::Receiver<AppUpdate>, tempfile::TempDir) {
    let temp_dir = tempfile::Builder::new()
        .prefix(&format!("iris-chat-rs-test-{label}-"))
        .tempdir()
        .expect("temp dir");
    let (update_tx, update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    (core, update_rx, temp_dir)
}

fn signed_pairwise_message_event_for_test(
    sender_keys: &Keys,
    header: &str,
    content: &str,
) -> Event {
    EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), content)
        .tag(nostr::Tag::parse(["header", header]).expect("header tag"))
        .sign_with_keys(sender_keys)
        .expect("signed pairwise message event")
}

fn drain_app_updates(update_rx: &flume::Receiver<AppUpdate>) {
    while update_rx.try_recv().is_ok() {}
}

#[test]
fn protocol_engine_load_or_create_creates_owner_bound_local_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let storage =
        Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;

    let engine =
        ProtocolEngine::load_or_create_for_local_device(storage, owner.public_key(), &device)
            .expect("protocol engine");

    let invite = engine.local_invite().expect("local invite");
    assert_eq!(
        invite.inviter_owner_pubkey,
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()))
    );
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_eq!(invite.purpose.as_deref(), None);
    assert_eq!(invite.max_uses, None);

    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_device = NdrDevicePubkey::from_bytes(device.public_key().to_bytes());
    let snapshot = engine.session_manager_snapshot_for_test();
    let local_user = snapshot
        .users
        .iter()
        .find(|user| user.owner_pubkey == local_owner)
        .expect("local user");
    let roster = local_user.roster.as_ref().expect("local roster");
    assert_eq!(roster.devices, vec![AuthorizedDevice::new(local_device, invite.created_at)]);
}

#[test]
fn protocol_engine_load_or_create_installs_legacy_device_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    let device_id = device.public_key().to_hex();
    let storage_key = format!("device-invite/{device_id}");

    let mut legacy_invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None)
            .expect("legacy invite");
    legacy_invite.created_at = NdrUnixSeconds(123);
    storage
        .put(&storage_key, legacy_invite.serialize().expect("legacy invite json"))
        .expect("store legacy invite");

    let engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone() as Arc<dyn StorageAdapter>,
        owner.public_key(),
        &device,
    )
    .expect("protocol engine");

    let invite = engine.local_invite().expect("local invite");
    assert_eq!(
        invite.inviter_ephemeral_public_key,
        legacy_invite.inviter_ephemeral_public_key
    );
    assert_eq!(
        invite.inviter_owner_pubkey,
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()))
    );
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_eq!(invite.created_at, NdrUnixSeconds(123));
}

#[test]
fn protocol_engine_load_or_create_prefers_persisted_protocol_invite() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    let device_id = device.public_key().to_hex();
    let storage_key = format!("device-invite/{device_id}");
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_device = NdrDevicePubkey::from_bytes(device.public_key().to_bytes());

    let mut protocol_invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None)
            .expect("protocol invite");
    protocol_invite.owner_public_key = Some(owner.public_key());
    protocol_invite.created_at = NdrUnixSeconds(111);
    let seed_session_manager = SessionManagerSnapshot {
        local_owner_pubkey: local_owner,
        local_device_pubkey: local_device,
        local_invite: Some(protocol_invite.clone()),
        verified_peer_app_keys_events: Vec::new(),
        users: Vec::new(),
    };
    seed_protocol_storage_for_test(
        storage.as_ref(),
        seed_session_manager,
        GroupEventManager::new(local_owner).snapshot(),
    )
    .expect("seed protocol state");

    let mut legacy_invite =
        Invite::create_new(device.public_key(), Some(device_id), None).expect("legacy invite");
    legacy_invite.owner_public_key = Some(owner.public_key());
    legacy_invite.created_at = NdrUnixSeconds(222);
    storage
        .put(&storage_key, legacy_invite.serialize().expect("legacy invite json"))
        .expect("store legacy invite");

    let engine = ProtocolEngine::load_or_create_for_local_device(
        storage.clone() as Arc<dyn StorageAdapter>,
        owner.public_key(),
        &device,
    )
    .expect("protocol engine");

    let invite = engine.local_invite().expect("local invite");
    assert_eq!(
        invite.inviter_ephemeral_public_key,
        protocol_invite.inviter_ephemeral_public_key
    );
    assert_eq!(
        invite.inviter_owner_pubkey,
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()))
    );
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_eq!(invite.created_at, NdrUnixSeconds(111));
    let stored_legacy = Invite::deserialize(
        &storage
            .get(&storage_key)
            .expect("read legacy invite")
            .expect("legacy invite json"),
    )
    .expect("stored legacy invite");
    assert_eq!(
        stored_legacy.inviter_ephemeral_public_key,
        legacy_invite.inviter_ephemeral_public_key
    );

    let snapshot = engine.session_manager_snapshot_for_test();
    let local_user = snapshot
        .users
        .iter()
        .find(|user| user.owner_pubkey == local_owner)
        .expect("local user");
    let roster = local_user.roster.as_ref().expect("local roster");
    assert_eq!(roster.created_at, NdrUnixSeconds(111));
}

#[test]
fn restoring_invalid_secret_key_shows_normie_error() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.handle_action(AppAction::RestoreSession {
        owner_nsec: "not a secret key".to_string(),
    });

    assert_eq!(core.state.toast.as_deref(), Some("Invalid key."));
    assert!(!core.state.busy.restoring_session);
}

#[test]
fn removing_last_message_server_leaves_empty_list() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec![
        "wss://relay-one.example".to_string(),
        "wss://relay-two.example".to_string(),
    ];
    core.rebuild_state();

    core.handle_action(AppAction::RemoveNostrRelay {
        relay_url: "wss://relay-one.example".to_string(),
    });
    core.handle_action(AppAction::RemoveNostrRelay {
        relay_url: "wss://relay-two.example".to_string(),
    });

    assert!(core.preferences.nostr_relay_urls.is_empty());
    assert!(core.state.preferences.nostr_relay_urls.is_empty());
    assert_eq!(core.state.toast, None);
}

#[test]
fn direct_message_with_no_relays_is_queued_locally() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(owner.clone(), owner, false, false)
        .expect("primary session");

    core.handle_action(AppAction::CreateChat {
        peer_input: peer.public_key().to_hex(),
    });
    let chat_id = core
        .state
        .current_chat
        .as_ref()
        .expect("created chat")
        .chat_id
        .clone();

    core.handle_action(AppAction::SendMessage {
        chat_id,
        text: "queued offline".to_string(),
    });

    let current = core.state.current_chat.as_ref().expect("current chat");
    assert_eq!(core.state.toast, None);
    assert!(current.messages.iter().any(|message| {
        message.body == "queued offline"
            && message.is_outgoing
            && message.delivery == DeliveryState::Queued
    }));
}

#[test]
fn queued_runtime_publish_retries_when_message_servers_return() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let (core_tx, core_rx) = flume::unbounded();
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        temp_dir.path().to_string_lossy().to_string(),
    );
    core.core_sender = core_tx.clone();
    core.priority_sender = core_tx;

    let chat_id = peer.public_key().to_hex();
    let message_id = "retry-message".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "offline relay retry".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );
    core.logged_in
        .as_mut()
        .expect("logged in")
        .relay_urls
        .clear();
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "retry body")
        .sign_with_keys(&device)
        .expect("event");
    let event_id = event.id.to_string();

    core.publish_runtime_event(
        event,
        "runtime",
        Some((message_id.clone(), chat_id.clone())),
    );

    assert!(core.pending_relay_publishes.contains_key(&event_id));
    assert_eq!(
        core.threads
            .get(&chat_id)
            .and_then(|thread| thread
                .messages
                .iter()
                .find(|message| message.id == message_id))
            .map(|message| &message.delivery),
        Some(&DeliveryState::Queued)
    );

    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        logged_in.relay_urls = relay_urls.clone();
        let client = logged_in.client.clone();
        core.runtime
            .block_on(ensure_session_relays_configured(&client, &relay_urls));
    }
    core.retry_pending_relay_publishes("test");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        while let Ok(msg) = core_rx.try_recv() {
            core.handle_message(msg);
        }
        if relay.events().iter().any(|event| {
            event.get("id").and_then(|value| value.as_str()) == Some(event_id.as_str())
        }) && !core.pending_relay_publishes.contains_key(&event_id)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        relay.events().iter().any(|event| {
            event.get("id").and_then(|value| value.as_str()) == Some(event_id.as_str())
        }),
        "retry should publish the queued event to the relay"
    );
    assert!(!core.pending_relay_publishes.contains_key(&event_id));
    assert_eq!(
        core.threads
            .get(&chat_id)
            .and_then(|thread| thread
                .messages
                .iter()
                .find(|message| message.id == message_id))
            .map(|message| &message.delivery),
        Some(&DeliveryState::Sent)
    );
}

#[test]
fn first_contact_publishes_bootstrap_and_payload_durably() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("staged-first-contact-queue", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "first-contact-message".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "staged".to_string(),
        unix_now().get(),
        None,
        DeliveryState::Pending,
    );
    let bootstrap = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "bootstrap")
        .sign_with_keys(&device)
        .expect("bootstrap event");
    let payload = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "payload")
        .sign_with_keys(&device)
        .expect("payload event");
    let payload_id = payload.id.to_string();
    let bootstrap_publish = ProtocolPublish {
        event: bootstrap,
        chat_id: chat_id.clone(),
        inner_event_id: None,
    };
    let payload_publish = ProtocolPublish {
        event: payload,
        chat_id: chat_id.clone(),
        inner_event_id: Some(message_id.clone()),
    };

    core.process_protocol_engine_effects(vec![
        ProtocolEffect::Publish(bootstrap_publish),
        ProtocolEffect::Publish(payload_publish),
    ]);

    let pending = core
        .pending_relay_publishes
        .get(&payload_id)
        .expect("payload should be queued");
    assert_eq!(pending.label, APPCORE_PROTOCOL_LABEL);
    assert_eq!(pending.inner_event_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(pending.chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(
        core.pending_relay_publishes
            .values()
            .any(|pending| pending.inner_event_id.is_none()
                && pending.chat_id.as_deref() == Some(chat_id.as_str())),
        "bootstrap should be durable with chat context but without message delivery metadata"
    );
}

#[test]
fn liveness_retries_pending_relay_publish_without_active_protocol_subscription() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let (core_tx, core_rx) = flume::unbounded();
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        temp_dir.path().to_string_lossy().to_string(),
    );
    core.core_sender = core_tx.clone();
    core.priority_sender = core_tx;
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        logged_in.relay_urls = relay_urls;
    }
    assert!(
        core.protocol_subscription_runtime.desired_plan.is_none(),
        "test should cover pending publish retry without subscription state"
    );

    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "retry body")
        .sign_with_keys(&device)
        .expect("event");
    let event_id = event.id.to_string();
    core.pending_relay_publishes.insert(
        event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: event_id.clone(),
            label: "app-keys".to_string(),
            event_json: serde_json::to_string(&event).expect("event json"),
            inner_event_id: None,
            chat_id: None,
            created_at_secs: event.created_at.as_secs(),
            attempt_count: 0,
            last_error: Some("initial offline publish failed".to_string()),
        },
    );

    core.handle_protocol_subscription_liveness_check(core.protocol_liveness_token);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        while let Ok(msg) = core_rx.try_recv() {
            core.handle_message(msg);
        }
        if relay.events().iter().any(|event| {
            event.get("id").and_then(|value| value.as_str()) == Some(event_id.as_str())
        }) && !core.pending_relay_publishes.contains_key(&event_id)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        relay.events().iter().any(|event| {
            event.get("id").and_then(|value| value.as_str()) == Some(event_id.as_str())
        }),
        "liveness must retry queued relay publishes even without active protocol subscriptions"
    );
    assert!(!core.pending_relay_publishes.contains_key(&event_id));
}

#[test]
fn fetch_catch_up_events_requeues_large_batches_in_chunks() {
    let keys = Keys::generate();
    let events = (0..=super::lifecycle::CATCH_UP_EVENT_PROCESS_CHUNK_SIZE)
        .map(|index| {
            EventBuilder::new(Kind::TextNote, format!("catch-up-{index}"))
                .sign_with_keys(&keys)
                .expect("event")
        })
        .collect::<Vec<_>>();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let (core_tx, core_rx) = flume::unbounded();
    let mut core = AppCore::new(
        flume::unbounded().0,
        core_tx,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.handle_internal(InternalEvent::FetchCatchUpEvents(events));

    let requeued = core_rx.try_recv().expect("requeued remainder");
    match requeued {
        CoreMsg::Internal(event) => match *event {
            InternalEvent::FetchCatchUpEvents(remainder) => {
                assert_eq!(remainder.len(), 1);
            }
            other => panic!("unexpected internal event: {other:?}"),
        },
        other => panic!("unexpected core message: {other:?}"),
    }
}

#[test]
fn failed_publish_drain_batches_results_and_schedules_one_retry() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("publish-drain-fail-batch", &owner, &device);
    core.relay_transport_runtime.publish_drain_token = 7;
    core.relay_transport_runtime.publish_drain_in_flight = true;
    core.relay_transport_runtime.publish_drain_dirty = true;

    let mut results = Vec::new();
    for index in 0..3 {
        let event = EventBuilder::new(
            Kind::from(MESSAGE_EVENT_KIND as u16),
            format!("failed publish {index}"),
        )
        .sign_with_keys(&device)
        .expect("event");
        let event_id = event.id.to_string();
        core.pending_relay_publishes.insert(
            event_id.clone(),
            PendingRelayPublish {
                owner_pubkey_hex: owner.public_key().to_hex(),
                event_id: event_id.clone(),
                label: "test".to_string(),
                event_json: serde_json::to_string(&event).expect("event json"),
                inner_event_id: None,
                chat_id: None,
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 0,
                last_error: None,
            },
        );
        results.push(RelayPublishDrainResult {
            event_id,
            success: false,
            relay_urls: Vec::new(),
            detail: "publish failed".to_string(),
        });
    }

    let rev_before = core.state.rev;
    core.handle_relay_publish_drain_finished(7, results);

    assert_eq!(
        core.relay_transport_runtime.retry_backoff_attempt, 1,
        "one failed drain should schedule one transport retry, not one per event"
    );
    assert!(
        core.protocol_subscription_runtime.liveness_due_at.is_some(),
        "failed drain should schedule a retry wakeup"
    );
    assert!(!core.relay_transport_runtime.publish_drain_in_flight);
    assert_eq!(
        core.state.rev,
        rev_before + 1,
        "drain results should rebuild and emit one full state"
    );
    assert!(core
        .pending_relay_publishes
        .values()
        .all(|pending| pending.attempt_count == 1));
}

#[test]
fn pending_relay_publish_batch_selection_is_bounded_and_ordered() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("pending-relay-batch-selection", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();

    let insert_pending = |core: &mut AppCore,
                          event_id: &str,
                          label: &str,
                          created_at_secs: u64,
                          is_message: bool| {
        core.pending_relay_publishes.insert(
            event_id.to_string(),
            PendingRelayPublish {
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                event_id: event_id.to_string(),
                label: label.to_string(),
                event_json: format!("not parsed by batch selector {event_id}"),
                inner_event_id: is_message.then(|| format!("inner-{event_id}")),
                chat_id: is_message.then(|| "chat-id".to_string()),
                created_at_secs,
                attempt_count: 0,
                last_error: None,
            },
        );
    };

    insert_pending(&mut core, "event-00", "relay", 1, false);
    insert_pending(&mut core, "event-01", "relay", 2, false);
    insert_pending(&mut core, "event-02", "relay", 3, false);
    insert_pending(&mut core, "event-03", "relay", 4, false);
    insert_pending(&mut core, "event-04", "relay", 0, false);
    insert_pending(&mut core, "event-10", "message", 20, true);
    insert_pending(&mut core, "event-11", "message", 10, true);
    insert_pending(&mut core, "event-12", "message", 30, true);
    core.pending_relay_publish_inflight
        .insert("event-00".to_string());

    let (event_ids, truncated) = core.pending_relay_publish_batch_event_ids(4);

    assert!(truncated, "eligible backlog should be larger than the batch");
    assert_eq!(
        event_ids,
        vec![
            "event-11".to_string(),
            "event-10".to_string(),
            "event-12".to_string(),
            "event-04".to_string(),
        ],
        "message publishes stay ahead of oldest non-message publishes, and inflight items are skipped"
    );
}

#[test]
fn pending_relay_control_publish_cap_keeps_message_linked_publishes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("pending-relay-control-cap", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();

    let insert_pending = |core: &mut AppCore,
                          event_id: &str,
                          label: &str,
                          created_at_secs: u64,
                          is_message: bool| {
        core.pending_relay_publishes.insert(
            event_id.to_string(),
            PendingRelayPublish {
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                event_id: event_id.to_string(),
                label: label.to_string(),
                event_json: format!("cap test {event_id}"),
                inner_event_id: is_message.then(|| format!("inner-{event_id}")),
                chat_id: Some("chat-id".to_string()),
                created_at_secs,
                attempt_count: 0,
                last_error: None,
            },
        );
    };

    insert_pending(&mut core, "message-old", APPCORE_PROTOCOL_LABEL, 1, true);
    insert_pending(&mut core, "app-keys-old", "app-keys", 1, false);
    insert_pending(&mut core, "protocol-old", APPCORE_PROTOCOL_LABEL, 10, false);
    insert_pending(&mut core, "protocol-mid", APPCORE_PROTOCOL_LABEL, 20, false);
    insert_pending(&mut core, "protocol-new", APPCORE_PROTOCOL_LABEL, 30, false);

    core.prune_pending_relay_control_publish_backlog_to_limit(2, "test");

    assert!(core.pending_relay_publishes.contains_key("message-old"));
    assert!(core.pending_relay_publishes.contains_key("app-keys-old"));
    assert!(core.pending_relay_publishes.contains_key("protocol-new"));
    assert!(!core.pending_relay_publishes.contains_key("protocol-mid"));
    assert!(!core.pending_relay_publishes.contains_key("protocol-old"));
}

#[test]
fn app_keys_publish_uses_durable_pending_publish_queue() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        temp_dir.path().to_string_lossy().to_string(),
    );

    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    let previous_created_at = core
        .app_keys
        .get_mut(&owner.public_key().to_hex())
        .expect("local AppKeys")
        .created_at_secs;
    let linked_device = Keys::generate();
    core.upsert_local_app_key_device(owner.public_key(), linked_device.public_key());
    let app_keys_created_at = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("updated AppKeys")
        .created_at_secs;
    assert!(
        app_keys_created_at > previous_created_at,
        "AppKeys updates must advance replaceable-event timestamps even inside one Unix second"
    );

    core.publish_local_app_keys();

    let pending = core
        .pending_relay_publishes
        .values()
        .find(|pending| pending.label == "app-keys")
        .expect("AppKeys should be tracked as a durable publish");
    let event: Event = serde_json::from_str(&pending.event_json).expect("stored event json");

    assert!(is_app_keys_event(&event));
    assert_eq!(event.pubkey, owner.public_key());
    assert_eq!(event.created_at.as_secs(), app_keys_created_at);
    let device_tags = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.clone().to_vec();
            (values.first().map(|value| value.as_str()) == Some("device"))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    assert!(device_tags.contains(&device.public_key().to_hex()));
    assert!(device_tags.contains(&linked_device.public_key().to_hex()));
    assert!(
        pending
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("skipped=no_servers"),
        "empty-relay publish should remain queued with a retryable error"
    );
}

#[test]
fn app_keys_publish_prunes_superseded_pending_publish() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = logged_in_test_core_at_data_dir(
        &owner,
        &device,
        temp_dir.path().to_string_lossy().to_string(),
    );

    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    core.publish_local_app_keys();
    let first_pending = core
        .pending_relay_publishes
        .values()
        .find(|pending| pending.label == "app-keys")
        .cloned()
        .expect("first AppKeys publish should be queued");
    let first_event: Event =
        serde_json::from_str(&first_pending.event_json).expect("first AppKeys event");

    let linked_device = Keys::generate();
    core.upsert_local_app_key_device(owner.public_key(), linked_device.public_key());
    core.publish_local_app_keys();

    let app_key_publishes = core
        .pending_relay_publishes
        .values()
        .filter(|pending| pending.label == "app-keys")
        .collect::<Vec<_>>();
    assert_eq!(
        app_key_publishes.len(),
        1,
        "stale replaceable AppKeys publishes should not block relay drain"
    );
    assert!(
        !core
            .pending_relay_publishes
            .contains_key(&first_pending.event_id),
        "older AppKeys event should be removed from the durable queue"
    );
    let current_event: Event =
        serde_json::from_str(&app_key_publishes[0].event_json).expect("current AppKeys event");
    assert_eq!(current_event.pubkey, owner.public_key());
    assert!(current_event.created_at.as_secs() > first_event.created_at.as_secs());
}

#[test]
fn protocol_invite_response_publish_prunes_superseded_pending_bootstrap() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("protocol-bootstrap-prune", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    let older = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "older bootstrap")
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&device)
        .expect("older bootstrap");
    let older_id = older.id.to_string();
    let newer = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "newer bootstrap")
        .custom_created_at(Timestamp::from_secs(20))
        .sign_with_keys(&device)
        .expect("newer bootstrap");
    let newer_id = newer.id.to_string();

    assert!(core.publish_protocol_event(ProtocolPublish {
        event: older,
        chat_id: chat_id.clone(),
        inner_event_id: None,
    }));
    assert!(core.pending_relay_publishes.contains_key(&older_id));

    assert!(core.publish_protocol_event(ProtocolPublish {
        event: newer,
        chat_id,
        inner_event_id: None,
    }));

    assert!(
        !core.pending_relay_publishes.contains_key(&older_id),
        "newer bootstrap should replace the older pending bootstrap for the same chat/pubkey"
    );
    assert!(core.pending_relay_publishes.contains_key(&newer_id));
    assert_eq!(
        core.pending_relay_publishes
            .values()
            .filter(|pending| pending.label == APPCORE_PROTOCOL_LABEL
                && pending.inner_event_id.is_none())
            .count(),
        1
    );
}

#[test]
fn storage_prunes_superseded_protocol_invite_response_rows_before_load() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let peer = Keys::generate();
    let core = logged_in_test_core("storage-bootstrap-prune", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();
    let chat_id = peer.public_key().to_hex();

    let older = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "older bootstrap")
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&device)
        .expect("older bootstrap");
    let older_id = older.id.to_string();
    let newer = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "newer bootstrap")
        .custom_created_at(Timestamp::from_secs(20))
        .sign_with_keys(&device)
        .expect("newer bootstrap");
    let newer_id = newer.id.to_string();
    let other_pubkey = EventBuilder::new(
        Kind::from(INVITE_RESPONSE_KIND as u16),
        "other response pubkey",
    )
    .custom_created_at(Timestamp::from_secs(5))
    .sign_with_keys(&other_device)
    .expect("other bootstrap");
    let other_id = other_pubkey.id.to_string();

    for event in [&older, &newer, &other_pubkey] {
        core.app_store
            .upsert_pending_relay_publish(&PendingRelayPublish {
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                event_id: event.id.to_string(),
                label: APPCORE_PROTOCOL_LABEL.to_string(),
                event_json: serde_json::to_string(event).expect("event json"),
                inner_event_id: None,
                chat_id: Some(chat_id.clone()),
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 0,
                last_error: None,
            })
            .expect("store pending publish");
    }

    let pruned = core
        .app_store
        .prune_superseded_protocol_invite_response_publishes(&owner_pubkey_hex)
        .expect("prune protocol bootstraps");
    let pending = core
        .app_store
        .load_pending_relay_publishes(&owner_pubkey_hex)
        .expect("load pending publishes");
    let pending_ids = pending
        .into_iter()
        .map(|pending| pending.event_id)
        .collect::<Vec<_>>();

    assert_eq!(pruned, 1);
    assert!(!pending_ids.contains(&older_id));
    assert!(pending_ids.contains(&newer_id));
    assert!(pending_ids.contains(&other_id));
}

#[test]
fn protocol_control_publish_prunes_superseded_rows_already_in_storage() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-control-storage-prune", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();
    let chat_id = "group:storage-protocol-control".to_string();

    let older = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "older control")
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&device)
        .expect("older control");
    let middle = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "middle control")
        .custom_created_at(Timestamp::from_secs(20))
        .sign_with_keys(&device)
        .expect("middle control");
    let newest = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "newest control")
        .custom_created_at(Timestamp::from_secs(30))
        .sign_with_keys(&device)
        .expect("newest control");
    let newest_id = newest.id.to_string();

    for event in [&older, &middle] {
        core.app_store
            .upsert_pending_relay_publish(&PendingRelayPublish {
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                event_id: event.id.to_string(),
                label: APPCORE_PROTOCOL_LABEL.to_string(),
                event_json: serde_json::to_string(event).expect("event json"),
                inner_event_id: None,
                chat_id: Some(chat_id.clone()),
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 0,
                last_error: None,
            })
            .expect("store pending publish");
    }

    assert!(core.publish_protocol_event(ProtocolPublish {
        event: newest,
        chat_id: chat_id.clone(),
        inner_event_id: None,
    }));

    let pending = core
        .app_store
        .load_pending_relay_publishes(&owner_pubkey_hex)
        .expect("load pending publishes");
    let matching = pending
        .iter()
        .filter(|pending| {
            pending.label == APPCORE_PROTOCOL_LABEL
                && pending.inner_event_id.is_none()
                && pending.chat_id.as_deref() == Some(chat_id.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].event_id, newest_id);
    assert!(core.pending_relay_publishes.contains_key(&newest_id));
}

#[test]
fn older_protocol_control_publish_is_skipped_when_newer_row_is_already_stored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-control-storage-skip", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();
    let chat_id = "group:storage-protocol-control-skip".to_string();

    let older = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "older control")
        .custom_created_at(Timestamp::from_secs(10))
        .sign_with_keys(&device)
        .expect("older control");
    let older_id = older.id.to_string();
    let newer = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "newer control")
        .custom_created_at(Timestamp::from_secs(20))
        .sign_with_keys(&device)
        .expect("newer control");
    let newer_id = newer.id.to_string();

    core.app_store
        .upsert_pending_relay_publish(&PendingRelayPublish {
            owner_pubkey_hex: owner_pubkey_hex.clone(),
            event_id: newer.id.to_string(),
            label: APPCORE_PROTOCOL_LABEL.to_string(),
            event_json: serde_json::to_string(&newer).expect("event json"),
            inner_event_id: None,
            chat_id: Some(chat_id.clone()),
            created_at_secs: newer.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        })
        .expect("store pending publish");

    assert!(!core.publish_protocol_event(ProtocolPublish {
        event: older,
        chat_id: chat_id.clone(),
        inner_event_id: None,
    }));

    let pending_ids = core
        .app_store
        .load_pending_relay_publishes(&owner_pubkey_hex)
        .expect("load pending publishes")
        .into_iter()
        .map(|pending| pending.event_id)
        .collect::<Vec<_>>();
    assert!(pending_ids.contains(&newer_id));
    assert!(!pending_ids.contains(&older_id));
    assert!(!core.pending_relay_publishes.contains_key(&older_id));
}

#[test]
fn storage_caps_control_pending_publishes_without_pruning_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("storage-control-cap", &owner, &device);
    let owner_pubkey_hex = owner.public_key().to_hex();

    let pending = [
        ("message-old", APPCORE_PROTOCOL_LABEL, 1, Some("message-id")),
        ("app-keys-old", "app-keys", 1, None),
        ("protocol-old", APPCORE_PROTOCOL_LABEL, 10, None),
        ("protocol-mid", APPCORE_PROTOCOL_LABEL, 20, None),
        ("protocol-new", APPCORE_PROTOCOL_LABEL, 30, None),
    ];
    for (event_id, label, created_at_secs, inner_event_id) in pending {
        core.app_store
            .upsert_pending_relay_publish(&PendingRelayPublish {
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                event_id: event_id.to_string(),
                label: label.to_string(),
                event_json: format!("storage cap test {event_id}"),
                inner_event_id: inner_event_id.map(str::to_string),
                chat_id: Some("chat-id".to_string()),
                created_at_secs,
                attempt_count: 0,
                last_error: None,
            })
            .expect("store pending publish");
    }

    let pruned = core
        .app_store
        .prune_pending_relay_control_publishes_to_limit(&owner_pubkey_hex, 2)
        .expect("cap control pending publishes");
    let pending_ids = core
        .app_store
        .load_pending_relay_publishes(&owner_pubkey_hex)
        .expect("load pending publishes")
        .into_iter()
        .map(|pending| pending.event_id)
        .collect::<Vec<_>>();

    assert_eq!(pruned, 2);
    assert!(pending_ids.contains(&"message-old".to_string()));
    assert!(pending_ids.contains(&"app-keys-old".to_string()));
    assert!(pending_ids.contains(&"protocol-new".to_string()));
    assert!(!pending_ids.contains(&"protocol-mid".to_string()));
    assert!(!pending_ids.contains(&"protocol-old".to_string()));
}

#[test]
fn upserting_existing_local_app_key_device_is_protocol_noop() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("appkeys-existing-device-noop", &owner, &device);
    let owner_hex = owner.public_key().to_hex();
    core.upsert_local_app_key_device(owner.public_key(), device.public_key());
    let initial = core
        .app_keys
        .get(&owner_hex)
        .expect("local AppKeys")
        .clone();

    core.upsert_local_app_key_device(owner.public_key(), device.public_key());

    assert_eq!(
        core.app_keys.get(&owner_hex),
        Some(&initial),
        "restoring the same owner/device must not create a newer local AppKeys protocol state"
    );
}

#[test]
fn relay_duplicate_or_newer_replaceable_rejection_is_terminal_success() {
    assert!(relay_publish_failure_is_terminal_success(
        "duplicate: already have this event"
    ));
    assert!(relay_publish_failure_is_terminal_success(
        "replaced: have newer event"
    ));
    assert!(!relay_publish_failure_is_terminal_success(
        "rate-limited: slow down"
    ));
    assert!(!relay_publish_failure_is_terminal_success(
        "blocked: event rejected"
    ));
}

#[test]
fn network_status_includes_configured_relay_connection_status() {
    let owner = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];

    core.start_primary_session(owner.clone(), owner, false, false)
        .expect("primary session");

    let status = core.state.network_status.as_ref().expect("network status");
    assert_eq!(status.relay_urls, vec!["wss://relay.invalid".to_string()]);
    assert_eq!(status.relay_connections.len(), 1);
    assert_eq!(status.relay_connections[0].url, "wss://relay.invalid");
    assert_eq!(status.relay_connections[0].status, "connecting");
    assert_eq!(status.connected_relay_count, 0);
    assert!(status.all_relays_offline_since_secs.is_some());
}

#[test]
fn relay_status_events_match_normalized_relay_urls() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("relay-status-normalized", &owner, &device);
    core.preferences.nostr_relay_urls = vec!["wss://relay.example".to_string()];

    core.handle_relay_status_changed("wss://relay.example/".to_string(), RelayStatus::Connected);

    assert!(core.debug_log.iter().any(|entry| {
        entry.category == "relay.status" && entry.detail.starts_with("url=wss://relay.example ")
    }));
    let relay_status_log_count = core
        .debug_log
        .iter()
        .filter(|entry| entry.category == "relay.status")
        .count();

    core.handle_relay_status_changed("wss://relay.example".to_string(), RelayStatus::Connected);
    core.handle_relay_status_changed_for_generation(
        "wss://relay.example".to_string(),
        RelayStatus::Disconnected,
        core.relay_status_watch_generation.wrapping_add(1),
    );

    assert_eq!(
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "relay.status")
            .count(),
        relay_status_log_count
    );
    assert_eq!(core.relay_connected_count, 1);
}

#[test]
fn relay_status_bucket_only_changes_do_not_emit_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls = vec!["wss://relay.example".to_string()];
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });

    while update_rx.try_recv().is_ok() {}
    core.handle_relay_status_changed("wss://relay.example".to_string(), RelayStatus::Initialized);
    assert!(
        matches!(update_rx.try_recv(), Ok(AppUpdate::FullState(_))),
        "offline -> connecting should still emit once"
    );
    while update_rx.try_recv().is_ok() {}
    let rev = core.state.rev;
    let relay_status_log_count = core
        .debug_log
        .iter()
        .filter(|entry| entry.category == "relay.status")
        .count();

    core.handle_relay_status_changed("wss://relay.example".to_string(), RelayStatus::Pending);
    core.handle_relay_status_changed("wss://relay.example".to_string(), RelayStatus::Connecting);

    assert_eq!(
        core.state.rev, rev,
        "internal connecting-state churn must not push a new full state"
    );
    assert!(
        update_rx.try_recv().is_err(),
        "no FullState should be emitted for connecting -> connecting transitions"
    );
    assert_eq!(
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "relay.status")
            .count(),
        relay_status_log_count,
        "suppressed transitions should not churn the visible debug summary"
    );
}

#[test]
fn distinct_protocol_publishes_for_same_target_are_kept() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("distinct-protocol-publishes", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "duplicate-publish-message".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "keep both".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );
    let first = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "first")
        .sign_with_keys(&device)
        .expect("first event");
    let first_event_id = first.id.to_string();
    let second = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "second")
        .sign_with_keys(&device)
        .expect("second event");
    let second_event_id = second.id.to_string();

    assert!(core.publish_protocol_event(ProtocolPublish {
        event: first,
        chat_id: chat_id.clone(),
        inner_event_id: Some(message_id.clone()),
    }));
    assert!(core.publish_protocol_event(ProtocolPublish {
        event: second,
        chat_id: chat_id.clone(),
        inner_event_id: Some(message_id),
    }));

    assert!(core.pending_relay_publishes.contains_key(&first_event_id));
    assert!(core.pending_relay_publishes.contains_key(&second_event_id));
    assert_eq!(core.pending_relay_publishes.len(), 2);
}

#[test]
fn protocol_subscription_reconcile_defers_while_in_flight() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-reconcile-single-flight", &owner, &device);
    core.protocol_subscription_runtime.desired_plan = Some(protocol_plan_for_test(
        vec![device.public_key()],
        Vec::new(),
    ));
    core.protocol_subscription_runtime.refresh_in_flight = true;

    core.reconcile_protocol_subscriptions("test", true);

    assert!(core.protocol_subscription_runtime.refresh_in_flight);
    assert!(core.protocol_subscription_runtime.refresh_dirty);
    assert!(core.protocol_subscription_runtime.force_reconnect_dirty);
    assert_eq!(core.protocol_subscription_runtime.reconcile_token, 0);
}

#[test]
fn protocol_subscription_desired_plan_is_not_applied_before_reconcile_success() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-desired-not-applied", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();

    assert!(
        core.protocol_subscription_runtime.desired_plan.is_some(),
        "refresh computes the desired plan synchronously"
    );
    assert_eq!(
        core.protocol_subscription_runtime.applied_plan, None,
        "desired plan must not be reported as applied before relay apply succeeds"
    );
}

#[test]
fn failed_protocol_subscription_apply_clears_inflight_and_keeps_dirty() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-apply-failure", &owner, &device);
    let plan = protocol_plan_for_test(vec![device.public_key()], Vec::new());
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.protocol_subscription_runtime.applying_plan = Some(plan.clone());
    core.protocol_subscription_runtime.refresh_in_flight = true;
    core.protocol_subscription_runtime.reconcile_token = 3;

    core.handle_protocol_subscription_reconcile_completed(
        core.protocol_reconnect_token,
        3,
        "test_failure".to_string(),
        Some(plan),
        false,
        Some("injected failure".to_string()),
        vec![("wss://relay.example".to_string(), RelayStatus::Connected)],
        1,
        1,
        1,
    );

    assert!(!core.protocol_subscription_runtime.refresh_in_flight);
    assert!(core.protocol_subscription_runtime.applying_plan.is_none());
    assert!(core.protocol_subscription_runtime.refresh_dirty);
    assert!(core.protocol_subscription_runtime.applied_plan.is_none());
    assert!(core.protocol_subscription_runtime.liveness_due_at.is_some());
}

#[test]
fn successful_protocol_subscription_apply_sets_applied_plan() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-apply-success", &owner, &device);
    let plan = protocol_plan_for_test(vec![device.public_key()], Vec::new());
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.protocol_subscription_runtime.applying_plan = Some(plan.clone());
    core.protocol_subscription_runtime.refresh_in_flight = true;
    core.protocol_subscription_runtime.reconcile_token = 5;

    core.handle_protocol_subscription_reconcile_completed(
        core.protocol_reconnect_token,
        5,
        "test_success".to_string(),
        Some(plan.clone()),
        true,
        None,
        vec![("wss://relay.example".to_string(), RelayStatus::Connected)],
        1,
        1,
        1,
    );

    assert!(!core.protocol_subscription_runtime.refresh_in_flight);
    assert_eq!(core.protocol_subscription_runtime.applied_plan, Some(plan));
    assert!(core.protocol_subscription_runtime.applying_plan.is_none());
}

#[test]
fn liveness_scheduling_does_not_invalidate_subscription_apply_completion() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-apply-liveness-generation", &owner, &device);
    let plan = protocol_plan_for_test(vec![device.public_key()], Vec::new());
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.protocol_subscription_runtime.applying_plan = Some(plan.clone());
    core.protocol_subscription_runtime.refresh_in_flight = true;
    core.protocol_subscription_runtime.reconcile_token = 9;
    let apply_generation = core.protocol_reconnect_token;

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(2));

    core.handle_protocol_subscription_reconcile_completed(
        apply_generation,
        9,
        "test_success_after_liveness_schedule".to_string(),
        Some(plan.clone()),
        true,
        None,
        vec![("wss://relay.example".to_string(), RelayStatus::Connected)],
        1,
        1,
        1,
    );

    assert!(
        !core.protocol_subscription_runtime.refresh_in_flight,
        "liveness timers must not leave subscription apply permanently in-flight"
    );
    assert_eq!(core.protocol_subscription_runtime.applied_plan, Some(plan));
}

#[test]
fn stale_protocol_subscription_reconcile_completion_is_ignored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-reconcile-stale", &owner, &device);
    core.protocol_reconnect_token = 5;
    core.protocol_subscription_runtime.reconcile_token = 7;
    core.protocol_subscription_runtime.refresh_in_flight = true;

    core.handle_protocol_subscription_reconcile_completed(
        5,
        6,
        "stale".to_string(),
        None,
        false,
        None,
        vec![("wss://relay.example".to_string(), RelayStatus::Connected)],
        0,
        1,
        0,
    );

    assert!(core.protocol_subscription_runtime.refresh_in_flight);
    assert_eq!(core.relay_connected_count, 0);
}

#[test]
fn liveness_token_does_not_stale_subscription_completion() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-liveness-token", &owner, &device);
    let plan = protocol_plan_for_test(vec![device.public_key()], Vec::new());
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.protocol_subscription_runtime.applying_plan = Some(plan.clone());
    core.protocol_subscription_runtime.refresh_in_flight = true;
    core.protocol_subscription_runtime.reconcile_token = 9;
    let apply_generation = core.protocol_reconnect_token;
    let liveness_generation = core.protocol_liveness_token;
    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
    assert_eq!(
        core.protocol_reconnect_token, apply_generation,
        "liveness scheduling must not invalidate subscription apply generation"
    );
    assert_ne!(
        core.protocol_liveness_token, liveness_generation,
        "liveness scheduling should still advance the independent liveness token"
    );

    core.handle_protocol_subscription_reconcile_completed(
        apply_generation,
        9,
        "test_success_after_liveness_schedule".to_string(),
        Some(plan.clone()),
        true,
        None,
        vec![("wss://relay.example".to_string(), RelayStatus::Connected)],
        1,
        1,
        1,
    );

    assert!(!core.protocol_subscription_runtime.refresh_in_flight);
    assert_eq!(core.protocol_subscription_runtime.applied_plan, Some(plan));
}

#[test]
fn debug_snapshot_write_is_coalesced_while_in_flight() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("debug-snapshot-coalesce", &owner, &device);
    core.debug_snapshot_write_inflight = true;

    core.persist_debug_snapshot_best_effort();

    assert!(core.debug_snapshot_write_inflight);
    assert!(core.debug_snapshot_write_dirty);
}

#[test]
fn stale_debug_snapshot_write_completion_is_ignored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("debug-snapshot-stale", &owner, &device);
    core.debug_snapshot_write_generation = 10;
    core.debug_snapshot_write_inflight = true;
    core.debug_snapshot_write_dirty = true;

    core.handle_debug_snapshot_write_finished(9);

    assert!(core.debug_snapshot_write_inflight);
    assert!(core.debug_snapshot_write_dirty);
}

#[test]
fn liveness_replays_retained_appkeys_for_tracked_peer() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let mut core = logged_in_test_core("tracked-peer-liveness-backfill", &owner, &device);

    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    let app_keys_event = peer_app_keys
        .get_event_at(peer_owner.public_key(), 1)
        .sign_with_keys(&peer_owner)
        .expect("signed peer AppKeys");
    let app_keys_event_id = app_keys_event.id;
    let app_keys_event_id_hex = app_keys_event_id.to_hex();
    let publisher = Client::new(peer_owner.clone());
    core.runtime.block_on(async {
        ensure_session_relays_configured(&publisher, &relay_urls).await;
        connect_client_with_timeout(&publisher, Duration::from_secs(2)).await;
        publisher
            .send_event(&app_keys_event)
            .await
            .expect("publish retained peer AppKeys");
    });
    assert!(
        relay.events().iter().any(|event| {
            event.get("id").and_then(|value| value.as_str())
                == Some(app_keys_event_id_hex.as_str())
        }),
        "peer AppKeys must be retained before the core subscribes"
    );

    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    let mut notifications = {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        logged_in.relay_urls = relay_urls.clone();
        let client = logged_in.client.clone();
        let connected = core.runtime.block_on(async {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let connected = client
                    .relays()
                    .await
                    .values()
                    .filter(|relay| relay.status() == RelayStatus::Connected)
                    .count();
                if connected > 0 || tokio::time::Instant::now() >= deadline {
                    break connected;
                }
                sleep(Duration::from_millis(50)).await;
            }
        });
        assert!(connected > 0, "test relay must be connected");
        client.notifications()
    };

    let mut plan = protocol_plan_for_test(Vec::new(), Vec::new());
    plan.roster_authors = vec![peer_owner.public_key().to_hex()];
    let filters = build_protocol_subscription_filters(&plan);
    let app_keys_filter = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("AppKeys filter json"))
        .find(|filter| {
            filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(APP_KEYS_EVENT_KIND as u64))
                })
        })
        .expect("AppKeys subscription filter");
    assert!(
        app_keys_filter.get("since").is_none(),
        "AppKeys subscription replay must remain unbounded"
    );
    core.protocol_subscription_runtime.desired_plan = Some(plan.clone());
    core.protocol_subscription_runtime.applied_plan = Some(plan);
    core.refresh_relay_connection_status();
    assert!(
        core.relay_connected_count > 0,
        "core must observe the connected relay before liveness reconciliation"
    );

    core.debug_log.clear();
    let token = core.protocol_liveness_token;
    core.handle_protocol_subscription_liveness_check(token);
    assert!(core.protocol_subscription_runtime.refresh_in_flight);

    let retained = core.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match notifications.recv().await {
                    Ok(RelayPoolNotification::Event { event, .. })
                        if event.id == app_keys_event_id =>
                    {
                        break (*event).clone();
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(error) => panic!("relay notifications closed: {error}"),
                }
            }
        })
        .await
        .expect("retained AppKeys subscription replay")
    });
    core.handle_relay_event(retained);

    let known = core
        .app_keys
        .get(&peer_owner.public_key().to_hex())
        .expect("peer AppKeys cached after subscription replay");
    assert!(known.devices.iter().any(|device| {
        device.identity_pubkey_hex == peer_device.public_key().to_hex()
    }));
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .has_device_roster_entry_for_owner(
                peer_owner.public_key(),
                peer_device.public_key()
            ),
        "subscription replay must install the peer roster in ProtocolEngine"
    );

    assert!(
        !core
            .debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.fetch"),
        "connected-relay liveness must not procedural-fetch tracked peers with missing AppKeys"
    );
}

#[test]
fn protocol_liveness_scheduling_keeps_earliest_reconnect_deadline() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-liveness-earliest-deadline", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    assert!(
        core.protocol_subscription_runtime.desired_plan.is_some(),
        "logged-in session should derive protocol subscriptions"
    );
    core.protocol_subscription_runtime.liveness_due_at = None;
    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
    let first_token = core.protocol_liveness_token;
    let first_due = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("initial liveness should be scheduled");

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
    assert_eq!(
        core.protocol_liveness_token, first_token,
        "a later/equal liveness request must not cancel the pending reconnect"
    );
    assert_eq!(
        core.protocol_subscription_runtime.liveness_due_at,
        Some(first_due)
    );

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(2));
    let fast_token = core.protocol_liveness_token;
    let fast_due = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("fast reconnect should be scheduled");
    assert!(
        fast_token > first_token,
        "an earlier liveness request should replace the previous deadline"
    );
    assert!(
        fast_due < first_due,
        "fast reconnect should move the liveness deadline earlier"
    );

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
    assert_eq!(
        core.protocol_liveness_token, fast_token,
        "a later liveness request must not starve the fast reconnect"
    );
    assert_eq!(
        core.protocol_subscription_runtime.liveness_due_at,
        Some(fast_due)
    );
}

#[test]
fn tracked_peer_catch_up_scheduling_coalesces_to_earliest_deadline() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("tracked-peer-catch-up-coalesce", &owner, &device);

    core.schedule_tracked_peer_catch_up(Duration::from_secs(30));
    let first_token = core
        .protocol_subscription_runtime
        .tracked_peer_catch_up_token;
    let first_due = core
        .protocol_subscription_runtime
        .tracked_peer_catch_up_due_at
        .expect("initial catch-up should be scheduled");

    core.schedule_tracked_peer_catch_up(Duration::from_secs(30));
    assert_eq!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_token,
        first_token,
        "a later/equal tracked-peer catch-up must not spawn another timer"
    );
    assert_eq!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at,
        Some(first_due)
    );

    core.schedule_tracked_peer_catch_up(Duration::from_secs(2));
    assert!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_token
            > first_token,
        "an earlier tracked-peer catch-up should replace the old timer"
    );
    assert!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at
            .expect("fast catch-up should stay scheduled")
            < first_due
    );
}

#[test]
fn protocol_state_fetch_is_single_flight() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-fetch-single-flight", &owner, &device);

    core.protocol_subscription_runtime.protocol_fetch_in_flight = true;
    core.debug_log.clear();

    assert!(
        !core.fetch_recent_protocol_state(),
        "existing protocol fetch should block duplicate catch-up fetch"
    );
    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.skip"),
        "skipped duplicate fetch should be visible in debug output"
    );
}

#[test]
fn protocol_fetch_start_is_rate_limited() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-fetch-rate-limit", &owner, &device);

    core.protocol_subscription_runtime
        .protocol_fetch_last_started_at = Some(Instant::now());
    core.debug_log.clear();

    assert!(
        !core.fetch_recent_protocol_state(),
        "recent protocol fetch should rate-limit broad catch-up fetches"
    );
    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.skip"
                && entry.detail.contains("rate limited")),
        "rate-limited fetch should be visible in debug output"
    );
    assert!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at
            .is_some(),
        "rate-limited fetch should schedule one coalesced retry"
    );
}

#[test]
fn protocol_fetch_rate_limit_tolerates_future_start_time() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-fetch-future-rate-limit", &owner, &device);

    core.protocol_subscription_runtime
        .protocol_fetch_last_started_at = Some(Instant::now() + Duration::from_secs(30));
    core.debug_log.clear();

    assert!(
        !core.fetch_recent_protocol_state(),
        "future protocol fetch timestamp should rate-limit instead of panicking"
    );
    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.skip"
                && entry.detail.contains("rate limited")),
        "future timestamp should be reported as a rate-limit skip"
    );
    assert!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at
            .is_some(),
        "future timestamp should schedule one coalesced retry"
    );
}

#[test]
fn protocol_fetch_rate_limit_tolerates_stale_start_time() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("protocol-fetch-stale-rate-limit", &owner, &device);

    let Some(stale_started_at) = Instant::now().checked_sub(Duration::from_secs(60)) else {
        return;
    };
    core.protocol_subscription_runtime
        .protocol_fetch_last_started_at = Some(stale_started_at);
    core.debug_log.clear();

    core.fetch_recent_protocol_state();
    assert!(
        core.debug_log
            .iter()
            .all(|entry| entry.category != "protocol.catch_up.skip"
                || !entry.detail.contains("rate limited")),
        "stale protocol fetch timestamp should not trigger rate-limit subtraction"
    );
}

#[test]
fn liveness_retries_protocol_backfill_for_tracked_peer_with_roster_but_no_session() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let mut core = logged_in_test_core("tracked-peer-roster-no-session-backfill", &owner, &device);

    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        logged_in.relay_urls = relay_urls.clone();
        let client = logged_in.client.clone();
        let connected = core.runtime.block_on(async {
            ensure_session_relays_configured(&client, &relay_urls).await;
            connect_client_with_timeout(&client, Duration::from_secs(2)).await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                let connected = client
                    .relays()
                    .await
                    .values()
                    .filter(|relay| relay.status() == RelayStatus::Connected)
                    .count();
                if connected > 0 || tokio::time::Instant::now() >= deadline {
                    break connected;
                }
                sleep(Duration::from_millis(50)).await;
            }
        });
        assert!(connected > 0, "test relay must be connected");
    }

    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    {
        let peer_app_keys_event = peer_app_keys
            .get_event_at(peer_owner.public_key(), 1)
            .sign_with_keys(&peer_owner)
            .expect("signed peer appkeys");
        let batch = core
            .protocol_engine
            .as_mut()
            .expect("protocol engine")
            .ingest_app_keys_event(&peer_app_keys_event)
            .expect("ingest peer appkeys event");
        core.process_protocol_engine_retry_batch("test_app_keys", batch);
    }
    core.app_keys.insert(
        peer_owner.public_key().to_hex(),
        known_app_keys_from_ndr(peer_owner.public_key(), &peer_app_keys, 1),
    );
    core.active_chat_id = Some(peer_owner.public_key().to_hex());

    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(peer_owner.public_key())
            .is_empty(),
        "peer roster without invite response must not have message authors yet"
    );
    core.request_protocol_subscription_refresh_forced();
    assert!(
        core.protocol_subscription_runtime.desired_plan.is_some(),
        "tracked peer roster should derive protocol-state subscriptions"
    );

    core.debug_log.clear();
    let token = core.protocol_liveness_token;
    core.handle_protocol_subscription_liveness_check(token);

    assert!(
        !core
            .debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.fetch"),
        "connected-relay liveness must not procedural-fetch tracked peers that have AppKeys but no session authors"
    );
}

#[test]
fn protocol_backfill_fetches_configure_relays_before_network_fetch() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    for fn_name in [
        "fetch_recent_messages_for_author",
        "fetch_recent_group_sender_key_messages_for_author",
        "spawn_protocol_author_backfills",
        "fetch_recent_protocol_state_inner",
    ] {
        let start = protocol_source
            .find(&format!("pub(super) fn {fn_name}"))
            .or_else(|| protocol_source.find(&format!("fn {fn_name}")))
            .unwrap_or_else(|| panic!("missing {fn_name}"));
        let body = &protocol_source[start..];
        let end = body
            .find("\n    pub(super) fn ")
            .or_else(|| body.find("\n    fn "))
            .unwrap_or(body.len());
        let body = &body[..end];
        assert!(
            body.contains("ensure_session_relays_configured(&client, &relay_urls).await;")
                || body.contains("self.spawn_protocol_author_backfills("),
            "{fn_name} must configure relays, or delegate to the shared configured backfill helper, before fetching public-relay backfill"
        );
    }
}

#[test]
fn relay_connect_helper_does_not_disconnect_when_all_relays_stay_offline() {
    let core_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core.rs"),
    )
    .expect("read core source");
    let start = core_source
        .find("async fn connect_client_with_timeout")
        .expect("connect helper");
    let body = &core_source[start
        ..core_source[start..]
            .find("\nasync fn connected_relay_count_for_client")
            .map(|offset| start + offset)
            .unwrap_or(core_source.len())];
    assert!(
        body.contains("connected_relay_count_for_client(client).await > 0"),
        "connect helper must detect all-offline relay clients"
    );
    assert!(
        !body.contains("client.disconnect().await"),
        "normal relay connect helper must not tear down the shared client when every relay remains offline"
    );
    assert!(
        body.contains("client.connect().await"),
        "connect helper must start the shared relay client without owning disconnect lifecycle"
    );
}

#[test]
fn runtime_publish_uses_single_flight_transport_drain() {
    let publish_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/publishing.rs"),
    )
    .expect("read publishing source");
    let start = publish_source
        .find("pub(super) fn retry_pending_relay_publishes")
        .expect("pending relay publish retry");
    let body = &publish_source[start
        ..publish_source[start..]
            .find("\n    pub(super) fn handle_relay_publish_drain_finished")
            .map(|offset| start + offset)
            .unwrap_or(publish_source.len())];
    assert!(
        body.contains("publish_drain_in_flight") && body.contains("publish_drain_dirty"),
        "pending relay publishes must coalesce through one drain worker"
    );
    assert!(
        body.contains("pending_relay_publish_batch_event_ids"),
        "pending relay retry must choose a bounded batch before cloning event JSON"
    );
    let compact_body = body
        .chars()
        .filter(|char| !char.is_whitespace())
        .collect::<String>();
    assert!(
        !compact_body.contains(".values().cloned().collect::<Vec<_>>()"),
        "pending relay retry must not clone the full pending backlog on every drain"
    );
    assert!(
        body.contains("request_relay_connection"),
        "offline pending publish retry must request the shared relay transport connection"
    );
    assert!(
        body.contains("publish_event_to_any_connected_relay")
            && body.contains("PENDING_RELAY_DRAIN_CONCURRENCY"),
        "drain worker must publish with bounded connected-client/raw fallback attempts"
    );
    assert!(
        !body.contains("connect_client_with_timeout") && !body.contains("client.disconnect"),
        "drain worker must not connect or disconnect the shared relay client per pending event"
    );
    assert!(
        !publish_source.contains("spawn_relay_publish_attempt"),
        "pending relay publish retry must not spawn one connection-owning task per event"
    );
}

#[test]
fn direct_send_hot_path_does_not_force_global_catch_up_for_established_messages() {
    let chats_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/chats.rs"),
    )
    .expect("read chats source");
    let start = chats_source
        .find("pub(super) fn send_direct_message")
        .expect("send_direct_message");
    let body = &chats_source[start
        ..chats_source[start..]
            .find("\n    pub(super) fn send_group_message")
            .map(|offset| start + offset)
            .unwrap_or(chats_source.len())];

    assert!(
        !body.contains("request_protocol_subscription_refresh_forced_reconnect_if_offline"),
        "normal direct sends must not force relay/subscription reconnect on every message"
    );
    assert!(
        !body.contains("fetch_recent_messages_for_tracked_peers"),
        "normal direct sends must not launch global tracked-peer catch-up on every message"
    );
    assert!(
        !body.contains("schedule_tracked_peer_catch_up"),
        "normal direct sends must not schedule delayed global catch-up on every message"
    );
}

#[test]
fn zero_connected_transport_connect_schedules_backoff_retry() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    let start = protocol_source
        .find("pub(super) fn handle_relay_transport_connection_finished")
        .expect("transport connection handler");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    pub(super) fn schedule_relay_transport_retry")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        body.contains("schedule_relay_transport_retry(\"connect_failed\")")
            && body.contains("retry_pending_relay_publishes(\"relay_transport_connected\")"),
        "zero-connected transport checks must back off, while successful connects drain pending publishes"
    );
}

#[test]
fn pending_inbound_direct_message_schedules_fast_liveness_retry() {
    let relay_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/relay.rs"),
    )
    .expect("read relay source");
    let start = relay_source
        .find("\"appcore.protocol.message.pending\"")
        .expect("pending direct message branch");
    let body = &relay_source[start
        ..relay_source[start..]
            .find("Err(error)")
            .map(|offset| start + offset)
            .unwrap_or(relay_source.len())];
    assert!(
        body.contains("request_protocol_subscription_refresh()")
            && body.contains("schedule_fast_protocol_retry_if_pending()"),
        "pending inbound direct events must schedule a fast protocol retry instead of waiting for restart/foreground"
    );
}

#[test]
fn liveness_retries_pending_inbound_direct_events() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    let start = protocol_source
        .find("pub(super) fn handle_protocol_subscription_liveness_check")
        .expect("liveness handler");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    pub(super) fn reconcile_protocol_subscriptions")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    let retry_helpers_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/core/protocol/retry_helpers.rs"),
    )
    .expect("read retry helper source");
    assert!(
        retry_helpers_source.contains("has_pending_retry_work()")
            && body.contains("pending_protocol_retry_needed")
            && body.contains("if pending_protocol_retry_needed")
            && body.contains("self.retry_protocol_engine_pending_work(\"liveness_check\")"),
        "protocol liveness must retry durable pending protocol work even when tracked-peer backfill appears complete"
    );
}

#[test]
fn pending_inbound_liveness_does_not_force_global_message_backfill() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    let start = protocol_source
        .find("pub(super) fn handle_protocol_subscription_liveness_check")
        .expect("liveness handler");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    pub(super) fn reconcile_protocol_subscriptions")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        body.contains("pending_protocol_retry_needed")
            && body.contains("if pending_protocol_retry_needed"),
        "pending inbound direct events must still wake protocol-state retry"
    );
    assert!(
        !body.contains("let should_fetch_tracked_peer_messages"),
        "pending inbound retry alone must not trigger global tracked-peer message history fetches"
    );
    assert!(
        !body.contains("tracked_peer_backfill_needed"),
        "queued protocol-state retry alone must not trigger global tracked-peer message history fetches"
    );
    assert!(
        !body.contains("self.fetch_recent_messages_for_tracked_peers();"),
        "protocol liveness must not perform procedural tracked-peer message backfill"
    );
}

#[test]
fn scheduled_tracked_peer_catch_up_does_not_force_global_message_backfill() {
    let lifecycle_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/lifecycle.rs"),
    )
    .expect("read lifecycle source");
    let start = lifecycle_source
        .find("InternalEvent::FetchTrackedPeerCatchUp { token } => {")
        .expect("tracked peer catch-up handler");
    let body = &lifecycle_source[start
        ..lifecycle_source[start..]
            .find("InternalEvent::ProtocolSubscriptionLivenessCheck")
            .map(|offset| start + offset)
            .unwrap_or(lifecycle_source.len())];

    assert!(
        body.contains("fetch_recent_protocol_metadata_state"),
        "scheduled protocol catch-up should retry metadata/state without pulling all message history"
    );
    assert!(
        !body.contains("fetch_recent_protocol_state();"),
        "scheduled protocol catch-up must not use the full history fetch path unconditionally"
    );
    assert!(
        body.contains("if should_fetch_tracked_peer_messages")
            && body.contains("self.fetch_recent_messages_for_tracked_peers();"),
        "message history catch-up should be gated behind dirty or unapplied subscription state"
    );
}

fn read_protocol_engine_source() -> Option<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent().expect("core crate has repo parent");
    if !repo.join("chat-protocol/Cargo.toml").is_file() {
        return None;
    }
    Some([
        "chat-protocol/src/protocol_engine.rs",
        "chat-protocol/src/protocol_engine/types.rs",
        "chat-protocol/src/protocol_engine/engine_core.rs",
        "chat-protocol/src/protocol_engine/engine_sends.rs",
        "chat-protocol/src/protocol_engine/roster_helpers.rs",
        "chat-protocol/src/protocol_engine/engine_incoming_retry.rs",
        "chat-protocol/src/protocol_engine/engine_resolution.rs",
        "chat-protocol/src/protocol_engine/engine_sender_key_repair.rs",
        "chat-protocol/src/protocol_engine/engine_persistence.rs",
        "chat-protocol/src/protocol_engine/free_functions.rs",
    ]
    .into_iter()
    .map(|path| {
        std::fs::read_to_string(repo.join(path))
            .unwrap_or_else(|error| panic!("read {path}: {error}"))
    })
    .collect::<Vec<_>>()
    .join("\n"))
}

#[test]
fn appcore_sender_owner_resolution_keeps_claimed_device_pending_until_owner_verified() {
    let Some(protocol_source) = read_protocol_engine_source() else {
        return;
    };
    let start = protocol_source
        .find("fn resolve_message_sender_owner")
        .expect("sender resolver");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    fn ensure_local_roster")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        body.contains("PendingOwnerClaim"),
        "claimed owners must be represented as pending, not collapsed into a device owner"
    );
    assert!(
        !body.contains("NdrOwnerPubkey::from_bytes(envelope.sender.to_bytes())"),
        "message envelope sender is a ratchet sender key and must not become the canonical owner"
    );
}

#[test]
fn pending_inbound_owner_targets_use_cached_metadata_not_event_reparse() {
    let Some(protocol_source) = read_protocol_engine_source() else {
        return;
    };
    let start = protocol_source
        .find("fn pending_inbound_owner_claim_targets")
        .expect("pending inbound target collector");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    fn pending_group_pairwise_owner_claim_targets")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        body.contains("claimed_owner_pubkey_hex"),
        "pending inbound owner target collection must use cached owner metadata"
    );
    assert!(
        !body.contains("parse_message_event"),
        "pending inbound owner target collection runs on the relay hot path and must not verify every pending event"
    );
}

#[test]
fn relay_pending_inbound_replays_are_short_circuited() {
    let relay_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/relay.rs"),
    )
    .expect("read relay source");
    let start = relay_source
        .find("if kind == MESSAGE_EVENT_KIND")
        .expect("message event branch");
    let body = &relay_source[start
        ..relay_source[start..]
            .find("\n        self.remember_event(event_id);")
            .map(|offset| start + offset)
            .unwrap_or(relay_source.len())];
    assert!(
        body.contains("has_pending_inbound_direct_event_id")
            && body.contains("appcore.protocol.message.pending_replay"),
        "relay replays of already-durable pending inbound events must avoid reparsing and refetching immediately"
    );
}

#[test]
fn invalid_pairwise_message_errors_are_seen_without_state_emit() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let alice = Keys::generate();
    let (mut core, update_rx, _temp_dir) =
        logged_in_test_core_with_updates("invalid-pairwise-seen", &owner, &device);

    let (_valid_message, known_sender_keys) = appcore_direct_message_event_with_author_keys_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &alice,
        "prime sender session",
        200,
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .is_known_message_author(known_sender_keys.public_key()),
        "test sender should be known before the invalid direct message arrives"
    );
    drain_app_updates(&update_rx);

    let invalid =
        signed_pairwise_message_event_for_test(&known_sender_keys, "not-a-valid-header", "ciphertext");
    let event_id = invalid.id.to_string();
    let builds_before = core.debug_snapshot_build_count();

    core.handle_relay_event(invalid.clone());

    assert!(
        core.has_seen_event(&event_id),
        "unrecoverable pairwise parse errors should not be retried forever"
    );
    assert_eq!(
        core.debug_snapshot_build_count(),
        builds_before,
        "invalid direct messages should not force snapshot persistence"
    );
    assert!(
        update_rx.try_recv().is_err(),
        "invalid direct messages should not emit a full app state"
    );

    let message_count_after_first = core.debug_event_counters.message_events;
    core.handle_relay_event(invalid);
    assert_eq!(
        core.debug_event_counters.message_events, message_count_after_first,
        "seen invalid direct messages should short-circuit before parsing"
    );
    assert!(
        update_rx.try_recv().is_err(),
        "seen invalid direct messages should stay silent"
    );
}

#[test]
fn no_header_message_kind_event_is_not_pairwise_decrypted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let alice = Keys::generate();
    let (mut core, update_rx, _temp_dir) =
        logged_in_test_core_with_updates("no-header-message-kind", &owner, &device);

    let _valid_message = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &alice,
        "prime sender session",
        200,
    );
    drain_app_updates(&update_rx);

    let malformed = EventBuilder::new(
        Kind::from(MESSAGE_EVENT_KIND as u16),
        "not a group sender-key payload",
    )
    .sign_with_keys(&alice)
    .expect("signed malformed message event");
    let event_id = malformed.id.to_string();
    let builds_before = core.debug_snapshot_build_count();

    core.handle_relay_event(malformed);

    assert!(
        core.has_seen_event(&event_id),
        "malformed kind-1060 events without a pairwise header should be consumed once"
    );
    assert_eq!(
        core.debug_event_counters.message_events, 0,
        "events without a pairwise header should not enter direct-message decrypt"
    );
    assert_eq!(
        core.debug_snapshot_build_count(),
        builds_before,
        "malformed non-pairwise events should not force snapshot persistence"
    );
    assert!(
        update_rx.try_recv().is_err(),
        "malformed non-pairwise events should not emit a full app state"
    );
}

#[test]
fn group_sender_key_ignored_results_are_consumed_without_retry_queue() {
    let Some(protocol_source) = read_protocol_engine_source() else {
        return;
    };
    let process_start = protocol_source
        .find("fn process_group_outer_event")
        .expect("process group outer function");
    let process_body = &protocol_source[process_start
        ..protocol_source[process_start..]
            .find("fn process_group_pairwise_payload")
            .map(|offset| process_start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        process_body.contains("if result.pending"),
        "group outer handling must queue sender-key messages only for explicit pending results"
    );
    assert!(
        !process_body.contains("if result.events.is_empty()"),
        "ignored sender-key results have no events but must not be queued for retry"
    );

    let handle_start = protocol_source
        .find("fn handle_group_sender_key_message")
        .expect("handle sender key function");
    let handle_body = &protocol_source[handle_start
        ..protocol_source[handle_start..]
            .find("fn sender_key_repair_request_effects")
            .map(|offset| handle_start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        handle_body.contains("GroupSenderKeyHandleResult::Ignored")
            && handle_body.contains("consumed: true"),
        "ignored parsed sender-key events should be consumed so public-relay replays do not loop"
    );
}
