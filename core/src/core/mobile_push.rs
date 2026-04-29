use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use nostr::Tag;
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MOBILE_PUSH_REACTION_KIND: u64 = 7;
const MOBILE_PUSH_AUTH_KIND: u16 = 27_235;
const MOBILE_PUSH_CHAT_MESSAGE_KIND: u64 = CHAT_MESSAGE_KIND as u64;
const MOBILE_PUSH_INVITE_RESPONSE_KIND: u64 = INVITE_RESPONSE_KIND as u64;
const MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND: u64 = MESSAGE_EVENT_KIND as u64;
const MOBILE_PUSH_PRODUCTION_SERVER_URL: &str = "https://notifications.iris.to";
const MOBILE_PUSH_SANDBOX_SERVER_URL: &str = "https://notifications-sandbox.iris.to";
const MAX_PENDING_MOBILE_PUSH_EVENTS: usize = 32;

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
        let muted_direct_chat_ids: HashSet<String> = self
            .preferences
            .muted_chat_ids
            .iter()
            .filter(|chat_id| !is_group_chat_id(chat_id))
            .cloned()
            .collect();
        let mut message_author_pubkeys = HashSet::new();
        message_author_pubkeys.extend(
            self.known_message_author_hexes()
                .into_iter()
                .filter(|author| !muted_direct_chat_ids.contains(author)),
        );
        let message_author_pubkeys = sorted_hexes(message_author_pubkeys);
        let invite_response_pubkeys = if self.preferences.invite_acceptance_notifications_enabled {
            vec![logged_in.local_invite.inviter_ephemeral_public_key.to_hex()]
        } else {
            Vec::new()
        };

        MobilePushSyncSnapshot {
            owner_pubkey_hex: Some(logged_in.owner_pubkey.to_string()),
            message_author_pubkeys,
            invite_response_pubkeys,
            sessions: Vec::new(),
        }
    }

    pub(super) fn ingest_mobile_push_payload(&mut self, raw_payload_json: &str) {
        let Some(event) = mobile_push_event_from_payload(raw_payload_json) else {
            return;
        };
        let event_id = event.id.to_string();
        if self.logged_in.is_none() {
            if !self.has_seen_event(&event_id)
                && !self
                    .pending_mobile_push_events
                    .iter()
                    .any(|pending| pending.id.to_string() == event_id)
            {
                self.pending_mobile_push_events.push_back(event);
                while self.pending_mobile_push_events.len() > MAX_PENDING_MOBILE_PUSH_EVENTS {
                    self.pending_mobile_push_events.pop_front();
                }
            }
            return;
        }
        self.push_debug_log("push.event.ingest", format!("id={event_id}"));
        self.handle_relay_event(event);
    }

    pub(super) fn drain_pending_mobile_push_events(&mut self) {
        if self.logged_in.is_none() || self.pending_mobile_push_events.is_empty() {
            return;
        }
        self.enter_batch();
        while let Some(event) = self.pending_mobile_push_events.pop_front() {
            self.push_debug_log("push.event.ingest", format!("id={}", event.id));
            self.handle_relay_event(event);
        }
        self.exit_batch();
    }
}

fn mobile_push_event_from_payload(raw_payload_json: &str) -> Option<Event> {
    let payload_value: serde_json::Value = serde_json::from_str(raw_payload_json).ok()?;
    let payload_object = payload_value.as_object()?;
    for key in [
        "event",
        "outer_event",
        "outer_event_json",
        "nostr_event",
        "nostr_event_json",
        "inner_event_json",
    ] {
        let Some(value) = payload_object.get(key) else {
            continue;
        };
        let Some(event_json) = payload_event_json(value) else {
            continue;
        };
        if let Ok(event) = serde_json::from_str::<Event>(&event_json) {
            return Some(event);
        }
    }
    None
}

/// Decrypt the encrypted Nostr event the notification server forwarded
/// (key `event`), look up the sender's display name and the group
/// title (when the rumor carries a `["l", group_id]` tag) from the
/// shared SQLite database, and return a notification resolution whose
/// `title` and `body` are the decrypted plaintext — not the generic
/// "New activity" placeholder the server sent.
///
/// Designed to run in the FCM service / iOS Notification Service
/// Extension where there's no live `AppCore`. We open a fresh
/// connection to `data_dir/core.sqlite3`, wrap a `SqliteStorageAdapter`
/// in a read-through `NotificationPreviewStorage` overlay, run a
/// one-shot `NdrRuntime` against it, then drop everything. Writes stay
/// in the overlay so a notification preview cannot advance or
/// otherwise mutate the chat runtime's persisted ratchet state before
/// the foreground app processes the same relay event. Once we've
/// identified an encrypted outer event, later failures use the SQLite
/// preview or suppress the notification; they never fall back to the
/// server's generic placeholder text.
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
        .and_then(payload_event_json)
        .or_else(|| {
            payload_object
                .get("inner_event_json")
                .and_then(payload_event_json)
        });
    let Some(outer_event_json) = outer_event_json else {
        return fallback();
    };

    let outer_event: nostr::Event = match serde_json::from_str(&outer_event_json) {
        Ok(event) => event,
        Err(_)
            if payload_value_event_kind(payload_object.get("event"))
                == Some(MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND) =>
        {
            return suppressed_resolution();
        }
        Err(_) => return fallback(),
    };

    if outer_event.kind.as_u16() as u64 == MOBILE_PUSH_INVITE_RESPONSE_KIND {
        return invite_acceptance_push_resolution(payload_object, outer_event);
    }

    let outer_event_id = outer_event.id.to_string();
    // When NDR decrypt fails (the foreground app already advanced
    // the ratchet past this event) we look up the message body the
    // foreground stored in SQLite. If even that misses we suppress
    // rather than show a meaningless "New activity" — the foreground
    // saw the wrapper as a non-message rumor (typing, receipt,
    // reaction, settings) and there's nothing useful to show.
    let cached_fallback = || {
        lookup_mobile_push_preview_after_short_wait(&data_dir, &outer_event_id)
            .unwrap_or_else(suppressed_resolution)
    };

    // If the foreground app already wrote a chat message for this
    // wrapper event, prefer that body — it's faster than NDR decrypt
    // and matches exactly what the user would see in-app.
    if let Some(resolution) = lookup_mobile_push_preview(&data_dir, &outer_event_id) {
        return resolution;
    }

    let owner_pubkey = match nostr::PublicKey::parse(owner_pubkey_hex.trim()) {
        Ok(pubkey) => pubkey,
        Err(_) => return cached_fallback(),
    };
    let device_keys = match nostr::Keys::parse(device_nsec.trim()) {
        Ok(keys) => keys,
        Err(_) => return cached_fallback(),
    };

    let shared_conn = match super::storage::open_database(Path::new(&data_dir)) {
        Ok(conn) => conn,
        Err(_) => return cached_fallback(),
    };
    let base_storage = Arc::new(super::storage::SqliteStorageAdapter::new(
        shared_conn.clone(),
        owner_pubkey.to_hex(),
        device_keys.public_key().to_hex(),
    )) as Arc<dyn StorageAdapter>;
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
        return cached_fallback();
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
        return cached_fallback();
    };
    let inner_value: serde_json::Value = match serde_json::from_str(&inner_json) {
        Ok(value) => value,
        Err(_) => return cached_fallback(),
    };
    let inner_kind = inner_value
        .get("kind")
        .and_then(|value| value.as_u64())
        .unwrap_or(MOBILE_PUSH_CHAT_MESSAGE_KIND);

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
    let resolved_chat_id = group_id
        .as_ref()
        .map(|id| group_chat_id(id))
        .unwrap_or_else(|| sender_owner.to_hex());
    if is_chat_muted_in_data_dir(&data_dir, &resolved_chat_id) {
        return suppressed_resolution();
    }

    let body = decrypted_mobile_push_body(inner_kind, &inner_content);

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

    // Render kind-specific text for every kind we can decrypt, but
    // flag non-message kinds as "should not show" so platforms with
    // real suppression (Android FCM service) drop them. iOS NSE
    // can't suppress without the filtering entitlement, so its
    // Swift handler renders the body anyway when it's non-empty —
    // a "Reacted 👍" notification beats a blank one.
    MobilePushNotificationResolution {
        should_show: should_show_mobile_push_kind(inner_kind),
        title,
        body,
        payload_json: serde_json::to_string(&serde_json::Value::Object(resolved_payload))
            .unwrap_or_else(|_| "{}".to_string()),
    }
}

/// When the notification extension can't decrypt an event itself —
/// usually because the foreground app already advanced the ratchet —
/// fall back to the message the foreground app stored in SQLite. The
/// body and sender/group titles are reconstructed from the same tables
/// the chat list reads, so the notification matches what the user
/// would see if they tapped through.
fn lookup_mobile_push_preview(
    data_dir: &str,
    outer_event_id: &str,
) -> Option<MobilePushNotificationResolution> {
    let conn = open_lookup_connection(data_dir)?;
    let (chat_id, body, author_hex): (String, String, String) = conn
        .query_row(
            "SELECT chat_id, body, author
             FROM messages
             WHERE source_event_id = ?1
             LIMIT 1",
            [outer_event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;

    let group_id = chat_id.strip_prefix(GROUP_CHAT_PREFIX);
    if is_chat_muted_in(&conn, &chat_id) {
        return Some(suppressed_resolution());
    }
    let sender_pubkey = nostr::PublicKey::parse(&author_hex).ok();
    let sender_name = sender_pubkey
        .as_ref()
        .and_then(|pubkey| lookup_owner_display_name(&conn, pubkey))
        .filter(|value| !is_generic_sender_title(value))
        .unwrap_or_else(|| {
            // Author column on incoming messages is the resolved
            // display label the foreground app put there. If
            // owner_profiles doesn't have a better one yet, that label
            // is still our best bet — but only if it isn't a literal
            // hex pubkey.
            let fallback = author_hex.trim().to_string();
            if !fallback.is_empty()
                && !is_generic_sender_title(&fallback)
                && !looks_like_hex_pubkey(&fallback)
            {
                fallback
            } else {
                "Iris Chat".to_string()
            }
        });
    let group_title = group_id.and_then(|id| lookup_group_name_in(&conn, id));

    let title = match (&group_title, sender_name.as_str()) {
        (Some(group), sender) if !sender.is_empty() && sender != "Iris Chat" => {
            format!("{sender} in {group}")
        }
        (Some(group), _) => group.clone(),
        (None, sender) => sender.to_string(),
    };

    // Persisted body has the attachment markup stripped already; if
    // it's empty (e.g. an attachment-only message) fall back to the
    // chat-message placeholder so the user sees something rather than
    // a blank notification.
    let body_text = if body.trim().is_empty() {
        decrypted_mobile_push_body(MOBILE_PUSH_CHAT_MESSAGE_KIND, "")
    } else {
        body.clone()
    };

    let mut payload = serde_json::Map::new();
    payload.insert(
        "title".to_string(),
        serde_json::Value::String(title.clone()),
    );
    payload.insert(
        "body".to_string(),
        serde_json::Value::String(body_text.clone()),
    );
    payload.insert(
        "inner_kind".to_string(),
        serde_json::Value::String(MOBILE_PUSH_CHAT_MESSAGE_KIND.to_string()),
    );
    if let Some(pubkey) = sender_pubkey {
        payload.insert(
            "sender_pubkey".to_string(),
            serde_json::Value::String(pubkey.to_hex()),
        );
    }
    if let Some(group_id) = group_id {
        payload.insert(
            "group_id".to_string(),
            serde_json::Value::String(group_id.to_string()),
        );
    }
    let payload_json = serde_json::to_string(&serde_json::Value::Object(payload))
        .unwrap_or_else(|_| "{}".to_string());

    Some(MobilePushNotificationResolution {
        should_show: true,
        title,
        body: body_text,
        payload_json,
    })
}

fn lookup_mobile_push_preview_after_short_wait(
    data_dir: &str,
    outer_event_id: &str,
) -> Option<MobilePushNotificationResolution> {
    for delay_ms in [0_u64, 25, 75, 150] {
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
        if let Some(resolution) = lookup_mobile_push_preview(data_dir, outer_event_id) {
            return Some(resolution);
        }
    }
    None
}

fn looks_like_hex_pubkey(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn lookup_owner_display_name(
    conn: &rusqlite::Connection,
    pubkey: &nostr::PublicKey,
) -> Option<String> {
    let (display_name, name): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT display_name, name FROM owner_profiles WHERE owner_pubkey_hex = ?1",
            [pubkey.to_hex()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    display_name
        .or(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn lookup_group_name_in(conn: &rusqlite::Connection, group_id: &str) -> Option<String> {
    let name: String = conn
        .query_row(
            "SELECT name FROM groups WHERE group_id = ?1",
            [group_id],
            |row| row.get(0),
        )
        .ok()?;
    let trimmed = name.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_chat_muted_in_data_dir(data_dir: &str, chat_id: &str) -> bool {
    open_lookup_connection(data_dir)
        .as_ref()
        .is_some_and(|conn| is_chat_muted_in(conn, chat_id))
}

fn is_chat_muted_in(conn: &rusqlite::Connection, chat_id: &str) -> bool {
    let muted_json: Option<String> = conn
        .query_row(
            "SELECT muted_chat_ids_json FROM preferences WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let Some(muted_json) = muted_json else {
        return false;
    };
    let muted_chat_ids: Vec<String> = serde_json::from_str(&muted_json).unwrap_or_default();
    muted_chat_ids.iter().any(|muted| muted == chat_id)
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

fn suppressed_resolution() -> MobilePushNotificationResolution {
    MobilePushNotificationResolution {
        should_show: false,
        title: String::new(),
        body: String::new(),
        payload_json: "{}".to_string(),
    }
}

fn open_lookup_connection(data_dir: &str) -> Option<rusqlite::Connection> {
    let path = PathBuf::from(data_dir).join(super::storage::CORE_DB_FILENAME);
    if !path.exists() {
        return None;
    }
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn lookup_sender_display_name(data_dir: &str, sender: &nostr::PublicKey) -> Option<String> {
    let conn = open_lookup_connection(data_dir)?;
    let (display_name, name): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT display_name, name FROM owner_profiles WHERE owner_pubkey_hex = ?1",
            [sender.to_hex()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    display_name
        .or(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn lookup_direct_thread_sender_name(data_dir: &str, sender: &nostr::PublicKey) -> Option<String> {
    let conn = open_lookup_connection(data_dir)?;
    let mut stmt = conn
        .prepare(
            "SELECT author FROM messages
             WHERE chat_id = ?1 AND is_outgoing = 0
             ORDER BY created_at_secs DESC, id DESC",
        )
        .ok()?;
    let rows = stmt
        .query_map([sender.to_hex()], |row| row.get::<_, String>(0))
        .ok()?;
    for row in rows.flatten() {
        let author = row.trim().to_string();
        if !author.is_empty() && !is_generic_sender_title(&author) {
            return Some(author);
        }
    }
    None
}

fn lookup_group_name(data_dir: &str, group_id: &str) -> Option<String> {
    let conn = open_lookup_connection(data_dir)?;
    let name: String = conn
        .query_row(
            "SELECT name FROM groups WHERE group_id = ?1",
            [group_id],
            |row| row.get(0),
        )
        .ok()?;
    let trimmed = name.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
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
        .or_else(|| event_kind(payload.get("inner_event")));

    if inner_kind.is_none()
        && event_kind(payload.get("event")) == Some(MOBILE_PUSH_INVITE_RESPONSE_KIND)
    {
        return invite_acceptance_fallback_resolution(payload);
    }

    if inner_kind.is_some_and(|kind| !should_show_mobile_push_kind(kind)) {
        return MobilePushNotificationResolution {
            should_show: false,
            title: String::new(),
            body: String::new(),
            payload_json: "{}".to_string(),
        };
    }
    if inner_kind.is_none()
        && event_kind(payload.get("event")) == Some(MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND)
    {
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
    invite_response_pubkeys: Vec<String>,
    is_release: bool,
    server_url_override: Option<String>,
) -> Option<MobilePushSubscriptionRequest> {
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
        invite_response_pubkeys,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_mobile_push_update_subscription_request(
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
    let subscription_id = normalize_path_component(&subscription_id)?;
    let body_json = mobile_push_subscription_body_json(
        &platform_key,
        &push_token,
        apns_topic.as_deref(),
        message_author_pubkeys,
        invite_response_pubkeys,
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
    invite_response_pubkeys: Vec<String>,
) -> Option<String> {
    let platform = normalize_platform_key(platform_key);
    let token = push_token.trim();
    if token.is_empty() {
        return None;
    }
    let authors = normalize_hex_list(message_author_pubkeys);
    let invite_response_pubkeys = normalize_hex_list(invite_response_pubkeys);
    if authors.is_empty() && invite_response_pubkeys.is_empty() {
        return None;
    }
    let mut filters = Vec::new();
    if !authors.is_empty() {
        filters.push(serde_json::json!({
            "kinds": [MOBILE_PUSH_OUTER_MESSAGE_EVENT_KIND],
            "authors": authors,
        }));
    }
    if !invite_response_pubkeys.is_empty() {
        filters.push(serde_json::json!({
            "kinds": [MOBILE_PUSH_INVITE_RESPONSE_KIND],
            "#p": invite_response_pubkeys,
        }));
    }
    let primary_filter = filters.first()?.clone();

    let mut payload = serde_json::json!({
        "webhooks": [],
        "web_push_subscriptions": [],
        "fcm_tokens": if platform == "android" { vec![token.to_string()] } else { Vec::<String>::new() },
        "apns_tokens": if platform == "ios" { vec![token.to_string()] } else { Vec::<String>::new() },
        "filter": primary_filter,
        "filters": filters,
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

fn payload_event_json(value: &serde_json::Value) -> Option<String> {
    if let Some(raw) = value.as_str() {
        return Some(raw.to_string());
    }
    if value.is_object() {
        return serde_json::to_string(value).ok();
    }
    None
}

fn payload_value_event_kind(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    if let Some(kind) = value.get("kind").and_then(|kind| kind.as_u64()) {
        return Some(kind);
    }
    let raw = value.as_str()?;
    let decoded = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    decoded.get("kind")?.as_u64()
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

fn invite_acceptance_push_resolution(
    payload_object: &serde_json::Map<String, serde_json::Value>,
    event: nostr::Event,
) -> MobilePushNotificationResolution {
    let title = "Invite accepted".to_string();
    let body = "Someone joined your chat".to_string();
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
        "inner_kind".to_string(),
        serde_json::Value::String(MOBILE_PUSH_INVITE_RESPONSE_KIND.to_string()),
    );
    resolved_payload.insert(
        "invite_response_event_id".to_string(),
        serde_json::Value::String(event.id.to_hex()),
    );

    MobilePushNotificationResolution {
        should_show: true,
        title,
        body,
        payload_json: serde_json::to_string(&serde_json::Value::Object(resolved_payload))
            .unwrap_or_else(|_| "{}".to_string()),
    }
}

fn invite_acceptance_fallback_resolution(
    mut payload: BTreeMap<String, String>,
) -> MobilePushNotificationResolution {
    let title = "Invite accepted".to_string();
    let body = "Someone joined your chat".to_string();
    payload.insert("title".to_string(), title.clone());
    payload.insert("body".to_string(), body.clone());
    payload.insert(
        "inner_kind".to_string(),
        MOBILE_PUSH_INVITE_RESPONSE_KIND.to_string(),
    );

    MobilePushNotificationResolution {
        should_show: true,
        title,
        body,
        payload_json: serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
    }
}

fn decrypted_mobile_push_body(kind: u64, content: &str) -> String {
    let content = content.trim();
    match kind {
        MOBILE_PUSH_CHAT_MESSAGE_KIND => {
            if content.is_empty() {
                "New message".to_string()
            } else {
                content.to_string()
            }
        }
        MOBILE_PUSH_REACTION_KIND => reaction_push_body(content),
        kind if kind == TYPING_KIND as u64 => "is typing".to_string(),
        kind if kind == RECEIPT_KIND as u64 => "Seen".to_string(),
        kind if kind == GROUP_METADATA_KIND as u64 => "Updated group".to_string(),
        kind if kind == CHAT_SETTINGS_KIND as u64 => "Updated chat".to_string(),
        kind if kind == APP_KEYS_EVENT_KIND as u64 => "Updated devices".to_string(),
        MOBILE_PUSH_INVITE_RESPONSE_KIND => "Someone joined your chat".to_string(),
        _ => {
            if content.is_empty() {
                "New activity".to_string()
            } else {
                "Updated chat".to_string()
            }
        }
    }
}

fn reaction_push_body(content: &str) -> String {
    let emoji = content.trim();
    if emoji.is_empty() {
        "Reacted".to_string()
    } else if emoji.to_lowercase().starts_with("reacted") {
        emoji.to_string()
    } else {
        format!("Reacted {emoji}")
    }
}

fn should_show_mobile_push_kind(kind: u64) -> bool {
    // Only chat messages get a foreground push. Reactions, read
    // receipts, typing rumors, group/settings control events, and
    // app-keys updates are noise as standalone notifications — the
    // user sees the right state when they open the chat.
    matches!(
        kind,
        MOBILE_PUSH_CHAT_MESSAGE_KIND | MOBILE_PUSH_INVITE_RESPONSE_KIND
    )
}
