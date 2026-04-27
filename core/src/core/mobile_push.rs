use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use nostr::Tag;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MOBILE_PUSH_REACTION_KIND: u64 = 7;
const MOBILE_PUSH_RECEIPT_KIND: u64 = 15;
const MOBILE_PUSH_TYPING_KIND: u64 = 25;
const MOBILE_PUSH_GROUP_METADATA_KIND: u64 = 40;
const MOBILE_PUSH_SETTINGS_KIND: u64 = 30_078;
const MOBILE_PUSH_AUTH_KIND: u16 = 27_235;
const MOBILE_PUSH_DM_EVENT_KIND: u64 = MESSAGE_EVENT_KIND as u64;
const MOBILE_PUSH_PRODUCTION_SERVER_URL: &str = "https://notifications.iris.to";
const MOBILE_PUSH_SANDBOX_SERVER_URL: &str = "https://notifications-sandbox.iris.to";

impl AppCore {
    pub(super) fn build_mobile_push_sync_snapshot(&self) -> MobilePushSyncSnapshot {
        let Some(logged_in) = self.logged_in.as_ref() else {
            return MobilePushSyncSnapshot::default();
        };

        // Only `owner_pubkey_hex` and `message_author_pubkeys` are
        // consumed (by AndroidMobilePushRuntime / MobilePushRuntime to
        // build the subscription request body). The historical
        // `sessions: Vec<MobilePushSessionSnapshot>` field walked every
        // ratchet state and ran `serde_json::to_string` on each — ~440 ms
        // per call on Android debug, dominating the per-emit
        // `rebuild_state` cost — and nothing on the shell side ever
        // read it. Leave the vec empty; the schema stays stable.
        let mut message_author_pubkeys = HashSet::new();
        message_author_pubkeys.extend(self.known_message_author_hexes());
        let message_author_pubkeys = sorted_hexes(message_author_pubkeys);

        MobilePushSyncSnapshot {
            owner_pubkey_hex: Some(logged_in.owner_pubkey.to_string()),
            message_author_pubkeys,
            sessions: Vec::new(),
        }
    }
}

/// Decrypt the encrypted Nostr event the notification server forwarded
/// (key `event`), look up the sender's display name from
/// `profiles.json` or the direct chat thread, look up the group title
/// (when the rumor carries a `["l", group_id]` tag) from `groups.json`,
/// and return a notification resolution whose `title` and `body` are
/// the decrypted plaintext — not the generic "New activity" placeholder
/// the server sent.
///
/// Designed to run in the FCM service / iOS Notification Service
/// Extension where there's no live `AppCore`. We spin up a one-shot
/// `NdrRuntime` against a read-through preview of the same
/// `FileStorageAdapter` directory the main app uses, decrypt against
/// the persisted ratchet state, then drop the runtime. Writes stay in
/// the preview overlay so a notification preview cannot advance or
/// otherwise mutate the chat runtime's persisted ratchet state before
/// the foreground app processes the same relay event. If anything
/// fails (no keys, foreign event, storage unavailable) we fall through
/// to the vanilla `resolve_mobile_push_notification` so the user still
/// gets *some* notification — just the generic kind.
///
/// `data_dir` is the `app_data_dir` the FFI was constructed with;
/// `owner_pubkey_hex` and `device_nsec` come from the same secure
/// store the main app reads on launch.
pub(crate) fn decrypt_mobile_push_notification(
    data_dir: String,
    owner_pubkey_hex: String,
    device_nsec: String,
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    let fallback = || resolve_mobile_push_notification(raw_payload_json.clone());

    let payload_value: serde_json::Value = match serde_json::from_str(&raw_payload_json) {
        Ok(value) => value,
        Err(_) => return fallback(),
    };
    let payload_object = match payload_value.as_object() {
        Some(object) => object,
        None => return fallback(),
    };

    let outer_event_json = payload_object
        .get("event")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            payload_object
                .get("inner_event_json")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        });
    let Some(outer_event_json) = outer_event_json else {
        return fallback();
    };

    let outer_event: nostr::Event = match serde_json::from_str(&outer_event_json) {
        Ok(event) => event,
        Err(_) => return fallback(),
    };

    let owner_pubkey = match nostr::PublicKey::parse(owner_pubkey_hex.trim()) {
        Ok(pubkey) => pubkey,
        Err(_) => return fallback(),
    };
    let device_keys = match nostr::Keys::parse(device_nsec.trim()) {
        Ok(keys) => keys,
        Err(_) => return fallback(),
    };

    let storage_dir = PathBuf::from(&data_dir)
        .join("ndr_runtime")
        .join(owner_pubkey.to_hex())
        .join(device_keys.public_key().to_hex());
    let base_storage = match FileStorageAdapter::new(storage_dir) {
        Ok(adapter) => Arc::new(adapter) as Arc<dyn StorageAdapter>,
        Err(_) => return fallback(),
    };
    let storage =
        Arc::new(NotificationPreviewStorage::new(base_storage)) as Arc<dyn StorageAdapter>;

    let runtime = NdrRuntime::new(
        device_keys.public_key(),
        device_keys.secret_key().to_secret_bytes(),
        device_keys.public_key().to_hex(),
        owner_pubkey,
        Some(storage),
        None,
    );
    if runtime.init().is_err() {
        return fallback();
    }
    runtime.process_received_event(outer_event);

    let mut decrypted_inner_json: Option<String> = None;
    let mut decrypted_sender: Option<nostr::PublicKey> = None;
    for event in runtime.drain_events() {
        if let SessionManagerEvent::DecryptedMessage {
            sender, content, ..
        } = event
        {
            decrypted_inner_json = Some(content);
            decrypted_sender = Some(sender);
            break;
        }
    }

    let (Some(inner_json), Some(sender_owner)) = (decrypted_inner_json, decrypted_sender) else {
        return fallback();
    };
    let inner_value: serde_json::Value = match serde_json::from_str(&inner_json) {
        Ok(value) => value,
        Err(_) => return fallback(),
    };
    let inner_kind = inner_value
        .get("kind")
        .and_then(|value| value.as_u64())
        .unwrap_or(MOBILE_PUSH_DM_EVENT_KIND);
    if should_suppress_mobile_push_kind(inner_kind) {
        return MobilePushNotificationResolution {
            should_show: false,
            title: String::new(),
            body: String::new(),
            payload_json: "{}".to_string(),
        };
    }

    let inner_content = inner_value
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let inner_tags: Vec<Vec<String>> = inner_value
        .get("tags")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();

    let group_id = inner_tags.iter().find_map(|tag| match tag.as_slice() {
        [name, value, ..] if name == "l" && !value.is_empty() => Some(value.clone()),
        _ => None,
    });
    let sender_name = lookup_sender_display_name(&data_dir, &sender_owner)
        .or_else(|| lookup_direct_thread_sender_name(&data_dir, &sender_owner));
    let group_title = group_id
        .as_ref()
        .and_then(|id| lookup_group_name(&data_dir, id));

    let body = if inner_kind == MOBILE_PUSH_REACTION_KIND {
        let emoji = inner_content.trim();
        if emoji.is_empty() {
            "Reacted".to_string()
        } else if emoji.to_lowercase().starts_with("reacted") {
            emoji.to_string()
        } else {
            format!("Reacted {emoji}")
        }
    } else if !inner_content.trim().is_empty() {
        inner_content.trim().to_string()
    } else {
        "New message".to_string()
    };

    // Title shape:
    //   1-1 chat:    "<sender_name>"
    //   group chat:  "<sender_name> in <group_title>"   (or just the group
    //                title when we can't resolve a sender name)
    let resolved_sender_name = sender_name.unwrap_or_else(|| {
        payload_object
            .get("sender_name")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| "Iris Chat".to_string())
    });
    let title = match (&group_title, resolved_sender_name.as_str()) {
        (Some(group), sender) if !sender.is_empty() && sender != "Iris Chat" => {
            format!("{sender} in {group}")
        }
        (Some(group), _) => group.clone(),
        (None, sender) => sender.to_string(),
    };

    let mut resolved_payload = serde_json::Map::new();
    for (key, value) in payload_object {
        resolved_payload.insert(key.clone(), value.clone());
    }
    resolved_payload.insert(
        "title".to_string(),
        serde_json::Value::String(title.clone()),
    );
    resolved_payload.insert("body".to_string(), serde_json::Value::String(body.clone()));
    resolved_payload.insert(
        "inner_event_json".to_string(),
        serde_json::Value::String(inner_json),
    );
    resolved_payload.insert(
        "inner_kind".to_string(),
        serde_json::Value::String(inner_kind.to_string()),
    );
    resolved_payload.insert(
        "sender_pubkey".to_string(),
        serde_json::Value::String(sender_owner.to_hex()),
    );
    if let Some(group_id) = group_id {
        resolved_payload.insert("group_id".to_string(), serde_json::Value::String(group_id));
    }

    MobilePushNotificationResolution {
        should_show: true,
        title,
        body,
        payload_json: serde_json::to_string(&serde_json::Value::Object(resolved_payload))
            .unwrap_or_else(|_| "{}".to_string()),
    }
}

struct NotificationPreviewStorage {
    base: Arc<dyn StorageAdapter>,
    overlay: Mutex<BTreeMap<String, String>>,
    deleted: Mutex<HashSet<String>>,
}

impl NotificationPreviewStorage {
    fn new(base: Arc<dyn StorageAdapter>) -> Self {
        Self {
            base,
            overlay: Mutex::new(BTreeMap::new()),
            deleted: Mutex::new(HashSet::new()),
        }
    }
}

impl StorageAdapter for NotificationPreviewStorage {
    fn get(&self, key: &str) -> nostr_double_ratchet::Result<Option<String>> {
        if let Some(value) = self.overlay.lock().unwrap().get(key).cloned() {
            return Ok(Some(value));
        }
        if self.deleted.lock().unwrap().contains(key) {
            return Ok(None);
        }
        self.base.get(key)
    }

    fn put(&self, key: &str, value: String) -> nostr_double_ratchet::Result<()> {
        self.overlay.lock().unwrap().insert(key.to_string(), value);
        self.deleted.lock().unwrap().remove(key);
        Ok(())
    }

    fn del(&self, key: &str) -> nostr_double_ratchet::Result<()> {
        self.overlay.lock().unwrap().remove(key);
        self.deleted.lock().unwrap().insert(key.to_string());
        Ok(())
    }

    fn list(&self, prefix: &str) -> nostr_double_ratchet::Result<Vec<String>> {
        let mut keys: HashSet<String> = self.base.list(prefix)?.into_iter().collect();
        let deleted = self.deleted.lock().unwrap();
        keys.retain(|key| !deleted.contains(key));
        for key in self.overlay.lock().unwrap().keys() {
            if key.starts_with(prefix) && !deleted.contains(key) {
                keys.insert(key.clone());
            }
        }
        let mut keys: Vec<String> = keys.into_iter().collect();
        keys.sort();
        Ok(keys)
    }
}

fn lookup_sender_display_name(data_dir: &str, sender: &nostr::PublicKey) -> Option<String> {
    let profiles_path = PathBuf::from(data_dir).join("core").join("profiles.json");
    let bytes = std::fs::read(&profiles_path).ok()?;
    let profiles: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let map = profiles.as_object()?;
    let entry = map.get(&sender.to_hex())?.as_object()?;
    let name = entry
        .get("display_name")
        .and_then(|value| value.as_str())
        .or_else(|| entry.get("name").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    name
}

fn lookup_direct_thread_sender_name(data_dir: &str, sender: &nostr::PublicKey) -> Option<String> {
    let thread_path = PathBuf::from(data_dir)
        .join("core")
        .join("threads")
        .join(format!("{}.json", sender.to_hex()));
    let bytes = std::fs::read(thread_path).ok()?;
    let thread: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let messages = thread.get("messages")?.as_array()?;
    messages.iter().rev().find_map(|message| {
        let object = message.as_object()?;
        let is_outgoing = object
            .get("is_outgoing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if is_outgoing {
            return None;
        }
        object
            .get("author")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| !is_generic_sender_title(value))
            .map(str::to_string)
    })
}

fn lookup_group_name(data_dir: &str, group_id: &str) -> Option<String> {
    let groups_path = PathBuf::from(data_dir).join("core").join("groups.json");
    let bytes = std::fs::read(&groups_path).ok()?;
    let groups: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let array = groups.as_array()?;
    for entry in array {
        let object = entry.as_object()?;
        let id = object.get("id").and_then(|value| value.as_str())?;
        if id == group_id {
            return object
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
    }
    None
}

pub(crate) fn resolve_mobile_push_notification(
    raw_payload_json: String,
) -> MobilePushNotificationResolution {
    let payload = normalized_payload(&raw_payload_json);
    let title = resolved_title(&payload);
    let body = normalized_value(payload.get("body")).unwrap_or_else(|| "New activity".to_string());
    let inner_kind = payload
        .get("inner_kind")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or_else(|| event_kind(payload.get("inner_event_json")))
        .or_else(|| event_kind(payload.get("inner_event")))
        .or_else(|| event_kind(payload.get("event")));

    if inner_kind.is_some_and(should_suppress_mobile_push_kind) {
        return MobilePushNotificationResolution {
            should_show: false,
            title: String::new(),
            body: String::new(),
            payload_json: "{}".to_string(),
        };
    }

    let body = if inner_kind == Some(MOBILE_PUSH_REACTION_KIND) {
        let emoji = normalized_value(payload.get("body"))
            .or_else(|| event_content(payload.get("inner_event_json")))
            .or_else(|| event_content(payload.get("inner_event")))
            .unwrap_or_default();
        if emoji.is_empty() {
            "Reacted".to_string()
        } else if emoji.to_lowercase().starts_with("reacted") {
            emoji
        } else {
            format!("Reacted {emoji}")
        }
    } else {
        body
    };

    let mut resolved_payload = payload;
    resolved_payload.insert("title".to_string(), title.clone());
    resolved_payload.insert("body".to_string(), body.clone());
    if let Some(kind) = inner_kind {
        resolved_payload.insert("inner_kind".to_string(), kind.to_string());
    }

    MobilePushNotificationResolution {
        should_show: true,
        title,
        body,
        payload_json: serde_json::to_string(&resolved_payload).unwrap_or_else(|_| "{}".to_string()),
    }
}

pub(crate) fn resolve_mobile_push_server_url(
    platform_key: String,
    is_release: bool,
    override_url: Option<String>,
) -> String {
    let trimmed_override = override_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(value) = trimmed_override {
        return value.to_string();
    }
    let platform = platform_key.trim().to_ascii_lowercase();
    if !is_release && matches!(platform.as_str(), "ios" | "android") {
        return MOBILE_PUSH_SANDBOX_SERVER_URL.to_string();
    }
    MOBILE_PUSH_PRODUCTION_SERVER_URL.to_string()
}

pub(crate) fn mobile_push_stored_subscription_id_key(platform_key: String) -> String {
    format!(
        "settings.mobile_push_subscription_id.{}",
        normalize_platform_key(&platform_key)
    )
}

pub(crate) fn build_mobile_push_list_subscriptions_request(
    owner_nsec: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    build_mobile_push_subscription_request(
        owner_nsec,
        "GET",
        "/subscriptions",
        None,
        platform_key,
        is_release,
        server_url_override,
    )
}

pub(crate) fn build_mobile_push_create_subscription_request(
    owner_nsec: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
    )?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "POST",
        "/subscriptions",
        Some(body_json),
        platform_key,
        is_release,
        server_url_override,
    )
}

pub(crate) fn build_mobile_push_update_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    push_token: String,
    apns_topic: Option<String>,
    message_author_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let subscription_id = normalize_path_component(&subscription_id)?;
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
    )?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "POST",
        &format!("/subscriptions/{subscription_id}"),
        Some(body_json),
        platform_key,
        is_release,
        server_url_override,
    )
}

pub(crate) fn build_mobile_push_delete_subscription_request(
    owner_nsec: String,
    subscription_id: String,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let subscription_id = normalize_path_component(&subscription_id)?;
    build_mobile_push_subscription_request(
        owner_nsec,
        "DELETE",
        &format!("/subscriptions/{subscription_id}"),
        None,
        platform_key,
        is_release,
        server_url_override,
    )
}

fn build_mobile_push_subscription_request(
    owner_nsec: String,
    method: &str,
    path: &str,
    body_json: Option<String>,
    platform_key: String,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let method = method.trim().to_ascii_uppercase();
    let base_url = resolve_mobile_push_server_url(platform_key, is_release, server_url_override);
    let url = resolve_mobile_push_url(&base_url, path)?;
    let authorization_header = build_mobile_push_auth_header(&owner_nsec, &method, &url)?;
    Some(MobilePushSubscriptionRequest {
        method,
        url,
        authorization_header,
        body_json,
    })
}

fn build_mobile_push_auth_header(owner_nsec: &str, method: &str, url: &str) -> Option<String> {
    let keys = Keys::parse(owner_nsec.trim()).ok()?;
    let event = EventBuilder::new(Kind::from(MOBILE_PUSH_AUTH_KIND), "")
        .tag(Tag::parse(["u", url]).ok()?)
        .tag(Tag::parse(["method", method]).ok()?)
        .sign_with_keys(&keys)
        .ok()?;
    let encoded = BASE64_STANDARD.encode(serde_json::to_vec(&event).ok()?);
    Some(format!("Nostr {encoded}"))
}

fn mobile_push_subscription_body_json(
    platform_key: &str,
    push_token: &str,
    apns_topic: Option<&str>,
    message_author_pubkeys: Vec<String>,
) -> Option<String> {
    let platform = normalize_platform_key(platform_key);
    let token = push_token.trim();
    if token.is_empty() {
        return None;
    }
    let authors = normalize_hex_list(message_author_pubkeys);
    if authors.is_empty() {
        return None;
    }

    let mut payload = serde_json::json!({
        "webhooks": [],
        "web_push_subscriptions": [],
        "fcm_tokens": if platform == "android" { vec![token.to_string()] } else { Vec::<String>::new() },
        "apns_tokens": if platform == "ios" { vec![token.to_string()] } else { Vec::<String>::new() },
        "filter": {
            "kinds": [MOBILE_PUSH_DM_EVENT_KIND],
            "authors": authors,
        },
    });
    if platform == "ios" {
        if let Some(topic) = apns_topic.map(str::trim).filter(|value| !value.is_empty()) {
            payload["apns_topic"] = serde_json::Value::String(topic.to_string());
        }
    }
    serde_json::to_string(&payload).ok()
}

fn resolve_mobile_push_url(base_url: &str, path: &str) -> Option<String> {
    let mut url = url::Url::parse(base_url.trim()).ok()?;
    let base_path = url.path().trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    url.set_path(&format!("{base_path}/{normalized_path}"));
    Some(url.to_string())
}

fn normalize_platform_key(platform_key: &str) -> String {
    match platform_key.trim().to_ascii_lowercase().as_str() {
        "ios" => "ios".to_string(),
        "android" => "android".to_string(),
        _ => "unsupported".to_string(),
    }
}

fn normalize_path_component(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_hex_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = HashSet::new();
    for value in values {
        let candidate = value.trim().to_ascii_lowercase();
        if candidate.len() == 64 && candidate.chars().all(|char| char.is_ascii_hexdigit()) {
            normalized.insert(candidate);
        }
    }
    sorted_hexes(normalized)
}

fn normalized_payload(raw_payload_json: &str) -> BTreeMap<String, String> {
    let mut payload = BTreeMap::new();
    let Ok(decoded) = serde_json::from_str::<serde_json::Value>(raw_payload_json) else {
        return payload;
    };
    let Some(object) = decoded.as_object() else {
        return payload;
    };
    for (key, value) in object {
        if value.is_null() {
            continue;
        }
        let value = value
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| value.to_string());
        if !value.trim().is_empty() {
            payload.insert(key.clone(), value);
        }
    }
    payload
}

fn resolved_title(payload: &BTreeMap<String, String>) -> String {
    for value in [payload.get("sender_name"), payload.get("title")] {
        if let Some(title) = normalized_sender_title(value) {
            if !is_generic_sender_title(&title) {
                return title;
            }
        }
    }
    "Iris Chat".to_string()
}

fn normalized_sender_title(value: Option<&String>) -> Option<String> {
    let normalized = normalized_value(value)?;
    if normalized.to_lowercase().starts_with("dm by ") && normalized.len() > 6 {
        let stripped = normalized[6..].trim().to_string();
        return (!stripped.is_empty()).then_some(stripped);
    }
    Some(normalized)
}

fn normalized_value(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_generic_sender_title(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "" | "someone" | "new message" | "new activity" | "iris chat"
    )
}

fn event_kind(value: Option<&String>) -> Option<u64> {
    let decoded = serde_json::from_str::<serde_json::Value>(value?).ok()?;
    decoded.get("kind")?.as_u64()
}

fn event_content(value: Option<&String>) -> Option<String> {
    let decoded = serde_json::from_str::<serde_json::Value>(value?).ok()?;
    let content = decoded.get("content")?.as_str()?.to_string();
    normalized_value(Some(&content))
}

fn should_suppress_mobile_push_kind(kind: u64) -> bool {
    matches!(
        kind,
        MOBILE_PUSH_RECEIPT_KIND
            | MOBILE_PUSH_TYPING_KIND
            | MOBILE_PUSH_GROUP_METADATA_KIND
            | MOBILE_PUSH_SETTINGS_KIND
    )
}
