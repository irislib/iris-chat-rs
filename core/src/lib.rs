mod actions;
mod core;
pub mod desktop_nearby;
pub mod image_proxy;
pub mod local_relay;
pub mod perflog;
mod qr;
mod state;
mod updates;

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{panic, panic::AssertUnwindSafe};

use flume::{Receiver, Sender};

pub use actions::AppAction;
pub use qr::*;
pub use state::*;
pub use updates::*;

use crate::core::AppCore;

uniffi::setup_scaffolding!();

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DesktopNearbyPeerSnapshot {
    pub id: String,
    pub name: String,
    pub owner_pubkey_hex: Option<String>,
    pub picture_url: Option<String>,
    pub profile_event_id: Option<String>,
    pub last_seen_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DesktopNearbySnapshot {
    pub visible: bool,
    pub status: String,
    pub peers: Vec<DesktopNearbyPeerSnapshot>,
}

#[uniffi::export(callback_interface)]
pub trait DesktopNearbyObserver: Send + Sync + 'static {
    fn desktop_nearby_changed(&self, snapshot: DesktopNearbySnapshot);
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
    /// Shared SQLite handle used by `search`. None when `try_new`
    /// failed (we surface a toast to the user instead of bringing up
    /// a working core).
    shared_db: Option<crate::core::SharedConnection>,
}

#[derive(uniffi::Object)]
pub struct FfiDesktopNearby {
    service: Arc<desktop_nearby::DesktopNearbyService>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String, _keychain_group: String, _app_version: String) -> Arc<Self> {
        match panic::catch_unwind(AssertUnwindSafe(|| new_ffi_app_inner(data_dir))) {
            Ok(app) => app,
            Err(payload) => ffi_app_failure(format!(
                "Iris could not start: {}",
                panic_payload_to_string(payload)
            )),
        }
    }

    pub fn state(&self) -> AppState {
        ffi_or("ffiapp.state", ffi_failure_state(), || {
            match self.shared_state.read() {
                Ok(slot) => slot.clone(),
                Err(poison) => poison.into_inner().clone(),
            }
        })
    }

    pub fn dispatch(&self, action: AppAction) {
        ffi_or("ffiapp.dispatch", (), || {
            crate::perflog!("ffi.dispatch action={:?}", std::mem::discriminant(&action));
            let _ = self.core_tx.send(CoreMsg::Action(action));
        })
    }

    /// Grouped Signal-style search: filters the in-memory chat list
    /// into contacts/groups by display name + subtitle + chat id, and
    /// runs the SQLite FTS5 index for the messages section. Optional
    /// `scope_chat_id` restricts message hits to a single thread (the
    /// "search in this chat" pill in the desktop sidebar). Returns an
    /// empty snapshot for empty / whitespace queries.
    pub fn search(
        &self,
        query: String,
        scope_chat_id: Option<String>,
        limit: u32,
    ) -> SearchResultSnapshot {
        ffi_or(
            "ffiapp.search",
            SearchResultSnapshot::empty(query.clone(), scope_chat_id.clone()),
            || {
                let trimmed = query.trim();
                if trimmed.is_empty() {
                    return SearchResultSnapshot::empty(query.clone(), scope_chat_id.clone());
                }
                let limit = limit.max(1) as usize;
                let state_snapshot = match self.shared_state.read() {
                    Ok(slot) => slot.clone(),
                    Err(poison) => poison.into_inner().clone(),
                };
                let (contacts, groups) = if scope_chat_id.is_some() {
                    (Vec::new(), Vec::new())
                } else {
                    filter_threads_for_search(&state_snapshot.chat_list, trimmed)
                };
                let messages = match self.shared_db.as_ref() {
                    Some(shared) => match shared.lock() {
                        Ok(conn) => crate::core::search_messages_fts(
                            &conn,
                            trimmed,
                            scope_chat_id.as_deref(),
                            limit,
                        )
                        .unwrap_or_default(),
                        Err(poison) => crate::core::search_messages_fts(
                            &poison.into_inner(),
                            trimmed,
                            scope_chat_id.as_deref(),
                            limit,
                        )
                        .unwrap_or_default(),
                    },
                    None => Vec::new(),
                };
                let enriched = enrich_message_hits(messages, &state_snapshot.chat_list);
                // The shortcut row only makes sense for global search.
                // Once the user has scoped to a single chat, an npub
                // paste should still search that chat's messages, not
                // jump out of the scope.
                let shortcut = if scope_chat_id.is_none() {
                    chat_input_shortcut(trimmed)
                } else {
                    None
                };
                SearchResultSnapshot {
                    query,
                    scope_chat_id,
                    contacts,
                    groups,
                    messages: enriched,
                    shortcut,
                }
            },
        )
    }

    pub fn ingest_nearby_event_json(&self, event_json: String) -> bool {
        self.ingest_nearby_event_json_with_transport(event_json, String::new())
    }

    pub fn ingest_nearby_event_json_with_transport(
        &self,
        event_json: String,
        transport: String,
    ) -> bool {
        ffi_or("ffiapp.ingest_nearby_event_json", false, || {
            let event = match serde_json::from_str::<nostr_sdk::prelude::Event>(&event_json) {
                Ok(event) => event,
                Err(_) => return false,
            };
            if event.verify().is_err() {
                return false;
            }
            self.core_tx
                .send(CoreMsg::Internal(Box::new(InternalEvent::NearbyEvent {
                    event,
                    transport,
                })))
                .is_ok()
        })
    }

    pub fn build_nearby_presence_event_json(
        &self,
        peer_id: String,
        my_nonce: String,
        their_nonce: String,
        profile_event_id: String,
    ) -> String {
        ffi_or(
            "ffiapp.build_nearby_presence_event_json",
            String::new(),
            || {
                let (reply_tx, reply_rx) = flume::bounded(1);
                if self
                    .core_tx
                    .send(CoreMsg::BuildNearbyPresenceEvent {
                        peer_id,
                        my_nonce,
                        their_nonce,
                        profile_event_id,
                        reply_tx,
                    })
                    .is_err()
                {
                    return String::new();
                }
                reply_rx
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or_default()
            },
        )
    }

    pub fn verify_nearby_presence_event_json(
        &self,
        event_json: String,
        peer_id: String,
        my_nonce: String,
        their_nonce: String,
    ) -> String {
        ffi_or(
            "ffiapp.verify_nearby_presence_event_json",
            String::new(),
            || verify_nearby_presence_event_json(&event_json, &peer_id, &my_nonce, &their_nonce),
        )
    }

    pub fn nearby_encode_frame(&self, envelope_json: String) -> Vec<u8> {
        ffi_or("ffiapp.nearby_encode_frame", Vec::new(), || {
            nostr_double_ratchet_runtime::encode_nearby_frame_json(&envelope_json)
                .unwrap_or_default()
        })
    }

    pub fn nearby_decode_frame(&self, frame: Vec<u8>) -> String {
        ffi_or("ffiapp.nearby_decode_frame", String::new(), || {
            nostr_double_ratchet_runtime::decode_nearby_frame_json(&frame).unwrap_or_default()
        })
    }

    pub fn nearby_frame_body_len_from_header(&self, header: Vec<u8>) -> i32 {
        ffi_or("ffiapp.nearby_frame_body_len_from_header", -1, || {
            nostr_double_ratchet_runtime::nearby_frame_body_len_from_header(&header)
                .and_then(|len| i32::try_from(len).ok())
                .unwrap_or(-1)
        })
    }

    pub fn export_support_bundle_json(&self) -> String {
        ffi_or(
            "ffiapp.export_support_bundle_json",
            "{}".to_string(),
            || {
                let (reply_tx, reply_rx) = flume::bounded(1);
                if self
                    .core_tx
                    .send(CoreMsg::ExportSupportBundle(reply_tx))
                    .is_err()
                {
                    return "{}".to_string();
                }
                reply_rx
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or_else(|_| "{}".to_string())
            },
        )
    }

    pub fn peer_profile_debug(&self, owner_input: String) -> Option<PeerProfileDebugSnapshot> {
        ffi_or("ffiapp.peer_profile_debug", None, || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .core_tx
                .send(CoreMsg::PeerProfileDebug {
                    owner_input,
                    reply_tx,
                })
                .is_err()
            {
                return None;
            }
            reply_rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
        })
    }

    pub fn prepare_for_suspend(&self) {
        ffi_or("ffiapp.prepare_for_suspend", (), || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .core_tx
                .send(CoreMsg::PrepareForSuspend(reply_tx))
                .is_err()
            {
                return;
            }
            let _ = reply_rx.recv_timeout(Duration::from_secs(2));
        })
    }

    pub fn shutdown(&self) {
        ffi_or("ffiapp.shutdown", (), || {
            let (reply_tx, reply_rx) = flume::bounded(1);
            if self
                .core_tx
                .send(CoreMsg::Shutdown(Some(reply_tx)))
                .is_err()
            {
                return;
            }
            let _ = reply_rx.recv_timeout(Duration::from_secs(2));
        })
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let update_rx = self.update_rx.clone();
        let spawn_result = thread::Builder::new()
            .name("iris-updates".to_string())
            .spawn(move || {
                // Drain queued updates and deliver the latest FullState only.
                // The shell side already discards FullStates with stale `rev`,
                // but the JNI marshal of an AppState is itself ~20-30 ms and
                // each push triggers a full Compose recomposition (~400 ms on
                // Android debug). When the core emits a tight burst of 3-4
                // updates (OpenChat → SyncComplete → FetchCatchUpEvents → …)
                // the UI keeps re-rendering for seconds even though only the
                // final state mattered.
                //
                // PersistAccountBundle is a side-effect (key persistence), not
                // a UI update, so we never collapse those — every one must run.
                while let Ok(first) = update_rx.recv() {
                    let mut latest_full_state: Option<AppUpdate> = None;
                    let mut sidecar: Vec<AppUpdate> = Vec::new();
                    let process =
                        |update: AppUpdate,
                         latest: &mut Option<AppUpdate>,
                         side: &mut Vec<AppUpdate>| match update {
                            full @ AppUpdate::FullState(_) => *latest = Some(full),
                            other => side.push(other),
                        };
                    process(first, &mut latest_full_state, &mut sidecar);
                    while let Ok(next) = update_rx.try_recv() {
                        process(next, &mut latest_full_state, &mut sidecar);
                    }
                    for update in sidecar.into_iter().chain(latest_full_state) {
                        let kind = match &update {
                            AppUpdate::FullState(_) => "FullState",
                            AppUpdate::PersistAccountBundle { .. } => "PersistAccountBundle",
                            AppUpdate::NearbyPublishedEvent { .. } => "NearbyPublishedEvent",
                        };
                        let t0 = crate::perflog::now_ms();
                        crate::perflog!("reconcile.start kind={kind}");
                        if panic::catch_unwind(AssertUnwindSafe(|| reconciler.reconcile(update)))
                            .is_err()
                        {
                            crate::perflog!("reconcile.failed kind={kind}");
                            continue;
                        }
                        crate::perflog!(
                            "reconcile.end kind={kind} elapsed_ms={}",
                            crate::perflog::now_ms().saturating_sub(t0)
                        );
                    }
                }
            });
        if let Err(error) = spawn_result {
            crate::perflog!("updates.spawn.failed error={error}");
            self.listening.store(false, Ordering::SeqCst);
        }
    }
}

#[uniffi::export]
impl FfiDesktopNearby {
    #[uniffi::constructor]
    pub fn new(app: Arc<FfiApp>, observer: Box<dyn DesktopNearbyObserver>) -> Arc<Self> {
        Arc::new(Self {
            service: desktop_nearby::DesktopNearbyService::new(app, observer.into()),
        })
    }

    pub fn start(&self, local_name: String) {
        self.service.start(local_name);
    }

    pub fn stop(&self) {
        self.service.stop();
    }

    pub fn snapshot(&self) -> DesktopNearbySnapshot {
        self.service.snapshot()
    }

    pub fn publish(&self, event_id: String, kind: u32, created_at_secs: u64, event_json: String) {
        self.service
            .publish(event_id, kind, created_at_secs, event_json);
    }
}

fn new_ffi_app_inner(data_dir: String) -> Arc<FfiApp> {
    let (update_tx, update_rx) = flume::unbounded();
    let (core_tx, core_rx) = flume::unbounded();
    let shared_state = Arc::new(RwLock::new(AppState::empty()));

    let core_tx_for_thread = core_tx.clone();
    let shared_for_thread = shared_state.clone();
    let update_tx_for_error = update_tx.clone();
    let mut shared_db = None;
    match AppCore::try_new(update_tx, core_tx_for_thread, data_dir, shared_for_thread) {
        Ok(mut core) => {
            shared_db = Some(core.shared_db());
            let spawn_result =
                thread::Builder::new()
                    .name("iris-core".to_string())
                    .spawn(move || {
                        // Drain whatever is already queued and process it as one batch so
                        // a flurry of relay events + user actions produces a single UI
                        // update instead of N. Without this, tapping a chat while a
                        // relay backlog drains can take seconds because OpenChat sits
                        // behind every queued event and the UI recomposes between each.
                        while let Ok(first) = core_rx.recv() {
                            let mut batch = Vec::with_capacity(8);
                            batch.push(first);
                            while let Ok(next) = core_rx.try_recv() {
                                batch.push(next);
                            }
                            let batch_size = batch.len();
                            let t0 = crate::perflog::now_ms();
                            crate::perflog!("core.batch.start size={batch_size}");
                            match catch_core_batch(|| {
                                handle_core_batch_responsive(&mut core, batch)
                            }) {
                                Ok(true) => {}
                                Ok(false) => break,
                                Err(error) => {
                                    core.mark_core_panic(error);
                                    break;
                                }
                            }
                            crate::perflog!(
                                "core.batch.end size={batch_size} elapsed_ms={}",
                                crate::perflog::now_ms().saturating_sub(t0)
                            );
                        }
                    });
            if let Err(error) = spawn_result {
                let mut state = AppState::empty();
                state.toast = Some(format!("Iris could not start: {error}"));
                state.rev = 1;
                match shared_state.write() {
                    Ok(mut slot) => *slot = state.clone(),
                    Err(poison) => *poison.into_inner() = state.clone(),
                }
                let _ = update_tx_for_error.send(AppUpdate::FullState(state));
            }
        }
        Err(error) => {
            let mut state = AppState::empty();
            state.toast = Some(error.to_string());
            state.rev = 1;
            match shared_state.write() {
                Ok(mut slot) => *slot = state.clone(),
                Err(poison) => *poison.into_inner() = state.clone(),
            }
            let _ = update_tx_for_error.send(AppUpdate::FullState(state));
        }
    }

    Arc::new(FfiApp {
        core_tx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db,
    })
}

fn ffi_app_failure(message: String) -> Arc<FfiApp> {
    let (_update_tx, update_rx) = flume::unbounded();
    let (core_tx, _core_rx) = flume::unbounded();
    let mut state = AppState::empty();
    state.toast = Some(message);
    state.rev = 1;
    let shared_state = Arc::new(RwLock::new(state));
    Arc::new(FfiApp {
        core_tx,
        update_rx,
        listening: AtomicBool::new(false),
        shared_state,
        shared_db: None,
    })
}

fn handle_core_batch_responsive(core: &mut AppCore, messages: Vec<CoreMsg>) -> bool {
    if messages.len() <= 1 || !messages.iter().any(is_foreground_core_msg) {
        return core.handle_messages(messages);
    }

    let mut foreground = Vec::new();
    let mut background = Vec::new();
    for message in messages {
        if is_foreground_core_msg(&message) {
            foreground.push(message);
        } else {
            background.push(message);
        }
    }

    for message in foreground {
        if !core.handle_message(message) {
            return false;
        }
    }
    background.is_empty() || core.handle_messages(background)
}

fn catch_core_batch<F>(f: F) -> Result<bool, String>
where
    F: FnOnce() -> bool,
{
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(panic_payload_to_string)
}

fn ffi_or<T, F>(label: &'static str, fallback: T, f: F) -> T
where
    F: FnOnce() -> T,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            crate::perflog!(
                "ffi.panic label={label} detail={}",
                panic_payload_to_string(payload)
            );
            fallback
        }
    }
}

fn ffi_failure_state() -> AppState {
    let mut state = AppState::empty();
    state.toast = Some("Iris needs restart. Copy support bundle in Settings.".to_string());
    state
}

fn suppressed_mobile_push_resolution() -> MobilePushNotificationResolution {
    MobilePushNotificationResolution {
        should_show: false,
        title: String::new(),
        body: String::new(),
        payload_json: "{}".to_string(),
    }
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn is_foreground_core_msg(message: &CoreMsg) -> bool {
    !matches!(message, CoreMsg::Internal(_))
}

fn filter_threads_for_search(
    chat_list: &[ChatThreadSnapshot],
    query: &str,
) -> (Vec<ChatThreadSnapshot>, Vec<ChatThreadSnapshot>) {
    let needle = query.to_lowercase();
    let mut contacts = Vec::new();
    let mut groups = Vec::new();
    for chat in chat_list {
        if !thread_matches_query(chat, &needle) {
            continue;
        }
        match chat.kind {
            ChatKind::Direct => contacts.push(chat.clone()),
            ChatKind::Group => groups.push(chat.clone()),
        }
    }
    (contacts, groups)
}

fn thread_matches_query(chat: &ChatThreadSnapshot, needle_lower: &str) -> bool {
    let candidates: [&str; 3] = [
        &chat.display_name,
        chat.subtitle.as_deref().unwrap_or(""),
        &chat.chat_id,
    ];
    candidates
        .iter()
        .any(|field| field.to_lowercase().contains(needle_lower))
}

fn enrich_message_hits(
    hits: Vec<crate::core::PersistedMessageSearchHit>,
    chat_list: &[ChatThreadSnapshot],
) -> Vec<MessageSearchHit> {
    use std::collections::HashMap;
    let lookup: HashMap<&str, &ChatThreadSnapshot> = chat_list
        .iter()
        .map(|chat| (chat.chat_id.as_str(), chat))
        .collect();
    hits.into_iter()
        .map(|hit| {
            let parent = lookup.get(hit.chat_id.as_str());
            let display_name = parent
                .map(|chat| chat.display_name.clone())
                .unwrap_or_else(|| short_chat_label(&hit.chat_id));
            let picture_url = parent.and_then(|chat| chat.picture_url.clone());
            let kind = parent.map(|chat| chat.kind.clone()).unwrap_or(ChatKind::Direct);
            MessageSearchHit {
                chat_id: hit.chat_id,
                message_id: hit.message_id,
                chat_display_name: display_name,
                chat_picture_url: picture_url,
                chat_kind: kind,
                author_pubkey: hit.author,
                body: hit.body,
                is_outgoing: hit.is_outgoing,
                created_at_secs: hit.created_at_secs,
            }
        })
        .collect()
}

fn short_chat_label(chat_id: &str) -> String {
    let trimmed = chat_id.trim();
    if trimmed.len() > 12 {
        format!("{}…", &trimmed[..12])
    } else {
        trimmed.to_string()
    }
}

fn verify_nearby_presence_event_json(
    event_json: &str,
    peer_id: &str,
    my_nonce: &str,
    their_nonce: &str,
) -> String {
    let Ok(event) = serde_json::from_str::<nostr_sdk::prelude::Event>(event_json) else {
        return String::new();
    };
    if event.verify().is_err() || event.kind.as_u16() != crate::core::NEARBY_PRESENCE_KIND {
        return String::new();
    }
    let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
        return String::new();
    };
    let get = |key: &str| {
        content
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or("")
    };
    let transport = get("transport");
    if get("protocol") != "iris-nearby-v1"
        || !(transport == "ble" || transport == "nearby" || transport == "lan")
        || get("peer_id") != peer_id.trim()
        || get("my_nonce") != their_nonce.trim()
        || get("their_nonce") != my_nonce.trim()
    {
        return String::new();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_at = content
        .get("expires_at")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let created_at = event.created_at.as_secs();
    if expires_at < now
        || expires_at > now.saturating_add(300)
        || created_at.saturating_add(300) < now
        || created_at > now.saturating_add(300)
    {
        return String::new();
    }

    let profile_event_id = get("profile_event_id");
    let profile_event_id = if profile_event_id.len() == 64 {
        profile_event_id
    } else {
        ""
    };
    serde_json::json!({
        "owner_pubkey_hex": event.pubkey.to_hex(),
        "profile_event_id": profile_event_id,
    })
    .to_string()
}

impl Drop for FfiApp {
    fn drop(&mut self) {
        let _ = self.core_tx.send(CoreMsg::Shutdown(None));
    }
}

#[uniffi::export]
pub fn normalize_peer_input(input: String) -> String {
    ffi_or("normalize_peer_input", String::new(), || {
        crate::core::normalize_peer_input_for_display(&input)
    })
}

#[uniffi::export]
pub fn is_valid_peer_input(input: String) -> bool {
    ffi_or("is_valid_peer_input", false, || {
        crate::core::parse_peer_input(&input).is_ok()
    })
}

/// Single source of truth for "is this typed text an npub or an
/// invite URL?". Used by the New Chat paste field, the chat-list
/// search bar, and the deep-link handler so all three branches agree
/// on what counts as a chat-opening shortcut. Returns `None` for
/// regular search-style text.
#[uniffi::export]
pub fn classify_chat_input(input: String) -> Option<ChatInputShortcut> {
    ffi_or("classify_chat_input", None, || {
        chat_input_shortcut(&input)
    })
}

fn chat_input_shortcut(raw: &str) -> Option<ChatInputShortcut> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("://") && lower.contains('#') {
        return Some(ChatInputShortcut::Invite {
            invite_input: trimmed.to_string(),
            display: short_invite_display(trimmed),
        });
    }
    if crate::core::parse_peer_input(trimmed).is_ok() {
        use nostr::nips::nip19::ToBech32;
        let normalized = crate::core::normalize_peer_input_for_display(trimmed);
        if let Ok(pubkey) = nostr::PublicKey::parse(&normalized) {
            let npub = pubkey.to_bech32().unwrap_or_else(|_| normalized.clone());
            let display = short_npub_display(&npub);
            return Some(ChatInputShortcut::DirectPeer {
                peer_input: normalized,
                display,
                npub,
                pubkey_hex: pubkey.to_hex(),
            });
        }
    }
    None
}

fn short_npub_display(npub: &str) -> String {
    if npub.len() > 16 {
        format!("{}…{}", &npub[..10], &npub[npub.len() - 4..])
    } else {
        npub.to_string()
    }
}

fn short_invite_display(invite: &str) -> String {
    // Strip the scheme + host so the row reads "iris.to/invite/…" not
    // a 120-char URL. We don't try to parse the bech32 payload; the
    // visible host is enough context for the user.
    let after_scheme = invite.split_once("://").map(|(_, rest)| rest).unwrap_or(invite);
    if after_scheme.len() > 32 {
        format!("{}…", &after_scheme[..32])
    } else {
        after_scheme.to_string()
    }
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its
/// canonical lowercase-hex form. The empty string is returned when the
/// input can't be parsed as a public key — callers expecting hex
/// downstream can short-circuit on that.
#[uniffi::export]
pub fn peer_input_to_hex(input: String) -> String {
    ffi_or("peer_input_to_hex", String::new(), || {
        let normalized = crate::core::normalize_peer_input_for_display(&input);
        match nostr::PublicKey::parse(&normalized) {
            Ok(pubkey) => pubkey.to_hex(),
            Err(_) => String::new(),
        }
    })
}

/// Convert any pubkey-shaped input (hex, npub, nprofile, …) to its npub form.
/// Returns the original string when it can't be parsed as a public key.
#[uniffi::export]
pub fn peer_input_to_npub(input: String) -> String {
    ffi_or("peer_input_to_npub", String::new(), || {
        use nostr::nips::nip19::ToBech32;
        let normalized = crate::core::normalize_peer_input_for_display(&input);
        match nostr::PublicKey::parse(&normalized) {
            Ok(pubkey) => pubkey.to_bech32().unwrap_or(normalized),
            Err(_) => normalized,
        }
    })
}

#[uniffi::export]
pub fn build_summary() -> String {
    ffi_or("build_summary", String::new(), crate::core::build_summary)
}

#[uniffi::export]
pub fn relay_set_id() -> String {
    ffi_or("relay_set_id", String::new(), || {
        crate::core::relay_set_id().to_string()
    })
}

#[uniffi::export]
pub fn proxied_image_url(
    original_src: String,
    preferences: PreferencesSnapshot,
    width: Option<u32>,
    height: Option<u32>,
    square: bool,
) -> String {
    ffi_or("proxied_image_url", original_src.clone(), || {
        image_proxy::proxied_image_url(&original_src, &preferences, width, height, square)
    })
}

#[uniffi::export]
pub fn is_trusted_test_build() -> bool {
    ffi_or(
        "is_trusted_test_build",
        false,
        crate::core::trusted_test_build_flag,
    )
}

/// Marketing version baked in at build time from `IRIS_APP_VERSION_NAME`
/// (or `IRIS_APP_VERSION`), falling back to the crate semver. Use this
/// instead of `env!("CARGO_PKG_VERSION")` so UI/release artifacts agree
/// on a single version string.
#[uniffi::export]
pub fn app_version() -> String {
    crate::core::app_version_string().to_string()
}

#[uniffi::export]
pub fn resolve_mobile_push_notification_payload(
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    ffi_or(
        "resolve_mobile_push_notification_payload",
        suppressed_mobile_push_resolution(),
        || crate::core::resolve_mobile_push_notification(raw_payload_json),
    )
}

/// Decrypt a notification payload against the persisted double-ratchet
/// state under `data_dir`. Use from the FCM service (Android) or
/// Notification Service Extension (iOS) where there's no live `FfiApp`.
/// Falls back to the generic resolver when keys, payload, or storage
/// are unavailable so the user still gets *some* notification.
#[uniffi::export]
pub fn decrypt_mobile_push_notification_payload(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    ffi_or(
        "decrypt_mobile_push_notification_payload",
        suppressed_mobile_push_resolution(),
        || {
            crate::core::decrypt_mobile_push_notification(
                data_dir,
                owner_pubkey_hex,
                device_nsec,
                raw_payload_json,
            )
        },
    )
}

#[uniffi::export]
pub fn resolve_mobile_push_subscription_server_url(
    platform_key: String,
    is_release: bool,
    override_url: Option<String>,
) -> String {
    ffi_or(
        "resolve_mobile_push_subscription_server_url",
        String::new(),
        || crate::core::resolve_mobile_push_server_url(platform_key, is_release, override_url),
    )
}

#[uniffi::export]
pub fn mobile_push_subscription_id_key(platform_key: String) -> String {
    ffi_or("mobile_push_subscription_id_key", String::new(), || {
        crate::core::mobile_push_stored_subscription_id_key(platform_key)
    })
}

#[uniffi::export]
pub fn build_mobile_push_list_subscriptions_request(
    owner_nsec: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or("build_mobile_push_list_subscriptions_request", None, || {
        crate::core::build_mobile_push_list_subscriptions_request(
            owner_nsec,
            platform_key,
            is_release,
            server_url_override,
        )
    })
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_mobile_push_create_subscription_request(
    owner_nsec: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_create_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_create_subscription_request(
                owner_nsec,
                platform_key,
                push_token,
                apns_topic,
                message_author_pubkeys,
                invite_response_pubkeys,
                is_release,
                server_url_override,
            )
        },
    )
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_mobile_push_update_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_update_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_update_subscription_request(
                owner_nsec,
                subscription_id,
                platform_key,
                push_token,
                apns_topic,
                message_author_pubkeys,
                invite_response_pubkeys,
                is_release,
                server_url_override,
            )
        },
    )
}

#[uniffi::export]
pub fn build_mobile_push_delete_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    ffi_or(
        "build_mobile_push_delete_subscription_request",
        None,
        || {
            crate::core::build_mobile_push_delete_subscription_request(
                owner_nsec,
                subscription_id,
                platform_key,
                is_release,
                server_url_override,
            )
        },
    )
}

#[cfg(test)]
mod ffi_hardening_tests {
    use super::*;

    #[test]
    fn ffi_guard_returns_fallback_after_panic() {
        let value = ffi_or("test.panic", 42, || -> i32 {
            panic!("ffi boom");
        });

        assert_eq!(value, 42);
    }

    #[test]
    fn core_batch_guard_converts_panic_to_error() {
        let result = catch_core_batch(|| -> bool {
            panic!("batch boom");
        });

        assert_eq!(result, Err("batch boom".to_string()));
    }

    #[test]
    fn core_batch_guard_preserves_success_result() {
        assert_eq!(catch_core_batch(|| true), Ok(true));
        assert_eq!(catch_core_batch(|| false), Ok(false));
    }
}
