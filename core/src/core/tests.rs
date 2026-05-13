use super::protocol::build_protocol_subscription_filters;
use super::*;
use nostr_double_ratchet_runtime::{NdrRuntime, SessionManagerEvent};

#[derive(Clone)]
struct SwitchableFailStorage {
    inner: nostr_double_ratchet_runtime::InMemoryStorage,
    fail_puts: Arc<std::sync::atomic::AtomicBool>,
}

impl SwitchableFailStorage {
    fn new() -> Self {
        Self {
            inner: nostr_double_ratchet_runtime::InMemoryStorage::new(),
            fail_puts: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn set_fail_puts(&self, fail: bool) {
        self.fail_puts
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

impl StorageAdapter for SwitchableFailStorage {
    fn get(&self, key: &str) -> nostr_double_ratchet_runtime::Result<Option<String>> {
        self.inner.get(key)
    }

    fn put(&self, key: &str, value: String) -> nostr_double_ratchet_runtime::Result<()> {
        if self.fail_puts.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(nostr_double_ratchet_runtime::Error::Storage(
                "injected storage failure".to_string(),
            ));
        }
        self.inner.put(key, value)
    }

    fn del(&self, key: &str) -> nostr_double_ratchet_runtime::Result<()> {
        self.inner.del(key)
    }

    fn list(&self, prefix: &str) -> nostr_double_ratchet_runtime::Result<Vec<String>> {
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
    let invite = receiver_engine
        .local_invite_for_test()
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
    message_event(&sent.envelope).expect("message event")
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
    core.core_sender = core_tx;

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
        std::thread::sleep(std::time::Duration::from_millis(50));
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
fn staged_first_contact_queues_payload_durably_before_delayed_publish() {
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
    let completions = BTreeMap::from([(payload_id.clone(), (message_id.clone(), chat_id.clone()))]);

    core.process_protocol_engine_effects_with_completions(
        vec![ProtocolEffect::PublishStagedFirstContact {
            bootstrap: vec![ProtocolPublishEvent {
                event: bootstrap,
                inner_event_id: None,
                target_owner_pubkey_hex: None,
                target_device_id: None,
            }],
            payload: vec![ProtocolPublishEvent {
                event: payload,
                inner_event_id: Some(message_id.clone()),
                target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
                target_device_id: Some(peer.public_key().to_hex()),
            }],
        }],
        &completions,
    );

    let pending = core
        .pending_relay_publishes
        .get(&payload_id)
        .expect("payload should be queued before delayed publish");
    assert_eq!(pending.label, "appcore-protocol-first-contact");
    assert_eq!(pending.message_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(pending.chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(
        !core.pending_relay_publish_inflight.contains(&payload_id),
        "payload should be durable but not in flight until the first-contact delay fires"
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
    core.core_sender = core_tx;
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
            target_owner_pubkey_hex: None,
            target_device_id: None,
            message_id: None,
            chat_id: None,
            created_at_secs: event.created_at.as_secs(),
            attempt_count: 0,
            last_error: Some("initial offline publish failed".to_string()),
        },
    );

    core.handle_protocol_subscription_liveness_check(core.protocol_reconnect_token);

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
        std::thread::sleep(std::time::Duration::from_millis(50));
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
                target_owner_pubkey_hex: None,
                target_device_id: None,
                message_id: None,
                chat_id: None,
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 0,
                last_error: None,
            },
        );
        results.push(RelayPublishDrainResult {
            event_id,
            message_id: None,
            chat_id: None,
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
    assert!(
        ["connecting", "offline"].contains(&status.relay_connections[0].status.as_str()),
        "unexpected relay status: {}",
        status.relay_connections[0].status
    );
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
    let device_id = device.public_key().to_hex();
    let invite =
        Invite::create_new(device.public_key(), Some(device_id), None).expect("local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: Client::new(device),
        relay_urls: Vec::new(),
        local_invite: invite,
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
    let target_device_id = "target-device".to_string();
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

    assert!(core.publish_runtime_event_with_metadata(
        first,
        APPCORE_PROTOCOL_LABEL,
        Some((message_id.clone(), chat_id.clone())),
        Some("inner-message".to_string()),
        Some(chat_id.clone()),
        Some(target_device_id.clone()),
    ));
    assert!(core.publish_runtime_event_with_metadata(
        second,
        APPCORE_PROTOCOL_LABEL,
        Some((message_id, chat_id.clone())),
        Some("inner-message".to_string()),
        Some(chat_id),
        Some(target_device_id),
    ));

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
fn stale_protocol_subscription_reconcile_completion_is_ignored() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("subscription-reconcile-stale", &owner, &device);
    core.protocol_reconnect_token = 5;
    core.protocol_subscription_runtime.reconcile_token = 7;
    core.protocol_subscription_runtime.refresh_in_flight = true;

    core.handle_protocol_subscription_reconcile_completed(
        4,
        7,
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
fn liveness_retries_protocol_backfill_for_tracked_peer_missing_appkeys_when_connected() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let relay = crate::local_relay::TestRelay::start();
    let mut core = logged_in_test_core("tracked-peer-liveness-backfill", &owner, &device);

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

    core.active_chat_id = Some(peer.public_key().to_hex());
    core.request_protocol_subscription_refresh_forced();
    assert!(
        core.protocol_subscription_runtime.desired_plan.is_some(),
        "tracked peer setup should create runtime protocol subscriptions"
    );

    core.debug_log.clear();
    let token = core.protocol_reconnect_token;
    core.handle_protocol_subscription_liveness_check(token);

    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.fetch"),
        "connected-relay liveness must still backfill tracked peers with missing AppKeys"
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
    let first_token = core.protocol_reconnect_token;
    let first_due = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("initial liveness should be scheduled");

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(30));
    assert_eq!(
        core.protocol_reconnect_token, first_token,
        "a later/equal liveness request must not cancel the pending reconnect"
    );
    assert_eq!(
        core.protocol_subscription_runtime.liveness_due_at,
        Some(first_due)
    );

    core.schedule_protocol_subscription_liveness_check(Duration::from_secs(2));
    let fast_token = core.protocol_reconnect_token;
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
        core.protocol_reconnect_token, fast_token,
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
fn targeted_protocol_fetch_is_single_flight() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("targeted-protocol-fetch-single-flight", &owner, &device);

    core.protocol_subscription_runtime.protocol_fetch_in_flight = true;
    core.debug_log.clear();

    let filters = vec![Filter::new()
        .author(peer.public_key())
        .kind(Kind::Custom(APP_KEYS_EVENT_KIND as u16))];
    assert!(
        !core.fetch_protocol_state_for_filters(filters, "test"),
        "existing protocol fetch should block duplicate targeted engine fetch"
    );
    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.engine_fetch.skip"),
        "skipped targeted fetch should be visible in debug output"
    );
    assert!(
        core.protocol_subscription_runtime
            .tracked_peer_catch_up_due_at
            .is_some(),
        "skipped targeted fetch should schedule a coalesced retry"
    );
}

#[test]
fn protocol_fetch_start_is_rate_limited() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("protocol-fetch-rate-limit", &owner, &device);

    core.protocol_subscription_runtime
        .protocol_fetch_last_started_at = Some(Instant::now());
    core.debug_log.clear();

    let filters = vec![Filter::new()
        .author(peer.public_key())
        .kind(Kind::Custom(APP_KEYS_EVENT_KIND as u16))];
    assert!(
        !core.fetch_protocol_state_for_filters(filters, "test"),
        "recent protocol fetch should rate-limit targeted engine fetches"
    );
    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.engine_fetch.skip"
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
        let batch = core
            .protocol_engine
            .as_mut()
            .expect("protocol engine")
            .ingest_app_keys_snapshot(peer_owner.public_key(), peer_app_keys.clone(), 1)
            .expect("ingest peer appkeys");
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
    let token = core.protocol_reconnect_token;
    core.handle_protocol_subscription_liveness_check(token);

    assert!(
        core.debug_log
            .iter()
            .any(|entry| entry.category == "protocol.catch_up.fetch"),
        "connected-relay liveness must backfill tracked peers that have AppKeys but no session authors"
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
        "fetch_recent_protocol_state",
    ] {
        let start = protocol_source
            .find(&format!("pub(super) fn {fn_name}"))
            .unwrap_or_else(|| panic!("missing {fn_name}"));
        let body = &protocol_source[start..];
        let end = body
            .find("\n    pub(super) fn ")
            .or_else(|| body.find("\n    fn "))
            .unwrap_or(body.len());
        let body = &body[..end];
        assert!(
            body.contains("ensure_session_relays_configured(&client, &relay_urls).await;"),
            "{fn_name} must configure relays before fetching public-relay backfill"
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
        body.contains("request_relay_connection"),
        "offline pending publish retry must request the shared relay transport connection"
    );
    assert!(
        body.contains("publish_event_to_any_relay")
            && body.contains("PENDING_RELAY_DRAIN_CONCURRENCY"),
        "drain worker must publish with bounded no-connect attempts"
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
        body.contains("schedule_protocol_subscription_liveness_check")
            && body.contains("PROTOCOL_RECONNECT_CHECK_SECS"),
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
    assert!(
        body.contains("has_pending_inbound_direct_events")
            && body.contains("should_retry_backfill"),
        "protocol liveness must retry durable pending inbound direct events even when tracked-peer backfill appears complete"
    );
}

#[test]
fn appcore_sender_owner_resolution_keeps_claimed_device_pending_until_owner_verified() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol_engine.rs"),
    )
    .expect("read protocol engine source");
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
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol_engine.rs"),
    )
    .expect("read protocol engine source");
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
fn group_sender_key_ignored_results_are_consumed_without_retry_queue() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol_engine.rs"),
    )
    .expect("read protocol engine source");
    let process_start = protocol_source
        .find("pub(super) fn process_group_outer_event")
        .expect("process group outer function");
    let process_body = &protocol_source[process_start
        ..protocol_source[process_start..]
            .find("pub(super) fn process_group_pairwise_payload")
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
            .find("fn upsert_pending_outbound")
            .map(|offset| handle_start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        handle_body.contains("GroupSenderKeyHandleResult::Ignored")
            && handle_body.contains("consumed: true"),
        "ignored parsed sender-key events should be consumed so public-relay replays do not loop"
    );
}

#[test]
fn appcore_protocol_engine_missing_remote_owner_send_keeps_owner_pending() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let result = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "first",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    let published_peer_targets = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                target_owner_pubkey_hex,
                ..
            } => target_owner_pubkey_hex.clone(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !published_peer_targets.contains(&peer_owner.public_key().to_hex()),
        "peer owner must not be considered published before peer protocol state exists"
    );
    let owner_marker = format!("owner:{}", peer_owner.public_key().to_hex());
    let local_owner_marker = format!("owner:{}", owner.public_key().to_hex());
    assert!(result.queued_targets.contains(&owner_marker));
    assert!(result.queued_targets.contains(&local_owner_marker));
    let snapshot = engine.debug_snapshot();
    assert_eq!(snapshot.pending_outbound_count, 1);
    assert!(snapshot.pending_outbound_targets.contains(&owner_marker));
    assert!(snapshot
        .pending_outbound_targets
        .contains(&local_owner_marker));
}

#[test]
fn appcore_protocol_engine_retry_before_peer_discovery_keeps_missing_roster_pending() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let result = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "first",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    let owner_marker = format!("owner:{}", peer_owner.public_key().to_hex());
    let local_owner_marker = format!("owner:{}", owner.public_key().to_hex());
    assert!(result.queued_targets.contains(&owner_marker));
    assert!(result.queued_targets.contains(&local_owner_marker));

    let retries = engine
        .retry_pending_outbound(NdrUnixSeconds(10_000))
        .expect("retry pending outbound");
    assert_eq!(retries.len(), 1);
    assert_eq!(retries[0].message_id, result.message_id);
    assert!(retries[0].event_ids.is_empty());
    assert!(
        retries[0]
            .effects
            .iter()
            .any(|effect| matches!(effect, ProtocolEffect::FetchProtocolState { .. })),
        "retrying missing roster work should re-emit protocol backfill from the engine"
    );
    assert!(retries[0].queued_targets.contains(&owner_marker));
    let snapshot = engine.debug_snapshot();
    assert_eq!(snapshot.pending_outbound_count, 1);
    assert!(snapshot.pending_outbound_targets.contains(&owner_marker));
}

#[test]
fn appcore_direct_send_storage_failure_rolls_back_protocol_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut engine = test_protocol_engine_with_storage(
        &owner,
        &device,
        storage.clone() as Arc<dyn StorageAdapter>,
    );
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(2), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");
    engine
        .observe_invite_event(&invite_event)
        .expect("observe invite");
    let before = engine.session_manager_snapshot_for_test();

    storage.set_fail_puts(true);
    let result = engine.send_direct_text(
        peer_owner.public_key(),
        &peer_owner.public_key().to_hex(),
        "rollback",
        None,
        UnixSeconds(3),
    );

    assert!(result.is_err());
    assert_eq!(
        engine.session_manager_snapshot_for_test(),
        before,
        "failed persistence must roll back in-memory ratchet state"
    );
    assert_eq!(
        engine.debug_snapshot().pending_outbound_count,
        0,
        "failed persistence must not leave pending outbound state in memory"
    );
}

#[test]
fn appcore_group_create_storage_failure_rolls_back_protocol_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut engine = test_protocol_engine_with_storage(
        &owner,
        &device,
        storage.clone() as Arc<dyn StorageAdapter>,
    );
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let before_sessions = engine.session_manager_snapshot_for_test();
    let before_groups = engine.group_manager_snapshot_for_test();

    storage.set_fail_puts(true);
    let result = engine.create_group(
        "rollback group".to_string(),
        vec![peer_owner.public_key()],
        UnixSeconds(3),
    );

    assert!(result.is_err());
    assert_eq!(
        engine.session_manager_snapshot_for_test(),
        before_sessions,
        "failed group persistence must roll back session fanout preparation"
    );
    assert_eq!(
        engine.group_manager_snapshot_for_test(),
        before_groups,
        "failed group persistence must roll back group manager state"
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_fanout_count,
        0,
        "failed group persistence must not leave pending group fanouts in memory"
    );
}

#[test]
fn appcore_invite_event_wakes_device_queued_direct_send_before_retry_delay() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let send = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "queued until invite",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", peer_owner.public_key().to_hex())),
        "missing peer roster should be queued"
    );
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "local sibling discovery should remain queued until the owner roster is known to have no siblings"
    );

    let app_keys_batch = engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 4)]),
            4,
        )
        .expect("peer appkeys");
    assert_eq!(app_keys_batch.direct_results.len(), 1);
    assert!(
        app_keys_batch.direct_results[0]
            .queued_targets
            .contains(&peer_device.public_key().to_hex()),
        "after peer roster discovery, the peer device invite should remain missing"
    );
    assert!(
        !app_keys_batch.direct_results[0]
            .queued_targets
            .contains(&format!("owner:{}", peer_owner.public_key().to_hex())),
        "peer owner discovery should be drained after AppKeys arrive"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(5), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");

    let invite_batch = engine
        .observe_invite_event(&invite_event)
        .expect("observe invite");

    assert_eq!(invite_batch.direct_results.len(), 1);
    assert_eq!(invite_batch.direct_results[0].message_id, send.message_id);
    assert_eq!(invite_batch.direct_results[0].event_ids.len(), 1);
    assert!(
        !engine
            .debug_snapshot()
            .pending_outbound_targets
            .contains(&peer_device.public_key().to_hex()),
        "remote peer fanout should be fully drained"
    );
}

#[test]
fn seen_invite_event_replays_into_protocol_engine_for_queued_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("seen-invite-replay", &owner, &device);
    {
        let engine = core.protocol_engine.as_mut().expect("protocol engine");
        observe_current_device_appkeys_for_test(engine, &owner, &device);
        engine
            .ingest_app_keys_snapshot(
                peer_owner.public_key(),
                AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 4)]),
                4,
            )
            .expect("peer appkeys");
    }

    core.send_direct_message(
        &peer_owner.public_key().to_hex(),
        "queued until seen invite replays",
        UnixSeconds(5),
        None,
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .debug_snapshot()
            .pending_outbound_targets
            .contains(&peer_device.public_key().to_hex()),
        "send should wait for the peer device invite"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(6), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("peer invite");
    let invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(&peer_device)
        .expect("signed invite");

    core.remember_event(invite_event.id.to_string());
    core.handle_relay_event(invite_event);

    let debug = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert!(
        !debug
            .pending_outbound_targets
            .contains(&peer_device.public_key().to_hex()),
        "seen invite events must still rebuild protocol state and drain queued sends"
    );
}

#[test]
fn appcore_direct_send_keeps_local_sibling_probe_until_local_appkeys_and_invite_arrive() {
    let owner = Keys::generate();
    let fresh_device = Keys::generate();
    let old_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &fresh_device);

    let send = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "self sync should not be dropped",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", peer_owner.public_key().to_hex())),
        "remote owner discovery should remain queued"
    );
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "local sibling roster discovery must be queued until local AppKeys have been observed"
    );

    let local_app_keys_created_at = unix_now().get();
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(old_device.public_key(), 1),
        DeviceEntry::new(fresh_device.public_key(), local_app_keys_created_at),
    ]);
    let app_keys_batch = engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            local_app_keys,
            local_app_keys_created_at,
        )
        .expect("local appkeys");
    assert_eq!(app_keys_batch.direct_results.len(), 1);
    assert!(
        app_keys_batch.direct_results[0]
            .queued_targets
            .contains(&old_device.public_key().to_hex()),
        "local AppKeys should turn the local owner probe into the old device invite target"
    );

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(5), &mut rng);
    let old_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(old_device.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())),
        None,
    )
    .expect("old device invite");
    let old_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&old_invite)
        .expect("invite event")
        .sign_with_keys(&old_device)
        .expect("signed invite");

    let invite_batch = engine
        .observe_invite_event(&old_invite_event)
        .expect("observe old device invite");
    assert_eq!(invite_batch.direct_results.len(), 1);
    let retry = &invite_batch.direct_results[0];
    assert_eq!(retry.message_id, send.message_id);
    assert!(
        retry.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::PublishStagedFirstContact { payload, .. }
                if payload.iter().any(|publish| publish.target_owner_pubkey_hex.as_deref()
                    == Some(owner.public_key().to_hex().as_str())
                    && publish.target_device_id.as_deref()
                        == Some(old_device.public_key().to_hex().as_str()))
        )) || retry.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::PublishSignedForInnerEvent {
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(owner.public_key().to_hex().as_str())
                && target_device_id.as_deref() == Some(old_device.public_key().to_hex().as_str())
        )),
        "old local device should receive a sender-copy publish after its invite arrives"
    );
}

#[test]
fn appcore_local_appkeys_backfill_replaces_seeded_single_device_roster() {
    let owner = Keys::generate();
    let fresh_device = Keys::generate();
    let old_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &fresh_device);

    let send = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "older local appkeys should still discover old sibling",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");
    assert!(
        send.queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "freshly seeded local device should start with owner-level sibling discovery"
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![
                DeviceEntry::new(old_device.public_key(), 1),
                DeviceEntry::new(fresh_device.public_key(), 1),
            ]),
            1,
        )
        .expect("stale local appkeys");
    assert_eq!(batch.direct_results.len(), 1);
    assert!(
        batch.direct_results[0]
            .queued_targets
            .contains(&old_device.public_key().to_hex()),
        "AppCore's merged local roster is authoritative even when its event timestamp predates the seeded local invite"
    );

    let snapshot = engine.debug_snapshot();
    let pending = snapshot
        .pending_outbound_details
        .iter()
        .find(|pending| pending.message_id == send.message_id)
        .expect("pending send detail");
    assert_eq!(
        pending.remaining_local_sibling_targets,
        vec![old_device.public_key().to_hex()]
    );
    assert!(
        !pending
            .queued_targets
            .contains(&format!("owner:{}", owner.public_key().to_hex())),
        "once the merged local roster is installed, retries should target the concrete old device"
    );
}

#[test]
fn invite_response_observation_installs_session_author_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]),
            1,
        )
        .expect("peer appkeys");

    let invite = engine.local_invite_for_test().expect("local invite");
    let (_peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");

    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    assert!(
        !engine
            .message_author_pubkeys_for_owner(peer_owner.public_key())
            .is_empty(),
        "observing the invite response should install receiver state for the peer"
    );
}

#[test]
fn appcore_direct_message_from_unverified_claimed_owner_retries_after_appkeys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);

    let invite = engine.local_invite_for_test().expect("local invite");
    let (mut peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    let plan = peer_session
        .plan_send(b"hello-before-appkeys", NdrUnixSeconds(11))
        .expect("peer plans message");
    let sent = peer_session.apply_send(plan);
    let message_event =
        nostr_double_ratchet_nostr::message_event(&sent.envelope).expect("message event");

    let decrypted = engine
        .process_direct_message_event(&message_event)
        .expect("process direct message");
    assert!(
        decrypted.is_none(),
        "claimed-owner messages must wait until the owner claim is verified"
    );
    assert_eq!(engine.debug_snapshot().pending_inbound_count, 1);
    let sender_message_pubkey_hex = sent.envelope.sender.to_hex();
    let peer_owner_hex = peer_owner.public_key().to_hex();
    let pending_inbound = engine.pending_inbound_for_test();
    let pending = pending_inbound.first().expect("pending inbound");
    assert_eq!(pending.event_id, message_event.id.to_string());
    assert!(
        pending.has_envelope,
        "pending inbound must store the parsed envelope so retries do not verify the outer event again"
    );
    assert_eq!(
        pending.sender_message_pubkey_hex.as_deref(),
        Some(sender_message_pubkey_hex.as_str())
    );
    assert_eq!(
        pending.claimed_owner_pubkey_hex.as_deref(),
        Some(peer_owner_hex.as_str())
    );
    assert!(
        pending.metadata_verified,
        "queued pending inbound metadata should be produced by the already verified parse"
    );
    assert_eq!(
        engine.queued_owner_claim_targets(),
        vec![format!("owner:{}", peer_owner.public_key().to_hex())]
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)]),
            12,
        )
        .expect("peer appkeys");
    assert_eq!(batch.direct_messages.len(), 1);
    assert_eq!(batch.direct_messages[0].sender, peer_owner.public_key());
    assert_eq!(
        batch.direct_messages[0].sender_device,
        Some(peer_device.public_key())
    );
    assert_eq!(batch.direct_messages[0].content, "hello-before-appkeys");
    assert_eq!(engine.debug_snapshot().pending_inbound_count, 0);
}

#[test]
fn appcore_pending_group_payload_from_claimed_device_uses_owner_after_appkeys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);

    let invite = engine.local_invite_for_test().expect("local invite");
    let (_peer_session, response) = invite
        .accept_with_owner(
            peer_device.public_key(),
            peer_device.secret_key().to_secret_bytes(),
            Some(peer_device.public_key().to_hex()),
            Some(peer_owner.public_key()),
        )
        .expect("peer accepts invite");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    let group_id = "claimed-owner-group".to_string();
    let snapshot = test_group_snapshot(
        &group_id,
        "Claimed Owner Group",
        peer_owner.public_key(),
        vec![peer_owner.public_key(), owner.public_key()],
        vec![peer_owner.public_key()],
        1,
    );
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(peer_device.public_key()),
            created_at: NdrUnixSeconds(11),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("group metadata payload");

    let outcome = engine
        .process_group_pairwise_payload(
            &payload,
            peer_device.public_key(),
            Some(peer_device.public_key()),
        )
        .expect("process group payload");
    assert!(outcome.consumed);
    assert!(outcome.events.is_empty());
    assert_eq!(
        outcome.queued_targets,
        vec![format!("owner:{}", peer_owner.public_key().to_hex())]
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_pairwise_payload_count,
        1
    );

    let batch = engine
        .ingest_app_keys_snapshot(
            peer_owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 12)]),
            12,
        )
        .expect("peer appkeys");
    let created = batch
        .group_result
        .events
        .iter()
        .find_map(|event| match event {
            GroupIncomingEvent::MetadataUpdated(snapshot) if snapshot.group_id == group_id => {
                Some(snapshot)
            }
            _ => None,
        })
        .expect("group metadata applied after owner claim verification");
    assert_eq!(
        created.created_by,
        ndr_owner_pubkey(peer_owner.public_key())
    );
    assert_eq!(
        engine.debug_snapshot().pending_group_pairwise_payload_count,
        0
    );
}

#[test]
fn queued_direct_send_schedules_fast_protocol_retry_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-direct-fast-retry", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.send_direct_message(
        &peer.public_key().to_hex(),
        "queued until app keys arrive",
        UnixSeconds(1_777_000_000),
        None,
    );

    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("queued protocol work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "queued direct work should schedule a fast retry tick, not wait for the normal liveness interval"
    );
}

#[test]
fn queued_direct_send_starts_targeted_owner_protocol_fetch() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-direct-targeted-fetch", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;
    observe_current_device_appkeys_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &owner,
        &device,
    );

    core.send_direct_message(
        &peer.public_key().to_hex(),
        "queued until targeted app keys arrive",
        UnixSeconds(1_777_000_000),
        None,
    );

    let target = format!("owner:{}", peer.public_key().to_hex());
    assert!(
        core.debug_log.iter().any(|entry| {
            entry.category == "appcore.protocol.queued" && entry.detail.contains(&target)
        }) && core.debug_log.iter().any(|entry| {
            entry.category == "protocol.engine_fetch.fetch" && entry.detail.contains("filters=1")
        }),
        "queued direct owner work should start a narrow AppKeys fetch for {target}"
    );
}

#[test]
fn retry_batch_coalesces_duplicate_queued_protocol_fetches() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-retry-coalesce-fetches", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    let target = format!("owner:{}", peer.public_key().to_hex());
    let filters = vec![Filter::new()
        .author(peer.public_key())
        .kind(Kind::Custom(APP_KEYS_EVENT_KIND as u16))];
    let result = ProtocolRetryResult {
        message_id: "message-1".to_string(),
        chat_id: peer.public_key().to_hex(),
        effects: vec![ProtocolEffect::FetchProtocolState {
            filters,
            reason: "retry",
        }],
        queued_targets: vec![target.clone()],
        ..ProtocolRetryResult::default()
    };

    core.process_protocol_engine_retry_batch(
        "test_retry_dedupe",
        ProtocolRetryBatch {
            direct_results: vec![result.clone(), result],
            ..ProtocolRetryBatch::default()
        },
    );

    let retry_log = core
        .debug_log
        .iter()
        .find(|entry| entry.category == "appcore.protocol.retry")
        .expect("retry log");
    assert!(
        retry_log.detail.contains("queued_targets=1"),
        "retry log should count unique queued protocol targets: {}",
        retry_log.detail
    );
    let queued_log = core
        .debug_log
        .iter()
        .find(|entry| entry.category == "appcore.protocol.queued")
        .expect("queued log");
    assert_eq!(
        queued_log.detail.matches(&target).count(),
        1,
        "queued log should list each target once: {}",
        queued_log.detail
    );
    assert_eq!(
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "protocol.engine_fetch.fetch")
            .count(),
        1,
        "duplicate retry effects should schedule one targeted fetch"
    );
}

#[test]
fn queued_protocol_filters_are_narrow_for_missing_owner_roster() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    let result = engine
        .send_direct_text(
            peer.public_key(),
            &peer.public_key().to_hex(),
            "queued until appkeys",
            None,
            UnixSeconds(1_777_159_500),
        )
        .expect("direct send");
    let filters = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::FetchProtocolState { filters, .. } => Some(filters.clone()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(filters.len(), 1);
    assert!(has_filter_with_kind_author(
        &filters,
        APP_KEYS_EVENT_KIND,
        peer.public_key()
    ));
    assert!(
        !has_bootstrap_message_filter(&filters),
        "queued owner discovery must not depend on an unscoped message bootstrap filter"
    );
}

#[test]
fn queued_group_create_schedules_fast_protocol_retry_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-fast-retry", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.create_group("Queued group", &[peer.public_key().to_hex()]);

    let debug = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert!(
        debug.pending_group_fanout_count > 0,
        "missing group member protocol state should leave a durable group fanout"
    );
    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("queued group work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "queued group work should schedule a fast retry tick, not wait for the normal liveness interval"
    );
}

#[test]
fn current_queued_protocol_targets_includes_group_fanout_targets() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-targets", &owner, &device);

    core.create_group("Queued group", &[peer.public_key().to_hex()]);

    let targets = core.current_queued_protocol_targets();
    assert!(
        targets.contains(&peer.public_key().to_hex()),
        "queued protocol targets should include lightweight group fanout targets"
    );
}

#[test]
fn queued_group_retry_without_protocol_progress_reschedules_fast_tick() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("queued-group-retry-reschedule", &owner, &device);
    let relay_urls = relay_urls_from_strings(&["wss://relay.invalid".to_string()]);
    core.preferences.nostr_relay_urls = vec!["wss://relay.invalid".to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.create_group("Queued group", &[peer.public_key().to_hex()]);
    core.protocol_subscription_runtime.liveness_due_at = None;

    let retry_at = unix_now().get().saturating_add(10_000);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .retry_pending_protocol(NdrUnixSeconds(retry_at))
        .expect("retry pending protocol");
    assert!(
        !batch.group_result.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::PublishSigned(_)
                | ProtocolEffect::PublishUnsigned(_)
                | ProtocolEffect::PublishSignedForInnerEvent { .. }
                | ProtocolEffect::PublishStagedFirstContact { .. }
        )),
        "missing member protocol state should not produce group publishes yet"
    );
    assert!(
        !batch.group_result.queued_targets.is_empty(),
        "the retry batch must report the still-queued group target"
    );

    core.process_protocol_engine_retry_batch("test_group_retry", batch);

    let due_at = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("still-queued group work should schedule liveness");
    assert!(
        due_at <= Instant::now() + Duration::from_secs(5),
        "still-queued group work should keep a fast retry tick alive"
    );
}

#[test]
fn appcore_protocol_engine_partial_fanout_publishes_ready_device_and_queues_missing_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_phone = Keys::generate();
    let peer_laptop = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let peer_app_keys = AppKeys::new(vec![
        DeviceEntry::new(peer_phone.public_key(), 1),
        DeviceEntry::new(peer_laptop.public_key(), 1),
    ]);
    engine
        .ingest_app_keys_snapshot(peer_owner.public_key(), peer_app_keys, 1)
        .expect("peer appkeys");

    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(2), &mut rng);
    let phone_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_phone.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("phone invite");
    let phone_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&phone_invite)
        .expect("invite event")
        .sign_with_keys(&peer_phone)
        .expect("signed invite");
    engine
        .observe_invite_event(&phone_invite_event)
        .expect("observe phone invite");

    let result = engine
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "hello",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    assert_eq!(result.event_ids.len(), 1);
    assert!(
        result
            .queued_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "missing peer laptop should remain queued"
    );
    let staged = result
        .effects
        .iter()
        .find_map(|effect| match effect {
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => {
                Some((bootstrap, payload))
            }
            _ => None,
        })
        .expect("first contact should stage bootstrap before payload");
    assert!(
        staged
            .0
            .iter()
            .any(|publish| publish.event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND),
        "bootstrap phase should contain the invite response"
    );
    assert_eq!(
        staged.0[0].inner_event_id.as_deref(),
        Some(result.message_id.as_str()),
        "bootstrap publish must be tied to the app message so payload can wait on it"
    );
    assert_eq!(
        staged.0[0].target_owner_pubkey_hex.as_deref(),
        Some(peer_owner.public_key().to_hex().as_str())
    );
    assert_eq!(
        staged.1.len(),
        1,
        "payload phase should contain the ready phone delivery"
    );
    assert_eq!(
        staged.1[0].target_owner_pubkey_hex.as_deref(),
        Some(peer_owner.public_key().to_hex().as_str())
    );

    let mut ctx = ProtocolContext::new(NdrUnixSeconds(120), &mut rng);
    let laptop_invite = Invite::create_new_with_context(
        &mut ctx,
        NdrDevicePubkey::from_bytes(peer_laptop.public_key().to_bytes()),
        Some(NdrOwnerPubkey::from_bytes(
            peer_owner.public_key().to_bytes(),
        )),
        None,
    )
    .expect("laptop invite");
    let laptop_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&laptop_invite)
        .expect("invite event")
        .sign_with_keys(&peer_laptop)
        .expect("signed invite");
    let batch = engine
        .observe_invite_event(&laptop_invite_event)
        .expect("observe laptop invite");

    assert_eq!(batch.direct_results.len(), 1);
    let retry = &batch.direct_results[0];
    assert_eq!(retry.message_id, result.message_id);
    assert_eq!(retry.event_ids.len(), 1);
    assert!(
        !retry
            .queued_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "all remote devices should be prepared after the missing invite arrives"
    );
    assert!(
        !engine
            .debug_snapshot()
            .pending_outbound_targets
            .contains(&peer_laptop.public_key().to_hex()),
        "remote peer fanout should be fully drained"
    );
}

#[test]
fn appcore_message_author_tracking_includes_current_next_and_skipped_sender_keys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let current_sender = Keys::generate();
    let next_sender = Keys::generate();
    let skipped_sender = Keys::generate();
    let our_current = Keys::generate();
    let our_next = Keys::generate();
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_device = NdrDevicePubkey::from_bytes(device.public_key().to_bytes());
    let peer_owner_pubkey = NdrOwnerPubkey::from_bytes(peer_owner.public_key().to_bytes());
    let peer_device_pubkey = NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes());
    let current_sender_pubkey = NdrDevicePubkey::from_bytes(current_sender.public_key().to_bytes());
    let next_sender_pubkey = NdrDevicePubkey::from_bytes(next_sender.public_key().to_bytes());
    let skipped_sender_pubkey = NdrDevicePubkey::from_bytes(skipped_sender.public_key().to_bytes());
    let mut skipped_keys = BTreeMap::new();
    skipped_keys.insert(
        skipped_sender_pubkey,
        nostr_double_ratchet::SkippedKeysEntry::default(),
    );
    let session_state = SessionState {
        root_key: [1; 32],
        their_current_nostr_public_key: Some(current_sender_pubkey),
        their_next_nostr_public_key: Some(next_sender_pubkey),
        our_previous_nostr_key: None,
        our_current_nostr_key: Some(serializable_key_pair_for_test(&our_current)),
        our_next_nostr_key: serializable_key_pair_for_test(&our_next),
        receiving_chain_key: Some([2; 32]),
        sending_chain_key: Some([3; 32]),
        sending_chain_message_number: 0,
        receiving_chain_message_number: 0,
        previous_sending_chain_message_count: 0,
        skipped_keys,
    };
    let seed_session_manager = SessionManagerSnapshot {
        local_owner_pubkey: local_owner,
        local_device_pubkey: local_device,
        local_invite: None,
        users: vec![nostr_double_ratchet::UserRecordSnapshot {
            owner_pubkey: peer_owner_pubkey,
            roster: Some(DeviceRoster::new(
                NdrUnixSeconds(1),
                vec![AuthorizedDevice::new(peer_device_pubkey, NdrUnixSeconds(1))],
            )),
            devices: vec![nostr_double_ratchet::DeviceRecordSnapshot {
                device_pubkey: peer_device_pubkey,
                authorized: true,
                is_stale: false,
                stale_since: None,
                claimed_owner_pubkey: Some(peer_owner_pubkey),
                public_invite: None,
                invite_response_generated: true,
                active_session: Some(session_state),
                inactive_sessions: Vec::new(),
                last_activity: Some(NdrUnixSeconds(1)),
                created_at: NdrUnixSeconds(1),
            }],
        }],
    };
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let local_invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    let engine = ProtocolEngine::load_or_seed(
        storage,
        owner.public_key(),
        &device,
        local_invite,
        seed_session_manager,
        NostrGroupManager::new(local_owner).snapshot(),
    )
    .expect("protocol engine");

    let authors = engine.message_author_pubkeys_for_owner(peer_owner.public_key());
    assert!(
        authors.contains(&current_sender.public_key()),
        "current sender author must stay subscribed"
    );
    assert!(
        authors.contains(&next_sender.public_key()),
        "next sender author must be subscribed so the next ratchet event is not missed"
    );
    assert!(
        authors.contains(&skipped_sender.public_key()),
        "skipped sender author must be backfilled for out-of-order relay delivery"
    );
}

#[test]
fn local_sibling_direct_send_uses_author_known_before_publish() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let peer_owner = Keys::generate();
    let mut primary = test_protocol_engine(&owner, &primary_device);
    let mut linked = test_protocol_engine(&owner, &linked_device);

    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    primary
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
        .expect("primary local appkeys");
    linked
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("linked local appkeys");

    let linked_invite = linked.local_invite_for_test().expect("linked invite");
    let linked_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&linked_invite)
        .expect("linked invite event")
        .sign_with_keys(&linked_device)
        .expect("signed linked invite");
    primary
        .observe_invite_event(&linked_invite_event)
        .expect("primary observes linked invite");

    let (session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response)
        .expect("invite response event");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &response_event,
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");

    let known_authors_before = linked.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        !known_authors_before.is_empty(),
        "linked device must know at least one primary sender author after link setup"
    );

    let result = primary
        .send_direct_text(
            peer_owner.public_key(),
            &peer_owner.public_key().to_hex(),
            "sender copy should be immediately discoverable",
            None,
            UnixSeconds(3),
        )
        .expect("direct send");

    let local_sibling_events = result
        .effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                event,
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref()
                == Some(owner.public_key().to_hex().as_str())
                && target_device_id.as_deref()
                    == Some(linked_device.public_key().to_hex().as_str()) =>
            {
                Some(event)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        local_sibling_events.len(),
        1,
        "direct send should prepare one sender-copy event for the linked device"
    );
    assert!(
        known_authors_before.contains(&local_sibling_events[0].pubkey),
        "sender-copy event author {} must already be in the linked device's message subscriptions; known={:?}",
        local_sibling_events[0].pubkey.to_hex(),
        known_authors_before
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>()
    );
    assert!(
        !result.effects.iter().any(|effect| {
            matches!(
                effect,
                ProtocolEffect::PublishSigned(event)
                    if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND
            )
        }),
        "ordinary direct sender-copy fanout must not refresh the linked-device bootstrap session"
    );
}

#[test]
fn local_sibling_group_send_bootstrap_makes_staged_payload_author_fetchable() {
    let owner = Keys::generate();
    let primary_device = Keys::generate();
    let linked_device = Keys::generate();
    let admin_owner = Keys::generate();
    let admin_device = Keys::generate();
    let mut primary = test_protocol_engine(&owner, &primary_device);
    let mut linked = test_protocol_engine(&owner, &linked_device);

    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    primary
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys.clone(), 1)
        .expect("primary local appkeys");
    linked
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("linked local appkeys");

    let linked_invite = linked.local_invite_for_test().expect("linked invite");
    let (primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    primary
        .import_session_state(
            owner.public_key(),
            Some(linked_device.public_key().to_hex()),
            primary_session.state,
            UnixSeconds(2),
        )
        .expect("primary imports linked session");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet_nostr::invite_response_event(&response)
            .expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    linked
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");
    let mut primary_invite = primary
        .local_invite_for_test()
        .expect("primary invite for linked sibling");
    primary_invite.owner_public_key = Some(owner.public_key());
    primary_invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let primary_invite_event = nostr_double_ratchet_nostr::invite_unsigned_event(&primary_invite)
        .expect("primary invite unsigned")
        .sign_with_keys(&primary_device)
        .expect("primary invite event");
    linked
        .observe_invite_event(&primary_invite_event)
        .expect("linked observes primary invite");

    let admin_app_keys = AppKeys::new(vec![DeviceEntry::new(admin_device.public_key(), 1)]);
    primary
        .ingest_app_keys_snapshot(admin_owner.public_key(), admin_app_keys.clone(), 1)
        .expect("primary admin appkeys");
    linked
        .ingest_app_keys_snapshot(admin_owner.public_key(), admin_app_keys, 1)
        .expect("linked admin appkeys");

    let group_id = "linked-sibling-group".to_string();
    let mut snapshot = test_group_snapshot(
        &group_id,
        "Linked Sibling Group",
        admin_owner.public_key(),
        vec![admin_owner.public_key(), owner.public_key()],
        vec![admin_owner.public_key()],
        1,
    );
    snapshot.protocol = nostr_double_ratchet::GroupProtocol::PairwiseFanoutV1;
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(admin_device.public_key()),
            created_at: NdrUnixSeconds(11),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("metadata payload");
    primary
        .process_group_pairwise_payload(
            &metadata_payload,
            admin_owner.public_key(),
            Some(admin_device.public_key()),
        )
        .expect("primary processes group metadata");
    linked
        .process_group_pairwise_payload(
            &metadata_payload,
            admin_owner.public_key(),
            Some(admin_device.public_key()),
        )
        .expect("linked processes group metadata");

    let known_primary_authors = primary.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        !known_primary_authors.is_empty(),
        "primary must know linked-device message authors after sibling setup"
    );

    let result = linked
        .send_group_payload(
            &group_id,
            b"linked sibling group body".to_vec(),
            Some("linked-group-inner".to_string()),
        )
        .expect("linked group send");
    let target_owner_hex = owner.public_key().to_hex();
    let target_device_hex = primary_device.public_key().to_hex();
    let local_sibling_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                event,
                target_owner_pubkey_hex,
                target_device_id,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                && target_device_id.as_deref() == Some(target_device_hex.as_str()) =>
            {
                vec![event.clone()]
            }
            ProtocolEffect::PublishStagedFirstContact { payload, .. } => payload
                .iter()
                .filter(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                        && publish.target_device_id.as_deref() == Some(target_device_hex.as_str())
                })
                .map(|publish| publish.event.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    let local_sibling_bootstrap_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload }
                if payload.iter().any(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(target_owner_hex.as_str())
                        && publish.target_device_id.as_deref() == Some(target_device_hex.as_str())
                }) =>
            {
                bootstrap
                    .iter()
                    .map(|publish| publish.event.clone())
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();

    assert!(
        !local_sibling_events.is_empty(),
        "group send should prepare a local sibling copy for the primary device; queued={:?} pending_group_fanouts={} pending_targets={:?}",
        result.queued_targets,
        linked.debug_snapshot().pending_group_fanout_count,
        linked.debug_snapshot().pending_group_fanout_targets
    );
    assert!(
        !local_sibling_bootstrap_events.is_empty(),
        "first-contact local sibling group copy should include invite-response bootstrap"
    );
    for event in &local_sibling_bootstrap_events {
        primary
            .observe_invite_response_event(event)
            .expect("primary processes linked bootstrap response");
    }
    let known_primary_authors_after_bootstrap =
        primary.message_author_pubkeys_for_owner(owner.public_key());
    assert!(
        local_sibling_events
            .iter()
            .all(|event| known_primary_authors_after_bootstrap.contains(&event.pubkey)),
        "local sibling group event authors must be known after first-contact bootstrap; before={:?} after={:?} event_authors={:?}",
        known_primary_authors
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>(),
        known_primary_authors_after_bootstrap
            .iter()
            .map(PublicKey::to_hex)
            .collect::<Vec<_>>(),
        local_sibling_events
            .iter()
            .map(|event| event.pubkey.to_hex())
            .collect::<Vec<_>>()
    );

    let mut received_messages = Vec::new();
    for event in &local_sibling_events {
        let decrypted = primary
            .process_direct_message_event(event)
            .expect("primary processes linked group copy")
            .expect("primary decrypts linked group copy");
        let outcome = primary
            .process_group_pairwise_payload(
                decrypted.content.as_bytes(),
                decrypted.sender,
                decrypted.sender_device,
            )
            .expect("primary processes group payload from linked copy");
        received_messages.extend(outcome.events.into_iter().filter_map(|event| match event {
            GroupIncomingEvent::Message(message) => Some(message),
            _ => None,
        }));
    }
    assert!(
        received_messages.iter().any(|message| {
            message.group_id == group_id
                && message.sender_owner == ndr_owner_pubkey(owner.public_key())
                && message.sender_device == Some(ndr_device_pubkey(linked_device.public_key()))
                && message.body == b"linked sibling group body".to_vec()
        }),
        "primary should apply linked-device group copy as an owner-authored message"
    );
}

#[test]
fn local_sibling_publish_ack_does_not_mark_peer_recipient_sent() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("local-sibling-ack-direct-delivery", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "direct-first".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "first".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );

    let local_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "local sibling")
        .sign_with_keys(&device)
        .expect("local sibling event");
    let local_event_id = local_event.id.to_string();
    core.pending_relay_publishes.insert(
        local_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: local_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&local_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(owner.public_key().to_hex()),
            target_device_id: Some(sibling.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: local_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );
    core.handle_relay_publish_finished(
        local_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "local sibling ack".to_string(),
    );

    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after local ack");
    assert_eq!(message.delivery, DeliveryState::Pending);
    assert_eq!(message.recipient_deliveries.len(), 1);
    assert_eq!(
        message.recipient_deliveries[0].owner_pubkey_hex,
        peer.public_key().to_hex()
    );
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Pending
    );

    let lingering_local_event =
        EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "local still pending")
            .sign_with_keys(&device)
            .expect("lingering local event");
    let lingering_local_event_id = lingering_local_event.id.to_string();
    core.pending_relay_publishes.insert(
        lingering_local_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: lingering_local_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&lingering_local_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(owner.public_key().to_hex()),
            target_device_id: Some(sibling.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: lingering_local_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let peer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "peer")
        .sign_with_keys(&device)
        .expect("peer event");
    let peer_event_id = peer_event.id.to_string();
    core.pending_relay_publishes.insert(
        peer_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: peer_event_id.clone(),
            label: "test".to_string(),
            event_json: serde_json::to_string(&peer_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: Some(peer.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: peer_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );
    core.handle_relay_publish_finished(
        peer_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "peer ack".to_string(),
    );

    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after peer ack");
    assert!(
        core.pending_relay_publishes
            .contains_key(&lingering_local_event_id),
        "local sibling pending relay work should not decide peer recipient delivery"
    );
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Sent
    );
}

#[test]
fn first_contact_payload_waits_for_bootstrap_publish_success() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("first-contact-bootstrap-gates-payload", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "direct-first-contact".to_string();
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &chat_id,
        "first".to_string(),
        1_777_159_500,
        None,
        DeliveryState::Pending,
    );

    let bootstrap_event = EventBuilder::new(
        Kind::from(INVITE_RESPONSE_KIND as u16),
        "first contact bootstrap",
    )
    .sign_with_keys(&device)
    .expect("bootstrap event");
    let bootstrap_event_id = bootstrap_event.id.to_string();
    core.pending_relay_publishes.insert(
        bootstrap_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: bootstrap_event_id.clone(),
            label: APPCORE_PROTOCOL_BOOTSTRAP_LABEL.to_string(),
            event_json: serde_json::to_string(&bootstrap_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: None,
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: bootstrap_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let payload_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "payload")
        .sign_with_keys(&device)
        .expect("payload event");
    let payload_event_id = payload_event.id.to_string();
    core.pending_relay_publishes.insert(
        payload_event_id.clone(),
        PendingRelayPublish {
            owner_pubkey_hex: owner.public_key().to_hex(),
            event_id: payload_event_id.clone(),
            label: APPCORE_PROTOCOL_FIRST_CONTACT_LABEL.to_string(),
            event_json: serde_json::to_string(&payload_event).expect("event json"),
            inner_event_id: Some(message_id.clone()),
            target_owner_pubkey_hex: Some(peer.public_key().to_hex()),
            target_device_id: Some(peer.public_key().to_hex()),
            message_id: Some(message_id.clone()),
            chat_id: Some(chat_id.clone()),
            created_at_secs: payload_event.created_at.as_secs(),
            attempt_count: 0,
            last_error: None,
        },
    );

    let payload_pending = core
        .pending_relay_publishes
        .get(&payload_event_id)
        .expect("payload pending");
    assert!(
        core.should_delay_first_contact_payload_publish(payload_pending),
        "payload must not publish while its invite-response bootstrap is still pending"
    );

    core.handle_relay_publish_finished(
        bootstrap_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "bootstrap ack".to_string(),
    );
    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after bootstrap ack");
    assert_eq!(
        message.delivery,
        DeliveryState::Pending,
        "bootstrap relay ack must not mark the peer message sent"
    );
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Pending
    );
    let payload_pending = core
        .pending_relay_publishes
        .get(&payload_event_id)
        .expect("payload still pending after bootstrap");
    assert!(
        !core.should_delay_first_contact_payload_publish(payload_pending),
        "payload may publish after bootstrap succeeds"
    );

    core.handle_relay_publish_finished(
        payload_event_id,
        Some(message_id.clone()),
        Some(chat_id.clone()),
        true,
        vec!["wss://relay.example".to_string()],
        "payload ack".to_string(),
    );
    let message = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("message after payload ack");
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(
        message.recipient_deliveries[0].delivery,
        DeliveryState::Sent
    );
}

#[test]
fn appcore_hot_path_has_no_runtime_references() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![manifest.join("src/core.rs")];
    collect_core_rs_files(&manifest.join("src/core"), &mut files);

    let forbidden = [
        "NdrRuntime",
        "ndr_runtime",
        "setup_user",
        "process_runtime_effects",
        "RuntimeEffect",
    ];
    let mut hits = Vec::new();
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) == Some("tests.rs") {
            continue;
        }
        let content = std::fs::read_to_string(&path).expect("read core source");
        for needle in forbidden {
            if content.contains(needle) {
                hits.push(format!("{} contains {needle}", path.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "AppCore hot-path runtime references remain:\n{}",
        hits.join("\n")
    );
}

fn collect_core_rs_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read core dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_core_rs_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

#[test]
fn ndr_runtime_invite_session_round_trips_text() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        None,
        None,
    );
    bob.init().expect("bob init");
    accept_invite_and_deliver(&bob, &bob_keys, &invite, alice_keys.public_key(), &alice);
    complete_first_contact(&bob, &bob_keys, alice_keys.public_key(), &alice);

    alice
        .send_text(bob_keys.public_key(), "hello bob".to_string(), None)
        .expect("alice sends");
    deliver_published_events(&alice, &alice_keys, &bob);
    assert!(drain_text_messages(&bob)
        .iter()
        .any(|message| message == "hello bob"));

    bob.send_text(alice_keys.public_key(), "hello alice".to_string(), None)
        .expect("bob sends");
    deliver_published_events(&bob, &bob_keys, &alice);
    assert!(drain_text_messages(&alice)
        .iter()
        .any(|message| message == "hello alice"));
}

#[test]
fn local_identity_artifacts_offer_profile_metadata_to_nearby() {
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
    let device_id = device.public_key().to_hex();
    let invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None).expect("invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.owner_profiles.insert(
        owner.public_key().to_hex(),
        OwnerProfileRecord {
            name: None,
            display_name: None,
            picture: Some("htree://profile-picture".to_string()),
            updated_at_secs: 1,
        },
    );

    core.publish_local_identity_artifacts();

    let nearby_kinds = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent { kind, .. } => Some(kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        nearby_kinds.contains(&0),
        "profile metadata should be included in nearby inventory; got {nearby_kinds:?}"
    );
}

#[test]
fn create_account_without_name_still_offers_profile_metadata_to_nearby() {
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.create_account("");

    let account = core.state.account.as_ref().expect("account");
    let profile = core
        .owner_profiles
        .get(&account.public_key_hex)
        .expect("local profile record");
    assert_eq!(
        profile.display_name.as_deref(),
        Some(account.display_name.as_str())
    );

    let profile_event_json = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent {
                kind: 0,
                event_json,
                ..
            } => Some(event_json),
            _ => None,
        })
        .last()
        .expect("profile event");
    let profile_event: Event = serde_json::from_str(&profile_event_json).expect("profile event");
    assert_eq!(profile_event.pubkey.to_hex(), account.public_key_hex);
}

#[test]
fn nearby_presence_event_binds_owner_to_ble_nonce_pair() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite =
        Invite::create_new(device.public_key(), Some(device_id.clone()), None).expect("invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });

    let event_json = core.build_nearby_presence_event_json(
        "peer-a",
        "nonce-a",
        "nonce-b",
        "f".repeat(64).as_str(),
    );
    let event: Event = serde_json::from_str(&event_json).expect("presence event");
    assert_eq!(event.kind.as_u16(), NEARBY_PRESENCE_KIND);
    event.verify().expect("valid signature");
    assert_eq!(event.pubkey, owner.public_key());

    let content: serde_json::Value = serde_json::from_str(&event.content).expect("content");
    assert_eq!(content["protocol"], "iris-nearby-v1");
    assert_eq!(content["peer_id"], "peer-a");
    assert_eq!(content["my_nonce"], "nonce-a");
    assert_eq!(content["their_nonce"], "nonce-b");
    assert_eq!(content["profile_event_id"], "f".repeat(64));

    let verified =
        crate::verify_nearby_presence_event_json(&event_json, "peer-a", "nonce-b", "nonce-a");
    let verified: serde_json::Value = serde_json::from_str(&verified).expect("verified presence");
    assert_eq!(verified["owner_pubkey_hex"], owner.public_key().to_hex());
    assert_eq!(verified["profile_event_id"], "f".repeat(64));

    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "peer-a",
        "wrong-receiver-nonce",
        "nonce-a",
    )
    .is_empty());
    assert!(crate::verify_nearby_presence_event_json(
        &event_json,
        "wrong-peer",
        "nonce-b",
        "nonce-a",
    )
    .is_empty());
}

#[test]
fn mobile_push_decrypt_preview_does_not_mutate_persisted_ratchet_state() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_engine =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_storage.clone());
    let message = "closed-app preview stays read-only";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    let state_key = "appcore/protocol-engine-state-v1";
    let before = bob_storage
        .get(state_key)
        .expect("read stored appcore state before notification")
        .expect("stored appcore state before notification");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&message_event).expect("outer event json"),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();
    let apns_payload = serde_json::json!({
        "event": message_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, message);
    let apns_resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        apns_payload,
    );
    assert!(apns_resolution.should_show);
    assert_eq!(apns_resolution.body, message);

    let after = bob_storage
        .get(state_key)
        .expect("read stored appcore state after notification")
        .expect("stored appcore state after notification");
    assert_eq!(
        before, after,
        "notification preview must not advance persisted protocol state"
    );

    let bob_restarted_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("restarted db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_restarted =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_restarted_storage);
    let decrypted = bob_restarted
        .process_direct_message_event(&message_event)
        .expect("foreground protocol decrypt")
        .expect("foreground decrypted message");
    let runtime_rumor = parse_runtime_rumor(&decrypted.content).expect("runtime rumor");
    assert_eq!(runtime_rumor.content, message);
}

#[test]
fn mobile_push_decrypts_compacted_apns_event_payload() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let mut bob_engine = test_protocol_engine_with_storage(&bob_keys, &bob_keys, bob_storage);
    let message = "compacted apns preview";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    for (key, event_payload) in [
        ("event", compact_event_payload_for_apns_test(&message_event)),
        (
            "outer_event",
            compact_event_payload_for_apns_test(&message_event),
        ),
        (
            "outer_event_json",
            serde_json::Value::String(
                serde_json::to_string(&message_event).expect("outer event json"),
            ),
        ),
        (
            "nostr_event_json",
            serde_json::Value::String(
                serde_json::to_string(&message_event).expect("nostr event json"),
            ),
        ),
    ] {
        let mut payload = serde_json::json!({
            "aps": {
                "alert": {
                    "title": "Iris Chat",
                    "body": "New message",
                },
                "mutable-content": 1,
            },
            "title": "New message",
            "body": "New message",
        });
        payload[key] = event_payload;

        let resolution = decrypt_mobile_push_notification(
            data_dir.to_string_lossy().to_string(),
            bob_keys.public_key().to_hex(),
            bob_keys
                .secret_key()
                .to_bech32()
                .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
            payload.to_string(),
        );

        assert!(
            resolution.should_show,
            "{key} payload should decrypt to a visible message"
        );
        assert_eq!(resolution.body, message, "{key} payload body");
    }
}

#[test]
fn mobile_push_payload_ingest_feeds_full_event_into_runtime() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let bob_storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    let mut bob_engine =
        test_protocol_engine_with_storage(&bob_keys, &bob_keys, Arc::clone(&bob_storage));
    let message = "push-only event";
    let message_event =
        appcore_direct_message_event_for_test(&mut bob_engine, &alice_keys, message, 200);
    let payload = serde_json::json!({
        "event": message_event.clone(),
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let local_invite = Invite::create_new(
        bob_keys.public_key(),
        Some(bob_keys.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload.clone(),
    });
    let chat_id = alice_keys.public_key().to_hex();
    assert!(
        !core.threads.contains_key(&chat_id),
        "push event waits for session restore"
    );

    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        local_invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    install_test_protocol_engine(&mut core, &bob_keys, &bob_keys, bob_storage, None, None);

    core.drain_pending_mobile_push_events();
    let thread = core.threads.get(&chat_id).expect("sender thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, message);
    let event_id = message_event.id.to_string();
    assert_eq!(
        thread.messages[0].source_event_id.as_deref(),
        Some(event_id.as_str())
    );

    core.handle_action(AppAction::IngestMobilePushPayload {
        payload_json: payload,
    });
    let thread = core.threads.get(&chat_id).expect("sender thread after dup");
    assert_eq!(thread.messages.len(), 1, "duplicate push event is ignored");
}

#[test]
fn mobile_push_decrypt_suppresses_typing_rumors() {
    // Even with valid keys and a working ratchet, the notification
    // extension should suppress non-chat-message rumors. Typing/seen/
    // reaction wrappers are noise as standalone notifications — the
    // chat list updates when the user opens the app.
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("bob db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    invite.owner_public_key = Some(alice_keys.public_key());

    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(invite.clone()),
    );
    alice.init().expect("alice init");

    let bob = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_storage),
        None,
    );
    bob.init().expect("bob init");
    accept_invite_and_deliver(&bob, &bob_keys, &invite, alice_keys.public_key(), &alice);
    complete_first_contact(&bob, &bob_keys, alice_keys.public_key(), &alice);

    alice
        .send_typing(bob_keys.public_key(), None)
        .expect("alice sends typing");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let typing_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("typing event for Bob");
    let payload = serde_json::json!({
        "event": typing_event,
        "title": "New message",
        "body": "New activity",
    })
    .to_string();

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );

    assert!(
        !resolution.should_show,
        "typing rumors must not surface as standalone notifications"
    );
}

#[test]
fn mobile_push_fallback_suppresses_opaque_encrypted_events() {
    let encrypted_outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&Keys::generate())
        .expect("outer event");
    let payload = serde_json::json!({
        "event": encrypted_outer_event,
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert!(resolution.title.is_empty());
    assert!(resolution.body.is_empty());
}

#[test]
fn mobile_push_fallback_suppresses_opaque_encrypted_alias_events_with_string_kind() {
    let encrypted_outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&Keys::generate())
        .expect("outer event");
    let mut event_json =
        serde_json::to_value(&encrypted_outer_event).expect("outer event json value");
    event_json["kind"] = serde_json::Value::String(MESSAGE_EVENT_KIND.to_string());
    let payload = serde_json::json!({
        "outer_event_json": event_json.to_string(),
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(!resolution.should_show);
    assert!(resolution.title.is_empty());
    assert!(resolution.body.is_empty());
}

#[test]
fn mobile_push_preview_resolves_from_sqlite_when_decrypt_fails() {
    // Simulates the "main app already consumed this event, ratchet has
    // moved on, push extension can't decrypt anymore" race: the
    // foreground app stored a message keyed by the outer event id; the
    // extension can't decrypt the same event, so it falls back to the
    // body/sender already on disk.
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    // Pretend Alice's profile is already known.
    let alice = Keys::generate();
    core.owner_profiles.insert(
        alice.public_key().to_hex(),
        OwnerProfileRecord {
            name: Some("alice".to_string()),
            display_name: Some("Alice from work".to_string()),
            picture: None,
            updated_at_secs: 1,
        },
    );
    let outer_event_id = "a".repeat(64);
    let chat_id = alice.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: 200,
            messages: vec![ChatMessageSnapshot {
                id: "rumor-1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: alice.public_key().to_hex(),
                body: "lunch?".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 199,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: Some(outer_event_id.clone()),
            }],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();

    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&alice)
        .expect("outer event");
    let payload = serde_json::json!({
        "event": serde_json::json!({
            "id": outer_event_id,
            "kind": MESSAGE_EVENT_KIND,
            "content": "",
            "tags": [],
            "pubkey": alice.public_key().to_hex(),
            "created_at": 200,
            "sig": outer_event.sig.to_string(),
        }),
        "title": "DM by Someone",
        "body": "New message",
    })
    .to_string();

    // Use bogus device keys so NDR decrypt fails immediately and we
    // exercise the SQLite-backed preview path.
    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        owner.public_key().to_hex(),
        "nsec1invalid".to_string(),
        payload,
    );
    assert!(resolution.should_show);
    assert_eq!(resolution.body, "lunch?");
    assert_eq!(resolution.title, "Alice from work");
}

#[test]
fn appcore_persists_pending_group_sender_key_outer_when_no_group_message_emits() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_event = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .expect("primary session");
    let outer = unknown_group_sender_key_outer_event(&sender_event);
    let event_id = outer.id.to_string();

    core.handle_relay_event(outer);

    assert!(
        core.seen_event_ids.contains(&event_id),
        "unknown group sender-key outer should be consumed by group runtime instead of falling through to pairwise decrypt"
    );
    assert_eq!(
        stored_pending_group_sender_key_message_count(&core, &owner, &device),
        1,
        "group runtime pending outer must be durably stored even when no app-visible group message is emitted yet"
    );
}

#[test]
fn appcore_defers_decrypted_delivery_ack_until_app_state_is_persisted() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir.to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let bob_local_invite = Invite::create_new(
        bob_keys.public_key(),
        Some(bob_keys.public_key().to_hex()),
        None,
    )
    .expect("bob local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: bob_keys.public_key(),
        owner_keys: Some(bob_keys.clone()),
        device_keys: bob_keys.clone(),
        client: Client::new(bob_keys.clone()),
        relay_urls: Vec::new(),
        local_invite: bob_local_invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, &bob_keys, &bob_keys, storage, None, None);

    let message = "ack after app persist";
    let message_event = appcore_direct_message_event_for_test(
        core.protocol_engine.as_mut().expect("protocol engine"),
        &alice_keys,
        message,
        200,
    );

    core.enter_batch();
    core.handle_relay_event(message_event);

    assert!(
        core.threads
            .get(&alice_keys.public_key().to_hex())
            .is_some_and(|thread| thread
                .messages
                .iter()
                .any(|message| message.body == "ack after app persist")),
        "decrypted message should be applied in memory immediately"
    );
    assert_eq!(
        stored_message_count(&core),
        1,
        "notification-preview durability may write the message row immediately, but full app-state persistence is still batch-deferred"
    );
    assert_eq!(
        stored_pending_decrypted_delivery_count(&core, &bob_keys, &bob_keys),
        1,
        "runtime decrypted delivery must remain pending until app state is durably saved"
    );

    core.exit_batch();

    assert_eq!(stored_message_count(&core), 1);
    assert_eq!(
        stored_pending_decrypted_delivery_count(&core, &bob_keys, &bob_keys),
        0,
        "persisting app state should ack and clear the runtime decrypted delivery"
    );
}

#[test]
fn mobile_push_fallback_suppresses_decrypted_non_message_kinds() {
    for kind in [
        0_u64,
        TYPING_KIND as u64,
        RECEIPT_KIND as u64,
        40_u64,
        CHAT_SETTINGS_KIND as u64,
        12345,
    ] {
        let inner_event = serde_json::json!({
            "kind": kind,
            "content": "not a chat message",
            "created_at": 1_777_159_483u64,
            "tags": [],
            "pubkey": "0".repeat(64),
            "id": format!("{kind:064x}"),
        });
        let payload = serde_json::json!({
            "inner_event_json": inner_event.to_string(),
            "title": "Iris Chat",
            "body": "New message",
        })
        .to_string();

        let resolution = resolve_mobile_push_notification(payload);

        assert!(
            !resolution.should_show,
            "inner kind {kind} should not produce a visible mobile push"
        );
    }
}

#[test]
fn mobile_push_fallback_allows_chat_message_kind() {
    let payload = serde_json::json!({
        "inner_kind": CHAT_MESSAGE_KIND.to_string(),
        "title": "Alice",
        "body": "hello",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Alice");
    assert_eq!(resolution.body, "hello");
}

#[test]
fn mobile_push_fallback_renders_invite_acceptance() {
    let keys = Keys::generate();
    let p_tag_pubkey = "a".repeat(64);
    let event = EventBuilder::new(Kind::from(INVITE_RESPONSE_KIND as u16), "ciphertext")
        .tag(nostr::Tag::parse(["p", p_tag_pubkey.as_str()]).expect("p tag"))
        .sign_with_keys(&keys)
        .expect("invite response event");
    let payload = serde_json::json!({
        "event": serde_json::to_string(&event).expect("event json"),
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    let resolution = resolve_mobile_push_notification(payload);

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Invite accepted");
    assert_eq!(resolution.body, "Someone joined your chat");
}

#[test]
fn mobile_push_subscription_body_includes_invite_response_filter() {
    let owner = Keys::generate();
    let author = Keys::generate().public_key().to_hex();
    let invite_response_pubkey = Keys::generate().public_key().to_hex();
    let request = build_mobile_push_create_subscription_request(
        owner
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| owner.secret_key().to_secret_hex()),
        "ios".to_string(),
        "apns-token".to_string(),
        Some("to.iris.chat".to_string()),
        vec![author.clone()],
        vec![invite_response_pubkey.clone()],
        true,
        None,
    )
    .expect("subscription request");
    let body: serde_json::Value =
        serde_json::from_str(request.body_json.as_deref().expect("body json")).expect("json");

    assert_eq!(body["filter"]["authors"][0].as_str(), Some(author.as_str()));
    assert_eq!(
        body["filters"][1]["kinds"][0],
        serde_json::json!(INVITE_RESPONSE_KIND)
    );
    assert_eq!(
        body["filters"][1]["#p"][0].as_str(),
        Some(invite_response_pubkey.as_str())
    );
}

#[test]
fn mobile_push_snapshot_tracks_local_invite_when_enabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("mobile-push-invite-response", &owner, &device);

    let snapshot = core.build_mobile_push_sync_snapshot();

    let invite_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_hex();
    assert_eq!(snapshot.invite_response_pubkeys, vec![invite_pubkey]);
}

#[test]
fn mobile_push_snapshot_tracks_private_invite_when_enabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-private-invite-response", &owner, &device);
    core.handle_action(AppAction::CreatePublicInvite);

    let snapshot = core.build_mobile_push_sync_snapshot();

    let local_invite_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_string();
    let private_invite_pubkey = core
        .private_chat_invites
        .values()
        .next()
        .expect("private invite")
        .inviter_ephemeral_public_key
        .to_string();

    assert!(snapshot
        .invite_response_pubkeys
        .contains(&local_invite_pubkey));
    assert!(snapshot
        .invite_response_pubkeys
        .contains(&private_invite_pubkey));
}

#[test]
fn mobile_push_snapshot_omits_local_invite_when_disabled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("mobile-push-invite-response-disabled", &owner, &device);
    core.set_invite_acceptance_notifications_enabled(false);

    let snapshot = core.build_mobile_push_sync_snapshot();

    assert!(snapshot.invite_response_pubkeys.is_empty());
}

#[test]
fn typing_indicators_default_to_opt_in() {
    assert!(!PersistedPreferences::default().send_typing_indicators);
    assert!(!AppState::empty().preferences.send_typing_indicators);
}

#[test]
fn startup_at_login_defaults_to_enabled() {
    assert!(PersistedPreferences::default().startup_at_login_enabled);
    assert!(AppState::empty().preferences.startup_at_login_enabled);
}

#[test]
fn nearby_bluetooth_defaults_to_disabled() {
    assert!(!PersistedPreferences::default().nearby_bluetooth_enabled);
    assert!(!AppState::empty().preferences.nearby_bluetooth_enabled);
}

#[test]
fn nearby_lan_defaults_to_disabled() {
    assert!(!PersistedPreferences::default().nearby_lan_enabled);
    assert!(!AppState::empty().preferences.nearby_lan_enabled);
}

#[test]
fn app_keys_device_projection_is_deterministic() {
    let owner = Keys::generate().public_key();
    let device_a = Keys::generate().public_key();
    let device_b = Keys::generate().public_key();
    let app_keys = AppKeys::new(vec![
        DeviceEntry::new(device_b, 20),
        DeviceEntry::new(device_a, 10),
    ]);

    let known = known_app_keys_from_ndr(owner, &app_keys, 30);

    assert_eq!(known.owner_pubkey_hex, owner.to_hex());
    assert_eq!(known.created_at_secs, 30);
    let mut expected_devices = vec![device_a.to_hex(), device_b.to_hex()];
    expected_devices.sort();
    assert_eq!(
        known
            .devices
            .iter()
            .map(|device| device.identity_pubkey_hex.clone())
            .collect::<Vec<_>>(),
        expected_devices
    );
    assert_eq!(
        known_app_keys_to_ndr(&known)
            .expect("convert back")
            .get_all_devices()
            .len(),
        2
    );
}

#[test]
fn peer_profile_debug_reports_known_user_context() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("peer-profile-debug", &owner, &device);
    let peer_hex = peer.public_key().to_hex();
    let peer_device_hex = peer_device.public_key().to_hex();
    let app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 10)]);

    core.apply_known_app_keys_snapshot(peer.public_key(), &app_keys, 10);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(peer.public_key(), app_keys, 10)
        .expect("ingest app keys");
    core.process_protocol_engine_retry_batch("test_app_keys", batch);
    core.remember_recent_handshake_peer(peer_hex.clone(), peer_device_hex, 123);
    core.handle_action(AppAction::CreateChat {
        peer_input: peer_hex.clone(),
    });

    let debug = core
        .build_peer_profile_debug_snapshot(&peer_hex)
        .expect("peer debug");
    assert_eq!(debug.owner_pubkey_hex, peer_hex);
    assert_eq!(debug.roster_device_count, 1);
    assert_eq!(debug.known_device_count, 1);
    assert_eq!(debug.session_count, 0);
    assert_eq!(debug.active_session_count, 0);
    assert_eq!(debug.recent_handshake_device_count, 1);
    assert_eq!(debug.last_handshake_at_secs, Some(123));
    assert!(debug.tracked_for_messages);
}

#[test]
fn linked_device_authorization_follows_app_keys() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!("iris-chat-rs-test-{}", owner.public_key().to_hex()))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    assert_eq!(
        core.local_authorization_state(None, owner.public_key(), device.public_key(), None),
        LocalAuthorizationState::AwaitingApproval
    );

    let other_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 10)]),
        10,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), other_keys);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::AwaitingApproval
    );
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Revoked
    );

    let approved_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), approved_keys);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::AwaitingApproval
    );

    core.start_session(owner.public_key(), None, device.clone(), false, false)
        .expect("linked session");
    let approved_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys
        .insert(owner.public_key().to_hex(), approved_keys);
    install_local_sibling_session_for_test(&mut core, &owner, &device, &other_device);
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::AwaitingApproval),
        ),
        LocalAuthorizationState::Authorized
    );
}

#[test]
fn restored_authorized_linked_device_is_not_revoked_by_cached_roster() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-restored-auth-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 10)]),
            10,
        ),
    );

    assert_eq!(
        core.restored_local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Authorized
    );
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::Revoked
    );
}

#[test]
fn restored_linked_device_uses_persisted_protocol_session_for_authorization() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            data_dir.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.preferences.nostr_relay_urls.clear();
        core.start_session(
            owner.public_key(),
            None,
            linked_device.clone(),
            false,
            false,
        )
        .expect("linked session");
        core.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::new(vec![
                    DeviceEntry::new(primary_device.public_key(), 10),
                    DeviceEntry::new(linked_device.public_key(), 11),
                ]),
                11,
            ),
        );
        install_local_sibling_session_for_test(&mut core, &owner, &linked_device, &primary_device);
        core.refresh_local_authorization_state();
        core.persist_best_effort();

        assert_eq!(
            core.logged_in
                .as_ref()
                .expect("logged in")
                .authorization_state,
            LocalAuthorizationState::Authorized
        );
    }

    let mut restored = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    restored
        .start_session(owner.public_key(), None, linked_device, true, false)
        .expect("restored linked session");

    assert_eq!(
        restored
            .logged_in
            .as_ref()
            .expect("logged in")
            .authorization_state,
        LocalAuthorizationState::Authorized
    );
    assert_eq!(
        restored
            .state
            .account
            .as_ref()
            .expect("account")
            .authorization_state,
        DeviceAuthorizationState::Authorized
    );
}

#[test]
fn linked_device_missing_local_session_exposes_link_code() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_session(owner.public_key(), None, device.clone(), false, false)
        .expect("linked session");
    let app_keys = known_app_keys_from_ndr(
        owner.public_key(),
        &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
        20,
    );
    core.app_keys.insert(owner.public_key().to_hex(), app_keys);
    core.refresh_local_authorization_state();
    core.rebuild_state();

    let account = core.state.account.as_ref().expect("account");
    assert_eq!(
        account.authorization_state,
        DeviceAuthorizationState::AwaitingApproval
    );
    let snapshot = core
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot");
    let invite =
        nostr_double_ratchet_nostr::parse_invite_url(&snapshot.url).expect("parse link invite");
    assert_eq!(invite.purpose.as_deref(), Some("link"));
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_eq!(
        invite.inviter.to_bech32().ok().as_deref(),
        Some(snapshot.device_input.as_str())
    );
}

#[test]
fn app_keys_cache_merges_older_roster_events_additively() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let old_device = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-roster-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device.public_key(), 20)]),
            20,
        ),
    );

    let stale = AppKeys::new(vec![DeviceEntry::new(old_device.public_key(), 10)]);
    assert!(
        core.apply_known_app_keys_snapshot(owner.public_key(), &stale, 10)
            .is_some(),
        "older owner-signed app-key events should add missing devices without replacing the cached timestamp"
    );

    let cached = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("cached roster");
    assert_eq!(cached.created_at_secs, 20);
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device.public_key().to_hex()));
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == old_device.public_key().to_hex()));
    assert_eq!(
        core.local_authorization_state(
            None,
            owner.public_key(),
            device.public_key(),
            Some(LocalAuthorizationState::Authorized),
        ),
        LocalAuthorizationState::AwaitingApproval,
        "a cached roster without an active local protocol session must not mark the linked device approved"
    );
}

#[test]
fn app_keys_cache_merges_same_timestamp_roster_events() {
    let owner = Keys::generate();
    let device_a = Keys::generate();
    let device_b = Keys::generate();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-roster-merge-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device_a.public_key(), 20)]),
            20,
        ),
    );

    let concurrent = AppKeys::new(vec![DeviceEntry::new(device_b.public_key(), 20)]);
    let applied = core
        .apply_known_app_keys_snapshot(owner.public_key(), &concurrent, 20)
        .expect("same-timestamp roster should merge");
    assert_eq!(applied.1, 20);

    let cached = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("cached roster");
    assert_eq!(cached.created_at_secs, 20);
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_a.public_key().to_hex()));
    assert!(cached
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device_b.public_key().to_hex()));
}

#[test]
fn app_keys_runtime_storage_failure_does_not_mark_seen_or_mutate_app_cache() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let remote_owner = Keys::generate();
    let remote_device = Keys::generate();
    let runtime_storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "appkeys-runtime-storage-failure",
        &owner,
        &device,
        runtime_storage.clone() as Arc<dyn StorageAdapter>,
    );
    let remote_event = AppKeys::new(vec![DeviceEntry::new(remote_device.public_key(), 1)])
        .get_event(remote_owner.public_key())
        .sign_with_keys(&remote_owner)
        .expect("remote app keys event");
    let event_id = remote_event.id.to_string();

    runtime_storage.set_fail_puts(true);
    core.handle_relay_event(remote_event.clone());

    assert!(
        !core.seen_event_ids.contains(&event_id),
        "transient runtime persistence failure must not dedupe the protocol event"
    );
    assert!(
        !core
            .app_keys
            .contains_key(&remote_owner.public_key().to_hex()),
        "app projection must not commit AppKeys that runtime failed to persist"
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .known_device_identity_pubkeys_for_owner(remote_owner.public_key())
            .is_empty(),
        "runtime roster must remain unchanged after failed persistence"
    );

    runtime_storage.set_fail_puts(false);
    core.handle_relay_event(remote_event);

    assert!(core.seen_event_ids.contains(&event_id));
    assert!(core
        .app_keys
        .get(&remote_owner.public_key().to_hex())
        .is_some_and(|known| known
            .devices
            .iter()
            .any(|entry| { entry.identity_pubkey_hex == remote_device.public_key().to_hex() })));
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .known_device_identity_pubkeys_for_owner(remote_owner.public_key()),
        vec![remote_device.public_key()]
    );
}

#[test]
fn invite_runtime_storage_failure_does_not_mark_seen() {
    use nostr_double_ratchet_nostr::InviteNostrExt;

    let owner = Keys::generate();
    let device = Keys::generate();
    let remote_owner = Keys::generate();
    let remote_device = Keys::generate();
    let runtime_storage = Arc::new(SwitchableFailStorage::new());
    let mut core = logged_in_test_core_with_storage(
        "invite-runtime-storage-failure",
        &owner,
        &device,
        runtime_storage.clone() as Arc<dyn StorageAdapter>,
    );
    let mut invite = Invite::create_new(
        remote_device.public_key(),
        Some(remote_device.public_key().to_hex()),
        Some(1),
    )
    .expect("remote invite");
    invite.owner_public_key = Some(remote_owner.public_key());
    let invite_event = invite
        .get_event()
        .expect("invite unsigned event")
        .sign_with_keys(&remote_device)
        .expect("invite event");
    let event_id = invite_event.id.to_string();

    runtime_storage.set_fail_puts(true);
    core.handle_relay_event(invite_event.clone());

    assert!(
        !core.seen_event_ids.contains(&event_id),
        "transient runtime persistence failure must not dedupe invite events"
    );

    runtime_storage.set_fail_puts(false);
    core.handle_relay_event(invite_event);

    assert!(core.seen_event_ids.contains(&event_id));
}

#[test]
fn restored_owner_session_does_not_publish_single_device_app_keys_before_backfill() {
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

    core.start_primary_session(owner, device, true, false)
        .expect("restored session");

    let app_keys_events = update_rx
        .try_iter()
        .filter(|update| {
            if let AppUpdate::NearbyPublishedEvent { event_json, .. } = update {
                return serde_json::from_str::<Event>(event_json)
                    .map(|event| is_app_keys_event(&event))
                    .unwrap_or(false);
            }
            false
        })
        .count();
    assert_eq!(
        app_keys_events, 0,
        "restored nsec login must not overwrite relay AppKeys before fetching them"
    );
}

#[test]
fn restored_account_bundle_publishes_existing_device_app_keys_on_startup() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let owner_nsec = owner
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| owner.secret_key().to_secret_hex());
    let device_nsec = device
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| device.secret_key().to_secret_hex());
    let temp_dir = tempfile::TempDir::new().expect("temp dir");

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            temp_dir.path().to_string_lossy().to_string(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        core.start_session(
            owner.public_key(),
            Some(owner.clone()),
            device.clone(),
            false,
            true,
        )
        .expect("created account bundle session");
        core.shutdown();
    }

    let (update_tx, update_rx) = flume::unbounded();
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.restore_account_bundle(Some(owner_nsec), &owner.public_key().to_hex(), &device_nsec);
    assert_eq!(core.state.toast, None);

    let app_keys_events = update_rx
        .try_iter()
        .filter(|update| {
            if let AppUpdate::NearbyPublishedEvent { event_json, .. } = update {
                return serde_json::from_str::<Event>(event_json)
                    .map(|event| is_app_keys_event(&event))
                    .unwrap_or(false);
            }
            false
        })
        .count();
    assert!(
        app_keys_events > 0,
        "same-device startup must continue publishing the current AppKeys roster"
    );
}

#[test]
fn restored_account_bundle_defers_app_keys_when_roster_was_not_backfilled() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let owner_nsec = owner
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| owner.secret_key().to_secret_hex());
    let device_nsec = device
        .secret_key()
        .to_bech32()
        .unwrap_or_else(|_| device.secret_key().to_secret_hex());
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.restore_account_bundle(Some(owner_nsec), &owner.public_key().to_hex(), &device_nsec);
    assert_eq!(core.state.toast, None);

    let app_keys_events = update_rx
        .try_iter()
        .filter(|update| {
            if let AppUpdate::NearbyPublishedEvent { event_json, .. } = update {
                return serde_json::from_str::<Event>(event_json)
                    .map(|event| is_app_keys_event(&event))
                    .unwrap_or(false);
            }
            false
        })
        .count();
    assert_eq!(
        app_keys_events, 0,
        "restored account bundle must not publish a one-device AppKeys roster before backfill"
    );
}

#[test]
fn restored_owner_app_keys_backfill_merges_current_device_and_republishes() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let web_device = Keys::generate();
    let (update_tx, update_rx) = flume::unbounded();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        update_tx,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    core.start_primary_session(owner.clone(), device.clone(), true, false)
        .expect("restored session");
    while update_rx.try_recv().is_ok() {}

    let remote_app_keys = AppKeys::new(vec![DeviceEntry::new(web_device.public_key(), 10)]);
    let remote_event = remote_app_keys
        .get_event(owner.public_key())
        .sign_with_keys(&owner)
        .expect("remote app keys event");
    core.apply_app_keys_event(&remote_event)
        .expect("apply remote app keys");

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("known app keys");
    assert!(known
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == web_device.public_key().to_hex()));
    assert!(known
        .devices
        .iter()
        .any(|entry| entry.identity_pubkey_hex == device.public_key().to_hex()));

    let published_app_keys = update_rx
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent { event_json, .. } => {
                serde_json::from_str::<Event>(&event_json)
                    .ok()
                    .filter(is_app_keys_event)
                    .and_then(|event| AppKeys::from_event(&event).ok())
            }
            _ => None,
        })
        .last()
        .expect("republished merged app keys");
    assert!(published_app_keys
        .get_device(&web_device.public_key())
        .is_some());
    assert!(published_app_keys
        .get_device(&device.public_key())
        .is_some());
}

#[test]
fn app_keys_event_rerenders_device_roster_even_when_authorization_is_unchanged() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.start_primary_session(owner.clone(), device.clone(), false, false)
        .expect("primary session");
    assert_eq!(
        core.state
            .device_roster
            .as_ref()
            .expect("device roster")
            .devices
            .len(),
        1
    );

    let local_created_at = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("local app keys")
        .created_at_secs;
    let remote_created_at = local_created_at + 1;
    let remote_app_keys = AppKeys::new(vec![
        DeviceEntry::new(device.public_key(), local_created_at),
        DeviceEntry::new(other_device.public_key(), remote_created_at),
    ]);
    let remote_event = remote_app_keys
        .get_event_at(owner.public_key(), remote_created_at)
        .sign_with_keys(&owner)
        .expect("app keys event");
    core.apply_app_keys_event(&remote_event)
        .expect("apply remote app keys");

    let roster = core.state.device_roster.as_ref().expect("device roster");
    assert_eq!(roster.devices.len(), 2);
    assert!(roster
        .devices
        .iter()
        .any(|entry| entry.device_pubkey_hex == other_device.public_key().to_hex()));
}

#[test]
fn start_linked_device_creates_ownerless_link_invite() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();

    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let snapshot = core
        .state
        .link_device
        .as_ref()
        .expect("link-device snapshot");
    let invite =
        nostr_double_ratchet_nostr::parse_invite_url(&snapshot.url).expect("parse link invite");
    assert_eq!(invite.purpose.as_deref(), Some("link"));
    assert!(invite.owner_public_key.is_none());
    assert_eq!(
        invite.inviter.to_bech32().ok().as_deref(),
        Some(snapshot.device_input.as_str())
    );
    assert!(matches!(core.screen_stack.as_slice(), [Screen::AddDevice]));
}

#[test]
fn owner_device_accepts_link_invite_and_registers_new_device() {
    let owner = Keys::generate();
    let new_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(owner.clone(), owner.clone(), false, false)
        .expect("primary session");

    let mut invite = Invite::create_new(
        new_device.public_key(),
        Some(new_device.public_key().to_hex()),
        Some(1),
    )
    .expect("link invite");
    invite.purpose = Some("link".to_string());
    let invite_url =
        nostr_double_ratchet_nostr::invite_url(&invite, CHAT_INVITE_ROOT_URL).expect("invite url");

    core.handle_action(AppAction::AddAuthorizedDevice {
        device_input: invite_url,
    });

    let known = core
        .app_keys
        .get(&owner.public_key().to_hex())
        .expect("owner app keys");
    assert!(known
        .devices
        .iter()
        .any(|device| device.identity_pubkey_hex == new_device.public_key().to_hex()));
    assert_eq!(core.state.toast, None);
}

#[test]
fn pending_linked_device_finishes_when_owner_accepts_invite() {
    let owner = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.handle_action(AppAction::StartLinkedDevice {
        owner_input: String::new(),
    });

    let pending = core
        .pending_linked_device
        .as_ref()
        .expect("pending link invite");
    let (_owner_session, response_envelope) = pending
        .invite
        .accept_with_owner(
            owner.public_key(),
            owner.secret_key().to_secret_bytes(),
            Some(owner.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("owner accepts");
    let response_event = nostr_double_ratchet_nostr::invite_response_event(&response_envelope)
        .expect("invite response event");

    core.handle_relay_event(response_event);

    let logged_in = core.logged_in.as_ref().expect("linked session");
    assert_eq!(logged_in.owner_pubkey, owner.public_key());
    assert_eq!(
        logged_in.authorization_state,
        LocalAuthorizationState::AwaitingApproval
    );
    assert!(core.pending_linked_device.is_none());
    assert!(core
        .protocol_engine
        .as_ref()
        .is_some_and(|engine| engine.active_session_count_for_owner(owner.public_key()) > 0));
}

#[test]
fn recent_protocol_filters_include_runtime_invite_response_backfill() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let core = logged_in_test_core("protocol-backfill-invite-response", &owner, &device);
    let invite_response_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_hex();

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let response_filter = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .find(|filter| {
            let has_response_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(INVITE_RESPONSE_KIND as u64))
                });
            let has_invite_pubkey = filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|pubkey| pubkey.as_str() == Some(invite_response_pubkey.as_str()))
                });
            has_response_kind && has_invite_pubkey
        })
        .expect("invite response backfill filter");

    assert_eq!(
        response_filter
            .get("since")
            .and_then(|since| since.as_u64()),
        Some(1_777_159_500 - DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS)
    );
}

#[test]
fn protocol_filters_track_invite_responses_by_known_device_authors() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("protocol-invite-response-author", &owner, &device);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    core.app_keys.insert(
        peer_owner.public_key().to_hex(),
        known_app_keys_from_ndr(peer_owner.public_key(), &peer_app_keys, 1),
    );
    core.active_chat_id = Some(peer_owner.public_key().to_hex());

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        has_filter_with_kind_author(&filters, INVITE_RESPONSE_KIND, peer_device.public_key()),
        "invite response backfill should not depend only on #p indexing"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        has_filter_with_kind_author(&active_filters, INVITE_EVENT_KIND, peer_device.public_key()),
        "live invite subscription should track known device authors, not owner pubkeys"
    );
    assert!(
        has_filter_with_kind_author(
            &active_filters,
            INVITE_RESPONSE_KIND,
            peer_device.public_key()
        ),
        "live invite response subscription should also track known peer device authors"
    );
}

#[test]
fn single_protocol_plan_builds_filters_for_all_protocol_inputs() {
    let owner = Keys::generate();
    let invite_author = Keys::generate();
    let message_author = Keys::generate();
    let group_author = Keys::generate();
    let invite_response_recipient = Keys::generate();
    let plan = ProtocolSubscriptionPlan {
        runtime_subscriptions: vec!["ndr-protocol".to_string()],
        roster_authors: vec![owner.public_key().to_hex()],
        invite_authors: vec![invite_author.public_key().to_hex()],
        message_authors: vec![message_author.public_key().to_hex()],
        group_sender_key_authors: vec![group_author.public_key().to_hex()],
        invite_response_recipient: Some(invite_response_recipient.public_key().to_hex()),
    };

    let filters = build_protocol_subscription_filters(&plan);

    assert!(
        has_filter_with_kind_author(&filters, APP_KEYS_EVENT_KIND, owner.public_key()),
        "app-key filters must be derived from roster authors"
    );
    assert!(
        has_filter_with_kind_author_tag(
            &filters,
            APP_KEYS_EVENT_KIND,
            owner.public_key(),
            "#d",
            NDR_APP_KEYS_D_TAG
        ),
        "app-key filters must not fetch unrelated parameterized app data"
    );
    assert!(
        has_filter_with_kind_author(&filters, INVITE_EVENT_KIND, invite_author.public_key()),
        "invite filters must be derived from known device authors"
    );
    assert!(
        has_filter_with_kind_author_tag(
            &filters,
            INVITE_EVENT_KIND,
            invite_author.public_key(),
            "#l",
            NDR_INVITES_L_TAG
        ),
        "invite filters must not fetch unrelated parameterized app data"
    );
    assert!(
        has_filter_with_kind_author(&filters, INVITE_RESPONSE_KIND, invite_author.public_key()),
        "invite-response author filters must be derived from known device authors"
    );
    assert!(
        has_filter_with_kind_author(&filters, MESSAGE_EVENT_KIND, message_author.public_key()),
        "message filters must be derived from message authors"
    );
    assert!(
        has_filter_with_kind_author(
            &filters,
            GROUP_SENDER_KEY_MESSAGE_KIND,
            group_author.public_key()
        ),
        "group sender-key filters must be derived from group authors"
    );
    assert!(
        has_filter_with_kind_pubkey(
            &filters,
            INVITE_RESPONSE_KIND,
            invite_response_recipient.public_key()
        ),
        "private invite-response filters must be derived from recipient #p values"
    );
}

#[test]
fn recent_protocol_filters_do_not_include_unscoped_message_backfill_for_cold_tracked_peer() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("protocol-backfill-cold-peer", &owner, &device);
    core.active_chat_id = Some(peer.public_key().to_hex());

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    assert!(
        !has_bootstrap_message_filter(&filters),
        "cold peer discovery must fetch protocol state, not unscoped public message events"
    );
}

#[test]
fn unknown_direct_message_author_is_ignored_instead_of_bootstrapping_public_backfill() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let mallory_keys = Keys::generate();
    let carol_keys = Keys::generate();
    let mut alice_invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("invite");
    alice_invite.owner_public_key = Some(alice_keys.public_key());
    let alice = NdrRuntime::new(
        alice_keys.public_key(),
        alice_keys.secret_key().to_secret_bytes(),
        alice_keys.public_key().to_hex(),
        alice_keys.public_key(),
        None,
        Some(alice_invite.clone()),
    );
    alice.init().expect("alice init");
    let bob_runtime = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        None,
        None,
    );
    bob_runtime.init().expect("bob init");
    accept_invite_and_deliver(
        &bob_runtime,
        &bob_keys,
        &alice_invite,
        alice_keys.public_key(),
        &alice,
    );
    complete_first_contact(&bob_runtime, &bob_keys, alice_keys.public_key(), &alice);
    let alice_session_state = bob_runtime
        .get_message_push_session_states(alice_keys.public_key())
        .into_iter()
        .next()
        .expect("Bob has Alice session")
        .state;

    let mut mallory_invite = Invite::create_new(
        mallory_keys.public_key(),
        Some(mallory_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("mallory invite");
    mallory_invite.owner_public_key = Some(mallory_keys.public_key());
    let mallory = NdrRuntime::new(
        mallory_keys.public_key(),
        mallory_keys.secret_key().to_secret_bytes(),
        mallory_keys.public_key().to_hex(),
        mallory_keys.public_key(),
        None,
        Some(mallory_invite.clone()),
    );
    mallory.init().expect("mallory init");
    let carol_runtime = NdrRuntime::new(
        carol_keys.public_key(),
        carol_keys.secret_key().to_secret_bytes(),
        carol_keys.public_key().to_hex(),
        carol_keys.public_key(),
        None,
        None,
    );
    carol_runtime.init().expect("carol init");
    accept_invite_and_deliver(
        &carol_runtime,
        &carol_keys,
        &mallory_invite,
        mallory_keys.public_key(),
        &mallory,
    );
    complete_first_contact(
        &carol_runtime,
        &carol_keys,
        mallory_keys.public_key(),
        &mallory,
    );
    mallory
        .send_text(
            carol_keys.public_key(),
            "queued until unrelated protocol state arrives".to_string(),
            None,
        )
        .expect("mallory sends");
    let carol_message_authors = carol_runtime.get_all_message_push_author_pubkeys();
    let message_event = drain_signed_events(&mallory, &mallory_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && carol_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Carol");
    let message_event_id = message_event.id.to_string();

    let mut core = logged_in_test_core("pending-inbound-keeps-bootstrap", &bob_keys, &bob_keys);
    core.active_chat_id = Some(alice_keys.public_key().to_hex());
    let alice_app_keys = AppKeys::new(vec![DeviceEntry::new(alice_keys.public_key(), 1)]);
    let batch = core
        .protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(alice_keys.public_key(), alice_app_keys.clone(), 1)
        .expect("alice appkeys");
    core.process_protocol_engine_retry_batch("test_alice_appkeys", batch);
    core.app_keys.insert(
        alice_keys.public_key().to_hex(),
        known_app_keys_from_ndr(alice_keys.public_key(), &alice_app_keys, 1),
    );
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .import_session_state(
            alice_keys.public_key(),
            Some(alice_keys.public_key().to_hex()),
            alice_session_state,
            UnixSeconds(2),
        )
        .expect("alice session import");
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(alice_keys.public_key())
            .is_empty()
            == false,
        "tracked peer starts with app keys and known message authors"
    );
    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "without pending inbound work the known peer no longer needs broad bootstrap"
    );
    core.handle_relay_event(message_event);
    assert!(
        !core
            .protocol_engine
            .as_ref()
            .expect("protocol engine")
            .has_pending_inbound_direct_events(),
        "unknown public message authors must not become durable pending inbound work"
    );
    assert!(
        !core.has_seen_event(&message_event_id),
        "ignored encrypted message events must stay retryable because later bootstrap state can make the sender decryptable"
    );
    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "ignored unknown messages must not enable unscoped public backfill"
    );
}

#[test]
fn direct_message_discovery_backfill_stays_scoped_for_partial_tracked_peer_state() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core(
        "partial-tracked-peer-message-bootstrap",
        &owner,
        &linked_device,
    );

    install_local_sibling_session_for_test(&mut core, &owner, &linked_device, &primary_device);
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(owner.public_key())
            .len()
            > 0,
        "linked device should already know a primary-device message author"
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(peer.public_key())
            .is_empty(),
        "fresh peer should still need message-author discovery"
    );
    core.active_chat_id = Some(peer.public_key().to_hex());

    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "partial peer state must not trigger unscoped public message backfill"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        !has_bootstrap_message_filter(&active_filters),
        "unscoped live message subscriptions flood public relays; bootstrap discovery must stay bounded to backfill"
    );
}

#[test]
fn direct_message_discovery_does_not_install_cold_peer_live_bootstrap_subscription() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("cold-peer-no-live-bootstrap", &owner, &device);
    core.active_chat_id = Some(peer.public_key().to_hex());

    assert!(
        !has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "cold tracked peer discovery should stay on protocol-state filters"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = desired_protocol_filters(&core);
    assert!(
        !has_bootstrap_message_filter(&active_filters),
        "cold direct chats should not create an unscoped live message subscription"
    );
}

fn has_bootstrap_message_filter(filters: &[Filter]) -> bool {
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_message_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.as_u64() == Some(MESSAGE_EVENT_KIND as u64))
                });
            has_message_kind && filter.get("authors").is_none()
        })
}

fn desired_protocol_filters(core: &AppCore) -> Vec<Filter> {
    build_protocol_subscription_filters(
        core.protocol_subscription_runtime
            .desired_plan
            .as_ref()
            .expect("desired protocol plan"),
    )
}

fn has_filter_with_kind_author(filters: &[Filter], kind: u32, author: PublicKey) -> bool {
    let author_hex = author.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_author = filter
                .get("authors")
                .and_then(|authors| authors.as_array())
                .is_some_and(|authors| {
                    authors
                        .iter()
                        .any(|value| value.as_str() == Some(author_hex.as_str()))
                });
            has_kind && has_author
        })
}

fn has_filter_with_kind_author_tag(
    filters: &[Filter],
    kind: u32,
    author: PublicKey,
    tag_name: &str,
    tag_value: &str,
) -> bool {
    let author_hex = author.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_author = filter
                .get("authors")
                .and_then(|authors| authors.as_array())
                .is_some_and(|authors| {
                    authors
                        .iter()
                        .any(|value| value.as_str() == Some(author_hex.as_str()))
                });
            let has_tag = filter
                .get(tag_name)
                .and_then(|values| values.as_array())
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(tag_value)));
            has_kind && has_author && has_tag
        })
}

fn has_filter_with_kind_pubkey(filters: &[Filter], kind: u32, pubkey: PublicKey) -> bool {
    let pubkey_hex = pubkey.to_hex();
    filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            let has_kind = filter
                .get("kinds")
                .and_then(|kinds| kinds.as_array())
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|value| value.as_u64() == Some(kind as u64))
                });
            let has_pubkey = filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|value| value.as_str() == Some(pubkey_hex.as_str()))
                });
            has_kind && has_pubkey
        })
}

fn install_local_sibling_session_for_test(
    core: &mut AppCore,
    owner: &Keys,
    linked_device: &Keys,
    primary_device: &Keys,
) {
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(primary_device.public_key(), 1),
        DeviceEntry::new(linked_device.public_key(), 1),
    ]);
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .ingest_app_keys_snapshot(owner.public_key(), local_app_keys, 1)
        .expect("local appkeys");

    let linked_invite = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .local_invite_for_test()
        .expect("linked invite");
    let (_primary_session, response) = linked_invite
        .accept_with_owner(
            primary_device.public_key(),
            primary_device.secret_key().to_secret_bytes(),
            Some(primary_device.public_key().to_hex()),
            Some(owner.public_key()),
        )
        .expect("primary accepts linked invite");
    let linked_response = nostr_double_ratchet_nostr::process_invite_response_event(
        &linked_invite,
        &nostr_double_ratchet_nostr::invite_response_event(&response)
            .expect("invite response event"),
        linked_device.secret_key().to_secret_bytes(),
    )
    .expect("linked processes invite response")
    .expect("response addressed to linked invite");
    core.protocol_engine
        .as_mut()
        .expect("protocol engine")
        .import_session_state(
            owner.public_key(),
            Some(primary_device.public_key().to_hex()),
            linked_response.session.state,
            UnixSeconds(2),
        )
        .expect("linked imports primary session");
}

#[test]
fn create_invite_generates_private_link_without_public_republish() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("private-invite-create", &owner, &device);
    core.pending_relay_publishes.clear();

    let local_invite_response_pubkey = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .inviter_ephemeral_public_key
        .to_string();

    core.handle_action(AppAction::CreatePublicInvite);

    assert_eq!(core.state.toast, None);
    let snapshot = core
        .state
        .public_invite
        .as_ref()
        .expect("private invite snapshot");
    let invite =
        nostr_double_ratchet_nostr::parse_invite_url(&snapshot.url).expect("parse private invite");
    assert_eq!(invite.purpose.as_deref(), Some("private"));
    assert_eq!(invite.max_uses, Some(1));
    assert_eq!(invite.owner_public_key, Some(owner.public_key()));
    assert_ne!(
        invite.inviter_ephemeral_public_key.to_string(),
        local_invite_response_pubkey,
        "private invite links must not reuse the relay-published local invite secret"
    );
    assert_eq!(
        core.private_chat_invites
            .values()
            .next()
            .map(|invite| invite.inviter_ephemeral_public_key),
        Some(invite.inviter_ephemeral_public_key)
    );
    assert!(
        pending_events_with_kind(&core, INVITE_EVENT_KIND).is_empty(),
        "creating a private invite link must not publish a relay-discoverable invite event"
    );

    let invite_pubkey_hex = invite.inviter_ephemeral_public_key.to_string();
    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let subscribed_for_response = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .any(|filter| {
            filter
                .get("#p")
                .and_then(|pubkeys| pubkeys.as_array())
                .is_some_and(|pubkeys| {
                    pubkeys
                        .iter()
                        .any(|pubkey| pubkey.as_str() == Some(invite_pubkey_hex.as_str()))
                })
        });
    assert!(subscribed_for_response);
}

#[test]
fn private_invite_first_message_installs_creator_session() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();

    let mut alice = logged_in_test_core(
        "private-invite-roundtrip-alice",
        &alice_owner,
        &alice_device,
    );
    alice.pending_relay_publishes.clear();
    alice.handle_action(AppAction::CreatePublicInvite);
    let invite_url = alice
        .state
        .public_invite
        .as_ref()
        .expect("alice invite")
        .url
        .clone();

    let mut bob = logged_in_test_core("private-invite-roundtrip-bob", &bob_owner, &bob_device);
    bob.pending_relay_publishes.clear();
    bob.handle_action(AppAction::AcceptInvite {
        invite_input: invite_url,
    });
    assert_eq!(bob.state.toast, None);
    assert_eq!(bob.active_chat_id, Some(alice_owner.public_key().to_hex()));

    bob.handle_action(AppAction::SendMessage {
        chat_id: alice_owner.public_key().to_hex(),
        text: "hello from private invite".to_string(),
    });
    assert!(
        bob.protocol_engine
            .as_ref()
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty()),
        "sending through a private invite must install scoped message authors"
    );
    let response = pending_events_with_kind(&bob, INVITE_RESPONSE_KIND)
        .into_iter()
        .next()
        .expect("invite response event");
    alice.handle_relay_event(response);

    assert!(
        alice.protocol_engine.as_ref().is_some_and(|engine| {
            engine.active_session_count_for_owner(bob_owner.public_key()) > 0
        }),
        "Alice should install Bob's session from the private invite response"
    );
    assert!(
        alice.private_chat_invites.is_empty(),
        "one-use private invite should be removed after a matching response"
    );
    assert!(
        alice
            .protocol_engine
            .as_ref()
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty()),
        "private invite response import must immediately enable scoped peer message discovery"
    );
}

#[test]
fn queued_runtime_publish_completion_uses_inner_message_id() {
    let owner = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let inner_message_id = "inner-rumor-id".to_string();
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&owner)
        .expect("outer event");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-completion-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 1,
            messages: vec![ChatMessageSnapshot {
                id: inner_message_id.clone(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: owner.public_key().to_hex(),
                body: "queued".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: true,
                created_at_secs: 1,
                expires_at_secs: None,
                delivery: DeliveryState::Queued,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
            }],

            draft: String::new(),
        },
    );

    assert_eq!(
        core.runtime_publish_completion(
            &outer_event.id.to_string(),
            Some(&inner_message_id),
            &BTreeMap::new(),
        ),
        Some((inner_message_id, chat_id))
    );
}

#[test]
fn web_runtime_message_duplicates_dedupe_by_inner_rumor_id() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-dedupe", &owner, &device);
    let first_outer_id = "b".repeat(64);
    let second_outer_id = "c".repeat(64);
    let (content, inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "ok",
        1_777_159_493,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        content.clone(),
        Some(first_outer_id),
    );
    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some(second_outer_id));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    let matching = thread
        .messages
        .iter()
        .filter(|message| message.body == "ok")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, inner_id);
}

#[test]
fn remote_runtime_rumor_pubkey_must_match_authenticated_sender() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-remote-route", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        forged_peer.public_key(),
        CHAT_MESSAGE_KIND,
        "forged",
        1_777_159_493,
        vec![vec!["p".to_string(), forged_peer.public_key().to_hex()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("2".repeat(64)));

    let sender_chat_id = sender.public_key().to_hex();
    let forged_chat_id = forged_peer.public_key().to_hex();
    assert!(!core
        .threads
        .get(&sender_chat_id)
        .is_some_and(|thread| { thread.messages.iter().any(|message| message.id == inner_id) }));
    assert!(!core.threads.contains_key(&forged_chat_id));
}

#[test]
fn self_sync_runtime_metadata_overrides_malicious_inner_p_tag() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let real_peer = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-self-sync-route", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "self sync route comes from runtime metadata",
        1_777_159_500,
        vec![vec!["p".to_string(), forged_peer.public_key().to_hex()]],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        owner.public_key(),
        Some(sibling_device.public_key()),
        Some(real_peer.public_key()),
        content,
        Some("4".repeat(64)),
    );

    let real_chat_id = real_peer.public_key().to_hex();
    let forged_chat_id = forged_peer.public_key().to_hex();
    assert!(core
        .threads
        .get(&real_chat_id)
        .is_some_and(|thread| thread.messages.iter().any(|message| message.id == inner_id)));
    assert!(
        !core.threads.contains_key(&forged_chat_id),
        "self-sync plaintext p tag must not override authenticated runtime conversation metadata"
    );
}

#[test]
fn self_synced_direct_message_from_device_claim_routes_to_peer_owner() {
    let owner = Keys::generate();
    let linked_device = Keys::generate();
    let primary_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-device-claim-route", &owner, &linked_device);
    let local_app_keys = AppKeys::new(vec![
        DeviceEntry::new(linked_device.public_key(), 1),
        DeviceEntry::new(primary_device.public_key(), 1),
    ]);
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(owner.public_key(), &local_app_keys, 1),
    );
    let peer_chat_id = peer.public_key().to_hex();
    let primary_device_chat_id = primary_device.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from primary device",
        1_777_159_501,
        vec![vec!["p".to_string(), peer_chat_id.clone()]],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        primary_device.public_key(),
        Some(primary_device.public_key()),
        Some(peer.public_key()),
        content,
        Some("9".repeat(64)),
    );

    let thread = core.threads.get(&peer_chat_id).expect("peer thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.id, inner_id);
    assert_eq!(message.body, "sent from primary device");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert!(
        !core.threads.contains_key(&primary_device_chat_id),
        "known local device metadata must not create a direct chat with the device identity"
    );
}

#[test]
fn incoming_direct_message_from_known_peer_device_routes_to_owner_thread() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("remote-known-device-route", &owner, &local_device);
    let peer_app_keys = AppKeys::new(vec![DeviceEntry::new(peer_device.public_key(), 1)]);
    core.app_keys.insert(
        peer_owner.public_key().to_hex(),
        known_app_keys_from_ndr(peer_owner.public_key(), &peer_app_keys, 1),
    );
    let peer_owner_chat_id = peer_owner.public_key().to_hex();
    let peer_device_chat_id = peer_device.public_key().to_hex();
    core.ensure_thread_record(&peer_owner_chat_id, 1);
    let (content, inner_id) = runtime_rumor_json(
        peer_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from peer device",
        1_777_159_502,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message_with_metadata(
        peer_device.public_key(),
        Some(peer_device.public_key()),
        None,
        content,
        Some("b".repeat(64)),
    );

    let thread = core
        .threads
        .get(&peer_owner_chat_id)
        .expect("peer owner thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].id, inner_id);
    assert_eq!(thread.messages[0].body, "sent from peer device");
    assert!(!thread.messages[0].is_outgoing);
    assert!(
        !core.threads.contains_key(&peer_device_chat_id),
        "known peer device metadata must not create a direct chat with the device identity"
    );
}

#[test]
fn incoming_direct_message_from_tracked_claimed_peer_device_routes_to_owner_thread() {
    let owner = Keys::generate();
    let local_device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut core = logged_in_test_core("remote-claimed-device-route", &owner, &local_device);
    let peer_owner_chat_id = peer_owner.public_key().to_hex();
    let peer_device_chat_id = peer_device.public_key().to_hex();
    core.ensure_thread_record(&peer_owner_chat_id, 1);

    let mut session_snapshot = SessionManager::new(
        NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
        local_device.secret_key().to_secret_bytes(),
    )
    .snapshot();
    session_snapshot
        .users
        .push(nostr_double_ratchet::UserRecordSnapshot {
            owner_pubkey: NdrOwnerPubkey::from_bytes(peer_device.public_key().to_bytes()),
            roster: None,
            devices: vec![nostr_double_ratchet::DeviceRecordSnapshot {
                device_pubkey: NdrDevicePubkey::from_bytes(peer_device.public_key().to_bytes()),
                authorized: true,
                is_stale: false,
                stale_since: None,
                claimed_owner_pubkey: Some(NdrOwnerPubkey::from_bytes(
                    peer_owner.public_key().to_bytes(),
                )),
                public_invite: None,
                invite_response_generated: false,
                active_session: None,
                inactive_sessions: Vec::new(),
                last_activity: Some(NdrUnixSeconds(1)),
                created_at: NdrUnixSeconds(1),
            }],
        });
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(
        &mut core,
        &owner,
        &local_device,
        storage,
        Some(session_snapshot),
        None,
    );

    let (content, inner_id) = runtime_rumor_json(
        peer_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent before app keys backfill",
        1_777_159_503,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message_with_metadata(
        peer_device.public_key(),
        Some(peer_device.public_key()),
        None,
        content,
        Some("d".repeat(64)),
    );

    let thread = core
        .threads
        .get(&peer_owner_chat_id)
        .expect("peer owner thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].id, inner_id);
    assert_eq!(thread.messages[0].body, "sent before app keys backfill");
    assert!(
        !core.threads.contains_key(&peer_device_chat_id),
        "tracked claimed peer device must route to the owner chat while AppKeys are still catching up"
    );
}

#[test]
fn receipt_runtime_rumor_uses_authenticated_sender_chat_not_malicious_tags() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-receipt-route", &owner, &device);
    let sender_chat_id = sender.public_key().to_hex();
    let message_id = "5".repeat(64);
    core.push_outgoing_message_with_id(
        message_id.clone(),
        &sender_chat_id,
        "pending outbound".to_string(),
        1_777_159_492,
        None,
        DeliveryState::Sent,
    );
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        RECEIPT_KIND,
        "seen",
        1_777_159_493,
        vec![
            vec!["e".to_string(), message_id.clone()],
            vec!["p".to_string(), forged_peer.public_key().to_hex()],
        ],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("7".repeat(64)));

    let message = core
        .threads
        .get(&sender_chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.id == message_id)
        })
        .expect("sender chat message");
    assert_eq!(message.delivery, DeliveryState::Seen);
    assert!(!core
        .threads
        .contains_key(&forged_peer.public_key().to_hex()));
}

#[test]
fn self_synced_seen_receipt_marks_incoming_message_seen() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-seen-receipt", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let message_id = "6".repeat(64);

    core.push_incoming_message_from(
        &chat_id,
        Some(message_id.clone()),
        "read on another device".to_string(),
        1_777_159_492,
        None,
        Some(chat_id.clone()),
        Some("outer-incoming".to_string()),
    );
    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 1);

    let (content, _) = runtime_rumor_json(
        owner.public_key(),
        RECEIPT_KIND,
        "seen",
        1_777_159_493,
        vec![
            vec!["e".to_string(), message_id.clone()],
            vec!["p".to_string(), chat_id.clone()],
        ],
    );

    core.apply_decrypted_runtime_message_with_metadata(
        owner.public_key(),
        Some(sibling_device.public_key()),
        Some(peer.public_key()),
        content,
        Some("8".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 0);
    let message = thread
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .expect("incoming message");
    assert_eq!(message.delivery, DeliveryState::Seen);
}

#[test]
fn self_synced_direct_message_is_rendered_as_outgoing_on_linked_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-missing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from sibling",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(sibling_device.public_key()),
        content,
        Some("e".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.id, inner_id);
    assert_eq!(message.body, "sent from sibling");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn retry_batch_self_synced_direct_message_updates_open_chat_projection() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("retry-self-sync-open-chat", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent while open",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );

    core.open_chat(&chat_id);
    // `open_chat` defers the thread-load to an `OpenChatFinalize`
    // InternalEvent so the screen flip is instant; in the real app the
    // event loop drains the message right after. The test has no
    // message pump, so run the finalize step inline.
    core.open_chat_finalize(&chat_id);
    assert!(core
        .state
        .current_chat
        .as_ref()
        .expect("open chat")
        .messages
        .is_empty());

    core.process_protocol_engine_retry_batch(
        "test_self_sync",
        ProtocolRetryBatch {
            direct_messages: vec![ProtocolDecryptedMessage {
                sender: owner.public_key(),
                sender_device: Some(sibling_device.public_key()),
                conversation_owner: Some(peer.public_key()),
                content,
                event_id: Some("c".repeat(64)),
            }],
            ..ProtocolRetryBatch::default()
        },
    );

    let message = core
        .state
        .current_chat
        .as_ref()
        .expect("open chat after retry")
        .messages
        .iter()
        .find(|message| message.id == inner_id)
        .expect("self-synced message in open chat projection");
    assert_eq!(message.body, "sent while open");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
    assert_eq!(stored_message_count(&core), 1);
}

/// Notification-tap regression: when the iOS Notification Service
/// Extension writes a preview row to SQLite while the app is
/// suspended, the in-memory `threads` map doesn't know about that
/// chat. Tapping the notification used to flip the screen to
/// `Screen::Chat { chat_id }` before any thread record existed, so
/// `state.current_chat` came back `None` on the first paint and the
/// SwiftUI ChatScreen sat on its "Loading chat…" placeholder until
/// the deferred `OpenChatFinalize` event landed.
///
/// `open_chat` now stubs the thread record + loads its persisted
/// page inline, so `current_chat` is populated on the same emit that
/// flips the screen.
#[test]
fn open_chat_populates_current_chat_for_preview_only_thread() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("open-chat-preview", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    // Simulate the NSE preview-write side-effect: a message row lives
    // in SQLite under the peer's chat_id without any in-memory thread
    // record yet (the running core only learns about it on next relay
    // event ingest, which the suspended app didn't have a chance to
    // run).
    let preview = ChatMessageSnapshot {
        id: "1".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: peer.public_key().to_hex(),
        body: "ping from nse".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 1_777_159_500,
        expires_at_secs: None,
        delivery: DeliveryState::Received,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("0".repeat(64)),
    };
    core.app_store
        .upsert_notification_preview_message(&chat_id, 1, 1_777_159_500, &preview)
        .expect("preview upsert");
    assert!(core.threads.get(&chat_id).is_none(), "preconditions");

    core.open_chat(&chat_id);

    // First emit after open_chat must already carry `current_chat` —
    // that's what stops the SwiftUI shell from rendering "Loading
    // chat…" while the (background) `OpenChatFinalize` runs.
    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current_chat present on first paint");
    assert_eq!(current.chat_id, chat_id);
    assert_eq!(current.messages.len(), 1, "preview message loaded inline");
    assert_eq!(current.messages[0].body, "ping from nse");
    assert!(matches!(
        core.state.router.screen_stack.last(),
        Some(Screen::Chat { chat_id: shown }) if shown == &chat_id
    ));
}

/// Draft persistence (Signal-iOS parity): SetChatDraft saves the
/// composer's unsent text on the thread record, send_message clears
/// it. Both states survive a reload by way of the regular persist
/// pipeline, but the in-memory snapshot is enough for the contract.
#[test]
fn set_chat_draft_persists_until_send() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("draft-persist", &owner, &device);
    let chat_id = peer.public_key().to_hex();

    core.set_chat_draft(&chat_id, "ping…");
    let snap = core
        .threads
        .get(&chat_id)
        .expect("draft stubs a thread record");
    assert_eq!(snap.draft, "ping…");
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|c| c.chat_id == chat_id)
            .map(|c| c.draft.as_str()),
        Some("ping…")
    );

    // Idempotent: same text is a no-op (no extra emit) so the chat
    // list stays equal.
    let rev_before = core.state.rev;
    core.set_chat_draft(&chat_id, "ping…");
    assert_eq!(core.state.rev, rev_before);

    // Sending wipes the draft.
    core.send_message(&chat_id, "ping…", None);
    assert!(core
        .threads
        .get(&chat_id)
        .expect("thread still present after send")
        .draft
        .is_empty());
}

#[test]
fn self_synced_direct_message_updates_existing_local_outgoing_without_duplicate() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-existing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let (content, inner_id) = runtime_rumor_json(
        owner.public_key(),
        CHAT_MESSAGE_KIND,
        "sent from this device",
        1_777_159_500,
        vec![
            vec!["p".to_string(), chat_id.clone()],
            vec!["ms".to_string(), "1777159500123".to_string()],
        ],
    );
    core.push_outgoing_message_with_id(
        inner_id.clone(),
        &chat_id,
        "local optimistic".to_string(),
        1_777_159_499,
        None,
        DeliveryState::Pending,
    );

    core.apply_decrypted_runtime_message(
        owner.public_key(),
        Some(device.public_key()),
        content,
        Some("a".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    let message = &thread.messages[0];
    assert_eq!(message.body, "local optimistic");
    assert!(message.is_outgoing);
    assert_eq!(message.delivery, DeliveryState::Sent);
}

#[test]
fn web_runtime_typing_rumors_do_not_become_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing", &owner, &device);
    let outer_event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "")
        .sign_with_keys(&sender)
        .expect("outer event");
    let outer_event_id = outer_event.id.to_string();
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        TYPING_KIND,
        "typing",
        1_777_159_483,
        vec![
            vec!["ms".to_string(), "1777159483368".to_string()],
            vec!["expiration".to_string(), "1777159543".to_string()],
        ],
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        content,
        Some(outer_event_id.clone()),
    );

    let chat_id = sender.public_key().to_hex();
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
    assert!(core.typing_indicators.values().any(|record| {
        record.chat_id == chat_id && record.author_owner_hex == sender.public_key().to_hex()
    }));

    // Typing rumors aren't durable, so the SQLite-backed
    // notification preview path doesn't find them and falls through
    // to the generic resolver — which suppresses non-message kinds.
    // Only durable chat messages get a real preview after the
    // foreground app has consumed the rumor.
    let payload = serde_json::json!({
        "event": outer_event,
        "title": "Iris Chat",
        "body": "New message",
    })
    .to_string();
    let resolution = decrypt_mobile_push_notification(
        core.data_dir.to_string_lossy().to_string(),
        "invalid-owner".to_string(),
        "invalid-device".to_string(),
        payload,
    );
    assert!(!resolution.should_show);
}

#[test]
fn web_runtime_typing_stop_clears_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-typing-stop", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 1);
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        TYPING_KIND,
        "typing",
        1_777_159_484,
        vec![vec!["expiration".to_string(), "1".to_string()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("b".repeat(64)));

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
    assert!(core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.is_empty())
        .unwrap_or(true));
}

#[test]
fn newer_chat_message_clears_stale_typing_indicator() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-newer-message", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 10);
    core.push_outgoing_message_with_id(
        "newer-local-message".to_string(),
        &chat_id,
        "ok".to_string(),
        11,
        None,
        DeliveryState::Sent,
    );
    core.rebuild_state();

    assert!(!core
        .typing_indicators
        .values()
        .any(|record| { record.chat_id == chat_id && record.author_owner_hex == sender_hex }));
}

#[test]
fn first_received_message_clears_typing_indicator_in_chat_list() {
    // Repro for the "chat row stuck on Typing after the very first
    // peer message" complaint: the chat starts with no messages,
    // peer types, peer sends, the chat list row must drop the
    // typing badge as soon as the message lands. Goes through the
    // production path (`apply_runtime_text_message`), which is what
    // the relay event handler calls.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let row_before = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id);
    if let Some(row) = row_before {
        assert!(
            row.is_typing,
            "precondition: typing badge visible before the first message"
        );
    }

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(
        !row.is_typing,
        "chat list must drop the typing badge once the peer's first message lands"
    );
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn first_received_message_clears_typing_indicator_in_open_chat() {
    // Same case as the chat-list version, but with the chat actively
    // open. The in-chat typing badge reads from
    // `current_chat.typing_indicators`, which goes through the same
    // projection filter — must drop the indicator after the message.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-first-msg-open", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    // Open the chat: set active + create the (empty) thread record so
    // `current_chat` actually projects.
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 0,
            messages: Vec::new(),
            draft: String::new(),
        },
    );
    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.rebuild_state();
    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        !current.typing_indicators.is_empty(),
        "precondition: indicator visible in open chat"
    );

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        101,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let current = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat present");
    assert!(
        current.typing_indicators.is_empty(),
        "open chat must drop the typing badge after the first message"
    );
}

#[test]
fn first_received_message_at_same_second_clears_typing_indicator() {
    // Same bug, edge case: typing event and the chat message share
    // a one-second wire-clock tick. Production path again.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-same-second", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.set_typing_indicator(chat_id.clone(), sender_hex.clone(), 100);
    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.rebuild_state();

    let row = core
        .state
        .chat_list
        .iter()
        .find(|row| row.chat_id == chat_id)
        .expect("chat row");
    assert!(!row.is_typing);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn typing_floor_blocks_late_typing_after_message() {
    // Bug shape from peer apps that don't send a stop-typing event:
    // a typing rumor with `created_at_secs` strictly greater than the
    // latest message slips through the projection's `>` filter and
    // re-arms the indicator after we've already shown the message.
    //
    // The floor is bumped when the message lands and gates any
    // subsequent typing event with `event_secs <= floor`. Even though
    // the typing rumor here has T=200 > the message's T=100, the
    // floor is also at 100 (it's the same chat) — the typing must
    // strictly exceed the floor *and* the floor itself was set by
    // the message we already saw, so a new typing event has to be
    // genuinely after that message's wire-clock second to surface.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-late", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );

    // Typing rumor races in after the message with the *same* wire
    // second — the floor at 100 keeps it suppressed.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 100, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));

    // A genuinely newer typing event (peer is typing again) does
    // arm the indicator.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 101, None);
    assert!(core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn typing_floor_persists_across_message_deletion() {
    // iris-chat (JS) keeps `lastMessageAt` monotonic so that deleting
    // a message doesn't let a stale typing rumor slip through. Same
    // contract here: once the floor reaches a given second, deleting
    // the message that put it there leaves the floor in place.
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("typing-floor-delete", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let sender_hex = sender.public_key().to_hex();

    core.apply_runtime_text_message(
        sender.public_key(),
        Some(chat_id.clone()),
        "hi".to_string(),
        100,
        None,
        Some("msg-1".to_string()),
        None,
    );
    core.delete_local_message(&chat_id, "msg-1");

    // The thread is now empty (latest_message_secs would be 0) but
    // the floor must stay at 100 from the message we already saw.
    core.apply_typing_event(chat_id.clone(), sender_hex.clone(), 100, None);
    assert!(!core
        .typing_indicators
        .values()
        .any(|record| record.chat_id == chat_id && record.author_owner_hex == sender_hex));
}

#[test]
fn web_runtime_control_rumors_do_not_create_chat_messages() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let controls = [
        (
            RECEIPT_KIND,
            "seen",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
        (
            REACTION_KIND,
            "+",
            vec![vec!["e".to_string(), "1".to_string()]],
        ),
    ];

    for (index, (kind, body, tags)) in controls.into_iter().enumerate() {
        let mut core =
            logged_in_test_core(&format!("web-runtime-control-{index}"), &owner, &device);
        let (content, _) = runtime_rumor_json(
            sender.public_key(),
            kind,
            body,
            1_777_159_483 + index as u64,
            tags,
        );

        core.apply_decrypted_runtime_message(
            sender.public_key(),
            None,
            content,
            Some(format!("{:064x}", index + 20)),
        );

        let chat_id = sender.public_key().to_hex();
        assert!(
            core.threads
                .get(&chat_id)
                .map(|thread| thread.messages.is_empty())
                .unwrap_or(true),
            "control kind {kind} created a chat message"
        );
    }
}

#[test]
fn web_runtime_chat_settings_create_system_notice() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-chat-settings", &owner, &device);
    let (content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        "60",
        1_777_159_483,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("1".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert!(thread.messages[0]
        .body
        .contains("set disappearing messages timer"));
}

#[test]
fn web_runtime_chat_settings_update_and_clear_ttl() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-chat-settings-ttl", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let (set_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        &serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        })
        .to_string(),
        1_777_159_483,
        Vec::new(),
    );

    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        set_content,
        Some("b".repeat(64)),
    );

    assert_eq!(
        core.chat_message_ttl_seconds.get(&chat_id),
        Some(&3600),
        "incoming settings set chat ttl"
    );
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    let thread = core.threads.get(&chat_id).expect("thread");
    assert!(thread
        .messages
        .last()
        .expect("system notice")
        .body
        .contains("1 hour"));

    let (clear_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        "0",
        1_777_159_484,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        clear_content,
        Some("d".repeat(64)),
    );

    assert!(
        !core.chat_message_ttl_seconds.contains_key(&chat_id),
        "incoming settings clear chat ttl"
    );
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let thread = core.threads.get(&chat_id).expect("thread after clear");
    assert!(thread
        .messages
        .last()
        .expect("clear notice")
        .body
        .contains("Off"));
}

#[test]
fn web_runtime_chat_message_expiration_tag_is_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("web-runtime-expiring-message", &owner, &device);
    let (content, inner_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "secret",
        1_777_159_483,
        vec![vec!["expiration".to_string(), "1777159543".to_string()]],
    );

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("f".repeat(64)));

    let chat_id = sender.public_key().to_hex();
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "secret");
    assert_eq!(thread.messages[0].expires_at_secs, Some(1_777_159_543));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &inner_id),
        Some(1_777_159_543)
    );
}

#[test]
fn runtime_controls_settings_reactions_and_expiration_flow() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("runtime-controls-flow", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let (message_content, message_id) = runtime_rumor_json(
        sender.public_key(),
        CHAT_MESSAGE_KIND,
        "runtime message",
        1_777_159_483,
        vec![vec!["expiration".to_string(), "1777159543".to_string()]],
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        message_content,
        Some("2".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread after message");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "runtime message");
    assert_eq!(thread.messages[0].expires_at_secs, Some(1_777_159_543));

    core.apply_typing_event(
        chat_id.clone(),
        sender.public_key().to_hex(),
        1_777_159_484,
        None,
    );
    assert!(core.typing_indicators.values().any(|record| {
        record.chat_id == chat_id && record.author_owner_hex == sender.public_key().to_hex()
    }));

    core.apply_incoming_reaction_to_chat(&chat_id, &message_id, &sender.public_key().to_hex(), "+");
    let reacted = core
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.messages.first())
        .expect("reacted message");
    assert_eq!(reacted.reactions.len(), 1);
    assert_eq!(reacted.reactions[0].emoji, "+");

    let (settings_content, _) = runtime_rumor_json(
        sender.public_key(),
        CHAT_SETTINGS_KIND,
        &serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        })
        .to_string(),
        1_777_159_485,
        Vec::new(),
    );
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        settings_content,
        Some("4".repeat(64)),
    );
    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "local expiring reply".to_string(),
        expires_at_secs: 1_777_160_000,
    });
    let reply = core
        .threads
        .get(&chat_id)
        .and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.body == "local expiring reply")
        })
        .expect("local expiring reply");
    assert_eq!(reply.expires_at_secs, Some(1_777_160_000));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &reply.id),
        Some(1_777_160_000)
    );
}

fn ndr_owner_pubkey(pubkey: PublicKey) -> nostr_double_ratchet::OwnerPubkey {
    nostr_double_ratchet::OwnerPubkey::from_bytes(pubkey.to_bytes())
}

fn ndr_device_pubkey(pubkey: PublicKey) -> nostr_double_ratchet::DevicePubkey {
    nostr_double_ratchet::DevicePubkey::from_bytes(pubkey.to_bytes())
}

fn test_group_snapshot(
    group_id: &str,
    name: &str,
    created_by: PublicKey,
    members: Vec<PublicKey>,
    admins: Vec<PublicKey>,
    revision: u64,
) -> GroupSnapshot {
    GroupSnapshot {
        group_id: group_id.to_string(),
        protocol: nostr_double_ratchet::GroupProtocol::sender_key_v1(),
        name: name.to_string(),
        created_by: ndr_owner_pubkey(created_by),
        members: members.into_iter().map(ndr_owner_pubkey).collect(),
        admins: admins.into_iter().map(ndr_owner_pubkey).collect(),
        revision,
        created_at: nostr_double_ratchet::UnixSeconds(1),
        updated_at: nostr_double_ratchet::UnixSeconds(revision),
    }
}

#[test]
fn group_runtime_chat_message_is_persisted() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let mut core = logged_in_test_core("group-runtime-message", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-runtime-message".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        sender_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "group secret",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    let payload = payload.into_bytes();

    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
            body: payload,
            revision: 1,
        },
    ));

    let thread = core.threads.get(&chat_id).expect("group thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "group secret");
    assert_eq!(thread.messages[0].id, rumor_id);
    assert_eq!(thread.messages[0].expires_at_secs, None);
}

#[test]
fn group_rumor_id_dedupes_pairwise_fanout_copies() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let mut core = logged_in_test_core("group-pairwise-dedupe", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-pairwise-dedupe".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        sender_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "deduped group body",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    let payload = payload.into_bytes();

    for _ in 0..2 {
        core.apply_group_decrypted_event(GroupIncomingEvent::Message(
            nostr_double_ratchet::GroupReceivedMessage {
                group_id: group_id.clone(),
                sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
                sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
                body: payload.clone(),
                revision: 1,
            },
        ));
    }

    let thread = core.threads.get(&chat_id).expect("group thread");
    let matching = thread
        .messages
        .iter()
        .filter(|message| message.body == "deduped group body")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, rumor_id);
}

#[test]
fn group_runtime_rumor_pubkey_must_match_authenticated_sender() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender_owner = Keys::generate();
    let sender_device = Keys::generate();
    let forged_owner = Keys::generate();
    let mut core = logged_in_test_core("group-runtime-forged-author", &owner, &device);
    core.preferences.send_read_receipts = false;
    let group_id = "group-forged-author".to_string();
    let chat_id = group_chat_id(&group_id);
    let (payload, rumor_id) = runtime_rumor_json(
        forged_owner.public_key(),
        CHAT_MESSAGE_KIND,
        "forged group body",
        1_777_159_483,
        vec![vec!["l".to_string(), group_id.clone()]],
    );

    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
            body: payload.into_bytes(),
            revision: 1,
        },
    ));

    assert!(!core
        .threads
        .get(&chat_id)
        .is_some_and(|thread| thread.messages.iter().any(|message| message.id == rumor_id)));
}

#[test]
fn chat_ttl_applies_to_outgoing_message_expiration() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("outgoing-message-ttl", &owner, &device);
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 60);

    let before = unix_now().get();
    core.send_message(&chat_id, "secret", None);
    let after = unix_now().get();

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("sent message");
    let expires_at = message.expires_at_secs.expect("message expiration");
    assert!(expires_at >= before.saturating_add(60));
    assert!(expires_at <= after.saturating_add(60));
}

#[test]
fn set_chat_message_ttl_action_sets_clears_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core_at_data_dir(&owner, &device, data_dir.clone());

    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });

    assert_eq!(core.chat_message_ttl_seconds.get(&chat_id), Some(&3600));
    assert_eq!(stored_chat_ttl(&core, &chat_id), Some(3600));
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .message_ttl_seconds,
        Some(3600)
    );
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("persisted state");
    assert_eq!(loaded.chat_message_ttl_seconds.get(&chat_id), Some(&3600));

    let notice_count = core
        .threads
        .get(&chat_id)
        .map(|thread| thread.messages.len())
        .unwrap_or_default();
    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: Some(3600),
    });
    assert_eq!(
        core.threads
            .get(&chat_id)
            .map(|thread| thread.messages.len())
            .unwrap_or_default(),
        notice_count,
        "reselecting the active timer must not publish another chat-settings notice"
    );

    core.handle_action(AppAction::SetChatMessageTtl {
        chat_id: chat_id.clone(),
        ttl_seconds: None,
    });

    assert!(!core.chat_message_ttl_seconds.contains_key(&chat_id));
    assert_eq!(stored_chat_ttl(&core, &chat_id), None);
    let loaded = core
        .load_persisted()
        .expect("load persisted after clear")
        .expect("persisted state after clear");
    assert!(
        !loaded.chat_message_ttl_seconds.contains_key(&chat_id),
        "cleared ttl is not restored"
    );
}

#[test]
fn send_disappearing_message_action_uses_explicit_expiration_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("send-disappearing-message-action", &owner, &device);
    let expires_at = unix_now().get().saturating_add(600);

    core.handle_action(AppAction::SendDisappearingMessage {
        chat_id: chat_id.clone(),
        text: "secret".to_string(),
        expires_at_secs: expires_at,
    });

    let thread = core.threads.get(&chat_id).expect("thread");
    let message = thread
        .messages
        .iter()
        .find(|message| message.body == "secret")
        .expect("disappearing message");
    assert_eq!(message.expires_at_secs, Some(expires_at));
    assert_eq!(
        stored_message_expiration(&core, &chat_id, &message.id),
        Some(expires_at)
    );
    assert!(
        core.message_expiry_token > 0,
        "expiring sends schedule message pruning"
    );
}

struct SenderKeyMatrixDevice {
    owner: Keys,
    device: Keys,
    engine: ProtocolEngine,
}

impl SenderKeyMatrixDevice {
    fn new() -> Self {
        let owner = Keys::generate();
        let device = Keys::generate();
        let engine = test_protocol_engine(&owner, &device);
        Self {
            owner,
            device,
            engine,
        }
    }
}

fn sender_key_matrix_devices(count: usize) -> Vec<SenderKeyMatrixDevice> {
    let mut devices = (0..count)
        .map(|_| SenderKeyMatrixDevice::new())
        .collect::<Vec<_>>();
    observe_sender_key_matrix_protocol_state(&mut devices);
    devices
}

fn observe_sender_key_matrix_protocol_state(devices: &mut [SenderKeyMatrixDevice]) {
    let identities = devices
        .iter()
        .map(|device| {
            (
                device.owner.clone(),
                device.device.clone(),
                device.engine.local_invite_for_test().expect("local invite"),
            )
        })
        .collect::<Vec<_>>();
    for recipient in devices.iter_mut() {
        for (owner, device, invite) in &identities {
            if recipient.device.public_key() != device.public_key() {
                observe_local_invite_for_test(&mut recipient.engine, owner, device, invite);
            } else {
                observe_current_device_appkeys_for_test(&mut recipient.engine, owner, device);
            }
        }
    }
}

fn observe_local_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    invite: &Invite,
) {
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(
                device.public_key(),
                invite.created_at.get(),
            )]),
            invite.created_at.get(),
        )
        .expect("peer appkeys");
    let mut invite = invite.clone();
    invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    let event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(device)
        .expect("signed invite");
    engine
        .observe_invite_event(&event)
        .expect("observe peer local invite");
}

fn ordered_protocol_events(effects: &[ProtocolEffect]) -> Vec<Event> {
    effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event.clone()],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event.clone()],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| publish.event.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn sender_key_outer_count(effects: &[ProtocolEffect], event_ids: &[String]) -> usize {
    protocol_payload_events_for_result(effects, event_ids)
        .into_iter()
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .count()
}

fn apply_protocol_event_to_engine(
    engine: &mut ProtocolEngine,
    event: &Event,
    group_events: &mut Vec<GroupIncomingEvent>,
) {
    if event.kind.as_u16() as u32 == INVITE_EVENT_KIND {
        let retry = engine
            .observe_invite_event(event)
            .expect("observe invite event");
        group_events.extend(retry.group_result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.effects),
            group_events,
        );
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.group_result.effects),
            group_events,
        );
        return;
    }

    if event.kind.as_u16() as u32 == INVITE_RESPONSE_KIND {
        let retry = engine
            .observe_invite_response_event(event)
            .expect("observe invite response event");
        group_events.extend(retry.group_result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.effects),
            group_events,
        );
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&retry.group_result.effects),
            group_events,
        );
        return;
    }

    if parse_group_sender_key_message_event(event).is_ok() {
        let result = engine
            .process_group_outer_event(event)
            .expect("process sender-key outer event");
        group_events.extend(result.events);
        apply_protocol_events_to_engine(
            engine,
            &ordered_protocol_events(&result.effects),
            group_events,
        );
        return;
    }

    if parse_message_event(event).is_ok() {
        let decrypted = match engine.process_direct_message_event(event) {
            Ok(decrypted) => decrypted,
            Err(error)
                if error.to_string().contains("Invalid header")
                    || error.to_string().contains("invalid header")
                    || error
                        .to_string()
                        .contains("Failed to decrypt header with available keys") =>
            {
                None
            }
            Err(error) => panic!("process pairwise protocol event: {error}"),
        };
        if let Some(decrypted) = decrypted {
            let result = engine
                .process_group_pairwise_payload(
                    decrypted.content.as_bytes(),
                    decrypted.sender,
                    decrypted.sender_device,
                )
                .expect("process group pairwise payload");
            group_events.extend(result.events);
            apply_protocol_events_to_engine(
                engine,
                &ordered_protocol_events(&result.effects),
                group_events,
            );
        }
    }
}

fn apply_protocol_events_to_engine(
    engine: &mut ProtocolEngine,
    events: &[Event],
    group_events: &mut Vec<GroupIncomingEvent>,
) {
    for event in events {
        apply_protocol_event_to_engine(engine, event, group_events);
    }
}

fn deliver_protocol_effects_to_engine(
    engine: &mut ProtocolEngine,
    effects: &[ProtocolEffect],
) -> Vec<GroupIncomingEvent> {
    let mut group_events = Vec::new();
    apply_protocol_events_to_engine(engine, &ordered_protocol_events(effects), &mut group_events);
    group_events
}

fn group_events_contain_body(
    events: &[GroupIncomingEvent],
    group_id: &str,
    sender_owner: PublicKey,
    sender_device: PublicKey,
    body: &[u8],
) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            GroupIncomingEvent::Message(message)
                if message.group_id == group_id
                    && message.sender_owner == ndr_owner_pubkey(sender_owner)
                    && message.sender_device == Some(ndr_device_pubkey(sender_device))
                    && message.body == body
        )
    })
}

#[test]
fn appcore_create_group_defaults_to_sender_key_protocol() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let result = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(3))
        .expect("create sender-key group");
    let group = result.snapshot.expect("created group snapshot");

    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(
        engine.group_manager_snapshot_for_test().sender_keys.len(),
        1,
        "sender-key group creation should seed a local sender-key record"
    );
    assert_eq!(
        engine.known_group_sender_event_pubkeys().len(),
        1,
        "sender-key group creation should make its sender event author subscribable"
    );
}

#[test]
fn appcore_sender_key_group_send_publishes_one_outer_event() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let created = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(3))
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");

    let result = engine
        .send_group_payload(
            &group.group_id,
            b"sender-key message".to_vec(),
            Some("inner-message-id".to_string()),
        )
        .expect("send sender-key group payload");

    assert_eq!(result.event_ids.len(), 1);
    let outer_events = result
        .effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| &publish.event)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .collect::<Vec<_>>();

    assert_eq!(
        outer_events.len(),
        1,
        "sender-key group send should publish one shared outer event"
    );
    assert_eq!(outer_events[0].id.to_string(), result.event_ids[0]);
    assert_eq!(
        parse_group_sender_key_message_event(outer_events[0])
            .expect("sender-key outer event")
            .sender_event_pubkey
            .to_bytes(),
        outer_events[0].pubkey.to_bytes()
    );
}

#[test]
fn appcore_sender_key_outer_before_distribution_retries_after_control_state() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = test_protocol_engine(&alice_owner, &alice_device);
    let mut bob = test_protocol_engine(&bob_owner, &bob_device);
    observe_current_device_appkeys_for_test(&mut alice, &alice_owner, &alice_device);
    observe_current_device_appkeys_for_test(&mut bob, &bob_owner, &bob_device);
    observe_peer_device_invite_for_test(&mut alice, &bob_owner, &bob_device, 50);
    observe_peer_device_invite_for_test(&mut bob, &alice_owner, &alice_device, 50);

    let created = alice
        .create_group(
            "sender-key group".to_string(),
            vec![bob_owner.public_key()],
            UnixSeconds(51),
        )
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");
    let group_id = group.group_id.clone();
    let sender_key = alice
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("local sender-key record");
    let key_id = sender_key.latest_key_id.expect("latest sender key id");
    let state = sender_key
        .states
        .iter()
        .find(|state| state.key_id() == key_id)
        .expect("sender-key state");
    let distribution = nostr_double_ratchet::SenderKeyDistribution {
        group_id: group.group_id.clone(),
        key_id,
        sender_event_pubkey: sender_key.sender_event_pubkey,
        chain_key: state.chain_key(),
        iteration: state.iteration(),
        created_at: NdrUnixSeconds(51),
    };

    let sent = alice
        .send_group_payload(
            &group.group_id,
            b"queued until sender-key distribution".to_vec(),
            Some("sender-key-inner".to_string()),
        )
        .expect("send sender-key group payload");
    let outer = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .find(|event| parse_group_sender_key_message_event(event).is_ok())
        .expect("sender-key outer event");

    let pending = bob
        .process_group_outer_event(outer)
        .expect("process outer before distribution");
    assert!(pending.consumed);
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        1
    );

    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(52),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot {
            snapshot: group.clone(),
        },
    )
    .expect("metadata payload");
    let distribution_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(alice_device.public_key()),
            created_at: NdrUnixSeconds(53),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::SenderKeyDistribution { distribution },
    )
    .expect("sender-key distribution payload");

    let metadata_result = bob
        .process_group_pairwise_payload(
            &metadata_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("process metadata");
    let distribution_result = bob
        .process_group_pairwise_payload(
            &distribution_payload,
            alice_owner.public_key(),
            Some(alice_device.public_key()),
        )
        .expect("process distribution");
    assert!(matches!(
        metadata_result.events.as_slice(),
        [GroupIncomingEvent::MetadataUpdated(_)]
    ));
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        0
    );
    assert!(distribution_result.events.iter().any(|event| matches!(
        event,
        GroupIncomingEvent::Message(message)
            if message.group_id == group_id
                && message.sender_owner == ndr_owner_pubkey(alice_owner.public_key())
                && message.sender_device == Some(ndr_device_pubkey(alice_device.public_key()))
                && message.body == b"queued until sender-key distribution".to_vec()
    )));
    let retry = bob
        .retry_pending_protocol(NdrUnixSeconds(54))
        .expect("retry after pending sender-key outer already applied");
    assert!(
        retry.group_result.events.is_empty(),
        "applied pending sender-key outer must not replay on later retry"
    );
}

#[test]
fn appcore_sender_key_group_create_prepares_pairwise_metadata_and_distribution() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 10);

    let result = engine
        .create_group(
            "sender-key group".to_string(),
            vec![peer_owner.public_key()],
            UnixSeconds(20),
        )
        .expect("create sender-key group");
    let group = result.snapshot.expect("created group snapshot");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(
        result.event_ids.len(),
        2,
        "sender-key group creation should send metadata and sender-key distribution over pairwise control"
    );
    assert_eq!(payload_events.len(), 2);
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "sender-key group creation must not publish a group outer message before app payloads"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        2,
        "the peer should receive both metadata and sender-key distribution control messages"
    );
}

#[test]
fn appcore_sender_key_add_member_sends_current_distribution_pairwise() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);

    let created = engine
        .create_group("sender-key group".to_string(), Vec::new(), UnixSeconds(20))
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 21);

    let result = engine
        .add_group_members(&group.group_id, vec![peer_owner.public_key()])
        .expect("add sender-key group member");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        result.event_ids.len(),
        2,
        "adding a member should send metadata and the current sender-key distribution"
    );
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "add-member control traffic should remain pairwise"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        2
    );
}

#[test]
fn appcore_sender_key_remove_member_rotates_key_only_to_remaining_members() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let carol_owner = Keys::generate();
    let carol_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &bob_owner, &bob_device, 30);
    observe_peer_device_invite_for_test(&mut engine, &carol_owner, &carol_device, 31);

    let created = engine
        .create_group(
            "sender-key group".to_string(),
            vec![bob_owner.public_key(), carol_owner.public_key()],
            UnixSeconds(32),
        )
        .expect("create sender-key group");
    let group = created.snapshot.expect("created group snapshot");

    let result = engine
        .remove_group_member(&group.group_id, carol_owner.public_key())
        .expect("remove sender-key group member");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(
        result.event_ids.len(),
        3,
        "removal should send metadata to removed member and metadata plus rotated sender key to remaining member"
    );
    assert!(
        payload_events
            .iter()
            .all(|event| parse_message_event(event).is_ok()
                && parse_group_sender_key_message_event(event).is_err()),
        "remove-member control traffic should remain pairwise"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &bob_owner.public_key().to_hex()),
        2,
        "remaining member should receive metadata and rotated sender key"
    );
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &carol_owner.public_key().to_hex()),
        1,
        "removed member should receive metadata but not the rotated sender key"
    );
}

#[test]
fn appcore_existing_pairwise_group_still_uses_pairwise_fanout() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let mut engine = test_protocol_engine(&owner, &device);
    observe_current_device_appkeys_for_test(&mut engine, &owner, &device);
    observe_peer_device_invite_for_test(&mut engine, &peer_owner, &peer_device, 40);

    let group_id = "legacy-pairwise-group".to_string();
    let mut snapshot = test_group_snapshot(
        &group_id,
        "Legacy Pairwise Group",
        owner.public_key(),
        vec![owner.public_key(), peer_owner.public_key()],
        vec![owner.public_key()],
        1,
    );
    snapshot.protocol = nostr_double_ratchet::GroupProtocol::PairwiseFanoutV1;
    let codec = nostr_double_ratchet_nostr::JsonGroupPayloadCodecV1;
    let metadata_payload = nostr_double_ratchet::GroupPayloadCodec::encode_pairwise_command(
        &codec,
        nostr_double_ratchet::GroupPayloadEncodeContext {
            local_device_pubkey: ndr_device_pubkey(device.public_key()),
            created_at: NdrUnixSeconds(41),
        },
        &nostr_double_ratchet::GroupPairwiseCommand::MetadataSnapshot { snapshot },
    )
    .expect("metadata payload");
    engine
        .process_group_pairwise_payload(
            &metadata_payload,
            owner.public_key(),
            Some(device.public_key()),
        )
        .expect("install legacy pairwise group");

    let result = engine
        .send_group_payload(
            &group_id,
            b"legacy pairwise body".to_vec(),
            Some("legacy-inner".to_string()),
        )
        .expect("send legacy pairwise group payload");
    let payload_events = protocol_payload_events_for_result(&result.effects, &result.event_ids);

    assert_eq!(result.event_ids.len(), 1);
    assert_eq!(payload_events.len(), 1);
    assert!(parse_message_event(payload_events[0]).is_ok());
    assert!(parse_group_sender_key_message_event(payload_events[0]).is_err());
    assert_eq!(
        protocol_targeted_payload_count(&result.effects, &peer_owner.public_key().to_hex()),
        1
    );
}

#[test]
fn appcore_sender_key_four_member_matrix_delivers_one_outer_per_sender() {
    let mut devices = sender_key_matrix_devices(4);
    let member_pubkeys = devices
        .iter()
        .skip(1)
        .map(|device| device.owner.public_key())
        .collect::<Vec<_>>();
    let created = devices[0]
        .engine
        .create_group(
            "sender-key matrix".to_string(),
            member_pubkeys,
            UnixSeconds(100),
        )
        .expect("create sender-key matrix group");
    let group_id = created.snapshot.expect("created group").group_id;
    let create_effects = created.effects.clone();

    for recipient in devices.iter_mut().skip(1) {
        deliver_protocol_effects_to_engine(&mut recipient.engine, &create_effects);
    }

    for sender_index in 0..devices.len() {
        let sender_owner = devices[sender_index].owner.public_key();
        let sender_device = devices[sender_index].device.public_key();
        let body = format!("sender-key-matrix-{sender_index}").into_bytes();
        let sent = devices[sender_index]
            .engine
            .send_group_payload(
                &group_id,
                body.clone(),
                Some(format!("sender-key-matrix-inner-{sender_index}")),
            )
            .expect("send sender-key matrix group payload");

        assert_eq!(
            sender_key_outer_count(&sent.effects, &sent.event_ids),
            1,
            "sender-key message should publish one shared group outer event"
        );

        let outer_events = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
            .into_iter()
            .filter(|event| parse_group_sender_key_message_event(event).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(outer_events.len(), 1);

        for recipient_index in 0..devices.len() {
            if recipient_index == sender_index {
                continue;
            }
            let received = deliver_protocol_effects_to_engine(
                &mut devices[recipient_index].engine,
                &sent.effects,
            );
            assert!(
                group_events_contain_body(
                    &received,
                    &group_id,
                    sender_owner,
                    sender_device,
                    &body
                ),
                "recipient {recipient_index} did not decrypt message from sender {sender_index}; events={received:?}"
            );

            let duplicate = {
                let mut duplicate_events = Vec::new();
                apply_protocol_events_to_engine(
                    &mut devices[recipient_index].engine,
                    &outer_events,
                    &mut duplicate_events,
                );
                duplicate_events
            };
            assert!(
                !group_events_contain_body(
                    &duplicate,
                    &group_id,
                    sender_owner,
                    sender_device,
                    &body
                ),
                "duplicate sender-key relay replay emitted a duplicate app message"
            );
        }
    }
}

#[test]
fn appcore_sender_key_late_member_and_remove_member_enforce_membership_window() {
    let mut devices = sender_key_matrix_devices(4);
    let alice = 0;
    let bob = 1;
    let carol = 2;
    let dave = 3;
    let bob_owner_pubkey = devices[bob].owner.public_key();
    let carol_owner_pubkey = devices[carol].owner.public_key();
    let dave_owner_pubkey = devices[dave].owner.public_key();
    let alice_owner_pubkey = devices[alice].owner.public_key();
    let alice_device_pubkey = devices[alice].device.public_key();
    let created = devices[alice]
        .engine
        .create_group(
            "sender-key membership window".to_string(),
            vec![bob_owner_pubkey, carol_owner_pubkey],
            UnixSeconds(110),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [bob, carol] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }

    let before_add = b"before dave joined".to_vec();
    let before_add_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            before_add.clone(),
            Some("sender-key-before-add".to_string()),
        )
        .expect("send before late member add");
    let dave_before =
        deliver_protocol_effects_to_engine(&mut devices[dave].engine, &before_add_sent.effects);
    assert!(
        !group_events_contain_body(
            &dave_before,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            &before_add
        ),
        "late member must not decrypt messages from before membership"
    );

    let add_dave = devices[alice]
        .engine
        .add_group_members(&group_id, vec![dave_owner_pubkey])
        .expect("add late member");
    for recipient_index in [bob, carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &add_dave.effects,
        );
        assert!(
            !group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &before_add
            ),
            "sender-key distribution on add must not reveal older queued outers"
        );
    }

    let after_add = b"after dave joined".to_vec();
    let after_add_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            after_add.clone(),
            Some("sender-key-after-add".to_string()),
        )
        .expect("send after late member add");
    for recipient_index in [bob, carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &after_add_sent.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &after_add
            ),
            "current member {recipient_index} did not decrypt post-add sender-key message"
        );
    }

    let remove_bob = devices[alice]
        .engine
        .remove_group_member(&group_id, bob_owner_pubkey)
        .expect("remove member");
    for recipient_index in [bob, carol, dave] {
        deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &remove_bob.effects,
        );
    }

    let after_remove = b"after bob removed".to_vec();
    let after_remove_sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            after_remove.clone(),
            Some("sender-key-after-remove".to_string()),
        )
        .expect("send after member removal");
    let bob_events =
        deliver_protocol_effects_to_engine(&mut devices[bob].engine, &after_remove_sent.effects);
    assert!(
        !group_events_contain_body(
            &bob_events,
            &group_id,
            alice_owner_pubkey,
            alice_device_pubkey,
            &after_remove
        ),
        "removed member must not decrypt future sender-key messages"
    );
    for recipient_index in [carol, dave] {
        let events = deliver_protocol_effects_to_engine(
            &mut devices[recipient_index].engine,
            &after_remove_sent.effects,
        );
        assert!(
            group_events_contain_body(
                &events,
                &group_id,
                alice_owner_pubkey,
                alice_device_pubkey,
                &after_remove
            ),
            "remaining member {recipient_index} did not decrypt post-removal message"
        );
    }
}

#[test]
fn appcore_sender_key_pending_outer_survives_restart_and_applies_once() {
    let alice_owner = Keys::generate();
    let alice_device = Keys::generate();
    let bob_owner = Keys::generate();
    let bob_device = Keys::generate();
    let mut alice = test_protocol_engine(&alice_owner, &alice_device);
    observe_current_device_appkeys_for_test(&mut alice, &alice_owner, &alice_device);
    let alice_invite = alice.local_invite_for_test().expect("alice local invite");

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_string_lossy().to_string();
    let mut bob_core = logged_in_test_core_at_data_dir(&bob_owner, &bob_device, data_dir.clone());
    {
        let bob = bob_core
            .protocol_engine
            .as_mut()
            .expect("bob protocol engine");
        observe_current_device_appkeys_for_test(bob, &bob_owner, &bob_device);
        observe_local_invite_for_test(bob, &alice_owner, &alice_device, &alice_invite);
    }
    let bob_invite = bob_core
        .protocol_engine
        .as_ref()
        .expect("bob protocol engine")
        .local_invite_for_test()
        .expect("bob local invite");
    observe_local_invite_for_test(&mut alice, &bob_owner, &bob_device, &bob_invite);

    let created = alice
        .create_group(
            "sender-key restart".to_string(),
            vec![bob_owner.public_key()],
            UnixSeconds(122),
        )
        .expect("create sender-key group");
    let group_id = created.snapshot.expect("created group").group_id;
    let sent = alice
        .send_group_payload(
            &group_id,
            b"queued across restart".to_vec(),
            Some("sender-key-restart-inner".to_string()),
        )
        .expect("send sender-key message");
    let outer_events = protocol_payload_events_for_result(&sent.effects, &sent.event_ids)
        .into_iter()
        .filter(|event| parse_group_sender_key_message_event(event).is_ok())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(outer_events.len(), 1);

    {
        let bob = bob_core
            .protocol_engine
            .as_mut()
            .expect("bob protocol engine");
        let mut pending_events = Vec::new();
        apply_protocol_events_to_engine(bob, &outer_events, &mut pending_events);
        assert!(pending_events.is_empty());
        assert_eq!(
            bob.debug_snapshot().pending_group_sender_key_message_count,
            1
        );
    }

    drop(bob_core);
    let mut restarted = logged_in_test_core_at_data_dir(&bob_owner, &bob_device, data_dir);
    let bob = restarted
        .protocol_engine
        .as_mut()
        .expect("restarted bob protocol engine");
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        1,
        "pending sender-key outer should be durable across restart"
    );
    let applied = deliver_protocol_effects_to_engine(bob, &created.effects);
    assert!(
        group_events_contain_body(
            &applied,
            &group_id,
            alice_owner.public_key(),
            alice_device.public_key(),
            b"queued across restart"
        ),
        "pending sender-key outer should apply after persisted restart and control state arrival"
    );
    assert_eq!(
        bob.debug_snapshot().pending_group_sender_key_message_count,
        0
    );
    let retry = bob
        .retry_pending_protocol(NdrUnixSeconds(123))
        .expect("retry after persisted pending outer applied");
    assert!(
        retry.group_result.events.is_empty(),
        "applied persisted sender-key outer must not replay"
    );
}

#[test]
#[ignore = "long-running exploratory sender-key group soak"]
fn appcore_sender_key_stochastic_group_soak() {
    let mut devices = sender_key_matrix_devices(6);
    let member_one = devices[1].owner.public_key();
    let member_two = devices[2].owner.public_key();
    let member_three = devices[3].owner.public_key();
    let member_four = devices[4].owner.public_key();
    let created = devices[0]
        .engine
        .create_group(
            "sender-key soak".to_string(),
            vec![member_one, member_two],
            UnixSeconds(200),
        )
        .expect("create sender-key soak group");
    let group_id = created.snapshot.expect("created group").group_id;
    for recipient_index in [1, 2] {
        deliver_protocol_effects_to_engine(&mut devices[recipient_index].engine, &created.effects);
    }
    let mut active = vec![0usize, 1, 2];

    for step in 0..90 {
        if step == 15 {
            let add = devices[0]
                .engine
                .add_group_members(&group_id, vec![member_three])
                .expect("add fourth soak member");
            active.push(3);
            for recipient_index in active.iter().copied() {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &add.effects,
                );
            }
        }
        if step == 35 {
            let add = devices[0]
                .engine
                .add_group_members(&group_id, vec![member_four])
                .expect("add fifth soak member");
            active.push(4);
            for recipient_index in active.iter().copied() {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &add.effects,
                );
            }
        }
        if step == 60 {
            let remove = devices[0]
                .engine
                .remove_group_member(&group_id, member_one)
                .expect("remove soak member");
            active.retain(|index| *index != 1);
            for recipient_index in [1usize, 0, 2, 3, 4] {
                deliver_protocol_effects_to_engine(
                    &mut devices[recipient_index].engine,
                    &remove.effects,
                );
            }
        }

        let sender_index = active[step % active.len()];
        let body = format!("sender-key-soak-{step}").into_bytes();
        let sent = devices[sender_index]
            .engine
            .send_group_payload(
                &group_id,
                body.clone(),
                Some(format!("sender-key-soak-inner-{step}")),
            )
            .expect("send soak payload");
        assert_eq!(sender_key_outer_count(&sent.effects, &sent.event_ids), 1);
        let sender_owner = devices[sender_index].owner.public_key();
        let sender_device = devices[sender_index].device.public_key();
        for recipient_index in active.iter().copied() {
            if recipient_index == sender_index {
                continue;
            }
            let events = deliver_protocol_effects_to_engine(
                &mut devices[recipient_index].engine,
                &sent.effects,
            );
            assert!(group_events_contain_body(
                &events,
                &group_id,
                sender_owner,
                sender_device,
                &body
            ));
        }
        let removed_events =
            deliver_protocol_effects_to_engine(&mut devices[1].engine, &sent.effects);
        if !active.contains(&1) && sender_index != 1 {
            assert!(!group_events_contain_body(
                &removed_events,
                &group_id,
                sender_owner,
                sender_device,
                &body
            ));
        }
    }
}

#[test]
fn create_group_allows_self_only_group() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("self-only-group", &owner, &device);

    core.handle_action(AppAction::CreateGroup {
        name: "Notes".to_string(),
        member_inputs: Vec::new(),
    });

    let current = core.state.current_chat.as_ref().expect("opened group chat");
    let group_id = current.group_id.as_ref().expect("group id").clone();
    let group = core.groups.get(&group_id).expect("stored group");
    let owner = ndr_owner_pubkey(owner.public_key());
    assert_eq!(group.name, "Notes");
    assert_eq!(
        group.protocol,
        nostr_double_ratchet::GroupProtocol::sender_key_v1()
    );
    assert_eq!(group.members, vec![owner]);
    assert_eq!(group.admins, vec![owner]);
}

#[test]
fn group_metadata_changes_create_system_notices() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("group-metadata-notices", &owner, &device);
    let group_id = "group-notice-test".to_string();
    let chat_id = group_chat_id(&group_id);
    let owner_pubkey = owner.public_key();
    let member = Keys::generate().public_key();
    let initial = test_group_snapshot(
        &group_id,
        "Original",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        1,
    );
    let renamed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        2,
    );
    let with_member = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey],
        3,
    );
    let member_removed = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey],
        vec![owner_pubkey],
        4,
    );

    core.apply_group_metadata_notice(None, &initial);
    core.apply_group_metadata_notice(Some(&initial), &renamed);
    core.apply_group_metadata_notice(Some(&renamed), &with_member);
    core.apply_group_metadata_notice(Some(&with_member), &member_removed);
    let with_admin = test_group_snapshot(
        &group_id,
        "Renamed",
        owner_pubkey,
        vec![owner_pubkey, member],
        vec![owner_pubkey, member],
        5,
    );
    core.apply_group_metadata_notice(Some(&with_member), &with_admin);

    let messages = &core.threads.get(&chat_id).expect("group thread").messages;
    assert!(messages
        .iter()
        .any(|message| message.body == "Group created: Original"));
    assert!(messages
        .iter()
        .any(|message| message.body == "Group renamed to Renamed"));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("joined the group")));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("left the group")));
    assert!(messages
        .iter()
        .any(|message| message.kind == ChatMessageKind::System));
    assert!(messages
        .iter()
        .any(|message| message.body.contains("became an admin")));
}

#[test]
fn appcore_restart_restores_threads_groups_and_seen_events() {
    // End-to-end check that the persistence/load round trip survives a
    // full AppCore drop+recreate against the same `data_dir`.
    let owner = Keys::generate();
    let device = Keys::generate();
    let other_device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();

    let chat_id = "deadbeef".repeat(8);
    let group_id = "group-restart".to_string();
    let group_chat = group_chat_id(&group_id);

    {
        let mut core = AppCore::new(
            flume::unbounded().0,
            flume::unbounded().0,
            data_dir_str.clone(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        let invite = Invite::create_new(
            device.public_key(),
            Some(device.public_key().to_hex()),
            None,
        )
        .expect("local invite");
        core.logged_in = Some(LoggedInState {
            owner_pubkey: owner.public_key(),
            owner_keys: Some(owner.clone()),
            device_keys: device.clone(),
            client: Client::new(device.clone()),
            relay_urls: Vec::new(),
            local_invite: invite,
            authorization_state: LocalAuthorizationState::Authorized,
        });

        core.next_message_id = 17;
        core.active_chat_id = Some(chat_id.clone());
        core.app_keys.insert(
            owner.public_key().to_hex(),
            known_app_keys_from_ndr(
                owner.public_key(),
                &AppKeys::new(vec![DeviceEntry::new(other_device.public_key(), 5)]),
                10,
            ),
        );
        core.groups.insert(
            group_id.clone(),
            test_group_snapshot(
                &group_id,
                "Brunch",
                owner.public_key(),
                vec![owner.public_key()],
                vec![owner.public_key()],
                1_000,
            ),
        );
        core.threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 3,
                updated_at_secs: 200,
                messages: vec![
                    ChatMessageSnapshot {
                        id: "m1".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: owner.public_key().to_hex(),
                        body: "hello world".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: true,
                        created_at_secs: 100,
                        expires_at_secs: None,
                        delivery: DeliveryState::Sent,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                    ChatMessageSnapshot {
                        id: "m2".to_string(),
                        chat_id: chat_id.clone(),
                        kind: ChatMessageKind::User,
                        author: "peer".to_string(),
                        body: "right back atcha".to_string(),
                        attachments: Vec::new(),
                        reactions: Vec::new(),
                        reactors: Vec::new(),
                        is_outgoing: false,
                        created_at_secs: 110,
                        expires_at_secs: None,
                        delivery: DeliveryState::Received,
                        recipient_deliveries: Vec::new(),
                        delivery_trace: Default::default(),
                        source_event_id: None,
                    },
                ],

                draft: String::new(),
            },
        );
        core.threads.insert(
            group_chat.clone(),
            ThreadRecord {
                chat_id: group_chat.clone(),
                unread_count: 0,
                updated_at_secs: 50,
                messages: vec![ChatMessageSnapshot {
                    id: "g-system".to_string(),
                    chat_id: group_chat.clone(),
                    kind: ChatMessageKind::System,
                    author: owner.public_key().to_hex(),
                    body: "Group created: Brunch".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 50,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                }],

                draft: String::new(),
            },
        );
        core.seen_event_order.push_back("evt-1".to_string());
        core.seen_event_order.push_back("evt-2".to_string());
        core.seen_event_ids = core.seen_event_order.iter().cloned().collect();
        core.preferences.send_typing_indicators = true;
        core.preferences.nearby_bluetooth_enabled = true;
        core.preferences.nearby_lan_enabled = true;

        core.persist_best_effort_inner();
    }

    // New AppCore over the same directory: load_persisted should return
    // exactly what we wrote.
    let mut restarted = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str,
        Arc::new(RwLock::new(AppState::empty())),
    );
    let loaded = restarted
        .load_persisted()
        .expect("load_persisted")
        .expect("state persisted");
    assert_eq!(loaded.next_message_id, 17);
    assert_eq!(loaded.active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(loaded.preferences.send_typing_indicators);
    assert!(loaded.preferences.nearby_bluetooth_enabled);
    assert!(loaded.preferences.nearby_lan_enabled);
    assert_eq!(loaded.threads.len(), 2);
    let dm_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .expect("dm thread present");
    assert_eq!(dm_thread.messages.len(), 2);
    assert_eq!(dm_thread.unread_count, 3);
    assert_eq!(dm_thread.messages[0].body, "hello world");
    assert_eq!(dm_thread.messages[1].body, "right back atcha");
    let group_thread = loaded
        .threads
        .iter()
        .find(|thread| thread.chat_id == group_chat)
        .expect("group thread present");
    assert!(matches!(
        group_thread.messages[0].kind,
        ChatMessageKind::System
    ));
    assert_eq!(loaded.groups.len(), 1);
    assert_eq!(loaded.groups[0].name, "Brunch");
    assert_eq!(loaded.app_keys.len(), 1);
    assert_eq!(loaded.seen_event_ids, vec!["evt-1", "evt-2"]);
    assert!(matches!(
        loaded.authorization_state,
        Some(PersistedAuthorizationState::Authorized)
    ));

    // Assert nothing was persisted in the legacy JSON layout.
    let legacy_meta = std::path::Path::new(&restarted.data_dir)
        .join("core")
        .join("meta.json");
    assert!(
        !legacy_meta.exists(),
        "legacy core/meta.json must not be created"
    );
}

#[test]
fn appcore_clear_persistence_drops_sqlite_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir_str = temp_dir.path().to_string_lossy().to_string();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir_str.clone(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.next_message_id = 5;
    core.persist_best_effort_inner();
    assert!(core.load_persisted().unwrap().is_some());

    core.clear_persistence_best_effort();
    assert!(core.load_persisted().unwrap().is_none());
}

#[test]
fn profile_picture_upload_propagates_to_account_snapshot() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("profile-picture-upload", &owner, &device);
    core.rebuild_state();
    assert!(core.state.account.is_some(), "account snapshot exists");
    assert!(
        core.state.account.as_ref().unwrap().picture_url.is_none(),
        "no picture before upload"
    );

    let picture_url = "https://cdn.iris.to/abc123".to_string();
    core.handle_profile_picture_upload_finished(Ok(picture_url.clone()));

    let account = core.state.account.as_ref().expect("account after upload");
    assert_eq!(
        account.picture_url.as_deref(),
        Some(picture_url.as_str()),
        "picture url propagated to account snapshot"
    );
}

#[test]
fn delete_chat_removes_thread_and_navigates_back() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("delete-chat", &owner, &device);
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 100,
            messages: vec![ChatMessageSnapshot {
                id: "m1".to_string(),
                chat_id: chat_id.clone(),
                kind: ChatMessageKind::User,
                author: chat_id.clone(),
                body: "hi".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                reactors: Vec::new(),
                is_outgoing: false,
                created_at_secs: 100,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
                recipient_deliveries: Vec::new(),
                delivery_trace: Default::default(),
                source_event_id: None,
            }],

            draft: String::new(),
        },
    );
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 3600);
    core.preferences.pinned_chat_ids.push(chat_id.clone());
    core.active_chat_id = Some(chat_id.clone());
    core.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];

    core.handle_action(AppAction::DeleteChat {
        chat_id: chat_id.clone(),
    });

    assert!(!core.threads.contains_key(&chat_id), "thread removed");
    assert!(
        !core.chat_message_ttl_seconds.contains_key(&chat_id),
        "ttl cleared"
    );
    assert!(
        !core
            .preferences
            .pinned_chat_ids
            .iter()
            .any(|pinned| pinned == &chat_id),
        "pinned state cleared"
    );
    assert!(core.active_chat_id.is_none(), "active chat cleared");
    assert!(
        !core
            .screen_stack
            .iter()
            .any(|s| matches!(s, Screen::Chat { chat_id: cid } if cid == &chat_id)),
        "chat screen popped"
    );
    assert!(
        !core
            .state
            .chat_list
            .iter()
            .any(|chat| chat.chat_id == chat_id),
        "chat_list snapshot reflects removal"
    );
}

#[test]
fn pinning_chat_moves_it_above_newer_unpinned_chats_and_persists() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("pin-chat", &owner, &device);
    let older_chat_id = Keys::generate().public_key().to_hex();
    let newer_chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        older_chat_id.clone(),
        ThreadRecord {
            chat_id: older_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(
                &older_chat_id,
                "older",
                "older",
                10,
                false,
            )],

            draft: String::new(),
        },
    );
    core.threads.insert(
        newer_chat_id.clone(),
        ThreadRecord {
            chat_id: newer_chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 20,
            messages: vec![test_chat_message(
                &newer_chat_id,
                "newer",
                "newer",
                20,
                false,
            )],

            draft: String::new(),
        },
    );
    core.rebuild_state();
    assert_eq!(core.state.chat_list[0].chat_id, newer_chat_id);

    core.handle_action(AppAction::SetChatPinned {
        chat_id: older_chat_id.clone(),
        pinned: true,
    });

    assert_eq!(core.state.chat_list[0].chat_id, older_chat_id);
    assert!(core.state.chat_list[0].is_pinned);
    assert_eq!(core.state.chat_list[1].chat_id, newer_chat_id);
    assert!(!core.state.chat_list[1].is_pinned);
    let loaded = core
        .load_persisted()
        .expect("load persisted")
        .expect("state persisted");
    assert_eq!(loaded.preferences.pinned_chat_ids, vec![older_chat_id]);
}

#[test]
fn set_chat_unread_toggles_local_unread_count() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("set-chat-unread", &owner, &device);
    let chat_id = Keys::generate().public_key().to_hex();
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: vec![test_chat_message(&chat_id, "m1", "hello", 10, false)],

            draft: String::new(),
        },
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: true,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 1);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        1
    );

    core.handle_action(AppAction::SetChatUnread {
        chat_id: chat_id.clone(),
        unread: false,
    });

    assert_eq!(core.threads.get(&chat_id).unwrap().unread_count, 0);
    assert_eq!(
        core.state
            .chat_list
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .expect("chat snapshot")
            .unread_count,
        0
    );
}

#[test]
fn redelivered_persisted_message_after_restart_does_not_increment_unread() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("redelivered-persisted-message", &owner, &device);
    let old_message = ChatMessageSnapshot {
        id: "old-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        body: "already read".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 100,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-old".to_string()),
    };
    let latest_message = ChatMessageSnapshot {
        id: "latest-message".to_string(),
        chat_id: chat_id.clone(),
        kind: ChatMessageKind::User,
        author: chat_id.clone(),
        body: "latest preview".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing: false,
        created_at_secs: 200,
        expires_at_secs: None,
        delivery: DeliveryState::Seen,
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: Some("outer-latest".to_string()),
    };
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 200,
            messages: vec![old_message, latest_message.clone()],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    // Restart restores only a preview for inactive chats. Catch-up can then
    // redeliver older stored events that are not in memory anymore.
    let thread = core.threads.get_mut(&chat_id).expect("thread");
    thread.messages = vec![latest_message];
    thread.unread_count = 0;
    core.active_chat_id = None;
    core.screen_stack.clear();

    core.push_incoming_message_from(
        &chat_id,
        Some("old-message".to_string()),
        "already read".to_string(),
        100,
        None,
        Some(chat_id.clone()),
        Some("outer-old".to_string()),
    );

    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(stored_message_count(&core), 2);
}

#[test]
fn prune_expired_messages_removes_loaded_messages_and_sqlite_rows() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-prune", &owner, &device);
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 200,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 100,
                    expires_at_secs: Some(150),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 200,
                    expires_at_secs: Some(300),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    let removed = core.prune_expired_messages(200);

    assert_eq!(removed, 1);
    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread");
    assert_eq!(thread.unread_count, 1);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .messages
            .len(),
        1
    );
}

/// Regression for iOS RUNNINGBOARD 0xdead10cc crashes: a relay event
/// queued just before `PrepareForSuspend` (or one that races in from
/// the FFI channel during the suspend window) used to keep running
/// inside `handle_relay_event_with_channel` and write to SQLite,
/// which iOS' watchdog kills mid-`pwrite`. After this fix the gate
/// in `handle_internal` drops queued background work once
/// `PrepareForSuspend` has run, and clears on `AppForegrounded`.
#[test]
fn suspend_gate_drops_internal_events_until_foregrounded() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        temp_dir.path().to_string_lossy().to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );

    // DebugLog is a simple internal-event signal: the handler appends
    // to `core.debug_log`, no other code path mutates it, and it's
    // not pruned by `rebuild_state` like typing indicators are.
    let send_debug_log = |core: &mut AppCore, detail: &str| {
        core.handle_message(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
            category: "test".to_string(),
            detail: detail.to_string(),
        })));
    };
    let log_count = |core: &AppCore| -> usize {
        core.debug_log
            .iter()
            .filter(|entry| entry.category == "test")
            .count()
    };

    // Sanity: an internal event before suspend lands in debug_log.
    send_debug_log(&mut core, "before-suspend");
    assert_eq!(log_count(&core), 1, "DebugLog must land before suspend");

    // Engage the gate via the real CoreMsg path that iOS uses.
    let (reply_tx, _reply_rx) = flume::bounded(1);
    core.handle_message(CoreMsg::PrepareForSuspend(reply_tx));

    // While suspended, internal events must be dropped — the gate is
    // what keeps SQLite from being written while iOS is killing us.
    send_debug_log(&mut core, "during-suspend");
    assert_eq!(
        log_count(&core),
        1,
        "suspend gate must drop internal events"
    );

    // Foregrounding lifts the gate; the next internal event is processed.
    core.handle_message(CoreMsg::Action(AppAction::AppForegrounded));
    send_debug_log(&mut core, "after-foreground");
    assert_eq!(
        log_count(&core),
        2,
        "after AppForegrounded, internal events flow again"
    );
}

#[test]
fn internal_prune_expired_messages_event_ignores_stale_tokens_and_updates_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let chat_id = peer.public_key().to_hex();
    let mut core = logged_in_test_core("message-expiry-internal-event", &owner, &device);
    let now = unix_now().get();
    core.active_chat_id = Some(chat_id.clone());
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 1,
            updated_at_secs: now,
            messages: vec![
                ChatMessageSnapshot {
                    id: "expired".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "gone".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now.saturating_sub(20),
                    expires_at_secs: Some(now.saturating_sub(1)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
                ChatMessageSnapshot {
                    id: "future".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: chat_id.clone(),
                    body: "stays".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: now,
                    expires_at_secs: Some(now.saturating_add(3600)),
                    delivery: DeliveryState::Received,
                    recipient_deliveries: Vec::new(),
                    delivery_trace: Default::default(),
                    source_event_id: None,
                },
            ],

            draft: String::new(),
        },
    );
    core.persist_best_effort_inner();
    assert_eq!(stored_message_count(&core), 2);

    core.handle_prune_expired_messages(core.message_expiry_token.wrapping_add(1));

    assert_eq!(stored_message_count(&core), 2, "stale token ignored");
    assert_eq!(
        core.threads
            .get(&chat_id)
            .expect("thread after stale token")
            .messages
            .len(),
        2
    );

    let valid_token = core.message_expiry_token;
    core.handle_prune_expired_messages(valid_token);

    assert_eq!(stored_message_count(&core), 1);
    let thread = core.threads.get(&chat_id).expect("thread after prune");
    assert_eq!(thread.unread_count, 0);
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "stays");
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat after prune")
            .messages
            .len(),
        1
    );
}

fn logged_in_test_core(label: &str, owner: &Keys, device: &Keys) -> AppCore {
    logged_in_test_core_at_data_dir(
        owner,
        device,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
    )
}

fn logged_in_test_core_with_storage(
    label: &str,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        std::env::temp_dir()
            .join(format!(
                "iris-chat-rs-test-{label}-{}",
                owner.public_key().to_hex()
            ))
            .to_string_lossy()
            .to_string(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite = Invite::create_new(device.public_key(), Some(device_id.clone()), None)
        .expect("local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn logged_in_test_core_at_data_dir(owner: &Keys, device: &Keys, data_dir: String) -> AppCore {
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        data_dir,
        Arc::new(RwLock::new(AppState::empty())),
    );
    let device_id = device.public_key().to_hex();
    let invite = Invite::create_new(device.public_key(), Some(device_id.clone()), None)
        .expect("local invite");
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
        local_invite: invite,
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(&mut core, owner, device, storage, None, None);
    core
}

fn test_chat_message(
    chat_id: &str,
    id: &str,
    body: &str,
    created_at_secs: u64,
    is_outgoing: bool,
) -> ChatMessageSnapshot {
    ChatMessageSnapshot {
        id: id.to_string(),
        chat_id: chat_id.to_string(),
        kind: ChatMessageKind::User,
        author: chat_id.to_string(),
        body: body.to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        reactors: Vec::new(),
        is_outgoing,
        created_at_secs,
        expires_at_secs: None,
        delivery: if is_outgoing {
            DeliveryState::Sent
        } else {
            DeliveryState::Received
        },
        recipient_deliveries: Vec::new(),
        delivery_trace: Default::default(),
        source_event_id: None,
    }
}

fn test_protocol_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    test_protocol_engine_with_storage(owner, device, storage)
}

fn test_protocol_engine_with_storage(
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> ProtocolEngine {
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    let session_manager =
        SessionManager::new(local_owner, device.secret_key().to_secret_bytes()).snapshot();
    let group_manager = NostrGroupManager::new(local_owner).snapshot();
    ProtocolEngine::load_or_seed(
        storage,
        owner.public_key(),
        device,
        local_invite,
        session_manager,
        group_manager,
    )
    .expect("protocol engine")
}

fn observe_current_device_appkeys_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
) {
    let created_at = unix_now().get();
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("local appkeys");
}

fn observe_peer_device_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    created_at: u64,
) {
    engine
        .ingest_app_keys_snapshot(
            owner.public_key(),
            AppKeys::new(vec![DeviceEntry::new(device.public_key(), created_at)]),
            created_at,
        )
        .expect("peer appkeys");
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(created_at), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(device.public_key()),
        Some(ndr_owner_pubkey(owner.public_key())),
        None,
    )
    .expect("peer invite");
    let event = nostr_double_ratchet_nostr::invite_unsigned_event(&invite)
        .expect("invite event")
        .sign_with_keys(device)
        .expect("signed invite");
    engine
        .observe_invite_event(&event)
        .expect("observe peer invite");
}

fn protocol_payload_events_for_result<'a>(
    effects: &'a [ProtocolEffect],
    event_ids: &[String],
) -> Vec<&'a Event> {
    let event_ids = event_ids.iter().cloned().collect::<HashSet<_>>();
    protocol_effect_events(effects)
        .into_iter()
        .filter(|event| event_ids.contains(&event.id.to_string()))
        .collect()
}

fn protocol_effect_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .flat_map(|effect| match effect {
            ProtocolEffect::PublishSigned(event) => vec![event],
            ProtocolEffect::PublishSignedForInnerEvent { event, .. } => vec![event],
            ProtocolEffect::PublishStagedFirstContact { bootstrap, payload } => bootstrap
                .iter()
                .chain(payload)
                .map(|publish| &publish.event)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn protocol_targeted_payload_count(effects: &[ProtocolEffect], owner_pubkey_hex: &str) -> usize {
    effects
        .iter()
        .map(|effect| match effect {
            ProtocolEffect::PublishSignedForInnerEvent {
                target_owner_pubkey_hex,
                ..
            } if target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex) => 1,
            ProtocolEffect::PublishStagedFirstContact { payload, .. } => payload
                .iter()
                .filter(|publish| {
                    publish.target_owner_pubkey_hex.as_deref() == Some(owner_pubkey_hex)
                })
                .count(),
            _ => 0,
        })
        .sum()
}

fn install_test_protocol_engine(
    core: &mut AppCore,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
    seed_session_manager: Option<SessionManagerSnapshot>,
    seed_group_manager: Option<GroupManagerSnapshot>,
) {
    let local_invite = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .local_invite
        .clone();
    let seed_session_manager = seed_session_manager.unwrap_or_else(|| {
        SessionManager::new(
            NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
            device.secret_key().to_secret_bytes(),
        )
        .snapshot()
    });
    let seed_group_manager = seed_group_manager.unwrap_or_else(|| {
        NostrGroupManager::new(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())).snapshot()
    });
    core.protocol_engine = Some(
        ProtocolEngine::load_or_seed(
            storage,
            owner.public_key(),
            device,
            local_invite,
            seed_session_manager,
            seed_group_manager,
        )
        .expect("protocol engine"),
    );
}

fn stored_message_count(core: &AppCore) -> i64 {
    let conn = core.app_store.shared();
    let count = conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    count
}

fn stored_message_expiration(core: &AppCore, chat_id: &str, message_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let expires_at: Option<i64> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT expires_at_secs FROM messages WHERE chat_id = ?1 AND id = ?2",
            rusqlite::params![chat_id, message_id],
            |row| row.get(0),
        )
        .unwrap();
    expires_at.map(|secs| secs as u64)
}

fn stored_chat_ttl(core: &AppCore, chat_id: &str) -> Option<u64> {
    let conn = core.app_store.shared();
    let conn = conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT ttl_seconds FROM chat_message_ttls WHERE chat_id = ?1")
        .unwrap();
    let mut rows = stmt.query([chat_id]).unwrap();
    rows.next()
        .unwrap()
        .map(|row| row.get::<_, i64>(0).unwrap() as u64)
}

fn runtime_state_json(core: &AppCore, owner: &Keys, device: &Keys) -> serde_json::Value {
    let storage = crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        owner.public_key().to_hex(),
        device.public_key().to_hex(),
    );
    let value = storage
        .get("appcore/protocol-engine-state-v1")
        .expect("read appcore protocol state")
        .expect("appcore protocol state exists");
    serde_json::from_str(&value).expect("runtime state json")
}

fn stored_pending_group_sender_key_message_count(
    core: &AppCore,
    owner: &Keys,
    device: &Keys,
) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_group_sender_key_messages")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn stored_pending_decrypted_delivery_count(core: &AppCore, owner: &Keys, device: &Keys) -> usize {
    runtime_state_json(core, owner, device)
        .get("pending_decrypted_deliveries")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default()
}

fn unknown_group_sender_key_outer_event(sender_event: &Keys) -> Event {
    use base64::Engine;

    let mut payload = Vec::new();
    payload.extend_from_slice(&7_u32.to_be_bytes());
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&[42_u8; 32]);
    let content = base64::engine::general_purpose::STANDARD.encode(payload);
    EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), content)
        .sign_with_keys(sender_event)
        .expect("unknown group sender-key outer")
}

fn delivered_texts() -> &'static std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>> {
    static DELIVERED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<usize, Vec<String>>>,
    > = std::sync::OnceLock::new();
    DELIVERED.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn runtime_key(runtime: &NdrRuntime) -> usize {
    runtime as *const NdrRuntime as usize
}

fn deliver_published_events(from: &NdrRuntime, signer: &Keys, to: &NdrRuntime) {
    for event in drain_signed_events(from, signer) {
        deliver_event_to_runtime(to, event);
    }
}

fn deliver_runtime_effects(
    from: &NdrRuntime,
    signer: &Keys,
    effects: Vec<SessionManagerEvent>,
    to: &NdrRuntime,
) {
    apply_runtime_persist_effects(from, &effects);
    let events = signed_events_from_effects(effects, signer);
    for event in &events {
        deliver_event_to_runtime(to, event.clone());
    }
    for event in events {
        from.ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(from, &from.drain_events());
    }
}

fn accept_invite_and_deliver(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    invite: &Invite,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .accept_invite(invite, Some(inviter_pubkey))
        .expect("accept invite");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn deliver_event_to_runtime(to: &NdrRuntime, event: Event) {
    to.process_received_event(event);
    let effects = to.drain_events();
    apply_runtime_persist_effects(to, &effects);
    let mut messages = Vec::new();
    for effect in effects {
        if let SessionManagerEvent::DecryptedMessage { content, .. } = effect {
            messages.push(
                serde_json::from_str::<UnsignedEvent>(&content)
                    .ok()
                    .map(|event| event.content)
                    .unwrap_or(content),
            );
        }
    }
    if !messages.is_empty() {
        delivered_texts()
            .lock()
            .unwrap()
            .entry(runtime_key(to))
            .or_default()
            .extend(messages);
    }
}

fn apply_runtime_persist_effects(_runtime: &NdrRuntime, _effects: &[SessionManagerEvent]) {
    // Runtime persistence is internal. This helper keeps existing simulated
    // relay-delivery tests readable where they previously modeled app steps.
}

fn pending_events_with_kind(core: &AppCore, kind: u32) -> Vec<Event> {
    core.pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .filter(|event| event.kind.as_u16() as u32 == kind)
        .collect()
}

fn complete_first_contact(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    acceptor
        .send_text(
            inviter_pubkey,
            "__ndr_first_contact_bootstrap__".to_string(),
            None,
        )
        .expect("first-contact bootstrap send");
    deliver_runtime_effects(acceptor, acceptor_keys, acceptor.drain_events(), inviter);
}

fn signed_events_from_effects(effects: Vec<SessionManagerEvent>, signer: &Keys) -> Vec<Event> {
    effects
        .into_iter()
        .filter_map(|event| match event {
            SessionManagerEvent::Publish(unsigned) if unsigned.pubkey == signer.public_key() => {
                unsigned.sign_with_keys(signer).ok()
            }
            SessionManagerEvent::PublishSigned(event) => Some(event),
            SessionManagerEvent::PublishSignedForInnerEvent { event, .. } => Some(event),
            _ => None,
        })
        .collect()
}

fn drain_signed_events(runtime: &NdrRuntime, signer: &Keys) -> Vec<Event> {
    let mut effects = runtime.drain_events();
    if effects.is_empty() {
        runtime.reload_from_storage().expect("reload runtime");
        effects.extend(runtime.drain_events());
    }
    let mut seen = HashSet::new();
    let events = signed_events_from_effects(effects, signer)
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .collect::<Vec<_>>();
    for event in &events {
        runtime
            .ack_prepared_publish(&event.id.to_string())
            .expect("ack prepared publish");
        apply_runtime_persist_effects(runtime, &runtime.drain_events());
    }
    events
}

fn serializable_key_pair_for_test(keys: &Keys) -> nostr_double_ratchet::SerializableKeyPair {
    nostr_double_ratchet::SerializableKeyPair {
        public_key: NdrDevicePubkey::from_bytes(keys.public_key().to_bytes()),
        private_key: keys.secret_key().to_secret_bytes(),
    }
}

fn compact_event_payload_for_apns_test(event: &Event) -> serde_json::Value {
    let mut value = serde_json::to_value(event).expect("event json");
    if let Some(object) = value.as_object_mut() {
        let header_tags = object
            .get("tags")
            .and_then(|tags| tags.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|tag| {
                tag.as_array()
                    .and_then(|items| items.first())
                    .and_then(|name| name.as_str())
                    == Some("header")
            })
            .collect();
        object.insert("tags".to_string(), serde_json::Value::Array(header_tags));
    }
    value
}

fn drain_text_messages(runtime: &NdrRuntime) -> Vec<String> {
    delivered_texts()
        .lock()
        .unwrap()
        .remove(&runtime_key(runtime))
        .unwrap_or_default()
}

/// End-to-end round-trip: upload a real image to the hashtree network and
/// verify the same bytes can be read back via the same path the iOS shell
/// uses. Marked `ignore` because it depends on external network reachability.
/// Run manually with: cargo test profile_picture_hashtree_roundtrip --ignored -- --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn profile_picture_hashtree_roundtrip() {
    let owner = Keys::generate();
    let secret_hex = owner.secret_key().to_secret_hex();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ios/UITests/Fixtures/cat.jpg");
    let url = super::attachment_upload::upload_profile_picture_to_hashtree(&secret_hex, &path)
        .await
        .expect("upload");
    let nhash = url.strip_prefix("htree://").expect("htree:// prefix");
    let b64 = super::attachment_upload::download_hashtree_attachment_base64(nhash)
        .await
        .expect("download bytes");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("b64 decode");
    let original = std::fs::read(&path).expect("read original");
    assert_eq!(bytes, original, "downloaded bytes match original");
}
