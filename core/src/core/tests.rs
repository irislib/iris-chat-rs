use super::*;
use nostr_double_ratchet_runtime::{NdrRuntime, RuntimeEffect};

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
        core.protocol_subscription_runtime
            .active_subscriptions
            .is_empty(),
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
        !core
            .protocol_subscription_runtime
            .active_subscriptions
            .is_empty(),
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
        !core
            .protocol_subscription_runtime
            .active_subscriptions
            .is_empty(),
        "logged-in session should derive protocol subscriptions"
    );
    let first_token = core.protocol_reconnect_token;
    let first_due = core
        .protocol_subscription_runtime
        .liveness_due_at
        .expect("refresh should schedule liveness");

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
fn liveness_retries_protocol_backfill_for_tracked_peer_with_roster_but_no_session() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let peer_device = Keys::generate();
    let unrelated_author = Keys::generate();
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

    let unrelated_filter = Filter::new()
        .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
        .authors(vec![unrelated_author.public_key()]);
    core.direct_message_subscriptions.register_subscription(
        "ndr-runtime-messages",
        serde_json::to_string(&unrelated_filter).expect("filter json"),
    );
    assert!(
        !core
            .direct_message_subscriptions
            .tracked_authors()
            .is_empty(),
        "the regression requires other active message authors"
    );
    assert!(
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .message_author_pubkeys_for_owner(peer_owner.public_key())
            .is_empty(),
        "peer roster without invite response must not have message authors yet"
    );
    core.upsert_protocol_subscription(
        "ndr-runtime-protocol".to_string(),
        Filter::new().kind(Kind::from(APP_KEYS_EVENT_KIND as u16)),
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
fn relay_connect_helper_retries_from_clean_disconnect_when_all_relays_stay_offline() {
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
        body.contains("client.disconnect().await"),
        "connect helper must force a clean reconnect when every relay remains offline"
    );
    assert!(
        body.contains("client.try_connect"),
        "connect helper must use nostr-sdk's timed connect path instead of a bare connect"
    );
}

#[test]
fn runtime_publish_uses_durable_relay_connect_helper() {
    let publish_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/publish_helpers.rs"),
    )
    .expect("read publish helpers source");
    let start = publish_source
        .find("async fn ensure_publish_connection")
        .expect("publish connection helper");
    let body = &publish_source[start
        ..publish_source[start..]
            .find("\npub(super) async fn publish_event_once")
            .map(|offset| start + offset)
            .unwrap_or(publish_source.len())];
    assert!(
        body.contains("ensure_session_relays_configured(client, relay_urls).await"),
        "publish connection helper must configure relays before sending"
    );
    assert!(
        body.contains("connect_client_with_timeout"),
        "publish connection helper must share the durable relay connect helper"
    );
    assert!(
        !body.contains("Duration::from_millis(500)"),
        "publish connection helper must not give Android public relays only 500ms to connect"
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
fn zero_connected_session_check_schedules_fast_reconnect() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    let start = protocol_source
        .find("pub(super) fn handle_relay_connection_checked")
        .expect("connection checked handler");
    let body = &protocol_source[start
        ..protocol_source[start..]
            .find("\n    pub(super) fn refresh_relay_connection_status")
            .map(|offset| start + offset)
            .unwrap_or(protocol_source.len())];
    assert!(
        body.contains("else if configured_relay_count > 0")
            && body.contains("PROTOCOL_RECONNECT_CHECK_SECS")
            && body.contains("schedule_protocol_subscription_liveness_check"),
        "zero-connected startup checks must schedule a fast reconnect/liveness pass"
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
            .find("\n    pub(super) fn upsert_protocol_subscription")
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
fn invite_response_observation_emits_targeted_owner_message_backfill() {
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

    let batch = engine
        .observe_invite_response_event(&response_event)
        .expect("observe invite response");

    assert!(
        !engine
            .message_author_pubkeys_for_owner(peer_owner.public_key())
            .is_empty(),
        "observing the invite response should install receiver state for the peer"
    );
    assert!(
        batch.effects.iter().any(|effect| matches!(
            effect,
            ProtocolEffect::FetchRecentMessagesForOwner {
                owner_pubkey,
                reason,
                ..
            } if *owner_pubkey == peer_owner.public_key() && *reason == "invite_response"
        )),
        "learning a peer from an invite response must trigger owner-targeted message backfill"
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
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device_id,
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
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
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device_id,
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
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
    let bob_shared_conn = crate::core::storage::open_database(&data_dir).expect("bob db");
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        bob_shared_conn.clone(),
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
        Some(bob_storage.clone()),
        None,
    );
    bob.init().expect("bob init");
    accept_invite_and_deliver(&bob, &bob_keys, &invite, alice_keys.public_key(), &alice);
    complete_first_contact(&bob, &bob_keys, alice_keys.public_key(), &alice);

    let user_record_key = "v2/runtime-state";
    let before = bob_storage
        .get(&user_record_key)
        .expect("read stored ratchet before notification")
        .expect("stored ratchet before notification");

    let message = "closed-app preview stays read-only";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let published_events = drain_signed_events(&alice, &alice_keys);
    let message_event = published_events
        .iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "message event for Bob; published={}",
                serde_json::to_string(&published_events).unwrap_or_default()
            )
        });
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
        .get(&user_record_key)
        .expect("read stored ratchet after notification")
        .expect("stored ratchet after notification");
    assert_eq!(
        before, after,
        "notification preview must not advance persisted ratchet state"
    );

    let bob_restarted_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        crate::core::storage::open_database(&data_dir).expect("restarted db"),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    let bob_restarted = NdrRuntime::new(
        bob_keys.public_key(),
        bob_keys.secret_key().to_secret_bytes(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key(),
        Some(bob_restarted_storage),
        None,
    );
    bob_restarted.init().expect("bob restarted init");
    deliver_event_to_runtime(&bob_restarted, message_event);
    assert!(
        drain_text_messages(&bob_restarted)
            .iter()
            .any(|body| body == message),
        "foreground runtime must still decrypt the relay event after notification preview"
    );
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

    let message = "compacted apns preview";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let bob_message_authors = bob.get_all_message_push_author_pubkeys();
    let message_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Bob");
    let payload = serde_json::json!({
        "aps": {
            "alert": {
                "title": "Iris Chat",
                "body": "New message",
            },
            "mutable-content": 1,
        },
        "event": compact_event_payload_for_apns_test(&message_event),
        "title": "New message",
        "body": "New message",
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
}

#[test]
fn mobile_push_payload_ingest_feeds_full_event_into_runtime() {
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
        &invite,
        alice_keys.public_key(),
        &alice,
    );
    complete_first_contact(&bob_runtime, &bob_keys, alice_keys.public_key(), &alice);

    let message = "push-only event";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let bob_message_authors = bob_runtime.get_all_message_push_author_pubkeys();
    let message_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Bob");
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
    let storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        core.app_store.shared(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
    install_test_protocol_engine(
        &mut core,
        &bob_keys,
        &bob_keys,
        storage,
        Some(bob_runtime.session_manager_snapshot()),
        Some(bob_runtime.group_manager_snapshot()),
    );

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
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device.public_key().to_hex(),
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
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
fn mobile_push_preview_survives_foreground_batch_ratchet_race() {
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let bob_shared_conn = crate::core::storage::open_database(&data_dir).expect("bob db");
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        bob_shared_conn.clone(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut alice_invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("alice invite");
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
        Some(bob_storage.clone()),
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
    let bob_message_authors = bob_runtime.get_all_message_push_author_pubkeys();
    let user_record_key = "v2/runtime-state";
    let legacy_runtime_before = bob_storage
        .get(&user_record_key)
        .expect("read bob ratchet before message")
        .expect("bob ratchet before message");

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
    let protocol_state_before = runtime_state_json(&core, &bob_keys, &bob_keys).to_string();
    core.owner_profiles.insert(
        alice_keys.public_key().to_hex(),
        OwnerProfileRecord {
            name: Some("alice".to_string()),
            display_name: Some("Alice".to_string()),
            picture: None,
            updated_at_secs: 1,
        },
    );

    let message = "foreground batch preview";
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let published_events = drain_signed_events(&alice, &alice_keys);
    let message_event = published_events
        .iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "message event for Bob; published={}",
                serde_json::to_string(&published_events).unwrap_or_default()
            )
        });
    let payload = serde_json::json!({
        "event": message_event,
        "title": "Iris Chat",
        "body": "New activity",
    })
    .to_string();

    core.enter_batch();
    core.handle_relay_event(message_event);
    assert!(
        core.batch_dirty_persist,
        "full state save should still be deferred inside the core batch"
    );
    let protocol_state_after = runtime_state_json(&core, &bob_keys, &bob_keys).to_string();
    assert_ne!(
        protocol_state_before, protocol_state_after,
        "foreground AppCore protocol engine should consume the relay event before the push handler runs"
    );
    let legacy_runtime_after = bob_storage
        .get(&user_record_key)
        .expect("read bob ratchet after message")
        .expect("bob ratchet after message");
    assert_eq!(
        legacy_runtime_before, legacy_runtime_after,
        "foreground AppCore handling must not mutate legacy runtime storage"
    );

    let resolution = decrypt_mobile_push_notification(
        data_dir.to_string_lossy().to_string(),
        bob_keys.public_key().to_hex(),
        bob_keys
            .secret_key()
            .to_bech32()
            .unwrap_or_else(|_| bob_keys.secret_key().to_secret_hex()),
        payload,
    );
    core.exit_batch();

    assert!(resolution.should_show);
    assert_eq!(resolution.title, "Alice");
    assert_eq!(resolution.body, message);
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
    let bob_shared_conn = crate::core::storage::open_database(&data_dir).expect("bob db");
    let bob_storage = Arc::new(crate::core::storage::SqliteStorageAdapter::new(
        bob_shared_conn.clone(),
        bob_keys.public_key().to_hex(),
        bob_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;

    let mut alice_invite = Invite::create_new(
        alice_keys.public_key(),
        Some(alice_keys.public_key().to_hex()),
        Some(1),
    )
    .expect("alice invite");
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
        Some(bob_storage.clone()),
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
    let bob_message_authors = bob_runtime.get_all_message_push_author_pubkeys();

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
    alice
        .send_text(bob_keys.public_key(), message.to_string(), None)
        .expect("alice sends");
    let message_event = drain_signed_events(&alice, &alice_keys)
        .into_iter()
        .find(|event| {
            event.kind.as_u16() == MESSAGE_EVENT_KIND as u16
                && bob_message_authors.contains(&event.pubkey)
        })
        .expect("message event for Bob");

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
        LocalAuthorizationState::Authorized
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
    let active_filters = core
        .protocol_subscription_runtime
        .active_subscriptions
        .values()
        .map(|subscription| subscription.filter.clone())
        .collect::<Vec<_>>();
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
fn recent_protocol_filters_include_bootstrap_message_backfill_for_cold_tracked_peer() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("protocol-backfill-cold-peer", &owner, &device);
    core.active_chat_id = Some(peer.public_key().to_hex());

    let filters = core.recent_protocol_filters(UnixSeconds(1_777_159_500));
    let bootstrap_filter = filters
        .iter()
        .map(|filter| serde_json::to_value(filter).expect("filter json"))
        .find(|filter| {
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
        .expect("bootstrap message backfill filter");

    assert_eq!(
        bootstrap_filter
            .get("since")
            .and_then(|since| since.as_u64()),
        Some(1_777_159_500 - NEW_MESSAGE_AUTHOR_BACKFILL_LOOKBACK_SECS)
    );
}

#[test]
fn pending_inbound_direct_events_keep_bootstrap_message_backfill_active() {
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
        core.protocol_engine
            .as_ref()
            .expect("protocol engine")
            .has_pending_inbound_direct_events(),
        "the unresolved relay event must be durable pending inbound work"
    );

    assert!(
        has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "pending inbound work must keep broad message discovery active until it can be applied"
    );
}

#[test]
fn direct_message_discovery_bootstrap_backfill_remains_for_partial_tracked_peer_state() {
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
        has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "message backfill must stay bootstrapped while any tracked peer has incomplete author state"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = core
        .protocol_subscription_runtime
        .active_subscriptions
        .values()
        .map(|subscription| subscription.filter.clone())
        .collect::<Vec<_>>();
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
        has_bootstrap_message_filter(&core.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "cold tracked peer still needs bounded message backfill"
    );

    let relay = crate::local_relay::TestRelay::start();
    let relay_urls = relay_urls_from_strings(&[relay.url().to_string()]);
    core.preferences.nostr_relay_urls = vec![relay.url().to_string()];
    core.logged_in.as_mut().expect("logged in").relay_urls = relay_urls;

    core.request_protocol_subscription_refresh_forced();
    let active_filters = core
        .protocol_subscription_runtime
        .active_subscriptions
        .values()
        .map(|subscription| subscription.filter.clone())
        .collect::<Vec<_>>();
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
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty())
            || has_bootstrap_message_filter(&bob.recent_protocol_filters(UnixSeconds(1_777_159_500))),
        "sending through a private invite must bootstrap message discovery until the creator author is known"
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
            .is_some_and(|engine| !engine.known_message_author_pubkeys().is_empty())
            || has_bootstrap_message_filter(
                &alice.recent_protocol_filters(UnixSeconds(1_777_159_500))
            ),
        "private invite response import must immediately enable peer message discovery"
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
    let inner_id = "a".repeat(64);
    let first_outer_id = "b".repeat(64);
    let second_outer_id = "c".repeat(64);
    let content = serde_json::json!({
        "content": "ok",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_493u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": inner_id,
    })
    .to_string();

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
fn remote_runtime_rumor_pubkey_and_p_tags_do_not_choose_direct_chat() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-remote-route", &owner, &device);
    let inner_id = "1".repeat(64);
    let content = serde_json::json!({
        "content": "route must stay on authenticated sender",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_493u64,
        "tags": [["p", forged_peer.public_key().to_hex()], ["recipient-owner", forged_peer.public_key().to_hex()]],
        "pubkey": forged_peer.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

    core.apply_decrypted_runtime_message(sender.public_key(), None, content, Some("2".repeat(64)));

    let sender_chat_id = sender.public_key().to_hex();
    let forged_chat_id = forged_peer.public_key().to_hex();
    assert!(core
        .threads
        .get(&sender_chat_id)
        .is_some_and(|thread| thread.messages.iter().any(|message| message.id == inner_id)));
    assert!(
        !core.threads.contains_key(&forged_chat_id),
        "remote plaintext p/pubkey hints must not create or select a forged peer chat"
    );
}

#[test]
fn self_sync_runtime_metadata_overrides_malicious_inner_p_tag() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let real_peer = Keys::generate();
    let forged_peer = Keys::generate();
    let mut core = logged_in_test_core("runtime-forged-self-sync-route", &owner, &device);
    let inner_id = "3".repeat(64);
    let content = serde_json::json!({
        "content": "self sync route comes from runtime metadata",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_500u64,
        "tags": [["p", forged_peer.public_key().to_hex()]],
        "pubkey": forged_peer.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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
    let inner_id = "8".repeat(64);
    let content = serde_json::json!({
        "content": "sent from primary device",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_501u64,
        "tags": [["p", peer_chat_id]],
        "pubkey": owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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
    let inner_id = "a".repeat(64);
    let content = serde_json::json!({
        "content": "sent from peer device",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_502u64,
        "tags": [],
        "pubkey": peer_owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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

    let inner_id = "c".repeat(64);
    let content = serde_json::json!({
        "content": "sent before app keys backfill",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_503u64,
        "tags": [],
        "pubkey": peer_owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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
    let content = serde_json::json!({
        "content": "seen",
        "kind": RECEIPT_KIND,
        "created_at": 1_777_159_493u64,
        "tags": [["e", message_id], ["p", forged_peer.public_key().to_hex()]],
        "pubkey": forged_peer.public_key().to_hex(),
        "id": "6".repeat(64),
    })
    .to_string();

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
fn self_synced_direct_message_is_rendered_as_outgoing_on_linked_device() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sibling_device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-missing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let inner_id = "d".repeat(64);
    let content = serde_json::json!({
        "content": "sent from sibling",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_500u64,
        "tags": [["p", chat_id], ["ms", "1777159500123"]],
        "pubkey": owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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
fn self_synced_direct_message_updates_existing_local_outgoing_without_duplicate() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer = Keys::generate();
    let mut core = logged_in_test_core("self-sync-existing-outgoing", &owner, &device);
    let chat_id = peer.public_key().to_hex();
    let inner_id = "f".repeat(64);
    core.push_outgoing_message_with_id(
        inner_id.clone(),
        &chat_id,
        "local optimistic".to_string(),
        1_777_159_499,
        None,
        DeliveryState::Pending,
    );
    let content = serde_json::json!({
        "content": "sent from this device",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_500u64,
        "tags": [["p", chat_id], ["ms", "1777159500123"]],
        "pubkey": owner.public_key().to_hex(),
        "id": inner_id,
    })
    .to_string();

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
    let content = serde_json::json!({
        "content": "typing",
        "kind": TYPING_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [["ms", "1777159483368"], ["expiration", "1777159543"]],
        "pubkey": "0".repeat(64),
        "id": "d".repeat(64),
    })
    .to_string();

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
    let content = serde_json::json!({
        "content": "typing",
        "kind": TYPING_KIND,
        "created_at": 1_777_159_484u64,
        "tags": [["expiration", "1"]],
        "pubkey": "0".repeat(64),
        "id": "a".repeat(64),
    })
    .to_string();

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
        let content = serde_json::json!({
            "content": body,
            "kind": kind,
            "created_at": 1_777_159_483u64 + index as u64,
            "tags": tags,
            "pubkey": "0".repeat(64),
            "id": format!("{:064x}", index + 10),
        })
        .to_string();

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
    let content = serde_json::json!({
        "content": "60",
        "kind": CHAT_SETTINGS_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": "f".repeat(64),
    })
    .to_string();

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
    let set_content = serde_json::json!({
        "content": serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        }).to_string(),
        "kind": CHAT_SETTINGS_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": "a".repeat(64),
    })
    .to_string();

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

    let clear_content = serde_json::json!({
        "content": "0",
        "kind": CHAT_SETTINGS_KIND,
        "created_at": 1_777_159_484u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": "c".repeat(64),
    })
    .to_string();
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
    let inner_id = "e".repeat(64);
    let content = serde_json::json!({
        "content": "secret",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [["expiration", "1777159543"]],
        "pubkey": "0".repeat(64),
        "id": inner_id,
    })
    .to_string();

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
fn prerelease_app_plaintext_controls_settings_reactions_and_expiration_flow() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let sender = Keys::generate();
    let mut core = logged_in_test_core("prerelease-app-plaintext-flow", &owner, &device);
    let chat_id = sender.public_key().to_hex();
    let message_id = "1".repeat(64);

    let message_content = serde_json::json!({
        "content": "pre-release app message",
        "kind": CHAT_MESSAGE_KIND,
        "created_at": 1_777_159_483u64,
        "tags": [["expiration", "1777159543"]],
        "pubkey": "0".repeat(64),
        "id": message_id,
    })
    .to_string();
    core.apply_decrypted_runtime_message(
        sender.public_key(),
        None,
        message_content,
        Some("2".repeat(64)),
    );

    let thread = core.threads.get(&chat_id).expect("thread after message");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "pre-release app message");
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

    let settings_content = serde_json::json!({
        "content": serde_json::json!({
            "type": "chat-settings",
            "v": 1,
            "messageTtlSeconds": 3600u64,
        }).to_string(),
        "kind": CHAT_SETTINGS_KIND,
        "created_at": 1_777_159_485u64,
        "tags": [],
        "pubkey": "0".repeat(64),
        "id": "3".repeat(64),
    })
    .to_string();
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

    core.apply_group_decrypted_event(GroupIncomingEvent::Message(
        nostr_double_ratchet::GroupReceivedMessage {
            group_id,
            sender_owner: ndr_owner_pubkey(sender_owner.public_key()),
            sender_device: Some(ndr_device_pubkey(sender_device.public_key())),
            body: b"group secret".to_vec(),
            revision: 1,
        },
    ));

    let thread = core.threads.get(&chat_id).expect("group thread");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].body, "group secret");
    assert_eq!(thread.messages[0].expires_at_secs, None);
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
        let runtime = NdrRuntime::new(
            device.public_key(),
            device.secret_key().to_secret_bytes(),
            device.public_key().to_hex(),
            owner.public_key(),
            None,
            Some(invite.clone()),
        );
        runtime.init().expect("runtime init");
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
    let runtime = NdrRuntime::new(
        device.public_key(),
        device.secret_key().to_secret_bytes(),
        device.public_key().to_hex(),
        owner.public_key(),
        None,
        Some(invite.clone()),
    );
    runtime.init().expect("runtime init");
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
        },
    );
    core.chat_message_ttl_seconds.insert(chat_id.clone(), 3600);
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

fn test_protocol_engine(owner: &Keys, device: &Keys) -> ProtocolEngine {
    let storage =
        Arc::new(nostr_double_ratchet_runtime::InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
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
        .or_else(|| storage.get("v2/runtime-state").expect("read runtime state"))
        .expect("protocol state exists");
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
    effects: Vec<RuntimeEffect>,
    to: &NdrRuntime,
) {
    apply_runtime_persist_effects(from, &effects);
    let events = signed_events_from_effects(effects, signer);
    for event in &events {
        deliver_event_to_runtime(to, event.clone());
    }
    for event in events {
        if let Ok(effects) = from.ack_prepared_publish(&event.id.to_string()) {
            apply_runtime_persist_effects(from, &effects);
        }
    }
}

fn accept_invite_and_deliver(
    acceptor: &NdrRuntime,
    acceptor_keys: &Keys,
    invite: &Invite,
    inviter_pubkey: PublicKey,
    inviter: &NdrRuntime,
) {
    let result = acceptor
        .accept_invite(invite, Some(inviter_pubkey))
        .expect("accept invite");
    deliver_runtime_effects(acceptor, acceptor_keys, result.effects, inviter);
}

fn deliver_event_to_runtime(to: &NdrRuntime, event: Event) {
    let Ok(effects) = to.process_received_event(event) else {
        return;
    };
    apply_runtime_persist_effects(to, &effects);
    let mut messages = Vec::new();
    for effect in effects {
        if let RuntimeEffect::EmitDecrypted { content, .. } = effect {
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

fn apply_runtime_persist_effects(_runtime: &NdrRuntime, _effects: &[RuntimeEffect]) {
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
    let result = acceptor
        .send_text(
            inviter_pubkey,
            "__ndr_first_contact_bootstrap__".to_string(),
            None,
        )
        .expect("first-contact bootstrap send");
    deliver_runtime_effects(acceptor, acceptor_keys, result.effects, inviter);
}

fn signed_events_from_effects(effects: Vec<RuntimeEffect>, signer: &Keys) -> Vec<Event> {
    effects
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEffect::PublishUnsigned(unsigned) if unsigned.pubkey == signer.public_key() => {
                unsigned.sign_with_keys(signer).ok()
            }
            RuntimeEffect::PublishSigned(event) => Some(event),
            RuntimeEffect::PublishSignedForInnerEvent { event, .. } => Some(event),
            _ => None,
        })
        .collect()
}

fn drain_signed_events(runtime: &NdrRuntime, signer: &Keys) -> Vec<Event> {
    let mut effects = runtime.prepared_publish_effects();
    if effects.is_empty() {
        effects = runtime.reload_from_storage().unwrap_or_default();
        effects.extend(runtime.prepared_publish_effects());
    }
    let mut seen = HashSet::new();
    let events = signed_events_from_effects(effects, signer)
        .into_iter()
        .filter(|event| seen.insert(event.id))
        .collect::<Vec<_>>();
    for event in &events {
        if let Ok(effects) = runtime.ack_prepared_publish(&event.id.to_string()) {
            apply_runtime_persist_effects(runtime, &effects);
        }
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
