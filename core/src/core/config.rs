use super::*;

pub(super) const FALLBACK_DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://relay.snort.social",
    "wss://temp.iris.to",
];
pub(super) const APP_VERSION: &str = env!("IRIS_APP_VERSION");
pub(super) const BUILD_CHANNEL: &str = env!("IRIS_BUILD_CHANNEL");
pub(super) const BUILD_GIT_SHA: &str = env!("IRIS_BUILD_GIT_SHA");
pub(super) const BUILD_TIMESTAMP_UTC: &str = env!("IRIS_BUILD_TIMESTAMP_UTC");
pub(super) const COMPILED_DEFAULT_RELAYS_CSV: &str = env!("IRIS_DEFAULT_RELAYS");
pub(super) const RELAY_SET_ID: &str = env!("IRIS_RELAY_SET_ID");
pub(super) const TRUSTED_TEST_BUILD: &str = env!("IRIS_TRUSTED_TEST_BUILD");
pub(super) const MAX_SEEN_EVENT_IDS: usize = 2048;
pub(super) const CATCH_UP_LOOKBACK_SECS: u64 = 7 * 24 * 60 * 60;
pub(super) const NEW_MESSAGE_AUTHOR_BACKFILL_LOOKBACK_SECS: u64 = 10 * 60;
pub(super) const DEVICE_INVITE_DISCOVERY_LOOKBACK_SECS: u64 = 30 * 24 * 60 * 60;
pub(super) const DEVICE_INVITE_DISCOVERY_LIMIT: usize = 256;
pub(super) const DEVICE_INVITE_DISCOVERY_POLL_SECS: u64 = 5;
pub(super) const NDR_APP_KEYS_D_TAG: &str = "double-ratchet/app-keys";
pub(super) const NDR_INVITES_D_TAG_PREFIX: &str = "double-ratchet/invites/";
pub(super) const NDR_INVITES_L_TAG: &str = "double-ratchet/invites";
pub(super) const RELAY_CONNECT_TIMEOUT_SECS: u64 = 5;
pub(super) const RELAY_SYNC_TIMEOUT_SECS: u64 = 5;
pub(super) const RESUBSCRIBE_CATCH_UP_DELAY_SECS: u64 = 5;
pub(super) const GROUP_CHAT_PREFIX: &str = "group:";
pub(super) const CHAT_INVITE_ROOT_URL: &str = "https://chat.iris.to/";
pub(super) const DEBUG_SNAPSHOT_FILENAME: &str = "iris_chat_runtime_debug.json";
/// Minimum wall-clock millis between debug-snapshot file rebuilds.
/// The snapshot fans out into a SessionManager clone × N known
/// users (see `support::build_runtime_debug_snapshot`); without
/// this floor every relay event triggered a rebuild — the macOS CPU
/// loop and the sluggish-over-time UX regression both traced back
/// here. 5 s is well under the multi-second poll budget every
/// harness test uses, so this stays invisible to tests.
pub(super) const DEBUG_SNAPSHOT_MIN_INTERVAL_MS: u64 = 5_000;
pub(super) const MAX_DEBUG_LOG_ENTRIES: usize = 128;
pub(super) const PERSISTED_STATE_VERSION: u32 = 12;

/// Whether the core should keep `iris_chat_runtime_debug.json`
/// on disk. The file is only ever read by harness tests
/// (`core/tests/cli_interop`, iOS `InteropHarnessTests`, Android
/// `RealRelayHarnessTest`) — production builds never touch it
/// (the user-facing support bundle rebuilds the snapshot
/// in-memory at export time). Enabled in debug builds for the
/// instrumentation lanes that read the file; release builds can
/// opt in via `IRIS_RUNTIME_DEBUG_SNAPSHOT=1` for the rare
/// release-flavoured test lane.
pub(crate) fn debug_snapshot_file_writes_enabled() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    matches!(
        std::env::var("IRIS_RUNTIME_DEBUG_SNAPSHOT").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

pub(crate) fn configured_relays() -> Vec<String> {
    let compiled_defaults = compiled_default_relays();
    let raw_relays = match std::env::var("IRIS_DEMO_RELAYS") {
        Ok(value) => value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Err(_) => compiled_defaults,
    };
    normalize_nostr_relay_urls(&raw_relays)
}

pub(super) fn relay_urls_from_strings(relays: &[String]) -> Vec<RelayUrl> {
    relays
        .iter()
        .filter_map(|relay| RelayUrl::parse(relay).ok())
        .collect()
}

pub(super) fn normalize_nostr_relay_url(raw_url: &str) -> Result<String, String> {
    let candidate = raw_url.trim();
    if candidate.is_empty() {
        return Err("Relay URL is required.".to_string());
    }

    let mut url = url::Url::parse(candidate)
        .map_err(|_| "Relay URL must be an absolute ws:// or wss:// URL.".to_string())?;
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "ws" && scheme != "wss" {
        return Err("Relay URL must use ws:// or wss://.".to_string());
    }
    if url.host_str().is_none() {
        return Err("Relay URL must include a host.".to_string());
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.set_scheme(&scheme)
        .map_err(|_| "Relay URL must use ws:// or wss://.".to_string())?;
    url.set_host(Some(&host))
        .map_err(|_| "Relay URL must include a host.".to_string())?;

    let mut normalized = url.to_string();
    if normalized.ends_with('/')
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
    {
        normalized.pop();
    }
    Ok(normalized)
}

pub(super) fn normalize_nostr_relay_urls(relays: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for relay in relays {
        if let Ok(url) = normalize_nostr_relay_url(relay) {
            if seen.insert(url.clone()) {
                normalized.push(url);
            }
        }
    }
    normalized
}

pub(super) fn compiled_default_relays() -> Vec<String> {
    let compiled = COMPILED_DEFAULT_RELAYS_CSV
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if compiled.is_empty() {
        FALLBACK_DEFAULT_RELAYS
            .iter()
            .map(|relay| (*relay).to_string())
            .collect()
    } else {
        compiled
    }
}

pub(super) fn trusted_test_build() -> bool {
    matches!(TRUSTED_TEST_BUILD, "1" | "true" | "TRUE" | "True")
}

pub(crate) fn build_summary() -> String {
    format!("{APP_VERSION} ({BUILD_GIT_SHA})")
}

pub(crate) fn app_version_string() -> &'static str {
    APP_VERSION
}

pub(crate) fn relay_set_id() -> &'static str {
    RELAY_SET_ID
}

pub(crate) fn trusted_test_build_flag() -> bool {
    trusted_test_build()
}

pub(super) async fn ensure_session_relays_configured(client: &Client, relay_urls: &[RelayUrl]) {
    for relay in relay_urls {
        let _ = tokio::time::timeout(
            Duration::from_secs(RELAY_SYNC_TIMEOUT_SECS),
            client.add_relay(relay.clone()),
        )
        .await;
    }
}

pub(super) async fn sync_session_relays(
    client: &Client,
    previous_relay_urls: &[RelayUrl],
    next_relay_urls: &[RelayUrl],
) {
    for relay in previous_relay_urls {
        if !next_relay_urls.iter().any(|next| next == relay) {
            let _ = tokio::time::timeout(
                Duration::from_secs(RELAY_SYNC_TIMEOUT_SECS),
                client.remove_relay(relay),
            )
            .await;
        }
    }
    ensure_session_relays_configured(client, next_relay_urls).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_relays_include_ndr_cli_defaults() {
        assert_eq!(
            compiled_default_relays(),
            vec![
                "wss://relay.damus.io",
                "wss://nos.lol",
                "wss://relay.primal.net",
                "wss://relay.snort.social",
                "wss://temp.iris.to",
            ]
        );
    }

    #[test]
    fn empty_relay_list_stays_disabled() {
        assert!(normalize_nostr_relay_urls(&[]).is_empty());
        assert!(relay_urls_from_strings(&[]).is_empty());
    }

    #[test]
    fn relay_url_normalization_is_stable_for_comparisons() {
        assert_eq!(
            normalize_nostr_relay_urls(&[
                " WSS://Relay.Example/ ".to_string(),
                "wss://relay.example".to_string(),
                "wss://relay.example/path/".to_string(),
            ]),
            vec![
                "wss://relay.example".to_string(),
                "wss://relay.example/path/".to_string(),
            ]
        );
    }
}
