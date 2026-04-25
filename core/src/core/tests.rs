use super::*;
use crate::core::chats::{
    apply_incoming_reaction, reaction_notification_body, toggle_local_reaction,
};
use crate::local_relay::{matches_filter, TestRelay};
use crate::FfiApp;
use nostr_double_ratchet::AuthorizedDevice;
use nostr_sdk::prelude::SecretKey;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tempfile::TempDir;

fn relay_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct RelayEnvGuard {
    previous: Option<String>,
}

impl RelayEnvGuard {
    fn local_only() -> Self {
        Self::custom("ws://127.0.0.1:4848")
    }

    fn custom(value: &str) -> Self {
        let previous = std::env::var("NDR_DEMO_RELAYS").ok();
        std::env::set_var("NDR_DEMO_RELAYS", value);
        Self { previous }
    }
}

impl Drop for RelayEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("NDR_DEMO_RELAYS", previous);
        } else {
            std::env::remove_var("NDR_DEMO_RELAYS");
        }
    }
}

fn nsec_for_fill(secret_fill: u8) -> String {
    let keys = Keys::new(SecretKey::from_slice(&[secret_fill; 32]).expect("secret key"));
    keys.secret_key().to_bech32().expect("nsec")
}

fn npub_for_fill(secret_fill: u8) -> String {
    let keys = Keys::new(SecretKey::from_slice(&[secret_fill; 32]).expect("secret key"));
    keys.public_key().to_bech32().expect("npub")
}

fn pubkey_hex_for_fill(secret_fill: u8) -> String {
    let keys = Keys::new(SecretKey::from_slice(&[secret_fill; 32]).expect("secret key"));
    keys.public_key().to_hex()
}

fn device_fill_for_owner(secret_fill: u8) -> u8 {
    secret_fill.wrapping_add(100)
}

fn wait_for_state(
    app: &Arc<FfiApp>,
    label: &str,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    wait_for_state_timeout(app, label, 15, predicate)
}

fn wait_for_state_timeout(
    app: &Arc<FfiApp>,
    label: &str,
    timeout_secs: u64,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    let deadline = Instant::now() + StdDuration::from_secs(timeout_secs);
    let mut last = app.state();

    while Instant::now() < deadline {
        last = app.state();
        if predicate(&last) {
            return last;
        }
        thread::sleep(StdDuration::from_millis(100));
    }

    panic!("timed out waiting for {label}: last state = {:?}", last);
}

fn app_with_dir(data_dir: &Path, secret_fill: u8) -> Arc<FfiApp> {
    let app = FfiApp::new(
        data_dir.to_string_lossy().into_owned(),
        String::new(),
        "test".to_string(),
    );
    app.dispatch(AppAction::RestoreAccountBundle {
        owner_nsec: Some(nsec_for_fill(secret_fill)),
        owner_pubkey_hex: pubkey_hex_for_fill(secret_fill),
        device_nsec: nsec_for_fill(device_fill_for_owner(secret_fill)),
    });
    wait_for_state(&app, "account restore", |state| {
        state.account.is_some() && !state.busy.restoring_session
    });
    app
}

fn app(secret_fill: u8) -> (TempDir, Arc<FfiApp>) {
    let temp_dir = TempDir::new().expect("temp dir");
    let app = app_with_dir(temp_dir.path(), secret_fill);
    (temp_dir, app)
}

fn test_core(data_dir: &Path) -> AppCore {
    let (update_tx, _update_rx) = flume::unbounded();
    let (core_tx, _core_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));
    AppCore::new(
        update_tx,
        core_tx,
        data_dir.to_string_lossy().into_owned(),
        shared_state,
    )
}

fn persisted_state(data_dir: &Path) -> PersistedState {
    let bytes = fs::read(data_dir.join("ndr_demo_core_state.json")).expect("read persisted state");
    serde_json::from_slice(&bytes).expect("parse persisted state")
}

fn wait_for_persisted_state(
    data_dir: &Path,
    label: &str,
    predicate: impl Fn(&PersistedState) -> bool,
) -> PersistedState {
    let deadline = Instant::now() + StdDuration::from_secs(15);
    while Instant::now() < deadline {
        if data_dir.join("ndr_demo_core_state.json").exists() {
            let state = persisted_state(data_dir);
            if predicate(&state) {
                return state;
            }
        }
        thread::sleep(StdDuration::from_millis(100));
    }
    panic!("timed out waiting for persisted state: {label}");
}

fn pending_publish_event_ids(core: &AppCore, message_id: &str) -> (Vec<String>, Vec<String>) {
    let pending = core
        .pending_outbound
        .iter()
        .find(|pending| pending.message_id == message_id)
        .expect("pending outbound");
    let batch = pending
        .prepared_publish
        .as_ref()
        .expect("prepared publish batch");
    (
        batch
            .invite_events
            .iter()
            .map(|event| event.id.to_string())
            .collect(),
        batch
            .message_events
            .iter()
            .map(|event| event.id.to_string())
            .collect(),
    )
}

fn keys_for_fill(secret_fill: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[secret_fill; 32]).expect("secret key"))
}

fn device_keys_for_fill(secret_fill: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[secret_fill.wrapping_add(100); 32]).expect("secret key"))
}

fn start_primary_test_session(
    core: &mut AppCore,
    owner_fill: u8,
    allow_restore: bool,
    allow_protocol_restore: bool,
) -> anyhow::Result<()> {
    core.start_primary_session(
        keys_for_fill(owner_fill),
        device_keys_for_fill(owner_fill),
        allow_restore,
        allow_protocol_restore,
    )
}

fn established_session_manager_pair(
    alice_fill: u8,
    bob_fill: u8,
    base_secs: u64,
) -> (SessionManager, SessionManager, String) {
    let alice_owner_keys = keys_for_fill(alice_fill);
    let bob_owner_keys = keys_for_fill(bob_fill);
    let alice_device_keys = device_keys_for_fill(alice_fill);
    let bob_device_keys = device_keys_for_fill(bob_fill);
    let alice_owner = local_owner_from_keys(&alice_owner_keys);
    let bob_owner = local_owner_from_keys(&bob_owner_keys);
    let alice_device = local_device_from_keys(&alice_device_keys);
    let bob_device = local_device_from_keys(&bob_device_keys);
    let now = UnixSeconds(base_secs);

    let mut alice_manager = SessionManager::new(
        alice_owner,
        alice_device_keys.secret_key().to_secret_bytes(),
    );
    let mut bob_manager =
        SessionManager::new(bob_owner, bob_device_keys.secret_key().to_secret_bytes());

    let alice_roster = DeviceRoster::new(now, vec![AuthorizedDevice::new(alice_device, now)]);
    let bob_roster = DeviceRoster::new(now, vec![AuthorizedDevice::new(bob_device, now)]);
    alice_manager.apply_local_roster(alice_roster.clone());
    bob_manager.apply_local_roster(bob_roster.clone());
    alice_manager.observe_peer_roster(bob_owner, bob_roster.clone());
    bob_manager.observe_peer_roster(alice_owner, alice_roster);

    let bob_invite = {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(base_secs + 1), &mut rng);
        bob_manager
            .ensure_local_invite(&mut ctx)
            .expect("ensure bob invite")
            .clone()
    };
    alice_manager
        .observe_device_invite(bob_owner, bob_invite)
        .expect("observe bob invite");

    let prepared = {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(base_secs + 2), &mut rng);
        alice_manager
            .prepare_send(&mut ctx, bob_owner, b"bootstrap".to_vec())
            .expect("prepare bootstrap message")
    };
    assert_eq!(prepared.invite_responses.len(), 1);
    assert_eq!(prepared.deliveries.len(), 1);

    {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(base_secs + 3), &mut rng);
        bob_manager
            .observe_invite_response(&mut ctx, &prepared.invite_responses[0])
            .expect("observe invite response")
            .expect("processed invite response");
    }
    {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(base_secs + 4), &mut rng);
        bob_manager
            .receive(&mut ctx, alice_owner, &prepared.deliveries[0].envelope)
            .expect("receive bootstrap delivery")
            .expect("bootstrap message");
    }

    (alice_manager, bob_manager, bob_owner.to_string())
}

fn logged_in_core_with_manager(
    data_dir: &Path,
    secret_fill: u8,
    session_manager: SessionManager,
) -> AppCore {
    let owner_keys = keys_for_fill(secret_fill);
    let device_keys = device_keys_for_fill(secret_fill);
    let mut core = test_core(data_dir);
    core.state.account = Some(AccountSnapshot {
        public_key_hex: owner_keys.public_key().to_hex(),
        npub: owner_keys.public_key().to_bech32().expect("npub"),
        display_name: owner_keys.public_key().to_bech32().expect("npub"),
        picture_url: None,
        device_public_key_hex: device_keys.public_key().to_hex(),
        device_npub: device_keys.public_key().to_bech32().expect("device npub"),
        has_owner_signing_authority: true,
        authorization_state: DeviceAuthorizationState::Authorized,
    });
    core.logged_in = Some(LoggedInState {
        owner_pubkey: local_owner_from_keys(&owner_keys),
        owner_keys: Some(owner_keys),
        device_keys: device_keys.clone(),
        client: Client::new(device_keys),
        relay_urls: configured_relay_urls(),
        session_manager,
        group_manager: GroupManager::new(local_owner_from_keys(&keys_for_fill(secret_fill))),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    core.rebuild_state();
    core
}

fn publish_local_relay_event(keys: &Keys, event: Event) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("publish runtime");
    runtime.block_on(async {
        let client = Client::new(keys.clone());
        let relay_urls = vec![RelayUrl::parse("ws://127.0.0.1:4848").expect("relay url")];
        ensure_session_relays_configured(&client, &relay_urls).await;
        client.connect().await;
        publish_event_with_retry(&client, &relay_urls, event, "test publish")
            .await
            .expect("publish event");
        let _ = client.shutdown().await;
    });
}

fn fetch_local_relay_events(filters: Vec<Filter>) -> Vec<Event> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fetch runtime");
    runtime.block_on(async move {
        let client = Client::new(keys_for_fill(200));
        client
            .add_relay("ws://127.0.0.1:4848")
            .await
            .expect("add relay");
        client.connect_with_timeout(Duration::from_secs(5)).await;
        let events = client
            .fetch_events(filters, Some(Duration::from_secs(5)))
            .await
            .expect("fetch events");
        let collected = events.into_iter().collect::<Vec<_>>();
        let _ = client.shutdown().await;
        collected
    })
}

#[test]
fn relay_filter_matches_pubkey_tags_and_time_bounds() {
    let event = json!({
        "pubkey": "aaaaaaaa",
        "kind": codec::INVITE_RESPONSE_KIND,
        "created_at": 100,
        "tags": [["p", "deadbeef"], ["d", "double-ratchet/invites/test"]],
    });

    assert!(matches_filter(
        &event,
        &json!({
            "kinds": [codec::INVITE_RESPONSE_KIND],
            "#p": ["deadbeef"],
            "since": 90,
            "until": 110,
        }),
    ));
    assert!(!matches_filter(
        &event,
        &json!({
            "kinds": [codec::INVITE_RESPONSE_KIND],
            "#p": ["cafebabe"],
        }),
    ));
    assert!(!matches_filter(
        &event,
        &json!({
            "kinds": [codec::INVITE_RESPONSE_KIND],
            "#p": ["deadbeef"],
            "since": 101,
        }),
    ));
}

#[test]
fn normalize_and_validate_peer_input_accepts_expected_forms() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let npub = npub_for_fill(11);
    let (hex, _) = parse_peer_input(&npub).expect("parse npub");
    let profile = nostr::nips::nip19::Nip19Profile::new(
        PublicKey::parse(&npub).expect("profile public key"),
        Vec::<&str>::new(),
    )
    .expect("profile");
    let nprofile = profile.to_bech32().expect("nprofile");

    assert_eq!(normalize_peer_input_for_display(&npub), npub);
    assert_eq!(
        normalize_peer_input_for_display(&format!("nostr:{npub}")),
        npub
    );
    assert_eq!(
        normalize_peer_input_for_display(&format!("https://chat.iris.to/\n#{npub}")),
        npub
    );
    assert_eq!(
        normalize_peer_input_for_display(&format!("https://chat.iris.to/#/{nprofile}")),
        npub
    );
    assert_eq!(normalize_peer_input_for_display(&hex), hex);
    assert!(parse_peer_input(&format!("nostr:{hex}")).is_ok());
    assert!(parse_peer_input(&format!("https://chat.iris.to/#/{nprofile}")).is_ok());
    assert!(parse_peer_input("not-a-key").is_err());
}

#[test]
fn start_session_restores_persisted_threads_only_when_enabled() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let chat_id = parse_peer_input(&npub_for_fill(22))
        .expect("peer chat id")
        .0;

    let mut seeded = test_core(data_dir.path());
    start_primary_test_session(&mut seeded, 21, false, false).expect("start session");
    seeded.remember_event("event-1".to_string());
    seeded.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 2,
            updated_at_secs: 55,
            messages: vec![ChatMessageSnapshot {
                id: "1".to_string(),
                chat_id: chat_id.clone(),
                author: "peer".to_string(),
                body: "restored".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                is_outgoing: false,
                created_at_secs: 55,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
            }],
        },
    );
    seeded.active_chat_id = Some(chat_id.clone());
    seeded.screen_stack = vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }];
    seeded.rebuild_state();
    seeded.persist_best_effort();

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 21, true, true).expect("restore session");
    assert_eq!(restored.active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert_eq!(restored.state.chat_list.len(), 1);
    assert_eq!(
        restored
            .state
            .current_chat
            .as_ref()
            .expect("current chat")
            .messages[0]
            .body,
        "restored"
    );
    assert!(restored.has_seen_event("event-1"));

    let mut fresh = test_core(data_dir.path());
    start_primary_test_session(&mut fresh, 21, false, false).expect("fresh session");
    assert!(fresh.state.chat_list.is_empty());
    assert!(fresh.active_chat_id.is_none());
    assert!(!fresh.has_seen_event("event-1"));
}

#[test]
fn local_reactions_and_deletes_are_core_state_and_persist() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let chat_id = parse_peer_input(&npub_for_fill(24))
        .expect("peer chat id")
        .0;

    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 23, false, false).expect("start session");
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 0,
            updated_at_secs: 60,
            messages: vec![
                ChatMessageSnapshot {
                    id: "1".to_string(),
                    chat_id: chat_id.clone(),
                    author: "peer".to_string(),
                    body: "react to me".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 55,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                },
                ChatMessageSnapshot {
                    id: "2".to_string(),
                    chat_id: chat_id.clone(),
                    author: "peer".to_string(),
                    body: "delete me".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 60,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                },
            ],
        },
    );
    core.update_screen_stack(vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }]);

    core.handle_action(AppAction::ToggleReaction {
        chat_id: chat_id.clone(),
        message_id: "1".to_string(),
        emoji: "❤️".to_string(),
    });
    let reactions = &core
        .state
        .current_chat
        .as_ref()
        .expect("current chat")
        .messages[0]
        .reactions;
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    assert_eq!(reactions[0].count, 1);
    assert!(reactions[0].reacted_by_me);

    core.handle_action(AppAction::DeleteLocalMessage {
        chat_id: chat_id.clone(),
        message_id: "2".to_string(),
    });
    let messages = &core
        .state
        .current_chat
        .as_ref()
        .expect("current chat")
        .messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "1");

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 23, true, true).expect("restore session");
    let restored_messages = &restored
        .state
        .current_chat
        .as_ref()
        .expect("restored current chat")
        .messages;
    assert_eq!(restored_messages.len(), 1);
    assert_eq!(restored_messages[0].id, "1");
    assert_eq!(restored_messages[0].reactions[0].emoji, "❤️");
    assert!(restored_messages[0].reactions[0].reacted_by_me);
}

#[test]
fn reaction_updates_aggregate_and_format_notification_text() {
    let mut message = ChatMessageSnapshot {
        id: "1".to_string(),
        chat_id: "chat".to_string(),
        author: "peer".to_string(),
        body: "Nice image".to_string(),
        attachments: Vec::new(),
        reactions: Vec::new(),
        is_outgoing: false,
        created_at_secs: 1,
        expires_at_secs: None,
        delivery: DeliveryState::Received,
    };

    assert!(apply_incoming_reaction(&mut message, "🔥"));
    toggle_local_reaction(&mut message, "🔥");
    assert_eq!(message.reactions.len(), 1);
    assert_eq!(message.reactions[0].emoji, "🔥");
    assert_eq!(message.reactions[0].count, 2);
    assert!(message.reactions[0].reacted_by_me);

    toggle_local_reaction(&mut message, "🔥");
    assert_eq!(message.reactions[0].count, 1);
    assert!(!message.reactions[0].reacted_by_me);

    assert_eq!(
        reaction_notification_body("🔥", "Nice image"),
        "Reaction 🔥 to \"Nice image\""
    );
    assert_eq!(reaction_notification_body("❤️", ""), "New reaction ❤️");
}

#[test]
fn receipts_advance_delivery_without_reverting() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    let chat_id = "chat";
    core.threads.insert(
        chat_id.to_string(),
        ThreadRecord {
            chat_id: chat_id.to_string(),
            unread_count: 1,
            updated_at_secs: 1,
            messages: vec![
                ChatMessageSnapshot {
                    id: "out".to_string(),
                    chat_id: chat_id.to_string(),
                    author: "me".to_string(),
                    body: "out".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    is_outgoing: true,
                    created_at_secs: 1,
                    expires_at_secs: None,
                    delivery: DeliveryState::Sent,
                },
                ChatMessageSnapshot {
                    id: "in".to_string(),
                    chat_id: chat_id.to_string(),
                    author: "peer".to_string(),
                    body: "in".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 2,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                },
            ],
        },
    );

    core.apply_receipt_to_messages(chat_id, &["out".to_string()], DeliveryState::Seen, false);
    core.apply_receipt_to_messages(
        chat_id,
        &["out".to_string()],
        DeliveryState::Received,
        false,
    );
    core.apply_receipt_to_messages(chat_id, &["in".to_string()], DeliveryState::Seen, true);

    let thread = core.threads.get(chat_id).expect("thread");
    assert!(matches!(thread.messages[0].delivery, DeliveryState::Seen));
    assert!(matches!(thread.messages[1].delivery, DeliveryState::Seen));
    assert_eq!(thread.unread_count, 0);
}

#[test]
fn typing_indicators_are_projected_and_expire() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    let chat_id = "chat";
    core.threads.insert(
        chat_id.to_string(),
        ThreadRecord {
            chat_id: chat_id.to_string(),
            unread_count: 0,
            updated_at_secs: 1,
            messages: Vec::new(),
        },
    );
    core.set_typing_indicator(chat_id.to_string(), "peer".to_string(), unix_now().get());
    core.rebuild_state();

    assert!(core.state.chat_list[0].is_typing);
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .map(|chat| chat.typing_indicators.len()),
        None
    );

    core.active_chat_id = Some(chat_id.to_string());
    core.rebuild_state();
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .typing_indicators
            .len(),
        1
    );

    for indicator in core.typing_indicators.values_mut() {
        indicator.expires_at_secs = 1;
    }
    core.rebuild_state();
    assert!(!core.state.chat_list[0].is_typing);
}

#[test]
fn typing_preference_updates_state_and_persists() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    assert!(core.state.preferences.send_typing_indicators);
    core.handle_action(AppAction::SetTypingIndicatorsEnabled { enabled: false });
    assert!(!core.state.preferences.send_typing_indicators);
    assert!(
        !persisted_state(data_dir.path())
            .preferences
            .send_typing_indicators
    );

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(!restored.state.preferences.send_typing_indicators);
}

#[test]
fn read_receipt_preference_updates_state_and_persists() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    assert!(core.state.preferences.send_read_receipts);
    core.handle_action(AppAction::SetReadReceiptsEnabled { enabled: false });
    assert!(!core.state.preferences.send_read_receipts);
    assert!(
        !persisted_state(data_dir.path())
            .preferences
            .send_read_receipts
    );

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(!restored.state.preferences.send_read_receipts);
}

#[test]
fn disabled_read_receipts_suppress_delivered_and_seen_controls() {
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, bob_chat_id) =
        established_session_manager_pair(24, 25, 1_900_000_000);
    let mut core = logged_in_core_with_manager(data_dir.path(), 24, alice_manager);
    core.handle_action(AppAction::SetReadReceiptsEnabled { enabled: false });

    let alice_chat_id = local_owner_from_keys(&keys_for_fill(24)).to_string();
    let bob_owner = local_owner_from_keys(&keys_for_fill(25));
    let payload =
        encode_app_direct_message_payload(&alice_chat_id, "remote-1", "hello").expect("payload");
    core.apply_decrypted_payload(bob_owner, &payload, 1_900_000_010, None)
        .expect("apply payload");

    let thread = core.threads.get(&bob_chat_id).expect("thread");
    assert_eq!(thread.messages.len(), 1);
    assert!(core.pending_outbound.is_empty());

    core.mark_messages_seen(&bob_chat_id, &["remote-1".to_string()]);

    let thread = core.threads.get(&bob_chat_id).expect("thread");
    assert!(matches!(thread.messages[0].delivery, DeliveryState::Seen));
    assert!(core.pending_outbound.is_empty());
}

#[test]
fn desktop_notification_preference_updates_state_and_persists() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    assert!(core.state.preferences.desktop_notifications_enabled);
    core.handle_action(AppAction::SetDesktopNotificationsEnabled { enabled: false });
    assert!(!core.state.preferences.desktop_notifications_enabled);
    assert!(
        !persisted_state(data_dir.path())
            .preferences
            .desktop_notifications_enabled
    );

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(!restored.state.preferences.desktop_notifications_enabled);
}

#[test]
fn startup_at_login_preference_updates_state_and_persists() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    assert!(!core.state.preferences.startup_at_login_enabled);
    core.handle_action(AppAction::SetStartupAtLoginEnabled { enabled: true });
    assert!(core.state.preferences.startup_at_login_enabled);
    assert!(
        persisted_state(data_dir.path())
            .preferences
            .startup_at_login_enabled
    );

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(restored.state.preferences.startup_at_login_enabled);
}

#[test]
fn nostr_relay_settings_validate_update_state_and_persist() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    core.handle_action(AppAction::AddNostrRelay {
        relay_url: " WSS://Relay.Example/ ".to_string(),
    });
    assert!(core
        .state
        .preferences
        .nostr_relay_urls
        .contains(&"wss://relay.example".to_string()));

    core.handle_action(AppAction::UpdateNostrRelay {
        old_relay_url: "wss://relay.example".to_string(),
        new_relay_url: "ws://LOCALHOST:4848/path".to_string(),
    });
    assert!(core
        .state
        .preferences
        .nostr_relay_urls
        .contains(&"ws://localhost:4848/path".to_string()));
    assert!(core
        .state
        .network_status
        .as_ref()
        .expect("network")
        .relay_urls
        .contains(&"ws://localhost:4848/path".to_string()));

    core.handle_action(AppAction::AddNostrRelay {
        relay_url: "https://relay.invalid".to_string(),
    });
    assert_eq!(
        core.state.toast.as_deref(),
        Some("Relay URL must use ws:// or wss://.")
    );

    let persisted = persisted_state(data_dir.path());
    assert!(persisted
        .preferences
        .nostr_relay_urls
        .contains(&"ws://localhost:4848/path".to_string()));

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(restored
        .state
        .preferences
        .nostr_relay_urls
        .contains(&"ws://localhost:4848/path".to_string()));

    restored.handle_action(AppAction::RemoveNostrRelay {
        relay_url: "ws://localhost:4848/path".to_string(),
    });
    assert!(!restored
        .state
        .preferences
        .nostr_relay_urls
        .contains(&"ws://localhost:4848/path".to_string()));
}

#[test]
fn image_proxy_preferences_update_state_and_persist() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 24, true, true).expect("start session");

    assert!(core.state.preferences.image_proxy_enabled);
    core.handle_action(AppAction::SetImageProxyEnabled { enabled: false });
    core.handle_action(AppAction::SetImageProxyUrl {
        url: " https://proxy.example ".to_string(),
    });
    core.handle_action(AppAction::SetImageProxyKeyHex {
        key_hex: "AA".repeat(32),
    });
    core.handle_action(AppAction::SetImageProxySaltHex {
        salt_hex: "BB".repeat(32),
    });

    let persisted = persisted_state(data_dir.path());
    assert!(!persisted.preferences.image_proxy_enabled);
    assert_eq!(
        persisted.preferences.image_proxy_url,
        "https://proxy.example"
    );
    assert_eq!(persisted.preferences.image_proxy_key_hex, "aa".repeat(32));
    assert_eq!(persisted.preferences.image_proxy_salt_hex, "bb".repeat(32));

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 24, true, true).expect("restore session");
    assert!(!restored.state.preferences.image_proxy_enabled);
    assert_eq!(
        restored.state.preferences.image_proxy_url,
        "https://proxy.example"
    );
    assert_eq!(
        restored.state.preferences.image_proxy_key_hex,
        "aa".repeat(32)
    );
    assert_eq!(
        restored.state.preferences.image_proxy_salt_hex,
        "bb".repeat(32)
    );

    restored.handle_action(AppAction::ResetImageProxySettings);
    assert!(restored.state.preferences.image_proxy_enabled);
    assert_eq!(
        restored.state.preferences.image_proxy_url,
        crate::image_proxy::DEFAULT_IMAGE_PROXY_URL
    );
}

#[test]
fn old_or_unversioned_persistence_is_ignored_after_schema_cut() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let peer_npub = npub_for_fill(32);
    let peer_hex = parse_peer_input(&peer_npub).expect("peer hex").0;

    let old_version = json!({
        "version": PERSISTED_STATE_VERSION - 1,
        "active_chat_id": peer_hex,
        "next_message_id": 2,
        "session_manager": null,
        "threads": [{
            "chat_id": peer_hex,
            "unread_count": 1,
            "updated_at_secs": 7,
            "messages": [{
                "id": "1",
                "chat_id": peer_hex,
                "author": peer_npub,
                "body": "old version",
                "is_outgoing": false,
                "created_at_secs": 7,
                "delivery": "Received"
            }]
        }]
    });
    fs::write(
        data_dir.path().join("ndr_demo_core_state.json"),
        serde_json::to_vec(&old_version).expect("old version json"),
    )
    .expect("write old version persistence");

    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 31, true, false).expect("ignore old version state");
    assert!(core.active_chat_id.is_none());
    assert!(core.state.chat_list.is_empty());

    let unversioned = json!({
        "active_chat_id": peer_hex,
        "next_message_id": 2,
        "session_manager": null,
        "threads": []
    });
    fs::write(
        data_dir.path().join("ndr_demo_core_state.json"),
        serde_json::to_vec(&unversioned).expect("unversioned json"),
    )
    .expect("write unversioned persistence");

    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 31, true, false).expect("ignore unversioned state");
    assert!(core.active_chat_id.is_none());
    assert!(core.state.chat_list.is_empty());
}

#[test]
fn malformed_current_persistence_is_ignored_without_crash() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let owner_hex = pubkey_hex_for_fill(33);
    let malformed = json!({
        "version": PERSISTED_STATE_VERSION,
        "active_chat_id": "group:bad-group",
        "next_message_id": 2,
        "session_manager": null,
        "group_manager": {
            "local_owner_pubkey": owner_hex,
            "groups": [{
                "group_id": "bad-group",
                "name": "BadGroup",
                "created_by": owner_hex,
                "members": [owner_hex],
                "admins": [owner_hex],
                "revision": 1,
                "created_at": 1_900_000_000u64,
                "updated_at": 1_900_000_000u64
            }]
        },
        "threads": []
    });
    fs::write(
        data_dir.path().join("ndr_demo_core_state.json"),
        serde_json::to_vec(&malformed).expect("malformed json"),
    )
    .expect("write malformed persistence");

    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 33, true, true).expect("ignore malformed current state");
    assert!(core.active_chat_id.is_none());
    assert!(core.state.chat_list.is_empty());
}

#[test]
fn update_screen_stack_opens_chat_and_clears_unread() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 40, false, false).expect("start session");
    let chat_id = parse_peer_input(&npub_for_fill(41))
        .expect("peer chat id")
        .0;
    core.threads.insert(
        chat_id.clone(),
        ThreadRecord {
            chat_id: chat_id.clone(),
            unread_count: 3,
            updated_at_secs: 100,
            messages: vec![ChatMessageSnapshot {
                id: "1".to_string(),
                chat_id: chat_id.clone(),
                author: "peer".to_string(),
                body: "hello".to_string(),
                attachments: Vec::new(),
                reactions: Vec::new(),
                is_outgoing: false,
                created_at_secs: 100,
                expires_at_secs: None,
                delivery: DeliveryState::Received,
            }],
        },
    );

    core.update_screen_stack(vec![Screen::Chat {
        chat_id: chat_id.clone(),
    }]);

    assert_eq!(core.active_chat_id.as_deref(), Some(chat_id.as_str()));
    assert_eq!(core.threads.get(&chat_id).expect("thread").unread_count, 0);
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .chat_id,
        chat_id
    );
}

#[test]
fn push_screen_allows_logged_out_onboarding_routes() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    core.push_screen(Screen::CreateAccount);
    assert!(matches!(core.state.router.default_screen, Screen::Welcome));
    assert!(matches!(
        core.state.router.screen_stack.as_slice(),
        [Screen::CreateAccount]
    ));

    core.push_screen(Screen::RestoreAccount);
    assert!(matches!(
        core.state.router.screen_stack.as_slice(),
        [Screen::RestoreAccount]
    ));

    core.push_screen(Screen::AddDevice);
    assert!(matches!(
        core.state.router.screen_stack.as_slice(),
        [Screen::AddDevice]
    ));
}

#[test]
fn update_screen_stack_logged_out_keeps_only_onboarding_routes() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    core.update_screen_stack(vec![
        Screen::CreateAccount,
        Screen::NewChat,
        Screen::RestoreAccount,
        Screen::ChatList,
        Screen::AddDevice,
    ]);

    assert!(matches!(core.state.router.default_screen, Screen::Welcome));
    assert_eq!(core.state.router.screen_stack.len(), 3);
    assert!(matches!(
        core.state.router.screen_stack[0],
        Screen::CreateAccount
    ));
    assert!(matches!(
        core.state.router.screen_stack[1],
        Screen::RestoreAccount
    ));
    assert!(matches!(
        core.state.router.screen_stack[2],
        Screen::AddDevice
    ));
    assert!(core.state.account.is_none());
    assert!(core.state.current_chat.is_none());
}

#[test]
fn logout_keeps_state_revision_monotonic() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 70, false, false).expect("start session");

    let rev_before_logout = core.state.rev;
    assert!(rev_before_logout > 0);
    assert!(core.state.account.is_some());

    core.logout();

    assert!(core.state.rev > rev_before_logout);
    assert!(matches!(core.state.router.default_screen, Screen::Welcome));
    assert!(core.state.router.screen_stack.is_empty());
    assert!(core.state.account.is_none());
    assert!(core.state.chat_list.is_empty());
    assert!(core.active_chat_id.is_none());
}

#[test]
fn protocol_subscription_uses_stable_id() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 69, false, false).expect("start session");

    let client = core.logged_in.as_ref().expect("logged in").client.clone();
    let protocol_id = SubscriptionId::new(PROTOCOL_SUBSCRIPTION_ID);

    let wait_for_subscription =
        |runtime: &tokio::runtime::Runtime, client: &Client, protocol_id: &SubscriptionId| {
            let deadline = Instant::now() + StdDuration::from_secs(5);
            while Instant::now() < deadline {
                let subscriptions = runtime.block_on(client.subscriptions());
                if subscriptions.contains_key(&protocol_id) {
                    return subscriptions;
                }
                thread::sleep(StdDuration::from_millis(50));
            }
            runtime.block_on(client.subscriptions())
        };

    let initial = wait_for_subscription(&core.runtime, &client, &protocol_id);
    assert_eq!(initial.len(), 1);
    assert!(initial.contains_key(&protocol_id));

    core.create_chat(&npub_for_fill(70));

    let after_create = wait_for_subscription(&core.runtime, &client, &protocol_id);
    assert_eq!(after_create.len(), 1);
    assert!(after_create.contains_key(&protocol_id));
}

#[test]
fn protocol_subscription_plan_sorts_roster_authors_stably() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 71, false, false).expect("start session");

    let owner_a = local_owner_from_keys(&keys_for_fill(90)).to_string();
    let owner_b = local_owner_from_keys(&keys_for_fill(89)).to_string();

    core.threads.insert(
        owner_a.clone(),
        ThreadRecord {
            chat_id: owner_a.clone(),
            unread_count: 0,
            updated_at_secs: 10,
            messages: Vec::new(),
        },
    );
    core.threads.insert(
        owner_b.clone(),
        ThreadRecord {
            chat_id: owner_b.clone(),
            unread_count: 0,
            updated_at_secs: 20,
            messages: Vec::new(),
        },
    );
    core.pending_outbound.push(PendingOutbound {
        message_id: "dup-owner-a".to_string(),
        chat_id: owner_a.clone(),
        body: "pending".to_string(),
        prepared_publish: None,
        publish_mode: OutboundPublishMode::WaitForPeer,
        reason: PendingSendReason::MissingRoster,
        next_retry_at_secs: 123,
        in_flight: false,
    });
    core.recent_handshake_peers.insert(
        "device-b".to_string(),
        RecentHandshakePeer {
            owner_hex: owner_b.clone(),
            device_hex: "device-b".to_string(),
            observed_at_secs: 50,
        },
    );

    let plan = core
        .compute_protocol_subscription_plan()
        .expect("subscription plan");
    let mut expected = vec![
        core.logged_in
            .as_ref()
            .expect("logged in")
            .owner_pubkey
            .to_string(),
        owner_a,
        owner_b,
    ];
    expected.sort();
    expected.dedup();

    assert_eq!(plan.roster_authors, expected);
}

#[test]
fn protocol_subscription_refresh_marks_dirty_while_refresh_is_in_flight() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 72, false, false).expect("start session");

    core.protocol_subscription_runtime.refresh_in_flight = true;
    core.protocol_subscription_runtime.refresh_dirty = false;
    let token_before = core.protocol_subscription_runtime.refresh_token;

    core.request_protocol_subscription_refresh();

    assert!(core.protocol_subscription_runtime.refresh_dirty);
    assert!(core.protocol_subscription_runtime.refresh_in_flight);
    assert_eq!(
        core.protocol_subscription_runtime.refresh_token,
        token_before
    );
}

#[test]
fn established_send_stays_pending_until_publish_ack() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, chat_id) =
        established_session_manager_pair(73, 74, 1_900_000_000);
    let mut core = logged_in_core_with_manager(data_dir.path(), 73, alice_manager);

    core.send_message(&chat_id, "offline direct send");

    let last_message = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat")
        .messages
        .last()
        .expect("outgoing message");
    assert_eq!(last_message.body, "offline direct send");
    assert!(matches!(last_message.delivery, DeliveryState::Pending));
    assert!(!core.state.busy.sending_message);
    assert_eq!(core.pending_outbound.len(), 1);
    assert_eq!(
        core.pending_outbound[0].publish_mode,
        OutboundPublishMode::OrdinaryFirstAck
    );
    assert_eq!(
        core.pending_outbound[0].reason,
        PendingSendReason::PublishRetry
    );
}

#[test]
fn incoming_messages_are_chronological_when_relay_events_arrive_out_of_order() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    let chat_id = "chat-peer";

    core.push_incoming_message_from(
        chat_id,
        None,
        "newer".to_string(),
        30,
        None,
        Some("peer".to_string()),
    );
    core.push_incoming_message_from(
        chat_id,
        None,
        "older".to_string(),
        10,
        None,
        Some("peer".to_string()),
    );
    core.push_incoming_message_from(
        chat_id,
        None,
        "middle".to_string(),
        20,
        None,
        Some("peer".to_string()),
    );
    core.rebuild_state();

    let thread = core.threads.get(chat_id).expect("thread");
    let bodies = thread
        .messages
        .iter()
        .map(|message| message.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, ["older", "middle", "newer"]);
    assert_eq!(thread.updated_at_secs, 30);

    let current = core
        .state
        .chat_list
        .iter()
        .find(|thread| thread.chat_id == chat_id)
        .expect("chat list thread");
    assert_eq!(current.last_message_preview.as_deref(), Some("newer"));
    assert_eq!(current.last_message_at_secs, Some(30));
}

#[test]
fn create_group_routes_into_group_chat() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, bob_owner_hex) =
        established_session_manager_pair(80, 81, 1_900_000_100);
    let mut core = logged_in_core_with_manager(data_dir.path(), 80, alice_manager);

    core.create_group("Trip crew", &[bob_owner_hex]);

    let current_chat = core.state.current_chat.as_ref().expect("current chat");
    assert!(current_chat.chat_id.starts_with(GROUP_CHAT_PREFIX));
    assert!(matches!(current_chat.kind, ChatKind::Group));
    assert_eq!(current_chat.display_name, "Trip crew");
    assert_eq!(current_chat.member_count, 2);
    assert_eq!(core.state.chat_list.len(), 1);
    assert_eq!(core.pending_group_controls.len(), 1);
}

#[test]
fn incoming_group_metadata_creates_group_thread() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let (mut alice_manager, bob_manager, bob_owner_hex) =
        established_session_manager_pair(82, 83, 1_900_000_200);
    let alice_owner = local_owner_from_keys(&keys_for_fill(82));
    let mut alice_groups = GroupManager::new(alice_owner);
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(UnixSeconds(1_900_000_201), &mut rng);
    let result = alice_groups
        .create_group(
            &mut alice_manager,
            &mut ctx,
            "Project".to_string(),
            vec![parse_owner_input(&bob_owner_hex).expect("bob owner")],
        )
        .expect("create group");

    let mut bob_core = logged_in_core_with_manager(data_dir.path(), 83, bob_manager);
    let mut receive_rng = OsRng;
    let mut receive_ctx = ProtocolContext::new(UnixSeconds(1_900_000_202), &mut receive_rng);
    let received = bob_core
        .logged_in
        .as_mut()
        .expect("logged in")
        .session_manager
        .receive(
            &mut receive_ctx,
            alice_owner,
            &result.prepared.remote.deliveries[0].envelope,
        )
        .expect("receive group create")
        .expect("group create payload");
    bob_core
        .apply_decrypted_payload(
            received.owner_pubkey,
            &received.payload,
            1_900_000_202,
            None,
        )
        .expect("apply group metadata");
    bob_core.rebuild_state();

    let group_chat_id = group_chat_id(&result.group.group_id);
    let thread = bob_core.threads.get(&group_chat_id).expect("group thread");
    assert!(thread.messages.is_empty());
    assert_eq!(bob_core.state.chat_list[0].display_name, "Project");
    assert!(matches!(bob_core.state.chat_list[0].kind, ChatKind::Group));
}

#[test]
fn group_metadata_changes_add_visible_notices_to_thread() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    let owner = local_owner_from_keys(&keys_for_fill(90));
    let first_member = local_owner_from_keys(&keys_for_fill(91));
    let second_member = local_owner_from_keys(&keys_for_fill(92));
    let group_id = "notice-group".to_string();
    let previous = GroupSnapshot {
        group_id: group_id.clone(),
        protocol: nostr_double_ratchet::GroupProtocol::PairwiseFanoutV1,
        name: "Old name".to_string(),
        created_by: owner,
        members: vec![owner, first_member],
        admins: vec![owner],
        revision: 1,
        created_at: UnixSeconds(100),
        updated_at: UnixSeconds(100),
    };
    let updated = GroupSnapshot {
        name: "New name".to_string(),
        members: vec![owner, first_member, second_member],
        revision: 2,
        updated_at: UnixSeconds(120),
        ..previous.clone()
    };

    core.apply_group_snapshot_to_threads(&previous, previous.updated_at.get());
    core.apply_group_snapshot_to_threads_with_notices(Some(&previous), &updated, 120);
    core.rebuild_state();

    let chat_id = group_chat_id(&group_id);
    let thread = core.threads.get(&chat_id).expect("group thread");
    let notices = thread
        .messages
        .iter()
        .map(|message| message.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[0], "Group renamed to New name");
    assert!(notices[1].ends_with(" added"));
    assert_eq!(
        core.state.chat_list[0].last_message_preview.as_deref(),
        Some(notices[1])
    );
}

#[test]
fn incoming_group_message_routes_to_group_thread() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let (mut alice_manager, bob_manager, bob_owner_hex) =
        established_session_manager_pair(84, 85, 1_900_000_300);
    let alice_owner = local_owner_from_keys(&keys_for_fill(84));
    let mut alice_groups = GroupManager::new(alice_owner);
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(UnixSeconds(1_900_000_301), &mut rng);
    let create = alice_groups
        .create_group(
            &mut alice_manager,
            &mut ctx,
            "Signals".to_string(),
            vec![parse_owner_input(&bob_owner_hex).expect("bob owner")],
        )
        .expect("create group");

    let mut bob_core = logged_in_core_with_manager(data_dir.path(), 85, bob_manager);
    let mut receive_rng = OsRng;
    let mut receive_ctx = ProtocolContext::new(UnixSeconds(1_900_000_302), &mut receive_rng);
    let create_message = bob_core
        .logged_in
        .as_mut()
        .expect("logged in")
        .session_manager
        .receive(
            &mut receive_ctx,
            alice_owner,
            &create.prepared.remote.deliveries[0].envelope,
        )
        .expect("receive create")
        .expect("create payload");
    bob_core
        .apply_decrypted_payload(
            create_message.owner_pubkey,
            &create_message.payload,
            1_900_000_302,
            None,
        )
        .expect("apply create");

    let message_send = alice_groups
        .send_message(
            &mut alice_manager,
            &mut ProtocolContext::new(UnixSeconds(1_900_000_303), &mut rng),
            &create.group.group_id,
            encode_app_group_message_payload("group-message-1", "hello group").expect("payload"),
        )
        .expect("send group message");
    let group_message = bob_core
        .logged_in
        .as_mut()
        .expect("logged in")
        .session_manager
        .receive(
            &mut ProtocolContext::new(UnixSeconds(1_900_000_304), &mut receive_rng),
            alice_owner,
            &message_send.remote.deliveries[0].envelope,
        )
        .expect("receive group message")
        .expect("group payload");
    bob_core
        .apply_decrypted_payload(
            group_message.owner_pubkey,
            &group_message.payload,
            1_900_000_304,
            None,
        )
        .expect("apply group message");

    let group_chat_id = group_chat_id(&create.group.group_id);
    let group_thread = bob_core.threads.get(&group_chat_id).expect("group thread");
    assert_eq!(group_thread.messages.len(), 1);
    assert_eq!(group_thread.messages[0].body, "hello group");
    assert!(!bob_core.threads.contains_key(&alice_owner.to_string()));
}

#[test]
fn restored_pending_group_create_rebuilds_publish_without_new_group_id() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let data_dir = TempDir::new().expect("temp dir");

    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 86, false, false).expect("start session");

    let bob_owner_keys = keys_for_fill(87);
    let bob_device_keys = device_keys_for_fill(87);
    let bob_owner = local_owner_from_keys(&bob_owner_keys);
    let bob_device = local_device_from_keys(&bob_device_keys);
    let mut bob_manager =
        SessionManager::new(bob_owner, bob_device_keys.secret_key().to_secret_bytes());

    core.create_group("Restore group", &[bob_owner.to_string()]);

    assert_eq!(core.pending_group_controls.len(), 1);
    assert!(core.pending_group_controls[0].prepared_publish.is_none());
    core.pending_group_controls[0].next_retry_at_secs = 0;
    core.pending_group_controls[0].in_flight = true;
    let original_group_id = core.pending_group_controls[0].group_id.clone();

    {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        let now = UnixSeconds(1_900_001_400);
        let bob_roster = DeviceRoster::new(now, vec![AuthorizedDevice::new(bob_device, now)]);
        logged_in
            .session_manager
            .observe_peer_roster(bob_owner, bob_roster);
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(1_900_001_401), &mut rng);
        let bob_invite = bob_manager
            .ensure_local_invite(&mut ctx)
            .expect("ensure bob invite")
            .clone();
        logged_in
            .session_manager
            .observe_device_invite(bob_owner, bob_invite)
            .expect("observe bob invite");
    }
    core.persist_best_effort();
    drop(core);

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 86, true, true).expect("restore session");

    let groups = restored
        .logged_in
        .as_ref()
        .expect("logged in")
        .group_manager
        .groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_id, original_group_id);

    let pending = restored
        .pending_group_controls
        .iter()
        .find(|pending| pending.group_id == original_group_id)
        .expect("pending group control restored");
    assert!(pending.prepared_publish.is_some());
    assert_eq!(pending.group_id, original_group_id);
}

#[test]
fn retry_pending_outbound_reuses_same_prepared_events_without_advancing_session() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, chat_id) =
        established_session_manager_pair(77, 78, 1_900_000_100);
    let mut core = logged_in_core_with_manager(data_dir.path(), 77, alice_manager);

    core.send_message(&chat_id, "retry me");

    let message_id = core.pending_outbound[0].message_id.clone();
    let event_ids_before = pending_publish_event_ids(&core, &message_id);

    core.handle_internal(InternalEvent::PublishFinished {
        message_id: message_id.clone(),
        chat_id: chat_id.clone(),
        success: false,
    });

    let pending_after_failure = core
        .pending_outbound
        .iter()
        .find(|pending| pending.message_id == message_id)
        .expect("pending outbound after failure");
    assert_eq!(
        pending_after_failure.publish_mode,
        OutboundPublishMode::OrdinaryFirstAck
    );
    assert_eq!(
        pending_after_failure.reason,
        PendingSendReason::PublishRetry
    );
    assert!(!pending_after_failure.in_flight);
    assert!(pending_after_failure.next_retry_at_secs > 0);
    let failed_snapshot = core.threads.get(&chat_id).expect("thread");
    assert!(matches!(
        failed_snapshot
            .messages
            .last()
            .expect("message after publish failure")
            .delivery,
        DeliveryState::Queued
    ));

    let snapshot_before_retry = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .session_manager
        .snapshot();
    if let Some(pending) = core
        .pending_outbound
        .iter_mut()
        .find(|pending| pending.message_id == message_id)
    {
        pending.next_retry_at_secs = 0;
    }

    core.retry_pending_outbound(UnixSeconds(1_900_000_200));

    let snapshot_after_retry = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .session_manager
        .snapshot();
    let event_ids_after = pending_publish_event_ids(&core, &message_id);

    assert_eq!(event_ids_after, event_ids_before);
    assert_eq!(snapshot_after_retry, snapshot_before_retry);
    assert!(
        core.pending_outbound
            .iter()
            .find(|pending| pending.message_id == message_id)
            .expect("pending outbound after retry")
            .in_flight
    );
    let retry_snapshot = core.threads.get(&chat_id).expect("thread");
    assert!(matches!(
        retry_snapshot
            .messages
            .last()
            .expect("message after retry")
            .delivery,
        DeliveryState::Pending
    ));
}

#[test]
fn publish_finished_success_marks_message_sent_and_clears_pending() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, chat_id) =
        established_session_manager_pair(79, 80, 1_900_000_300);
    let mut core = logged_in_core_with_manager(data_dir.path(), 79, alice_manager);

    core.send_message(&chat_id, "mark me sent");
    let message_id = core.pending_outbound[0].message_id.clone();

    core.handle_internal(InternalEvent::PublishFinished {
        message_id: message_id.clone(),
        chat_id: chat_id.clone(),
        success: true,
    });

    assert!(core.pending_outbound.is_empty());
    let last_message = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat")
        .messages
        .last()
        .expect("message");
    assert_eq!(last_message.id, message_id);
    assert!(matches!(last_message.delivery, DeliveryState::Sent));
}

#[test]
fn publish_events_first_ack_succeeds_when_any_relay_accepts() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:4848,ws://127.0.0.1:59999");
    let _relay = TestRelay::start();
    let owner_keys = keys_for_fill(81);
    let device_keys = device_keys_for_fill(81);
    let owner = local_owner_from_keys(&owner_keys);
    let roster = DeviceRoster::new(
        UnixSeconds(1_900_000_400),
        vec![AuthorizedDevice::new(
            local_device_from_keys(&device_keys),
            UnixSeconds(1_900_000_400),
        )],
    );
    let event = codec::roster_unsigned_event(owner, &roster)
        .expect("roster event")
        .sign_with_keys(&owner_keys)
        .expect("sign roster");
    let client = Client::new(device_keys);
    let relay_urls = configured_relay_urls();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        ensure_session_relays_configured(&client, &relay_urls).await;
        client.connect().await;
        publish_events_first_ack(&client, &relay_urls, &[event.clone()], "test first ack")
            .await
            .expect("first ack publish");
        let _ = client.shutdown().await;
    });

    let events = fetch_local_relay_events(vec![
        Filter::new().kind(Kind::from(codec::ROSTER_EVENT_KIND as u16))
    ]);
    assert!(events.iter().any(|candidate| candidate.id == event.id));
}

#[test]
fn publish_events_first_ack_fails_when_all_relays_fail() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let owner_keys = keys_for_fill(82);
    let device_keys = device_keys_for_fill(82);
    let owner = local_owner_from_keys(&owner_keys);
    let roster = DeviceRoster::new(
        UnixSeconds(1_900_000_500),
        vec![AuthorizedDevice::new(
            local_device_from_keys(&device_keys),
            UnixSeconds(1_900_000_500),
        )],
    );
    let event = codec::roster_unsigned_event(owner, &roster)
        .expect("roster event")
        .sign_with_keys(&owner_keys)
        .expect("sign roster");
    let client = Client::new(device_keys);
    let relay_urls = configured_relay_urls();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    let result = runtime.block_on(async {
        ensure_session_relays_configured(&client, &relay_urls).await;
        client.connect().await;
        let result =
            publish_events_first_ack(&client, &relay_urls, &[event], "test first ack").await;
        let _ = client.shutdown().await;
        result
    });

    assert!(result.is_err());
}

#[test]
fn prepared_pending_outbound_persists_across_restore() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::custom("ws://127.0.0.1:59999");
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, chat_id) =
        established_session_manager_pair(79, 80, 1_900_000_300);
    let mut core = logged_in_core_with_manager(data_dir.path(), 79, alice_manager);

    core.send_message(&chat_id, "persist me");
    let message_id = core.pending_outbound[0].message_id.clone();
    let event_ids_before = pending_publish_event_ids(&core, &message_id);
    core.persist_best_effort();

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 79, true, true)
        .expect("restore session with prepared pending outbound");

    let restored_message_id = restored.pending_outbound[0].message_id.clone();
    let event_ids_after = pending_publish_event_ids(&restored, &restored_message_id);

    assert_eq!(restored.pending_outbound.len(), 1);
    assert_eq!(event_ids_after, event_ids_before);
}

#[test]
fn pending_inbound_persists_across_restore() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let owner_keys = keys_for_fill(75);
    let device_keys = device_keys_for_fill(75);
    let owner = local_owner_from_keys(&owner_keys);
    let now = UnixSeconds(500);
    let local_device = local_device_from_keys(&device_keys);

    let mut session_manager =
        SessionManager::new(owner, device_keys.secret_key().to_secret_bytes());
    session_manager.apply_local_roster(DeviceRoster::new(
        now,
        vec![AuthorizedDevice::new(local_device, now)],
    ));

    let mut core = logged_in_core_with_manager(data_dir.path(), 75, session_manager);
    core.pending_inbound.push(PendingInbound::envelope(
        MessageEnvelope {
            sender: local_device_from_keys(&device_keys_for_fill(76)),
            signer_secret_key: [9; 32],
            created_at: UnixSeconds(501),
            encrypted_header: "header".to_string(),
            ciphertext: "ciphertext".to_string(),
        },
        None,
    ));
    core.persist_best_effort();

    let persisted = persisted_state(data_dir.path());
    assert_eq!(persisted.pending_inbound.len(), 1);
    assert_eq!(
        match &persisted.pending_inbound[0] {
            PendingInbound::Envelope { envelope, .. } => envelope.ciphertext.as_str(),
            PendingInbound::Decrypted { .. } => panic!("expected persisted envelope"),
        },
        "ciphertext"
    );

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 75, true, true)
        .expect("restore session with pending inbound");

    assert_eq!(restored.pending_inbound.len(), 1);
    assert_eq!(
        match &restored.pending_inbound[0] {
            PendingInbound::Envelope { envelope, .. } => envelope.encrypted_header.as_str(),
            PendingInbound::Decrypted { .. } => panic!("expected restored envelope"),
        },
        "header"
    );
}

#[test]
fn decrypted_pending_inbound_sender_is_tracked_for_recovery() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let owner_keys = keys_for_fill(75);
    let device_keys = device_keys_for_fill(75);
    let owner = local_owner_from_keys(&owner_keys);
    let now = UnixSeconds(500);
    let local_device = local_device_from_keys(&device_keys);

    let mut session_manager =
        SessionManager::new(owner, device_keys.secret_key().to_secret_bytes());
    session_manager.apply_local_roster(DeviceRoster::new(
        now,
        vec![AuthorizedDevice::new(local_device, now)],
    ));

    let mut core = logged_in_core_with_manager(data_dir.path(), 75, session_manager);
    let sender_owner = local_owner_from_keys(&keys_for_fill(76));
    core.pending_inbound.push(PendingInbound::decrypted(
        sender_owner,
        b"pending".to_vec(),
        now.get(),
        None,
    ));

    assert!(core
        .tracked_peer_owner_hexes()
        .contains(&sender_owner.to_string()));
}

#[test]
fn recent_handshake_peer_tracks_claimed_owner_after_roster_verification() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");

    let alice_device_keys = device_keys_for_fill(120);
    let alice_claimed_owner_keys = keys_for_fill(121);
    let bob_owner_keys = keys_for_fill(122);
    let bob_device_keys = device_keys_for_fill(122);

    let alice_device_owner = local_owner_from_keys(&alice_device_keys);
    let alice_claimed_owner = local_owner_from_keys(&alice_claimed_owner_keys);
    let bob_owner = local_owner_from_keys(&bob_owner_keys);
    let now = UnixSeconds(1_900_000_400);

    let mut alice_manager = SessionManager::new(
        alice_claimed_owner,
        alice_device_keys.secret_key().to_secret_bytes(),
    );
    let mut bob_manager =
        SessionManager::new(bob_owner, bob_device_keys.secret_key().to_secret_bytes());

    let bob_roster = DeviceRoster::new(
        now,
        vec![AuthorizedDevice::new(
            local_device_from_keys(&bob_device_keys),
            now,
        )],
    );
    bob_manager.apply_local_roster(bob_roster.clone());
    alice_manager.observe_peer_roster(bob_owner, bob_roster);

    let bob_invite = {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(now.get() + 1), &mut rng);
        bob_manager
            .ensure_local_invite(&mut ctx)
            .expect("ensure bob invite")
            .clone()
    };
    alice_manager
        .observe_device_invite(bob_owner, bob_invite)
        .expect("observe bob invite");

    let prepared = {
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(now.get() + 2), &mut rng);
        alice_manager
            .prepare_send(&mut ctx, bob_owner, b"claim migration".to_vec())
            .expect("prepare first-contact send")
    };
    assert_eq!(prepared.invite_responses.len(), 1);

    let mut core = logged_in_core_with_manager(data_dir.path(), 122, bob_manager);
    let processed = {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(UnixSeconds(now.get() + 3), &mut rng);
        logged_in
            .session_manager
            .observe_invite_response(&mut ctx, &prepared.invite_responses[0])
            .expect("observe invite response")
            .expect("processed invite response")
    };
    assert_eq!(processed.owner_pubkey, alice_device_owner);

    core.remember_recent_handshake_peer(
        processed.owner_pubkey.to_string(),
        processed.device_pubkey.to_string(),
        now.get(),
    );
    assert!(core
        .protocol_owner_hexes()
        .contains(&alice_device_owner.to_string()));
    assert!(core
        .protocol_owner_hexes()
        .contains(&alice_claimed_owner.to_string()));

    {
        let logged_in = core.logged_in.as_mut().expect("logged in");
        let alice_roster = DeviceRoster::new(
            UnixSeconds(now.get() + 4),
            vec![AuthorizedDevice::new(
                local_device_from_keys(&alice_device_keys),
                UnixSeconds(now.get() + 4),
            )],
        );
        logged_in
            .session_manager
            .observe_peer_roster(alice_claimed_owner, alice_roster);
    }

    let migrated_owner_hexes = core.reconcile_recent_handshake_peers();
    assert!(migrated_owner_hexes
        .iter()
        .any(|(_, owner_hex)| owner_hex == &alice_claimed_owner.to_string()));
    assert!(core
        .protocol_owner_hexes()
        .contains(&alice_claimed_owner.to_string()));
    assert!(!core
        .protocol_owner_hexes()
        .contains(&alice_device_owner.to_string()));
}

#[test]
fn protocol_state_catch_up_fetches_recent_message_events() {
    let plan = ProtocolSubscriptionPlan {
        roster_authors: vec![
            "0193d5c691ea39b12343c37ac26a0455cf4c64bdacfa218e29ee582c552859db".to_string(),
        ],
        invite_authors: vec![],
        invite_response_recipient: Some(
            "a3f5e52d1a528f41f4dcffc8ef7c46cc32299f92472fba3aa8006839becbfad6".to_string(),
        ),
        message_authors: vec![
            "e7d079b75972e1774a5cb0fe36566faea814999bab5330081cfb6d4763314915".to_string(),
        ],
    };

    let now = UnixSeconds(1_900_000_000);
    let filters = build_protocol_state_catch_up_filters(&plan, now);
    let serialized = serde_json::to_value(&filters).expect("serialize filters");
    let filters = serialized.as_array().expect("filter array");

    let message_filter = filters
        .iter()
        .find(|filter| {
            filter["kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|kind| kind.as_u64() == Some(codec::MESSAGE_EVENT_KIND as u64))
        })
        .expect("message catch-up filter");

    assert_eq!(
        message_filter["authors"]
            .as_array()
            .expect("authors array")
            .len(),
        1
    );
    assert_eq!(
        message_filter["since"].as_u64(),
        Some(now.get().saturating_sub(CATCH_UP_LOOKBACK_SECS))
    );
}

#[test]
fn mobile_push_snapshot_tracks_authors_and_session_states() {
    let data_dir = TempDir::new().expect("temp dir");
    let (_alice_manager, bob_manager, _bob_chat_id) =
        established_session_manager_pair(141, 142, 1_900_010_000);
    let chat_id = local_owner_from_keys(&keys_for_fill(141)).to_string();
    let mut core = logged_in_core_with_manager(data_dir.path(), 142, bob_manager);
    core.handle_action(AppAction::CreateChat {
        peer_input: chat_id.clone(),
    });

    let snapshot = &core.state.mobile_push;
    let expected_owner = local_owner_from_keys(&keys_for_fill(142)).to_string();
    assert_eq!(
        snapshot.owner_pubkey_hex.as_deref(),
        Some(expected_owner.as_str())
    );
    assert!(
        !snapshot.message_author_pubkeys.is_empty(),
        "message push sync must expose relay authors"
    );
    assert!(
        snapshot
            .sessions
            .iter()
            .any(|session| session.recipient_pubkey_hex == chat_id
                && !session.state_json.is_empty()
                && session.has_receiving_capability
                && !session.tracked_sender_pubkeys.is_empty()),
        "message push sync must expose decryptable session snapshots"
    );
}

#[test]
fn mobile_push_notification_filter_suppresses_typing_and_formats_messages() {
    let typing = resolve_mobile_push_notification(
        r#"{"title":"DM by Alice","body":"typing","inner_kind":"25"}"#.to_string(),
    );
    assert!(!typing.should_show);

    let message = resolve_mobile_push_notification(
        r#"{"title":"DM by Alice","body":"hello","inner_event_json":"{\"kind\":14,\"content\":\"hello\"}"}"#.to_string(),
    );
    assert!(message.should_show);
    assert_eq!(message.title, "Alice");
    assert_eq!(message.body, "hello");

    let reaction = resolve_mobile_push_notification(
        r#"{"title":"DM by Alice","inner_event_json":"{\"kind\":7,\"content\":\"🔥\"}"}"#
            .to_string(),
    );
    assert!(reaction.should_show);
    assert_eq!(reaction.body, "Reacted 🔥");
}

#[test]
fn mobile_push_subscription_requests_match_notification_server_contract() {
    let owner_nsec = nsec_for_fill(151);
    let author_a = local_owner_from_keys(&keys_for_fill(152)).to_string();
    let author_b = local_owner_from_keys(&keys_for_fill(153)).to_string();

    let request = build_mobile_push_create_subscription_request(
        owner_nsec.clone(),
        "ios".to_string(),
        "apns-token".to_string(),
        Some("social.innode.irischat".to_string()),
        vec![
            author_b.clone(),
            "not-hex".to_string(),
            author_a.clone(),
            author_a,
        ],
        false,
        None,
    )
    .expect("request");

    assert_eq!(request.method, "POST");
    assert_eq!(
        request.url,
        "https://notifications-sandbox.iris.to/subscriptions"
    );
    assert!(request.authorization_header.starts_with("Nostr "));

    let body: serde_json::Value =
        serde_json::from_str(request.body_json.as_deref().unwrap()).expect("subscription body");
    assert_eq!(body["webhooks"], serde_json::json!([]));
    assert_eq!(body["web_push_subscriptions"], serde_json::json!([]));
    assert_eq!(body["fcm_tokens"], serde_json::json!([]));
    assert_eq!(body["apns_tokens"], serde_json::json!(["apns-token"]));
    assert_eq!(body["apns_topic"], "social.innode.irischat");
    assert_eq!(body["filter"]["kinds"], serde_json::json!([1060]));
    let mut expected_authors = vec![
        author_b,
        local_owner_from_keys(&keys_for_fill(152)).to_string(),
    ];
    expected_authors.sort();
    assert_eq!(
        body["filter"]["authors"],
        serde_json::json!(expected_authors)
    );

    let update = build_mobile_push_update_subscription_request(
        owner_nsec.clone(),
        "sub-1".to_string(),
        "android".to_string(),
        "fcm-token".to_string(),
        None,
        vec![local_owner_from_keys(&keys_for_fill(154)).to_string()],
        true,
        Some("https://push.example.test/root/".to_string()),
    )
    .expect("update request");
    assert_eq!(update.method, "POST");
    assert_eq!(
        update.url,
        "https://push.example.test/root/subscriptions/sub-1"
    );
    let update_body: serde_json::Value =
        serde_json::from_str(update.body_json.as_deref().unwrap()).expect("update body");
    assert_eq!(update_body["fcm_tokens"], serde_json::json!(["fcm-token"]));
    assert_eq!(update_body["apns_tokens"], serde_json::json!([]));

    let delete = build_mobile_push_delete_subscription_request(
        owner_nsec,
        "sub-1".to_string(),
        "ios".to_string(),
        true,
        None,
    )
    .expect("delete request");
    assert_eq!(delete.method, "DELETE");
    assert_eq!(delete.body_json, None);
    assert_eq!(
        mobile_push_stored_subscription_id_key("ios".to_string()),
        "settings.mobile_push_subscription_id.ios"
    );
}

#[test]
fn invite_response_delivery_requires_matching_pubkey_tag() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let (_alice_dir, alice) = app(71);
    let (_bob_dir, bob) = app(72);
    let (charlie_dir, _charlie) = app(73);

    alice.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(72),
    });
    let alice_chat_id = wait_for_state(&alice, "alice chat create", |state| {
        state.current_chat.is_some()
    })
    .current_chat
    .expect("alice chat")
    .chat_id;

    alice.dispatch(AppAction::SendMessage {
        chat_id: alice_chat_id,
        text: "hello bob".to_string(),
    });

    wait_for_state(&bob, "bob receives hello", |state| {
        state
            .chat_list
            .iter()
            .any(|chat| chat.last_message_preview.as_deref() == Some("hello bob"))
    });

    let charlie_persisted = wait_for_persisted_state(
        charlie_dir.path(),
        "charlie stays scoped to local invite responses",
        |persisted| {
            persisted
                .session_manager
                .as_ref()
                .map(|snapshot| snapshot.users.len() == 1)
                .unwrap_or(false)
        },
    );
    assert_eq!(
        charlie_persisted
            .session_manager
            .expect("charlie session manager")
            .users
            .len(),
        1
    );
}

#[test]
fn pending_send_waits_for_missing_roster_then_delivers() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let (alice_dir, alice) = app(81);

    alice.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(82),
    });
    let chat_id = wait_for_state(&alice, "alice chat create", |state| {
        state.current_chat.is_some()
    })
    .current_chat
    .expect("alice chat")
    .chat_id;

    alice.dispatch(AppAction::SendMessage {
        chat_id,
        text: "waiting on roster".to_string(),
    });

    let alice_pending = wait_for_persisted_state(
        alice_dir.path(),
        "alice waits for missing roster",
        |persisted| {
            persisted.pending_outbound.len() == 1
                && persisted.pending_outbound[0].reason == PendingSendReason::MissingRoster
        },
    );
    assert_eq!(alice_pending.pending_outbound.len(), 1);

    let (_bob_dir, bob) = app(82);
    wait_for_state(&bob, "bob receives queued first message", |state| {
        state
            .chat_list
            .iter()
            .any(|chat| chat.last_message_preview.as_deref() == Some("waiting on roster"))
    });
    let alice_cleared = wait_for_persisted_state(
        alice_dir.path(),
        "alice clears pending outbound after roster arrives",
        |persisted| persisted.pending_outbound.is_empty(),
    );
    assert!(alice_cleared.pending_outbound.is_empty());
}

#[test]
fn pending_send_waits_for_missing_device_invite_then_publishes() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();

    let bob_owner_keys = keys_for_fill(92);
    let bob_device_keys = device_keys_for_fill(92);
    let bob_owner = local_owner_from_keys(&bob_owner_keys);
    let now = unix_now();
    let bob_roster = DeviceRoster::new(
        now,
        vec![AuthorizedDevice::new(
            local_device_from_keys(&bob_device_keys),
            now,
        )],
    );
    let roster_event = codec::roster_unsigned_event(bob_owner, &bob_roster)
        .expect("bob roster event")
        .sign_with_keys(&bob_owner_keys)
        .expect("sign bob roster");
    publish_local_relay_event(&bob_owner_keys, roster_event);

    let (_alice_dir, alice) = app(91);
    alice.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(92),
    });
    let chat_id = wait_for_state(&alice, "alice chat create", |state| {
        state.current_chat.is_some()
    })
    .current_chat
    .expect("alice chat")
    .chat_id;

    alice.dispatch(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "waiting on invite".to_string(),
    });

    wait_for_state(&alice, "alice send pending on missing invite", |state| {
        state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.messages.last())
            .map(|message| matches!(message.delivery, DeliveryState::Queued))
            .unwrap_or(false)
    });

    let mut session_manager =
        SessionManager::new(bob_owner, bob_device_keys.secret_key().to_secret_bytes());
    session_manager.apply_local_roster(bob_roster);
    let mut rng = OsRng;
    let mut ctx = ProtocolContext::new(unix_now(), &mut rng);
    let invite = session_manager
        .ensure_local_invite(&mut ctx)
        .expect("ensure bob invite")
        .clone();
    let invite_event = codec::invite_unsigned_event(&invite)
        .expect("bob invite event")
        .sign_with_keys(&bob_device_keys)
        .expect("sign bob invite");
    publish_local_relay_event(&bob_device_keys, invite_event);

    let alice_after_invite =
        wait_for_state(&alice, "alice publishes after invite appears", |state| {
            state
                .current_chat
                .as_ref()
                .and_then(|chat| chat.messages.last())
                .map(|message| matches!(message.delivery, DeliveryState::Sent))
                .unwrap_or(false)
        });
    assert_eq!(
        alice_after_invite
            .current_chat
            .expect("alice current chat")
            .messages
            .last()
            .expect("alice last message")
            .body,
        "waiting on invite"
    );
}

#[test]
fn local_relay_round_trip_and_reverse_initiation_work() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let (_alice_dir, alice) = app(51);
    let (_bob_dir, bob) = app(52);

    alice.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(52),
    });
    let alice_state = wait_for_state(&alice, "alice chat create", |state| {
        state.current_chat.is_some() && state.chat_list.len() == 1
    });
    let alice_chat_id = alice_state
        .current_chat
        .as_ref()
        .expect("alice chat")
        .chat_id
        .clone();

    alice.dispatch(AppAction::SendMessage {
        chat_id: alice_chat_id.clone(),
        text: "hello bob".to_string(),
    });
    let _alice_after_send = wait_for_state(&alice, "alice outbound queued", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| !chat.messages.is_empty())
            .unwrap_or(false)
    });

    let bob_state = wait_for_state(&bob, "bob receives first message", |state| {
        state.chat_list.iter().any(|chat| {
            chat.last_message_preview.as_deref() == Some("hello bob") && chat.unread_count == 1
        })
    });
    let bob_chat_id = bob_state.chat_list[0].chat_id.clone();
    bob.dispatch(AppAction::OpenChat {
        chat_id: bob_chat_id.clone(),
    });
    let bob_open = wait_for_state(&bob, "bob opens chat", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len() == 1)
            .unwrap_or(false)
            && state
                .chat_list
                .first()
                .map(|chat| chat.unread_count == 0)
                .unwrap_or(false)
    });
    assert_eq!(
        bob_open
            .current_chat
            .as_ref()
            .expect("bob current chat")
            .messages[0]
            .body,
        "hello bob"
    );

    bob.dispatch(AppAction::SendMessage {
        chat_id: bob_chat_id.clone(),
        text: "hi alice".to_string(),
    });
    let _bob_after_reply = wait_for_state(&bob, "bob sends reply", |state| {
        state
            .current_chat
            .as_ref()
            .and_then(|chat| chat.messages.last())
            .map(|message| {
                message.is_outgoing
                    && message.body == "hi alice"
                    && matches!(
                        message.delivery,
                        DeliveryState::Sent | DeliveryState::Received | DeliveryState::Seen
                    )
            })
            .unwrap_or(false)
    });
    let alice_with_reply = wait_for_state(&alice, "alice receives reply", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len() >= 2)
            .unwrap_or(false)
    });
    assert_eq!(
        alice_with_reply
            .current_chat
            .as_ref()
            .expect("alice current chat")
            .messages
            .last()
            .expect("alice last message")
            .body,
        "hi alice"
    );
    assert!(alice_with_reply.toast.is_none());

    let (_charlie_dir, charlie) = app(53);
    let (_dana_dir, dana) = app(54);
    dana.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(53),
    });
    let dana_chat_id = wait_for_state(&dana, "dana chat create", |state| {
        state.current_chat.is_some()
    })
    .current_chat
    .expect("dana chat")
    .chat_id;
    dana.dispatch(AppAction::SendMessage {
        chat_id: dana_chat_id,
        text: "reverse first".to_string(),
    });
    let charlie_state = wait_for_state(&charlie, "charlie receives reverse message", |state| {
        state
            .chat_list
            .iter()
            .any(|chat| chat.last_message_preview.as_deref() == Some("reverse first"))
    });
    assert_eq!(
        charlie_state.chat_list[0].last_message_preview.as_deref(),
        Some("reverse first")
    );
}

#[test]
fn primary_bootstrap_uses_separate_device_identity_and_one_device_roster() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    start_primary_test_session(&mut core, 140, false, false).expect("start primary session");

    let account = core.state.account.as_ref().expect("account");
    assert_ne!(account.public_key_hex, account.device_public_key_hex);
    assert!(account.has_owner_signing_authority);
    assert!(matches!(
        account.authorization_state,
        DeviceAuthorizationState::Authorized
    ));

    let snapshot = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .session_manager
        .snapshot();
    assert_eq!(
        snapshot.local_owner_pubkey.to_string(),
        account.public_key_hex
    );
    assert_eq!(
        snapshot.local_device_pubkey.to_string(),
        account.device_public_key_hex
    );

    let local_roster = snapshot
        .users
        .into_iter()
        .find(|user| user.owner_pubkey == snapshot.local_owner_pubkey)
        .and_then(|user| user.roster)
        .expect("local roster");
    assert_eq!(local_roster.devices().len(), 1);
    assert_eq!(
        local_roster.devices()[0].device_pubkey.to_string(),
        account.device_public_key_hex
    );
}

#[test]
fn linked_device_starts_awaiting_approval_with_device_invite() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    core.start_linked_device(&npub_for_fill(141));

    let account = core.state.account.as_ref().expect("account");
    assert!(!account.has_owner_signing_authority);
    assert!(matches!(
        account.authorization_state,
        DeviceAuthorizationState::AwaitingApproval
    ));
    assert!(matches!(
        core.state.router.default_screen,
        Screen::AwaitingDeviceApproval
    ));
    assert!(core
        .logged_in
        .as_ref()
        .expect("logged in")
        .session_manager
        .snapshot()
        .local_invite
        .is_some());
    let roster = core.state.device_roster.as_ref().expect("device roster");
    assert_eq!(roster.devices.len(), 1);
    assert!(!roster.devices[0].is_authorized);
    assert!(roster.devices[0].is_current_device);
}

#[test]
fn linked_device_becomes_authorized_when_owner_roster_arrives() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let owner_keys = keys_for_fill(142);
    let owner = local_owner_from_keys(&owner_keys);
    let mut core = test_core(data_dir.path());

    core.start_linked_device(&owner_keys.public_key().to_bech32().expect("owner npub"));
    let device_pubkey = parse_device_input(
        &core
            .state
            .account
            .as_ref()
            .expect("account")
            .device_public_key_hex,
    )
    .expect("device pubkey");
    let now = unix_now();
    let roster = DeviceRoster::new(now, vec![AuthorizedDevice::new(device_pubkey, now)]);
    let roster_event = codec::roster_unsigned_event(owner, &roster)
        .expect("roster event")
        .sign_with_keys(&owner_keys)
        .expect("sign roster");

    core.handle_relay_event(roster_event);

    assert!(matches!(
        core.state
            .account
            .as_ref()
            .expect("account")
            .authorization_state,
        DeviceAuthorizationState::Authorized
    ));
    assert!(matches!(core.state.router.default_screen, Screen::ChatList));
    assert!(core
        .state
        .device_roster
        .as_ref()
        .expect("roster")
        .devices
        .iter()
        .any(
            |device| device.device_pubkey_hex == device_pubkey.to_string() && device.is_authorized
        ));
}

#[test]
fn linked_device_transitions_to_revoked_when_owner_roster_excludes_it() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let owner_keys = keys_for_fill(143);
    let owner = local_owner_from_keys(&owner_keys);
    let mut core = test_core(data_dir.path());

    core.start_linked_device(&owner_keys.public_key().to_bech32().expect("owner npub"));
    let device_pubkey = parse_device_input(
        &core
            .state
            .account
            .as_ref()
            .expect("account")
            .device_public_key_hex,
    )
    .expect("device pubkey");

    let authorized_now = unix_now();
    let authorized_roster = DeviceRoster::new(
        authorized_now,
        vec![AuthorizedDevice::new(device_pubkey, authorized_now)],
    );
    let authorized_event = codec::roster_unsigned_event(owner, &authorized_roster)
        .expect("authorized roster event")
        .sign_with_keys(&owner_keys)
        .expect("sign authorized roster");
    core.handle_relay_event(authorized_event);

    let revoked_roster = DeviceRoster::new(UnixSeconds(authorized_now.get() + 1), Vec::new());
    let revoked_event = codec::roster_unsigned_event(owner, &revoked_roster)
        .expect("revoked roster event")
        .sign_with_keys(&owner_keys)
        .expect("sign revoked roster");
    core.handle_relay_event(revoked_event);

    assert!(matches!(
        core.state
            .account
            .as_ref()
            .expect("account")
            .authorization_state,
        DeviceAuthorizationState::Revoked
    ));
    assert!(matches!(
        core.state.router.default_screen,
        Screen::DeviceRevoked
    ));
}

#[test]
fn add_and_remove_authorized_device_updates_local_roster_snapshot() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 144, false, false).expect("start primary session");

    let added_device_npub = device_keys_for_fill(145)
        .public_key()
        .to_bech32()
        .expect("device npub");
    core.add_authorized_device(&added_device_npub);
    assert!(core
        .state
        .device_roster
        .as_ref()
        .expect("device roster")
        .devices
        .iter()
        .any(|device| device.device_npub == added_device_npub && device.is_authorized));

    let added_device_hex = normalize_peer_input_for_display(&added_device_npub);
    core.remove_authorized_device(&added_device_hex);
    assert!(!core
        .state
        .device_roster
        .as_ref()
        .expect("device roster")
        .devices
        .iter()
        .any(|device| device.device_pubkey_hex == added_device_hex));
}

#[test]
fn primary_identity_publishes_owner_signed_roster_and_device_signed_invite() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 146, false, false).expect("start primary session");

    let account = core.state.account.as_ref().expect("account");
    let owner_key = PublicKey::parse(&account.public_key_hex).expect("owner key");
    let device_key = PublicKey::parse(&account.device_public_key_hex).expect("device key");

    let deadline = Instant::now() + StdDuration::from_secs(15);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        events = fetch_local_relay_events(vec![
            Filter::new().kind(Kind::from(codec::ROSTER_EVENT_KIND as u16))
        ]);
        let has_roster = events
            .iter()
            .any(|event| codec::parse_roster_event(event).is_ok() && event.pubkey == owner_key);
        let has_invite = events
            .iter()
            .any(|event| codec::parse_invite_event(event).is_ok() && event.pubkey == device_key);
        if has_roster && has_invite {
            break;
        }
        thread::sleep(StdDuration::from_millis(100));
    }
    assert!(events
        .iter()
        .any(|event| { codec::parse_roster_event(event).is_ok() && event.pubkey == owner_key }));
    assert!(events
        .iter()
        .any(|event| { codec::parse_invite_event(event).is_ok() && event.pubkey == device_key }));
}

#[test]
fn local_relay_duplicate_replay_is_ignored_and_state_restores() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let relay = TestRelay::start();
    let alice_dir = TempDir::new().expect("alice dir");
    let bob_dir = TempDir::new().expect("bob dir");
    let alice = app_with_dir(alice_dir.path(), 61);
    let bob = app_with_dir(bob_dir.path(), 62);

    alice.dispatch(AppAction::CreateChat {
        peer_input: npub_for_fill(62),
    });
    let chat_id = wait_for_state(&alice, "alice chat create", |state| {
        state.current_chat.is_some()
    })
    .current_chat
    .expect("alice chat")
    .chat_id;
    alice.dispatch(AppAction::SendMessage {
        chat_id: chat_id.clone(),
        text: "dedupe".to_string(),
    });

    let bob_chat_id = wait_for_state(&bob, "bob first delivery", |state| {
        state
            .chat_list
            .iter()
            .any(|chat| chat.last_message_preview.as_deref() == Some("dedupe"))
    })
    .chat_list[0]
        .chat_id
        .clone();
    bob.dispatch(AppAction::OpenChat {
        chat_id: bob_chat_id.clone(),
    });
    wait_for_state(&bob, "bob chat open", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len() == 1)
            .unwrap_or(false)
    });

    relay.replay_stored();
    thread::sleep(StdDuration::from_secs(1));
    let bob_after_replay = bob.state();
    assert_eq!(
        bob_after_replay
            .current_chat
            .as_ref()
            .expect("bob current chat")
            .messages
            .len(),
        1
    );
    assert!(bob_after_replay.toast.is_none());

    drop(bob);
    let restored_bob = app_with_dir(bob_dir.path(), 62);
    let restored_state = wait_for_state(&restored_bob, "restored bob thread", |state| {
        !state.chat_list.is_empty()
    });
    assert_eq!(
        restored_state.chat_list[0].last_message_preview.as_deref(),
        Some("dedupe")
    );
    assert!(restored_state.toast.is_none());
}

#[test]
fn linked_device_fanout_and_revocation_work_on_local_relay() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let primary_dir = TempDir::new().expect("primary dir");
    let linked_dir = TempDir::new().expect("linked dir");
    let peer_dir = TempDir::new().expect("peer dir");

    let primary = app_with_dir(primary_dir.path(), 71);
    let primary_account =
        wait_for_state(&primary, "primary account", |state| state.account.is_some())
            .account
            .expect("primary account");

    let linked = FfiApp::new(
        linked_dir.path().to_string_lossy().into_owned(),
        String::new(),
        "test".to_string(),
    );
    linked.dispatch(AppAction::StartLinkedDevice {
        owner_input: primary_account.npub.clone(),
    });

    let linked_account = wait_for_state(&linked, "linked awaiting approval", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::AwaitingApproval
                )
            })
            .unwrap_or(false)
    })
    .account
    .expect("linked account");

    primary.dispatch(AppAction::AddAuthorizedDevice {
        device_input: linked_account.device_npub.clone(),
    });

    wait_for_state(&linked, "linked authorized", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::Authorized
                )
            })
            .unwrap_or(false)
    });

    let peer = app_with_dir(peer_dir.path(), 72);
    let peer_account = wait_for_state(&peer, "peer account", |state| state.account.is_some())
        .account
        .expect("peer account");

    let peer_chat_id = peer_account.public_key_hex.clone();
    let primary_chat_id = primary_account.public_key_hex.clone();

    primary.dispatch(AppAction::CreateChat {
        peer_input: peer_account.npub.clone(),
    });
    wait_for_state(&primary, "primary chat with peer", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.chat_id == peer_chat_id)
            .unwrap_or(false)
    });

    primary.dispatch(AppAction::SendMessage {
        chat_id: peer_chat_id.clone(),
        text: "m1".to_string(),
    });

    wait_for_state(&peer, "peer received m1", |state| {
        state.chat_list.iter().any(|thread| {
            thread.chat_id == primary_chat_id
                && thread.last_message_preview.as_deref() == Some("m1")
        })
    });
    let linked_after_m1 = wait_for_state(&linked, "linked received synced m1", |state| {
        state.chat_list.iter().any(|thread| {
            thread.chat_id == peer_chat_id && thread.last_message_preview.as_deref() == Some("m1")
        })
    });
    assert!(linked_after_m1.chat_list.iter().any(|thread| {
        thread.chat_id == peer_chat_id && thread.last_message_preview.as_deref() == Some("m1")
    }));

    peer.dispatch(AppAction::OpenChat {
        chat_id: primary_chat_id.clone(),
    });
    wait_for_state(&peer, "peer chat open", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| chat.chat_id == primary_chat_id)
            .unwrap_or(false)
    });
    peer.dispatch(AppAction::SendMessage {
        chat_id: primary_chat_id.clone(),
        text: "m2".to_string(),
    });

    wait_for_state(&primary, "primary received m2", |state| {
        state.chat_list.iter().any(|thread| {
            thread.chat_id == peer_chat_id && thread.last_message_preview.as_deref() == Some("m2")
        })
    });
    let linked_after_m2 = wait_for_state(&linked, "linked received m2", |state| {
        state.chat_list.iter().any(|thread| {
            thread.chat_id == peer_chat_id && thread.last_message_preview.as_deref() == Some("m2")
        })
    });
    linked.dispatch(AppAction::OpenChat {
        chat_id: peer_chat_id.clone(),
    });
    let linked_chat = wait_for_state(&linked, "linked chat open", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| {
                chat.chat_id == peer_chat_id
                    && chat
                        .messages
                        .iter()
                        .any(|message| message.body == "m1" && message.is_outgoing)
                    && chat
                        .messages
                        .iter()
                        .any(|message| message.body == "m2" && !message.is_outgoing)
            })
            .unwrap_or(false)
    })
    .current_chat
    .expect("linked chat");
    assert_eq!(linked_chat.chat_id, peer_chat_id);
    assert!(linked_after_m2.chat_list.iter().any(|thread| {
        thread.chat_id == peer_chat_id && thread.last_message_preview.as_deref() == Some("m2")
    }));

    linked.dispatch(AppAction::SendMessage {
        chat_id: peer_chat_id.clone(),
        text: "m3".to_string(),
    });

    wait_for_state(&peer, "peer received m3", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| {
                chat.chat_id == primary_chat_id
                    && chat
                        .messages
                        .iter()
                        .any(|message| message.body == "m3" && !message.is_outgoing)
            })
            .unwrap_or(false)
    });
    primary.dispatch(AppAction::OpenChat {
        chat_id: peer_chat_id.clone(),
    });
    wait_for_state(&primary, "primary received synced m3", |state| {
        state
            .current_chat
            .as_ref()
            .map(|chat| {
                chat.chat_id == peer_chat_id
                    && chat
                        .messages
                        .iter()
                        .any(|message| message.body == "m3" && message.is_outgoing)
            })
            .unwrap_or(false)
    });

    primary.dispatch(AppAction::RemoveAuthorizedDevice {
        device_pubkey_hex: linked_account.device_public_key_hex.clone(),
    });
    wait_for_state(&linked, "linked revoked", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::Revoked
                )
            })
            .unwrap_or(false)
    });

    let before_count = linked
        .state()
        .current_chat
        .as_ref()
        .map(|chat| chat.messages.len())
        .unwrap_or(0);
    linked.dispatch(AppAction::SendMessage {
        chat_id: peer_chat_id.clone(),
        text: "blocked".to_string(),
    });
    let blocked_state = wait_for_state(&linked, "linked send blocked", |state| {
        state.toast.as_deref()
            == Some("This device has been removed from the roster. Log out to continue.")
    });
    assert_eq!(
        blocked_state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len())
            .unwrap_or(0),
        before_count
    );
}

#[test]
fn linked_device_can_be_approved_after_single_owner_qr_scan() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let primary_dir = TempDir::new().expect("primary dir");
    let linked_dir = TempDir::new().expect("linked dir");

    let primary = app_with_dir(primary_dir.path(), 214);
    let primary_account =
        wait_for_state(&primary, "primary account", |state| state.account.is_some())
            .account
            .expect("primary account");

    let linked = FfiApp::new(
        linked_dir.path().to_string_lossy().into_owned(),
        String::new(),
        "test".to_string(),
    );
    linked.dispatch(AppAction::StartLinkedDevice {
        owner_input: primary_account.npub.clone(),
    });

    let linked_account = wait_for_state(&linked, "linked awaiting approval", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::AwaitingApproval
                )
            })
            .unwrap_or(false)
    })
    .account
    .expect("linked account");
    let linked_device_pubkey =
        PublicKey::parse(&linked_account.device_public_key_hex).expect("linked device pubkey");

    let invite_deadline = Instant::now() + StdDuration::from_secs(5);
    loop {
        let invites = fetch_local_relay_events(vec![
            Filter::new().kind(Kind::from(codec::INVITE_EVENT_KIND as u16))
        ]);
        let published = invites.iter().any(|event| {
            codec::parse_invite_event(event).is_ok() && event.pubkey == linked_device_pubkey
        });
        if published {
            break;
        }
        assert!(
            Instant::now() < invite_deadline,
            "timed out waiting for linked device invite publish"
        );
        thread::sleep(StdDuration::from_millis(50));
    }

    primary.dispatch(AppAction::PushScreen {
        screen: Screen::DeviceRoster,
    });

    wait_for_state(
        &primary,
        "primary discovered pending linked device",
        |state| {
            state
                .device_roster
                .as_ref()
                .map(|roster| {
                    roster.devices.iter().any(|device| {
                        device.device_pubkey_hex == linked_account.device_public_key_hex
                            && !device.is_authorized
                    })
                })
                .unwrap_or(false)
        },
    );

    primary.dispatch(AppAction::AddAuthorizedDevice {
        device_input: linked_account.device_public_key_hex.clone(),
    });

    wait_for_state(&primary, "primary authorized linked device", |state| {
        state
            .device_roster
            .as_ref()
            .map(|roster| {
                roster.devices.iter().any(|device| {
                    device.device_pubkey_hex == linked_account.device_public_key_hex
                        && device.is_authorized
                })
            })
            .unwrap_or(false)
    });
    wait_for_state(&linked, "linked authorized", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::Authorized
                )
            })
            .unwrap_or(false)
    });
}

#[test]
fn linked_device_appears_live_when_primary_device_roster_is_already_open() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let primary_dir = TempDir::new().expect("primary dir");
    let linked_dir = TempDir::new().expect("linked dir");

    let primary = app_with_dir(primary_dir.path(), 216);
    let primary_account =
        wait_for_state(&primary, "primary account", |state| state.account.is_some())
            .account
            .expect("primary account");

    primary.dispatch(AppAction::PushScreen {
        screen: Screen::DeviceRoster,
    });
    wait_for_state(&primary, "primary device roster open", |state| {
        state.device_roster.is_some()
    });

    let linked = FfiApp::new(
        linked_dir.path().to_string_lossy().into_owned(),
        String::new(),
        "test".to_string(),
    );
    linked.dispatch(AppAction::StartLinkedDevice {
        owner_input: primary_account.npub.clone(),
    });

    let linked_account = wait_for_state(&linked, "linked awaiting approval", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::AwaitingApproval
                )
            })
            .unwrap_or(false)
    })
    .account
    .expect("linked account");

    wait_for_state(
        &primary,
        "primary observed pending linked device via live relay invite",
        |state| {
            state
                .device_roster
                .as_ref()
                .map(|roster| {
                    roster.devices.iter().any(|device| {
                        device.device_pubkey_hex == linked_account.device_public_key_hex
                            && !device.is_authorized
                    })
                })
                .unwrap_or(false)
        },
    );
}

#[test]
fn primary_discovers_pending_linked_device_without_opening_device_roster() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let primary_dir = TempDir::new().expect("primary dir");
    let linked_dir = TempDir::new().expect("linked dir");

    let primary = app_with_dir(primary_dir.path(), 217);
    let primary_account =
        wait_for_state(&primary, "primary account", |state| state.account.is_some())
            .account
            .expect("primary account");

    let linked = FfiApp::new(
        linked_dir.path().to_string_lossy().into_owned(),
        String::new(),
        "test".to_string(),
    );
    linked.dispatch(AppAction::StartLinkedDevice {
        owner_input: primary_account.npub.clone(),
    });

    let linked_account = wait_for_state(&linked, "linked awaiting approval", |state| {
        state
            .account
            .as_ref()
            .map(|account| {
                matches!(
                    account.authorization_state,
                    DeviceAuthorizationState::AwaitingApproval
                )
            })
            .unwrap_or(false)
    })
    .account
    .expect("linked account");

    wait_for_state_timeout(
        &primary,
        "primary discovered pending linked device without opening device roster",
        15,
        |state| {
            state
                .device_roster
                .as_ref()
                .map(|roster| {
                    roster.devices.iter().any(|device| {
                        device.device_pubkey_hex == linked_account.device_public_key_hex
                            && !device.is_authorized
                    })
                })
                .unwrap_or(false)
        },
    );
}

struct ScenarioClient {
    owner_fill: Option<u8>,
    data_dir: TempDir,
    app: Arc<FfiApp>,
}

#[allow(dead_code)]
enum ScenarioStep {
    SendDirect {
        sender: String,
        recipient: String,
        text: String,
    },
    SendGroup {
        sender: String,
        chat_id: String,
        text: String,
    },
}

struct ScenarioRunner<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    _env: RelayEnvGuard,
    relay: TestRelay,
    clients: BTreeMap<String, ScenarioClient>,
}

impl<'a> ScenarioRunner<'a> {
    fn local() -> Self {
        Self {
            _guard: relay_test_lock()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
            _env: RelayEnvGuard::local_only(),
            relay: TestRelay::start(),
            clients: BTreeMap::new(),
        }
    }

    fn add_owner(&mut self, label: &str, fill: u8) {
        let data_dir = TempDir::new().expect("temp dir");
        let app = app_with_dir(data_dir.path(), fill);
        self.clients.insert(
            label.to_string(),
            ScenarioClient {
                owner_fill: Some(fill),
                data_dir,
                app,
            },
        );
    }

    fn add_linked(&mut self, label: &str, owner_label: &str) {
        let data_dir = TempDir::new().expect("temp dir");
        let app = FfiApp::new(
            data_dir.path().to_string_lossy().into_owned(),
            String::new(),
            "test".to_string(),
        );
        app.dispatch(AppAction::StartLinkedDevice {
            owner_input: self.owner_npub(owner_label),
        });
        let linked_account = wait_for_state(&app, "linked awaiting approval", |state| {
            state
                .account
                .as_ref()
                .map(|account| {
                    matches!(
                        account.authorization_state,
                        DeviceAuthorizationState::AwaitingApproval
                    )
                })
                .unwrap_or(false)
        })
        .account
        .expect("linked account");

        self.app(owner_label)
            .dispatch(AppAction::AddAuthorizedDevice {
                device_input: linked_account.device_npub.clone(),
            });
        wait_for_state(&app, "linked authorized", |state| {
            state
                .account
                .as_ref()
                .map(|account| {
                    matches!(
                        account.authorization_state,
                        DeviceAuthorizationState::Authorized
                    )
                })
                .unwrap_or(false)
        });

        self.clients.insert(
            label.to_string(),
            ScenarioClient {
                owner_fill: None,
                data_dir,
                app,
            },
        );
    }

    fn app(&self, label: &str) -> &Arc<FfiApp> {
        &self
            .clients
            .get(label)
            .unwrap_or_else(|| panic!("missing scenario client `{label}`"))
            .app
    }

    fn data_dir(&self, label: &str) -> &Path {
        self.clients
            .get(label)
            .unwrap_or_else(|| panic!("missing scenario client `{label}`"))
            .data_dir
            .path()
    }

    fn account(&self, label: &str) -> AccountSnapshot {
        wait_for_state(self.app(label), "scenario account", |state| {
            state.account.is_some()
        })
        .account
        .expect("account")
    }

    fn owner_npub(&self, label: &str) -> String {
        self.account(label).npub
    }

    fn owner_hex(&self, label: &str) -> String {
        self.account(label).public_key_hex
    }

    fn open_chat(&self, label: &str, chat_id: &str) {
        self.app(label).dispatch(AppAction::OpenChat {
            chat_id: chat_id.to_string(),
        });
        wait_for_state(self.app(label), "open chat", |state| {
            state
                .current_chat
                .as_ref()
                .map(|chat| chat.chat_id == chat_id)
                .unwrap_or(false)
        });
    }

    fn create_group(&self, sender: &str, name: &str, members: &[&str]) -> String {
        self.create_group_by_inputs(
            sender,
            name,
            members
                .iter()
                .map(|member| self.owner_npub(member))
                .collect(),
        )
    }

    fn create_group_by_inputs(
        &self,
        sender: &str,
        name: &str,
        member_inputs: Vec<String>,
    ) -> String {
        self.app(sender).dispatch(AppAction::CreateGroup {
            name: name.to_string(),
            member_inputs,
        });
        wait_for_state_timeout(self.app(sender), "group create", 30, |state| {
            state
                .current_chat
                .as_ref()
                .map(|chat| matches!(chat.kind, ChatKind::Group))
                .unwrap_or(false)
        })
        .current_chat
        .expect("group chat")
        .chat_id
    }

    #[allow(dead_code)]
    fn add_group_members(&self, sender: &str, group_id: &str, members: &[&str]) {
        self.app(sender).dispatch(AppAction::AddGroupMembers {
            group_id: group_id.to_string(),
            member_inputs: members
                .iter()
                .map(|member| self.owner_npub(member))
                .collect(),
        });
    }

    fn add_group_members_by_inputs(
        &self,
        sender: &str,
        group_id: &str,
        member_inputs: Vec<String>,
    ) {
        self.app(sender).dispatch(AppAction::AddGroupMembers {
            group_id: group_id.to_string(),
            member_inputs,
        });
    }

    fn remove_group_member(&self, sender: &str, group_id: &str, member: &str) {
        self.app(sender).dispatch(AppAction::RemoveGroupMember {
            group_id: group_id.to_string(),
            owner_pubkey_hex: self.owner_hex(member),
        });
    }

    fn run_step(&self, step: ScenarioStep) {
        match step {
            ScenarioStep::SendDirect {
                sender,
                recipient,
                text,
            } => {
                self.app(&sender).dispatch(AppAction::SendMessage {
                    chat_id: self.owner_hex(&recipient),
                    text,
                });
            }
            ScenarioStep::SendGroup {
                sender,
                chat_id,
                text,
            } => {
                self.app(&sender)
                    .dispatch(AppAction::SendMessage { chat_id, text });
            }
        }
    }

    fn wait_for_group(&self, label: &str, chat_id: &str, member_count: u64) {
        wait_for_state_timeout(self.app(label), "group thread", 45, |state| {
            state.chat_list.iter().any(|thread| {
                thread.chat_id == chat_id
                    && matches!(thread.kind, ChatKind::Group)
                    && thread.member_count == member_count
            })
        });
        self.open_chat(label, chat_id);
    }

    fn wait_for_message(&self, label: &str, chat_id: &str, text: &str, is_outgoing: bool) {
        self.open_chat(label, chat_id);
        wait_for_state_timeout(self.app(label), "group message", 45, |state| {
            state
                .current_chat
                .as_ref()
                .filter(|chat| chat.chat_id == chat_id)
                .map(|chat| {
                    chat.messages
                        .iter()
                        .any(|message| message.body == text && message.is_outgoing == is_outgoing)
                })
                .unwrap_or(false)
        });
    }

    fn wait_for_message_count(&self, label: &str, chat_id: &str, expected_count: usize) {
        self.open_chat(label, chat_id);
        wait_for_state_timeout(self.app(label), "message count", 60, |state| {
            state
                .current_chat
                .as_ref()
                .filter(|chat| chat.chat_id == chat_id)
                .map(|chat| chat.messages.len() == expected_count)
                .unwrap_or(false)
        });
    }

    fn restart_owner(&mut self, label: &str) {
        let mut client = self
            .clients
            .remove(label)
            .unwrap_or_else(|| panic!("missing scenario client `{label}`"));
        let owner_fill = client
            .owner_fill
            .expect("scenario restart only supports deterministic owner clients");
        client.app.shutdown_blocking();
        drop(client.app);
        client.app = app_with_dir(client.data_dir.path(), owner_fill);
        self.clients.insert(label.to_string(), client);
    }

    fn replay_stored(&self) {
        self.relay.replay_stored();
        thread::sleep(StdDuration::from_secs(1));
    }
}

#[test]
fn support_bundle_export_is_redacted_and_contains_build_metadata() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_data_dir, app) = app(211);

    let bundle = app.export_support_bundle_json();
    let parsed: Value = serde_json::from_str(&bundle).expect("support bundle json");

    assert_eq!(parsed["build"]["app_version"].as_str(), Some(APP_VERSION));
    assert_eq!(parsed["build"]["relay_set_id"].as_str(), Some(RELAY_SET_ID));
    assert!(parsed["relay_urls"].is_array());
    assert!(parsed["known_users"].is_array());
    assert!(parsed["pending_outbound"].is_array());
    assert!(!bundle.contains("secret_key"));
    assert!(!bundle.contains("\"body\""));
}

#[test]
fn network_status_snapshot_exposes_relay_and_debug_state() {
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    core.state.busy.syncing_network = true;
    core.push_debug_log("protocol.test", "visible status");
    core.rebuild_state();

    let status = core.state.network_status.as_ref().expect("network status");
    assert_eq!(status.relay_set_id, RELAY_SET_ID);
    assert_eq!(status.relay_urls, configured_relays());
    assert!(status.syncing);
    assert_eq!(status.recent_log_count, 1);
    assert_eq!(status.last_debug_category.as_deref(), Some("protocol.test"));
    assert_eq!(status.last_debug_detail.as_deref(), Some("visible status"));
}

#[test]
fn create_account_with_name_publishes_metadata_event() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());

    core.create_account("Alice");

    let owner_hex = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .owner_pubkey
        .to_string();
    let owner_pubkey = PublicKey::parse(&owner_hex).expect("owner pubkey");

    let deadline = Instant::now() + StdDuration::from_secs(5);
    let metadata_event = loop {
        let events = fetch_local_relay_events(vec![Filter::new()
            .kind(Kind::Metadata)
            .authors(vec![owner_pubkey])]);
        if let Some(event) = events.into_iter().next() {
            break event;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for metadata event"
        );
        thread::sleep(StdDuration::from_millis(50));
    };

    let metadata: NostrProfileMetadata =
        serde_json::from_str(&metadata_event.content).expect("metadata json");
    assert_eq!(metadata.name.as_deref(), Some("Alice"));
    assert_eq!(metadata.display_name.as_deref(), Some("Alice"));
    assert_eq!(
        core.state.account.as_ref().expect("account").display_name,
        "Alice"
    );
}

#[test]
fn update_profile_metadata_changes_local_profile_and_republishes() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _env = RelayEnvGuard::local_only();
    let _relay = TestRelay::start();
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    core.create_account("Alice");

    core.update_profile_metadata("Alicia", Some("https://example.com/alicia.png"));

    assert_eq!(
        core.state.account.as_ref().expect("account").display_name,
        "Alicia"
    );
    let owner_hex = core
        .logged_in
        .as_ref()
        .expect("logged in")
        .owner_pubkey
        .to_string();
    let owner_pubkey = PublicKey::parse(&owner_hex).expect("owner pubkey");
    let deadline = Instant::now() + StdDuration::from_secs(5);
    let metadata_event = loop {
        let events = fetch_local_relay_events(vec![Filter::new()
            .kind(Kind::Metadata)
            .authors(vec![owner_pubkey])]);
        if let Some(event) = events
            .into_iter()
            .find(|event| event.content.contains("Alicia"))
        {
            break event;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for updated metadata event"
        );
        thread::sleep(StdDuration::from_millis(50));
    };
    let metadata: NostrProfileMetadata =
        serde_json::from_str(&metadata_event.content).expect("metadata json");
    assert_eq!(metadata.name.as_deref(), Some("Alicia"));
    assert_eq!(metadata.display_name.as_deref(), Some("Alicia"));
    assert_eq!(
        metadata.picture.as_deref(),
        Some("https://example.com/alicia.png")
    );
    assert_eq!(
        core.state
            .account
            .as_ref()
            .expect("account")
            .picture_url
            .as_deref(),
        Some("https://example.com/alicia.png")
    );
}

#[test]
fn metadata_event_updates_direct_chat_display_name() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 212, false, false).expect("start session");

    let peer_keys = keys_for_fill(213);
    let peer_npub = peer_keys.public_key().to_bech32().expect("peer npub");
    let peer_hex = peer_keys.public_key().to_hex();

    core.create_chat(&peer_npub);

    let metadata_event = EventBuilder::new(
        Kind::Metadata,
        serde_json::to_string(&NostrProfileMetadata {
            name: Some("Bob".to_string()),
            display_name: Some("Bob".to_string()),
            picture: None,
        })
        .expect("metadata"),
    )
    .sign_with_keys(&peer_keys)
    .expect("metadata event");

    core.handle_relay_event(metadata_event);

    let chat = core
        .state
        .chat_list
        .iter()
        .find(|chat| chat.chat_id == peer_hex)
        .expect("chat row");
    assert_eq!(chat.display_name, "Bob");
    assert_eq!(chat.subtitle, None);
    assert_eq!(
        core.state
            .current_chat
            .as_ref()
            .expect("current chat")
            .display_name,
        "Bob"
    );
}

#[test]
fn metadata_event_updates_group_member_display_name() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let (alice_manager, _bob_manager, bob_owner_hex) =
        established_session_manager_pair(214, 215, 1_900_000_600);
    let mut core = logged_in_core_with_manager(data_dir.path(), 214, alice_manager);

    core.create_group("Project crew", std::slice::from_ref(&bob_owner_hex));
    let group_id = core
        .state
        .current_chat
        .as_ref()
        .expect("current chat")
        .group_id
        .clone()
        .expect("group id");

    let bob_keys = keys_for_fill(215);
    let metadata_event = EventBuilder::new(
        Kind::Metadata,
        serde_json::to_string(&NostrProfileMetadata {
            name: Some("Bob".to_string()),
            display_name: Some("Bobby".to_string()),
            picture: None,
        })
        .expect("metadata"),
    )
    .sign_with_keys(&bob_keys)
    .expect("metadata event");

    core.handle_relay_event(metadata_event);

    let details = core
        .build_group_details_snapshot(&group_id)
        .expect("group details");
    let bob_member = details
        .members
        .iter()
        .find(|member| member.owner_pubkey_hex == bob_owner_hex)
        .expect("bob member");
    assert_eq!(bob_member.display_name, "Bobby");
    assert_eq!(
        details
            .members
            .iter()
            .find(|member| member.is_local_owner)
            .expect("local member")
            .display_name,
        core.state.account.as_ref().expect("account").display_name
    );
}

#[test]
fn owner_profile_restores_display_name() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut seeded = test_core(data_dir.path());
    start_primary_test_session(&mut seeded, 216, false, false).expect("seed session");
    seeded.set_local_profile_name("Alice");
    seeded.rebuild_state();
    seeded.persist_best_effort();

    let mut restored = test_core(data_dir.path());
    start_primary_test_session(&mut restored, 216, true, true).expect("restore session");

    assert_eq!(
        restored
            .state
            .account
            .as_ref()
            .expect("account")
            .display_name,
        "Alice"
    );
}

#[test]
fn metadata_event_updates_local_profile_picture_url() {
    let _guard = relay_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let data_dir = TempDir::new().expect("temp dir");
    let mut core = test_core(data_dir.path());
    start_primary_test_session(&mut core, 217, false, false).expect("session");
    let owner_keys = keys_for_fill(217);

    let metadata_event = EventBuilder::new(
        Kind::Metadata,
        serde_json::to_string(&NostrProfileMetadata {
            name: Some("Alice".to_string()),
            display_name: Some("Alice".to_string()),
            picture: Some("https://example.com/alice.jpg".to_string()),
        })
        .expect("metadata"),
    )
    .sign_with_keys(&owner_keys)
    .expect("metadata event");

    assert!(core.apply_profile_metadata_event(&metadata_event));
    core.rebuild_state();

    assert_eq!(
        core.state
            .account
            .as_ref()
            .expect("account")
            .picture_url
            .as_deref(),
        Some("https://example.com/alice.jpg")
    );
}

#[test]
fn twenty_owner_group_converges() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 220);
    for offset in 1..20 {
        scenario.add_owner(&format!("owner{offset:02}"), 220 + offset as u8);
    }

    let mut member_labels = Vec::new();
    for offset in 1..20 {
        member_labels.push(format!("owner{offset:02}"));
    }
    let member_refs = member_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let chat_id = scenario.create_group("admin", "Twenty owners", &member_refs);

    for label in member_refs.iter().copied().chain(std::iter::once("admin")) {
        scenario.wait_for_group(label, &chat_id, 20);
    }

    let senders = std::iter::once("admin".to_string()).chain(member_labels.clone());
    for (index, sender) in senders.enumerate() {
        let text = format!("twenty-{index:02}");
        scenario.run_step(ScenarioStep::SendGroup {
            sender: sender.clone(),
            chat_id: chat_id.clone(),
            text: text.clone(),
        });
        for label in member_refs.iter().copied().chain(std::iter::once("admin")) {
            let is_outgoing = label == sender;
            scenario.wait_for_message(label, &chat_id, &text, is_outgoing);
        }
    }

    for label in member_refs.iter().copied().chain(std::iter::once("admin")) {
        scenario.wait_for_message_count(label, &chat_id, 20);
    }
}

#[test]
fn group_with_linked_devices_converges() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("primary", 240);
    scenario.add_linked("linked", "primary");
    scenario.add_owner("admin", 242);
    scenario.add_owner("member", 243);

    let chat_id = scenario.create_group("admin", "Linked converge", &["primary", "member"]);
    for label in ["admin", "primary", "linked", "member"] {
        scenario.wait_for_group(label, &chat_id, 3);
    }

    for (sender, text) in [
        ("admin", "from-admin"),
        ("linked", "from-linked"),
        ("member", "from-member"),
    ] {
        scenario.run_step(ScenarioStep::SendGroup {
            sender: sender.to_string(),
            chat_id: chat_id.clone(),
            text: text.to_string(),
        });
    }

    scenario.wait_for_message("admin", &chat_id, "from-admin", true);
    scenario.wait_for_message("primary", &chat_id, "from-admin", false);
    scenario.wait_for_message("linked", &chat_id, "from-admin", false);
    scenario.wait_for_message("member", &chat_id, "from-admin", false);

    scenario.wait_for_message("admin", &chat_id, "from-linked", false);
    scenario.wait_for_message("primary", &chat_id, "from-linked", true);
    scenario.wait_for_message("linked", &chat_id, "from-linked", true);
    scenario.wait_for_message("member", &chat_id, "from-linked", false);

    scenario.wait_for_message("admin", &chat_id, "from-member", false);
    scenario.wait_for_message("primary", &chat_id, "from-member", false);
    scenario.wait_for_message("linked", &chat_id, "from-member", false);
    scenario.wait_for_message("member", &chat_id, "from-member", true);
}

#[test]
fn linked_device_receives_existing_group_after_next_primary_send() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("primary", 244);
    scenario.add_owner("admin", 245);
    scenario.add_owner("member", 246);

    let chat_id = scenario.create_group("admin", "Late linked", &["primary", "member"]);
    scenario.wait_for_group("admin", &chat_id, 3);
    scenario.wait_for_group("primary", &chat_id, 3);
    scenario.wait_for_group("member", &chat_id, 3);

    scenario.add_linked("linked", "primary");
    let primary_owner_hex = scenario.owner_hex("primary");
    wait_for_persisted_state(
        scenario.data_dir("primary"),
        "primary sibling transport ready",
        |persisted| {
            persisted
                .session_manager
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .users
                        .iter()
                        .find(|user| user.owner_pubkey.to_string() == primary_owner_hex)
                })
                .and_then(|user| {
                    user.roster.as_ref().map(|roster| {
                        roster.devices.iter().all(|roster_device| {
                            user.devices.iter().any(|device| {
                                device.device_pubkey == roster_device.device_pubkey
                                    && device.public_invite.is_some()
                            })
                        })
                    })
                })
                .unwrap_or(false)
        },
    );
    wait_for_persisted_state(
        scenario.data_dir("linked"),
        "linked sibling transport ready",
        |persisted| {
            persisted
                .session_manager
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .users
                        .iter()
                        .find(|user| user.owner_pubkey.to_string() == primary_owner_hex)
                })
                .and_then(|user| {
                    user.roster.as_ref().map(|roster| {
                        roster.devices.iter().all(|roster_device| {
                            user.devices.iter().any(|device| {
                                device.device_pubkey == roster_device.device_pubkey
                                    && device.public_invite.is_some()
                            })
                        })
                    })
                })
                .unwrap_or(false)
        },
    );
    thread::sleep(StdDuration::from_secs(1));
    assert!(!scenario
        .app("linked")
        .state()
        .chat_list
        .iter()
        .any(|thread| thread.chat_id == chat_id));

    scenario.run_step(ScenarioStep::SendGroup {
        sender: "primary".to_string(),
        chat_id: chat_id.clone(),
        text: "after-link-bootstrap".to_string(),
    });

    scenario.wait_for_group("linked", &chat_id, 3);
    scenario.wait_for_message("linked", &chat_id, "after-link-bootstrap", true);
    scenario.wait_for_message("admin", &chat_id, "after-link-bootstrap", false);
    scenario.wait_for_message("member", &chat_id, "after-link-bootstrap", false);
}

#[test]
fn restart_mid_group_create_recovers() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 250);

    let pending_member_npub = npub_for_fill(251);
    let chat_id = scenario.create_group_by_inputs(
        "admin",
        "Restart create",
        vec![pending_member_npub.clone()],
    );
    wait_for_persisted_state(
        scenario.data_dir("admin"),
        "pending group create",
        |persisted| !persisted.pending_group_controls.is_empty(),
    );

    scenario.restart_owner("admin");
    scenario.add_owner("member", 251);

    scenario.wait_for_group("member", &chat_id, 2);
    scenario.run_step(ScenarioStep::SendGroup {
        sender: "admin".to_string(),
        chat_id: chat_id.clone(),
        text: "after-create-restart".to_string(),
    });
    scenario.wait_for_message("member", &chat_id, "after-create-restart", false);
}

#[test]
fn restart_mid_member_add_recovers() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 252);
    scenario.add_owner("member1", 253);

    let chat_id = scenario.create_group("admin", "Restart add", &["member1"]);
    scenario.wait_for_group("member1", &chat_id, 2);

    scenario.add_group_members_by_inputs("admin", &chat_id, vec![npub_for_fill(254)]);
    scenario.wait_for_group("admin", &chat_id, 3);
    let persisted_group_id = parse_group_id_from_chat_id(&chat_id).expect("group id");
    wait_for_persisted_state(
        scenario.data_dir("admin"),
        "pending member add",
        |persisted| {
            !persisted.pending_group_controls.is_empty()
                && persisted
                    .threads
                    .iter()
                    .any(|thread| thread.chat_id == chat_id)
                && persisted
                    .group_manager
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .groups
                            .iter()
                            .find(|group| group.group_id == persisted_group_id)
                    })
                    .map(|group| group.members.len() == 3)
                    .unwrap_or(false)
        },
    );

    scenario.restart_owner("admin");
    scenario.add_owner("member2", 254);

    for label in ["admin", "member1", "member2"] {
        scenario.wait_for_group(label, &chat_id, 3);
    }
    scenario.run_step(ScenarioStep::SendGroup {
        sender: "member2".to_string(),
        chat_id: chat_id.clone(),
        text: "member-two-arrived".to_string(),
    });
    scenario.wait_for_message("admin", &chat_id, "member-two-arrived", false);
    scenario.wait_for_message("member1", &chat_id, "member-two-arrived", false);
}

#[test]
fn duplicate_and_replayed_events_do_not_duplicate_threads_or_messages() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 244);
    scenario.add_owner("member", 246);

    let chat_id = scenario.create_group("admin", "Replay safe", &["member"]);
    scenario.wait_for_group("member", &chat_id, 2);

    scenario.run_step(ScenarioStep::SendGroup {
        sender: "admin".to_string(),
        chat_id: chat_id.clone(),
        text: "dedupe-group".to_string(),
    });
    scenario.wait_for_message("admin", &chat_id, "dedupe-group", true);
    scenario.wait_for_message("member", &chat_id, "dedupe-group", false);

    scenario.replay_stored();

    let admin_state = scenario.app("admin").state();
    let member_state = scenario.app("member").state();
    assert_eq!(
        admin_state
            .chat_list
            .iter()
            .filter(|thread| thread.chat_id == chat_id)
            .count(),
        1
    );
    assert_eq!(
        member_state
            .chat_list
            .iter()
            .filter(|thread| thread.chat_id == chat_id)
            .count(),
        1
    );
    scenario.wait_for_message_count("admin", &chat_id, 1);
    scenario.wait_for_message_count("member", &chat_id, 1);
}

#[test]
fn member_removal_blocks_future_sends() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 247);
    scenario.add_owner("member", 248);

    let chat_id = scenario.create_group("admin", "Removal block", &["member"]);
    scenario.wait_for_group("member", &chat_id, 2);
    scenario.remove_group_member("admin", &chat_id, "member");
    scenario.wait_for_group("admin", &chat_id, 1);
    scenario.wait_for_group("member", &chat_id, 1);

    scenario.open_chat("member", &chat_id);
    let before_count = scenario
        .app("member")
        .state()
        .current_chat
        .as_ref()
        .map(|chat| chat.messages.len())
        .unwrap_or(0);
    scenario.run_step(ScenarioStep::SendGroup {
        sender: "member".to_string(),
        chat_id: chat_id.clone(),
        text: "blocked-after-removal".to_string(),
    });
    thread::sleep(StdDuration::from_secs(1));
    let after_state = scenario.app("member").state();
    assert_eq!(
        after_state
            .current_chat
            .as_ref()
            .map(|chat| chat.messages.len())
            .unwrap_or(0),
        before_count
    );
}

#[test]
fn post_removal_messages_reach_only_remaining_members() {
    let mut scenario = ScenarioRunner::local();
    scenario.add_owner("admin", 249);
    scenario.add_owner("member1", 250);
    scenario.add_owner("member2", 251);

    let chat_id = scenario.create_group("admin", "Post removal", &["member1", "member2"]);
    for label in ["admin", "member1", "member2"] {
        scenario.wait_for_group(label, &chat_id, 3);
    }

    scenario.remove_group_member("admin", &chat_id, "member2");
    scenario.wait_for_group("admin", &chat_id, 2);
    scenario.wait_for_group("member1", &chat_id, 2);
    scenario.wait_for_group("member2", &chat_id, 2);

    scenario.run_step(ScenarioStep::SendGroup {
        sender: "admin".to_string(),
        chat_id: chat_id.clone(),
        text: "after-removal".to_string(),
    });
    scenario.wait_for_message("admin", &chat_id, "after-removal", true);
    scenario.wait_for_message("member1", &chat_id, "after-removal", false);
    thread::sleep(StdDuration::from_secs(2));
    let member2_state = scenario.app("member2").state();
    assert!(member2_state
        .current_chat
        .as_ref()
        .map(|chat| chat
            .messages
            .iter()
            .all(|message| message.body != "after-removal"))
        .unwrap_or(true));
}
