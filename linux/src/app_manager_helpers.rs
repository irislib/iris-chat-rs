use super::*;

pub(super) fn nearby_avatar_visible(
    state: &AppState,
    nearby: &DesktopNearbySnapshot,
    owner: &str,
) -> bool {
    !owner.is_empty()
        && state
            .account
            .as_ref()
            .is_some_and(|account| !account.public_key_hex.eq_ignore_ascii_case(owner))
        && state.preferences.nearby_enabled
        && nearby.visible
        && nearby.peers.iter().any(|peer| {
            peer.owner_pubkey_hex
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(owner))
        })
}

pub(super) fn active_chat_id(state: &AppState) -> Option<String> {
    let active = state
        .router
        .screen_stack
        .last()
        .unwrap_or(&state.router.default_screen);
    match active {
        Screen::Chat { chat_id } => Some(chat_id.trim().to_string()),
        _ => state
            .current_chat
            .as_ref()
            .map(|chat| chat.chat_id.trim().to_string()),
    }
}

pub(super) fn action_clears_pending_navigation(action: &AppAction) -> bool {
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

pub(super) fn local_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Iris".to_string())
}

pub(super) fn local_device_label() -> String {
    let os = linux_pretty_name().unwrap_or_else(|| "Linux".to_string());
    let name = local_device_name();
    if name.eq_ignore_ascii_case(&os) {
        name
    } else {
        format!("{name} - {os}")
    }
}

pub(super) fn non_empty_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn linux_pretty_name() -> Option<String> {
    let os_release = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in os_release.lines() {
        let Some(value) = line.strip_prefix("PRETTY_NAME=") else {
            continue;
        };
        let trimmed = value.trim().trim_matches('"').to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

pub(super) fn app_state_restart_required() -> AppState {
    let mut state = AppState::empty();
    state.toast = Some(RESTART_REQUIRED_TOAST.to_string());
    state
}

pub(super) fn current_device_revoked(state: &AppState) -> bool {
    state
        .account
        .as_ref()
        .is_some_and(|account| account.authorization_state == DeviceAuthorizationState::Revoked)
}

pub(super) fn empty_nearby_snapshot() -> DesktopNearbySnapshot {
    DesktopNearbySnapshot {
        visible: false,
        status: "Off".to_string(),
        peers: Vec::new(),
    }
}

pub(super) fn catch_ffi<T, F>(label: &str, fallback: T, body: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            eprintln!(
                "Iris Chat FFI call failed ({label}): {}",
                panic_payload_message(payload.as_ref())
            );
            fallback
        }
    }
}

pub(super) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "panic".to_string()
    }
}

pub(super) fn truncate_debug_detail(detail: &str) -> String {
    detail
        .chars()
        .take(MAX_CLIENT_DEBUG_LOG_DETAIL_CHARS)
        .collect()
}

pub(super) fn xdg_data_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".local/share")
}

pub(super) fn xdg_config_home() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".config")
}

pub(super) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn ensure_dir(path: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&path);
    path
}
