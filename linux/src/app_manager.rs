use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iris_chat_core::{
    AppAction, AppReconciler, AppState, AppUpdate, DesktopNearbyObserver, DesktopNearbySnapshot,
    FfiApp, FfiDesktopNearby, OutgoingAttachment, Router, Screen, SearchResultSnapshot,
};

use crate::secure_storage::{FileSecretStore, SecretStore, StoredAccountBundle};

const ACTIVE_CHAT_SEEN_IDLE_LIMIT: Duration = Duration::from_secs(5 * 60);
const NAVIGATION_OVERRIDE_TTL: Duration = Duration::from_secs(10);
const ROUTE_CHAT_SNAPSHOT_LIMIT: u32 = 80;

struct PendingNavigationOverride {
    stack: Vec<Screen>,
    expires_at: Instant,
}

/// Chat-list search box state. Lives on the UI thread; queries are
/// re-issued against the core whenever any field changes.
#[derive(Default, Clone)]
pub struct SearchUiState {
    pub query: String,
    /// When set, restricts the messages section to a single chat — the
    /// "search in this chat" pill from Signal Desktop. We also stash
    /// the resolved display name so the chip can render without
    /// another lookup.
    pub scope_chat_id: Option<String>,
    pub scope_display_name: Option<String>,
    /// Bumped by chat-screen header taps so the chat list grabs focus
    /// on the search entry after navigating back.
    pub focus_request: u64,
}

pub struct AppManager {
    ffi: Arc<FfiApp>,
    update_rx: async_channel::Receiver<AppUpdate>,
    update_tx_ui: async_channel::Sender<AppUpdate>,
    secret_store: Arc<dyn SecretStore>,
    data_dir: PathBuf,
    nearby: Arc<FfiDesktopNearby>,
    nearby_update_rx: async_channel::Receiver<DesktopNearbySnapshot>,
    nearby_snapshot: RefCell<DesktopNearbySnapshot>,
    nearby_first_open_path: PathBuf,
    local_state: RefCell<AppState>,
    last_rev_applied: Cell<u64>,
    pending_navigation_override: RefCell<Option<PendingNavigationOverride>>,
    staged_attachments: RefCell<HashMap<String, Vec<OutgoingAttachment>>>,
    last_focused_chat_id: RefCell<Option<String>>,
    search_ui: RefCell<SearchUiState>,
    window_active: Cell<bool>,
    last_user_activity: RefCell<Instant>,
}

struct Reconciler {
    tx: async_channel::Sender<AppUpdate>,
    secret_store: Arc<dyn SecretStore>,
}

struct NearbyObserver {
    tx: async_channel::Sender<DesktopNearbySnapshot>,
}

impl DesktopNearbyObserver for NearbyObserver {
    fn desktop_nearby_changed(&self, snapshot: DesktopNearbySnapshot) {
        let _ = self.tx.send_blocking(snapshot);
    }
}

impl AppReconciler for Reconciler {
    fn reconcile(&self, update: AppUpdate) {
        if let AppUpdate::PersistAccountBundle {
            owner_nsec,
            owner_pubkey_hex,
            device_nsec,
            ..
        } = &update
        {
            self.secret_store.save(&StoredAccountBundle {
                owner_nsec: owner_nsec.clone(),
                owner_pubkey_hex: owner_pubkey_hex.clone(),
                device_nsec: device_nsec.clone(),
            });
        }
        let _ = self.tx.send_blocking(update);
    }
}

impl AppManager {
    pub fn new() -> Self {
        let data_dir = ensure_dir(xdg_data_home().join("iris-chat"));
        let secrets_dir = ensure_dir(xdg_config_home().join("iris-chat"));
        let secret_store: Arc<dyn SecretStore> = Arc::new(FileSecretStore::new(&secrets_dir));

        let ffi = FfiApp::new(
            data_dir.to_string_lossy().to_string(),
            String::new(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        let (tx, rx) = async_channel::unbounded();
        let update_tx_ui = tx.clone();
        let (nearby_tx, nearby_rx) = async_channel::unbounded();
        ffi.listen_for_updates(Box::new(Reconciler {
            tx,
            secret_store: secret_store.clone(),
        }));
        let nearby = FfiDesktopNearby::new(
            ffi.clone(),
            Box::new(NearbyObserver {
                tx: nearby_tx.clone(),
            }),
        );
        let nearby_snapshot = nearby.snapshot();
        let initial_state = ffi.state();
        if crate::platform::startup::is_supported() {
            let _ = crate::platform::startup::set_enabled(
                initial_state.preferences.startup_at_login_enabled,
            );
        }

        if let Some(bundle) = secret_store.load() {
            ffi.dispatch(AppAction::RestoreAccountBundle {
                owner_nsec: bundle.owner_nsec,
                owner_pubkey_hex: bundle.owner_pubkey_hex,
                device_nsec: bundle.device_nsec,
            });
        }

        Self {
            ffi,
            update_rx: rx,
            update_tx_ui,
            secret_store,
            data_dir,
            nearby,
            nearby_update_rx: nearby_rx,
            nearby_snapshot: RefCell::new(nearby_snapshot),
            nearby_first_open_path: secrets_dir.join("nearby-first-open"),
            last_rev_applied: Cell::new(initial_state.rev),
            local_state: RefCell::new(initial_state),
            pending_navigation_override: RefCell::new(None),
            staged_attachments: RefCell::new(HashMap::new()),
            last_focused_chat_id: RefCell::new(None),
            search_ui: RefCell::new(SearchUiState::default()),
            window_active: Cell::new(false),
            last_user_activity: RefCell::new(Instant::now()),
        }
    }

    pub fn search_ui(&self) -> SearchUiState {
        self.search_ui.borrow().clone()
    }

    pub fn set_search_query(&self, query: String) {
        self.search_ui.borrow_mut().query = query;
    }

    pub fn enter_chat_scope(&self, chat_id: String, display_name: String) {
        let mut slot = self.search_ui.borrow_mut();
        slot.scope_chat_id = Some(chat_id);
        slot.scope_display_name = Some(display_name);
        slot.focus_request = slot.focus_request.wrapping_add(1);
        if slot.query.is_empty() {
            slot.query.clear();
        }
    }

    pub fn clear_chat_scope(&self) {
        let mut slot = self.search_ui.borrow_mut();
        slot.scope_chat_id = None;
        slot.scope_display_name = None;
    }

    pub fn clear_search(&self) {
        *self.search_ui.borrow_mut() = SearchUiState::default();
    }

    pub fn run_search(&self, limit: u32) -> SearchResultSnapshot {
        let snapshot = self.search_ui.borrow().clone();
        self.ffi
            .search(snapshot.query, snapshot.scope_chat_id, limit)
    }

    /// Re-emit the current `AppState` on the UI update channel so the
    /// window can rebuild the chat-list section after a search-bar
    /// edit. The channel already drives every other re-render, so we
    /// piggy-back on it instead of growing a parallel redraw path.
    pub fn redraw_ui(&self) {
        let state = self.current_state();
        let _ = self.update_tx_ui.send_blocking(AppUpdate::FullState(state));
    }

    pub fn should_focus_composer(&self, chat_id: &str) -> bool {
        let mut slot = self.last_focused_chat_id.borrow_mut();
        if slot.as_deref() == Some(chat_id) {
            return false;
        }
        *slot = Some(chat_id.to_string());
        true
    }

    pub fn staged_attachments(&self, chat_id: &str) -> Vec<OutgoingAttachment> {
        self.staged_attachments
            .borrow()
            .get(chat_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn stage_attachment(&self, chat_id: &str, attachment: OutgoingAttachment) {
        let mut staged = self.staged_attachments.borrow_mut();
        let entry = staged.entry(chat_id.to_string()).or_default();
        if !entry.iter().any(|a| a.file_path == attachment.file_path) {
            entry.push(attachment);
        }
    }

    pub fn unstage_attachment(&self, chat_id: &str, file_path: &str) {
        if let Some(entry) = self.staged_attachments.borrow_mut().get_mut(chat_id) {
            entry.retain(|a| a.file_path != file_path);
        }
    }

    pub fn take_staged_attachments(&self, chat_id: &str) -> Vec<OutgoingAttachment> {
        self.staged_attachments
            .borrow_mut()
            .remove(chat_id)
            .unwrap_or_default()
    }

    pub fn current_state(&self) -> AppState {
        self.local_state.borrow().clone()
    }

    pub fn update_rx(&self) -> async_channel::Receiver<AppUpdate> {
        self.update_rx.clone()
    }

    pub fn nearby_update_rx(&self) -> async_channel::Receiver<DesktopNearbySnapshot> {
        self.nearby_update_rx.clone()
    }

    pub fn nearby_snapshot(&self) -> DesktopNearbySnapshot {
        self.nearby_snapshot.borrow().clone()
    }

    pub fn apply_nearby_snapshot(&self, snapshot: DesktopNearbySnapshot) {
        *self.nearby_snapshot.borrow_mut() = snapshot;
    }

    pub fn dispatch(&self, action: AppAction) {
        if self.handle_optimistic_navigation(action.clone()) {
            return;
        }
        self.dispatch_to_rust(action, false);
    }

    pub fn apply_update(&self, update: AppUpdate) -> Option<AppUpdate> {
        match update {
            AppUpdate::FullState(state) => {
                if state.rev <= self.last_rev_applied.get() {
                    let local = self.local_state.borrow();
                    if state.rev == local.rev && state.router == local.router {
                        return Some(AppUpdate::FullState(local.clone()));
                    }
                    return None;
                }
                let rev = state.rev;
                let reconciled = self.state_by_reconciling_pending_navigation(state);
                self.last_rev_applied.set(rev);
                *self.local_state.borrow_mut() = reconciled.clone();
                Some(AppUpdate::FullState(reconciled))
            }
            other => Some(other),
        }
    }

    fn handle_optimistic_navigation(&self, action: AppAction) -> bool {
        match action {
            AppAction::NavigateBack => {
                let stack = self.local_state.borrow().router.screen_stack.clone();
                if stack.is_empty() {
                    return true;
                }
                let next_stack = stack[..stack.len() - 1].to_vec();
                self.navigate_optimistically(
                    next_stack.clone(),
                    AppAction::UpdateScreenStack { stack: next_stack },
                );
                true
            }
            AppAction::OpenChat { chat_id } => {
                let trimmed = chat_id.trim().to_string();
                if !trimmed.is_empty() {
                    self.navigate_optimistically(
                        vec![Screen::Chat {
                            chat_id: trimmed.clone(),
                        }],
                        AppAction::OpenChat { chat_id: trimmed },
                    );
                }
                true
            }
            AppAction::PushScreen { screen } => {
                if let Some(stack) = self.stack_by_applying_push_screen(&screen) {
                    self.navigate_optimistically(stack, AppAction::PushScreen { screen });
                } else {
                    self.dispatch_to_rust(AppAction::PushScreen { screen }, false);
                }
                true
            }
            AppAction::UpdateScreenStack { stack } => {
                self.navigate_optimistically(
                    stack.clone(),
                    AppAction::UpdateScreenStack { stack },
                );
                true
            }
            _ => false,
        }
    }

    fn stack_by_applying_push_screen(&self, screen: &Screen) -> Option<Vec<Screen>> {
        let state = self.local_state.borrow();
        if state.account.is_none() {
            return match screen {
                Screen::Welcome => Some(Vec::new()),
                Screen::CreateAccount | Screen::RestoreAccount | Screen::AddDevice => {
                    Some(vec![screen.clone()])
                }
                _ => None,
            };
        }

        match screen {
            Screen::ChatList => Some(Vec::new()),
            Screen::NewChat
            | Screen::NewGroup
            | Screen::CreateInvite
            | Screen::JoinInvite
            | Screen::Settings
            | Screen::DeviceRoster => Some(vec![screen.clone()]),
            Screen::Chat { chat_id } => {
                let trimmed = chat_id.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(vec![Screen::Chat {
                        chat_id: trimmed.to_string(),
                    }])
                }
            }
            Screen::GroupDetails { group_id } => {
                let trimmed = group_id.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let group_chat_id = format!("group:{trimmed}");
                let mut stack = if active_chat_id(&state).as_deref() == Some(group_chat_id.as_str())
                {
                    state.router.screen_stack.clone()
                } else {
                    vec![Screen::Chat {
                        chat_id: group_chat_id,
                    }]
                };
                let details = Screen::GroupDetails {
                    group_id: trimmed.to_string(),
                };
                if stack.last() != Some(&details) {
                    stack.push(details);
                }
                Some(stack)
            }
            Screen::CreateAccount
            | Screen::RestoreAccount
            | Screen::AddDevice
            | Screen::AwaitingDeviceApproval
            | Screen::DeviceRevoked
            | Screen::Welcome => None,
        }
    }

    fn navigate_optimistically(&self, stack: Vec<Screen>, action: AppAction) {
        *self.pending_navigation_override.borrow_mut() = Some(PendingNavigationOverride {
            stack: stack.clone(),
            expires_at: Instant::now() + NAVIGATION_OVERRIDE_TTL,
        });
        self.apply_local_screen_stack(stack);
        self.dispatch_to_rust(action, true);
    }

    fn state_by_reconciling_pending_navigation(&self, next_state: AppState) -> AppState {
        let mut pending_slot = self.pending_navigation_override.borrow_mut();
        let Some(pending) = pending_slot.as_ref() else {
            return next_state;
        };
        if next_state.account.is_none() {
            *pending_slot = None;
            return next_state;
        }
        if next_state.router.screen_stack == pending.stack {
            *pending_slot = None;
            return next_state;
        }
        if Instant::now() >= pending.expires_at {
            *pending_slot = None;
            return next_state;
        }
        self.state_by_applying_local_screen_stack(pending.stack.clone(), next_state)
    }

    fn apply_local_screen_stack(&self, stack: Vec<Screen>) {
        let next_state =
            self.state_by_applying_local_screen_stack(stack, self.local_state.borrow().clone());
        *self.local_state.borrow_mut() = next_state.clone();
        let _ = self.update_tx_ui.send_blocking(AppUpdate::FullState(next_state));
    }

    fn state_by_applying_local_screen_stack(
        &self,
        stack: Vec<Screen>,
        mut base_state: AppState,
    ) -> AppState {
        let active = stack
            .last()
            .cloned()
            .unwrap_or_else(|| base_state.router.default_screen.clone());
        base_state.router = Router {
            default_screen: base_state.router.default_screen.clone(),
            screen_stack: stack,
        };
        match active {
            Screen::Chat { chat_id } => {
                if base_state
                    .current_chat
                    .as_ref()
                    .map_or(true, |chat| chat.chat_id != chat_id)
                {
                    base_state.current_chat =
                        self.ffi.chat_snapshot(chat_id, ROUTE_CHAT_SNAPSHOT_LIMIT);
                }
                base_state.group_details = None;
            }
            Screen::GroupDetails { group_id } => {
                if base_state
                    .group_details
                    .as_ref()
                    .map_or(true, |details| details.group_id != group_id)
                {
                    base_state.group_details = None;
                }
            }
            _ => {
                base_state.current_chat = None;
                base_state.group_details = None;
            }
        }
        base_state
    }

    fn dispatch_to_rust(&self, action: AppAction, preserves_pending_navigation: bool) {
        if !preserves_pending_navigation && action_clears_pending_navigation(&action) {
            *self.pending_navigation_override.borrow_mut() = None;
        }
        self.ffi.dispatch(action);
    }

    pub fn set_window_active(&self, active: bool) {
        self.window_active.set(active);
        if active {
            self.record_user_activity();
        }
    }

    pub fn record_user_activity(&self) {
        *self.last_user_activity.borrow_mut() = Instant::now();
    }

    pub fn can_mark_active_chat_seen(&self) -> bool {
        self.window_active.get()
            && self.last_user_activity.borrow().elapsed() <= ACTIVE_CHAT_SEEN_IDLE_LIMIT
    }

    pub fn prepare_nearby_for_user_tap(&self) {
        let first_open = !self.nearby_first_open_path.exists();
        if first_open {
            let _ = std::fs::write(&self.nearby_first_open_path, b"1");
        }
        let prefs = self.current_state().preferences;
        if prefs.nearby_lan_enabled || first_open {
            self.set_nearby_lan_enabled(true);
        }
    }

    pub fn set_nearby_lan_enabled(&self, enabled: bool) {
        if enabled {
            self.nearby.start(local_device_name());
        } else {
            self.nearby.stop();
        }
        self.dispatch_to_rust(AppAction::SetNearbyLanEnabled { enabled }, false);
    }

    pub fn sync_nearby_preference(&self, state: &AppState) {
        if state.preferences.nearby_lan_enabled {
            self.nearby.start(local_device_name());
        } else {
            self.nearby.stop();
        }
    }

    pub fn publish_nearby_event(
        &self,
        event_id: String,
        kind: u32,
        created_at_secs: u64,
        event_json: String,
    ) {
        self.nearby
            .publish(event_id, kind, created_at_secs, event_json);
    }

    pub fn export_support_bundle_json(&self) -> String {
        self.ffi.export_support_bundle_json()
    }

    #[allow(dead_code)]
    pub fn logout(&self) {
        self.dispatch_to_rust(AppAction::Logout, false);
        self.secret_store.clear();
        let _ = std::fs::remove_dir_all(&self.data_dir);
        let _ = std::fs::create_dir_all(&self.data_dir);
    }
}

fn active_chat_id(state: &AppState) -> Option<String> {
    let active = state
        .router
        .screen_stack
        .last()
        .unwrap_or(&state.router.default_screen);
    match active {
        Screen::Chat { chat_id } => Some(chat_id.trim().to_string()),
        _ => state.current_chat.as_ref().map(|chat| chat.chat_id.trim().to_string()),
    }
}

fn action_clears_pending_navigation(action: &AppAction) -> bool {
    matches!(
        action,
        AppAction::OpenChat { .. }
            | AppAction::PushScreen { .. }
            | AppAction::UpdateScreenStack { .. }
            | AppAction::NavigateBack
            | AppAction::CreateChat { .. }
            | AppAction::CreateGroup { .. }
            | AppAction::CreateGroupWithPicture { .. }
            | AppAction::AcceptInvite { .. }
            | AppAction::Logout
            | AppAction::RestoreSession { .. }
            | AppAction::RestoreAccountBundle { .. }
    )
}

fn local_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Iris".to_string())
}

fn xdg_data_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".local/share")
}

fn xdg_config_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".config")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn ensure_dir(path: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&path);
    path
}
