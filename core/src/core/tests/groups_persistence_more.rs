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
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
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
        author_owner_pubkey_hex: Some(chat_id.clone()),
        author_picture_url: None,
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
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
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
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
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
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
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
                    author_owner_pubkey_hex: Some(chat_id.clone()),
                    author_picture_url: None,
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
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner.clone()),
        device_keys: device.clone(),
        client: Client::new(device.clone()),
        relay_urls: Vec::new(),
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
        author_owner_pubkey_hex: Some(chat_id.to_string()),
        author_picture_url: None,
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
        Arc::new(InMemoryStorage::new()) as Arc<dyn StorageAdapter>;
    test_protocol_engine_with_storage(owner, device, storage)
}

fn test_protocol_engine_with_storage(
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
) -> ProtocolEngine {
    let local_owner = NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes());
    let local_invite = stable_local_invite_for_test(owner, device);
    let mut session_manager =
        SessionManager::new(local_owner, device.secret_key().to_secret_bytes()).snapshot();
    session_manager.local_invite = Some(local_invite);
    let group_manager = GroupEventManager::new(local_owner).snapshot();
    seed_protocol_storage_if_missing_for_test(storage.as_ref(), session_manager, group_manager)
        .expect("seed protocol state");
    ProtocolEngine::load_or_create_for_local_device(
        storage,
        owner.public_key(),
        device,
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

fn observe_peer_appkeys_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    devices: &[PublicKey],
    created_at: u64,
) {
    let event = AppKeys::new(
        devices
            .iter()
            .copied()
            .map(|device| DeviceEntry::new(device, created_at))
            .collect(),
    )
    .get_event_at(owner.public_key(), created_at)
    .sign_with_keys(owner)
    .expect("signed peer appkeys");
    engine
        .ingest_app_keys_event(&event)
        .expect("peer appkeys event");
}

fn observe_peer_device_invite_for_test(
    engine: &mut ProtocolEngine,
    owner: &Keys,
    device: &Keys,
    created_at: u64,
) {
    observe_peer_appkeys_for_test(engine, owner, &[device.public_key()], created_at);
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(NdrUnixSeconds(created_at), &mut rng);
    let invite = Invite::create_new_with_context(
        &mut ctx,
        ndr_device_pubkey(device.public_key()),
        Some(ndr_owner_pubkey(owner.public_key())),
        None,
    )
    .expect("peer invite");
    let event = nostr_double_ratchet::invite_unsigned_event(&invite)
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
    protocol_publish_events(effects)
}

fn latest_sender_key_distribution_for_test(
    engine: &ProtocolEngine,
    group_id: &str,
    created_at: NdrUnixSeconds,
) -> nostr_double_ratchet::SenderKeyDistribution {
    let sender_key = engine
        .group_manager_snapshot_for_test()
        .sender_keys
        .into_iter()
        .find(|record| record.group_id == group_id)
        .expect("sender-key record for group");
    let key_id = sender_key.latest_key_id.expect("latest sender key id");
    let state = sender_key
        .states
        .iter()
        .find(|state| state.key_id() == key_id)
        .expect("sender-key state");
    nostr_double_ratchet::SenderKeyDistribution {
        group_id: group_id.to_string(),
        key_id,
        sender_event_pubkey: sender_key.sender_event_pubkey,
        chain_key: state.chain_key(),
        iteration: state.iteration(),
        created_at,
    }
}

fn install_test_protocol_engine(
    core: &mut AppCore,
    owner: &Keys,
    device: &Keys,
    storage: Arc<dyn StorageAdapter>,
    seed_session_manager: Option<SessionManagerSnapshot>,
    seed_group_manager: Option<GroupManagerSnapshot>,
) {
    let local_invite = stable_local_invite_for_test(owner, device);
    let mut seed_session_manager = seed_session_manager.unwrap_or_else(|| {
        SessionManager::new(
            NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes()),
            device.secret_key().to_secret_bytes(),
        )
        .snapshot()
    });
    if seed_session_manager.local_invite.is_none() {
        seed_session_manager.local_invite = Some(local_invite);
    }
    let seed_group_manager = seed_group_manager.unwrap_or_else(|| {
        GroupEventManager::new(NdrOwnerPubkey::from_bytes(owner.public_key().to_bytes())).snapshot()
    });
    seed_protocol_storage_if_missing_for_test(
        storage.as_ref(),
        seed_session_manager,
        seed_group_manager,
    )
    .expect("seed protocol state");
    core.protocol_engine = Some(
        ProtocolEngine::load_or_create_for_local_device(
            storage,
            owner.public_key(),
            device,
        )
        .expect("protocol engine"),
    );
}

fn stable_local_invite_for_test(owner: &Keys, device: &Keys) -> Invite {
    let mut invite = Invite::create_new(
        device.public_key(),
        Some(device.public_key().to_hex()),
        None,
    )
    .expect("local invite");
    invite.inviter_owner_pubkey = Some(ndr_owner_pubkey(owner.public_key()));
    invite.owner_public_key = Some(owner.public_key());
    invite
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

fn pending_events_with_kind(core: &AppCore, kind: u32) -> Vec<Event> {
    core.pending_relay_publishes
        .values()
        .filter_map(|pending| serde_json::from_str::<Event>(&pending.event_json).ok())
        .filter(|event| event.kind.as_u16() as u32 == kind)
        .collect()
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
