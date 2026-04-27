use super::super::{
    KnownAppKeyDevice, KnownAppKeys, OwnerProfileRecord, PersistedAuthorizationState,
    PersistedDeliveryState, PersistedMessage, PersistedPreferences, PersistedState,
    PersistedThread, ThreadRecord, PERSISTED_STATE_VERSION,
};
use super::SharedConnection;
use crate::state::{ChatMessageKind, DeliveryState, PreferencesSnapshot};
use nostr_double_ratchet::GroupData;
use rusqlite::{params, OptionalExtension, Transaction};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

const META_ACTIVE_CHAT_ID: &str = "active_chat_id";
const META_NEXT_MESSAGE_ID: &str = "next_message_id";
const META_AUTHORIZATION_STATE: &str = "authorization_state";

/// Per-slice fingerprints from the last successful save. We hash the
/// canonical wire form of each slice and skip writes when nothing has
/// changed, so a routine `persist_best_effort` tick that only opens a
/// chat doesn't rewrite preferences/profiles/groups/etc. The cache is
/// reset on `clear` and on construction (a fresh `AppStore` will issue
/// a full write the first time it sees state).
#[derive(Default)]
struct PersistCache {
    meta: Option<u64>,
    preferences: Option<u64>,
    owner_profiles: Option<u64>,
    chat_ttls: Option<u64>,
    app_keys: Option<u64>,
    groups: Option<u64>,
    seen_events: Option<u64>,
    threads: HashMap<String, u64>,
}

pub(crate) struct AppStore {
    conn: SharedConnection,
    cache: PersistCache,
}

impl AppStore {
    pub(crate) fn new(conn: SharedConnection) -> Self {
        Self {
            conn,
            cache: PersistCache::default(),
        }
    }

    pub(crate) fn shared(&self) -> SharedConnection {
        self.conn.clone()
    }

    /// Load the durable app state. Returns `Ok(None)` when the database
    /// is empty (no `next_message_id` entry).
    pub(crate) fn load_state(&mut self) -> anyhow::Result<Option<PersistedState>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;

        let next_message_id_text: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [META_NEXT_MESSAGE_ID],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(next_message_id_text) = next_message_id_text else {
            return Ok(None);
        };
        let next_message_id = next_message_id_text.parse::<u64>().unwrap_or(1);

        let active_chat_id: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [META_ACTIVE_CHAT_ID],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let authorization_state = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [META_AUTHORIZATION_STATE],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(parse_authorization_state);

        let preferences = load_preferences(&conn)?.unwrap_or_default();
        let owner_profiles = load_owner_profiles(&conn)?;
        let chat_message_ttl_seconds = load_chat_ttls(&conn)?;
        let app_keys = load_app_keys(&conn)?;
        let groups = load_groups(&conn)?;
        let threads = load_threads(&conn)?;
        let seen_event_ids = load_seen_events(&conn)?;

        Ok(Some(PersistedState {
            version: PERSISTED_STATE_VERSION,
            active_chat_id,
            next_message_id,
            owner_profiles,
            preferences,
            chat_message_ttl_seconds,
            app_keys,
            groups,
            threads,
            seen_event_ids,
            authorization_state,
        }))
    }

    /// Persist any slice of `snapshot` whose hash differs from the
    /// previous save. The whole batch lands in a single transaction so
    /// either everything or nothing is written.
    pub(crate) fn save_state(&mut self, snapshot: &SaveSnapshot<'_>) -> anyhow::Result<()> {
        let plan = SavePlan::compute(&self.cache, snapshot);
        if plan.is_empty() {
            return Ok(());
        }

        {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            plan.apply(&tx, snapshot)?;
            tx.commit()?;
        }

        plan.update_cache(&mut self.cache);
        Ok(())
    }

    /// Drop every row across every table and forget previous fingerprints.
    pub(crate) fn clear(&mut self) -> anyhow::Result<()> {
        {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            for table in TABLES_TO_CLEAR {
                tx.execute(&format!("DELETE FROM {table}"), [])?;
            }
            tx.commit()?;
        }
        self.cache = PersistCache::default();
        Ok(())
    }
}

const TABLES_TO_CLEAR: &[&str] = &[
    "messages",
    "threads",
    "seen_events",
    "groups",
    "app_keys",
    "owner_profiles",
    "chat_message_ttls",
    "preferences",
    "app_meta",
    "ndr_kv",
];

/// View into `AppCore` fields used to drive a single `save_state` call.
pub(crate) struct SaveSnapshot<'a> {
    pub active_chat_id: Option<&'a str>,
    pub next_message_id: u64,
    pub authorization_state: Option<PersistedAuthorizationState>,
    pub preferences: &'a PreferencesSnapshot,
    pub owner_profiles: &'a BTreeMap<String, OwnerProfileRecord>,
    pub chat_message_ttl_seconds: &'a BTreeMap<String, u64>,
    pub app_keys: &'a BTreeMap<String, KnownAppKeys>,
    pub groups: &'a BTreeMap<String, GroupData>,
    pub threads: &'a BTreeMap<String, ThreadRecord>,
    pub seen_event_order: &'a VecDeque<String>,
}

/// Decision tree for one save tick. Carries the new fingerprints so the
/// cache can be updated atomically with the write.
struct SavePlan {
    meta: Option<u64>,
    preferences: Option<u64>,
    owner_profiles: Option<u64>,
    chat_ttls: Option<u64>,
    app_keys: Option<u64>,
    groups: Option<u64>,
    seen_events: Option<u64>,
    /// chat_id -> new hash; only changed threads are listed here.
    threads_to_write: HashMap<String, u64>,
    /// chat_ids cached previously but no longer present in the snapshot.
    threads_to_delete: Vec<String>,
}

impl SavePlan {
    fn compute(cache: &PersistCache, snapshot: &SaveSnapshot<'_>) -> Self {
        let meta_hash = hash_meta(snapshot);
        let preferences_hash = hash_preferences(snapshot.preferences);
        let owner_profiles_hash = hash_value(snapshot.owner_profiles);
        let chat_ttls_hash = hash_value(snapshot.chat_message_ttl_seconds);
        let app_keys_hash = hash_value(snapshot.app_keys);
        let groups_hash = hash_groups(snapshot.groups);
        let seen_events_hash = hash_seen_events(snapshot.seen_event_order);

        let mut threads_to_write = HashMap::new();
        for (chat_id, thread) in snapshot.threads {
            let hash = hash_thread(thread);
            if cache.threads.get(chat_id) != Some(&hash) {
                threads_to_write.insert(chat_id.clone(), hash);
            }
        }
        let threads_to_delete: Vec<String> = cache
            .threads
            .keys()
            .filter(|chat_id| !snapshot.threads.contains_key(chat_id.as_str()))
            .cloned()
            .collect();

        Self {
            meta: changed(cache.meta, meta_hash),
            preferences: changed(cache.preferences, preferences_hash),
            owner_profiles: changed(cache.owner_profiles, owner_profiles_hash),
            chat_ttls: changed(cache.chat_ttls, chat_ttls_hash),
            app_keys: changed(cache.app_keys, app_keys_hash),
            groups: changed(cache.groups, groups_hash),
            seen_events: changed(cache.seen_events, seen_events_hash),
            threads_to_write,
            threads_to_delete,
        }
    }

    fn is_empty(&self) -> bool {
        self.meta.is_none()
            && self.preferences.is_none()
            && self.owner_profiles.is_none()
            && self.chat_ttls.is_none()
            && self.app_keys.is_none()
            && self.groups.is_none()
            && self.seen_events.is_none()
            && self.threads_to_write.is_empty()
            && self.threads_to_delete.is_empty()
    }

    fn apply(&self, tx: &Transaction, snapshot: &SaveSnapshot<'_>) -> anyhow::Result<()> {
        if self.meta.is_some() {
            write_meta(tx, snapshot)?;
        }
        if self.preferences.is_some() {
            write_preferences(tx, snapshot.preferences)?;
        }
        if self.owner_profiles.is_some() {
            write_owner_profiles(tx, snapshot.owner_profiles)?;
        }
        if self.chat_ttls.is_some() {
            write_chat_ttls(tx, snapshot.chat_message_ttl_seconds)?;
        }
        if self.app_keys.is_some() {
            write_app_keys(tx, snapshot.app_keys)?;
        }
        if self.groups.is_some() {
            write_groups(tx, snapshot.groups)?;
        }
        if self.seen_events.is_some() {
            write_seen_events(tx, snapshot.seen_event_order)?;
        }
        for chat_id in &self.threads_to_delete {
            // Cascades to messages.
            tx.execute("DELETE FROM threads WHERE chat_id = ?1", [chat_id])?;
        }
        if !self.threads_to_write.is_empty() {
            let mut thread_stmt = tx.prepare_cached(
                "INSERT INTO threads(chat_id, unread_count, updated_at_secs)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(chat_id) DO UPDATE SET
                    unread_count = excluded.unread_count,
                    updated_at_secs = excluded.updated_at_secs",
            )?;
            let mut delete_messages = tx.prepare_cached(
                "DELETE FROM messages WHERE chat_id = ?1",
            )?;
            let mut message_stmt = tx.prepare_cached(
                "INSERT INTO messages(
                    chat_id, id, kind, author, body, is_outgoing, created_at_secs,
                    expires_at_secs, delivery, attachments_json, reactions_json, reactors_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for chat_id in self.threads_to_write.keys() {
                let thread = snapshot
                    .threads
                    .get(chat_id)
                    .expect("plan.threads_to_write only references snapshot threads");
                thread_stmt.execute(params![
                    chat_id,
                    thread.unread_count as i64,
                    thread.updated_at_secs as i64,
                ])?;
                delete_messages.execute([chat_id])?;
                for message in &thread.messages {
                    message_stmt.execute(params![
                        chat_id,
                        message.id,
                        serialize_message_kind(&message.kind),
                        message.author,
                        message.body,
                        message.is_outgoing as i64,
                        message.created_at_secs as i64,
                        message.expires_at_secs.map(|secs| secs as i64),
                        serialize_delivery(&message.delivery),
                        serde_json::to_string(&message.attachments)?,
                        serde_json::to_string(&message.reactions)?,
                        serde_json::to_string(&message.reactors)?,
                    ])?;
                }
            }
        }
        Ok(())
    }

    fn update_cache(self, cache: &mut PersistCache) {
        if let Some(hash) = self.meta {
            cache.meta = Some(hash);
        }
        if let Some(hash) = self.preferences {
            cache.preferences = Some(hash);
        }
        if let Some(hash) = self.owner_profiles {
            cache.owner_profiles = Some(hash);
        }
        if let Some(hash) = self.chat_ttls {
            cache.chat_ttls = Some(hash);
        }
        if let Some(hash) = self.app_keys {
            cache.app_keys = Some(hash);
        }
        if let Some(hash) = self.groups {
            cache.groups = Some(hash);
        }
        if let Some(hash) = self.seen_events {
            cache.seen_events = Some(hash);
        }
        for chat_id in self.threads_to_delete {
            cache.threads.remove(&chat_id);
        }
        for (chat_id, hash) in self.threads_to_write {
            cache.threads.insert(chat_id, hash);
        }
    }
}

fn changed(previous: Option<u64>, current: u64) -> Option<u64> {
    if previous == Some(current) {
        None
    } else {
        Some(current)
    }
}

fn hash_value<T: serde::Serialize>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Ok(bytes) = serde_json::to_vec(value) {
        bytes.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_meta(snapshot: &SaveSnapshot<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    snapshot.active_chat_id.hash(&mut hasher);
    snapshot.next_message_id.hash(&mut hasher);
    serialize_authorization_state(snapshot.authorization_state.as_ref()).hash(&mut hasher);
    hasher.finish()
}

fn hash_preferences(preferences: &PreferencesSnapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    preferences.send_typing_indicators.hash(&mut hasher);
    preferences.send_read_receipts.hash(&mut hasher);
    preferences.desktop_notifications_enabled.hash(&mut hasher);
    preferences.startup_at_login_enabled.hash(&mut hasher);
    preferences.nostr_relay_urls.hash(&mut hasher);
    preferences.image_proxy_enabled.hash(&mut hasher);
    preferences.image_proxy_url.hash(&mut hasher);
    preferences.image_proxy_key_hex.hash(&mut hasher);
    preferences.image_proxy_salt_hex.hash(&mut hasher);
    preferences.mobile_push_server_url.hash(&mut hasher);
    hasher.finish()
}

fn hash_groups(groups: &BTreeMap<String, GroupData>) -> u64 {
    // GroupData isn't Hash, but its serde shape is canonical enough for
    // change detection.
    hash_value(groups)
}

fn hash_seen_events(seen_event_order: &VecDeque<String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for event_id in seen_event_order {
        event_id.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_thread(thread: &ThreadRecord) -> u64 {
    let mut hasher = DefaultHasher::new();
    thread.unread_count.hash(&mut hasher);
    thread.updated_at_secs.hash(&mut hasher);
    for message in &thread.messages {
        message.id.hash(&mut hasher);
        message.author.hash(&mut hasher);
        message.body.hash(&mut hasher);
        message.is_outgoing.hash(&mut hasher);
        message.created_at_secs.hash(&mut hasher);
        message.expires_at_secs.hash(&mut hasher);
        serialize_delivery(&message.delivery).hash(&mut hasher);
        serialize_message_kind(&message.kind).hash(&mut hasher);
        // Attachments / reactions / reactors are vec-of-struct; fall
        // back to JSON for a stable byte sequence.
        if let Ok(bytes) = serde_json::to_vec(&message.attachments) {
            bytes.hash(&mut hasher);
        }
        if let Ok(bytes) = serde_json::to_vec(&message.reactions) {
            bytes.hash(&mut hasher);
        }
        if let Ok(bytes) = serde_json::to_vec(&message.reactors) {
            bytes.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn write_meta(tx: &Transaction, snapshot: &SaveSnapshot<'_>) -> anyhow::Result<()> {
    let mut upsert = tx.prepare_cached(
        "INSERT INTO app_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    let mut delete = tx.prepare_cached("DELETE FROM app_meta WHERE key = ?1")?;

    match snapshot.active_chat_id {
        Some(value) => {
            upsert.execute(params![META_ACTIVE_CHAT_ID, value])?;
        }
        None => {
            delete.execute([META_ACTIVE_CHAT_ID])?;
        }
    }

    upsert.execute(params![
        META_NEXT_MESSAGE_ID,
        snapshot.next_message_id.to_string()
    ])?;

    match snapshot.authorization_state.as_ref() {
        Some(state) => {
            upsert.execute(params![
                META_AUTHORIZATION_STATE,
                serialize_authorization_state(Some(state))
            ])?;
        }
        None => {
            delete.execute([META_AUTHORIZATION_STATE])?;
        }
    }
    Ok(())
}

fn parse_authorization_state(raw: String) -> Option<PersistedAuthorizationState> {
    match raw.as_str() {
        "authorized" => Some(PersistedAuthorizationState::Authorized),
        "awaiting_approval" => Some(PersistedAuthorizationState::AwaitingApproval),
        "revoked" => Some(PersistedAuthorizationState::Revoked),
        _ => None,
    }
}

fn serialize_authorization_state(state: Option<&PersistedAuthorizationState>) -> &'static str {
    match state {
        Some(PersistedAuthorizationState::Authorized) => "authorized",
        Some(PersistedAuthorizationState::AwaitingApproval) => "awaiting_approval",
        Some(PersistedAuthorizationState::Revoked) => "revoked",
        None => "",
    }
}

fn load_preferences(conn: &rusqlite::Connection) -> anyhow::Result<Option<PersistedPreferences>> {
    let row = conn
        .query_row(
            "SELECT send_typing_indicators, send_read_receipts, desktop_notifications_enabled,
                    startup_at_login_enabled, nostr_relay_urls_json, image_proxy_enabled,
                    image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex,
                    mobile_push_server_url
             FROM preferences WHERE id = 1",
            [],
            |row| {
                Ok(PersistedPreferences {
                    send_typing_indicators: row.get::<_, i64>(0)? != 0,
                    send_read_receipts: row.get::<_, i64>(1)? != 0,
                    desktop_notifications_enabled: row.get::<_, i64>(2)? != 0,
                    startup_at_login_enabled: row.get::<_, i64>(3)? != 0,
                    nostr_relay_urls: serde_json::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or_default(),
                    image_proxy_enabled: row.get::<_, i64>(5)? != 0,
                    image_proxy_url: row.get::<_, String>(6)?,
                    image_proxy_key_hex: row.get::<_, String>(7)?,
                    image_proxy_salt_hex: row.get::<_, String>(8)?,
                    mobile_push_server_url: row.get::<_, String>(9)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn write_preferences(tx: &Transaction, preferences: &PreferencesSnapshot) -> anyhow::Result<()> {
    let nostr_relay_urls_json = serde_json::to_string(&preferences.nostr_relay_urls)?;
    tx.execute(
        "INSERT INTO preferences (
            id, send_typing_indicators, send_read_receipts, desktop_notifications_enabled,
            startup_at_login_enabled, nostr_relay_urls_json, image_proxy_enabled,
            image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex, mobile_push_server_url
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            send_typing_indicators = excluded.send_typing_indicators,
            send_read_receipts = excluded.send_read_receipts,
            desktop_notifications_enabled = excluded.desktop_notifications_enabled,
            startup_at_login_enabled = excluded.startup_at_login_enabled,
            nostr_relay_urls_json = excluded.nostr_relay_urls_json,
            image_proxy_enabled = excluded.image_proxy_enabled,
            image_proxy_url = excluded.image_proxy_url,
            image_proxy_key_hex = excluded.image_proxy_key_hex,
            image_proxy_salt_hex = excluded.image_proxy_salt_hex,
            mobile_push_server_url = excluded.mobile_push_server_url",
        params![
            preferences.send_typing_indicators as i64,
            preferences.send_read_receipts as i64,
            preferences.desktop_notifications_enabled as i64,
            preferences.startup_at_login_enabled as i64,
            nostr_relay_urls_json,
            preferences.image_proxy_enabled as i64,
            preferences.image_proxy_url,
            preferences.image_proxy_key_hex,
            preferences.image_proxy_salt_hex,
            preferences.mobile_push_server_url,
        ],
    )?;
    Ok(())
}

fn load_owner_profiles(
    conn: &rusqlite::Connection,
) -> anyhow::Result<BTreeMap<String, OwnerProfileRecord>> {
    let mut stmt = conn.prepare(
        "SELECT owner_pubkey_hex, name, display_name, picture, updated_at_secs
         FROM owner_profiles",
    )?;
    let rows = stmt.query_map([], |row| {
        let owner_pubkey_hex: String = row.get(0)?;
        let record = OwnerProfileRecord {
            name: row.get(1)?,
            display_name: row.get(2)?,
            picture: row.get(3)?,
            updated_at_secs: row.get::<_, i64>(4)? as u64,
        };
        Ok((owner_pubkey_hex, record))
    })?;
    let mut profiles = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        profiles.insert(key, value);
    }
    Ok(profiles)
}

fn write_owner_profiles(
    tx: &Transaction,
    profiles: &BTreeMap<String, OwnerProfileRecord>,
) -> anyhow::Result<()> {
    tx.execute("DELETE FROM owner_profiles", [])?;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO owner_profiles
            (owner_pubkey_hex, name, display_name, picture, updated_at_secs)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (owner_pubkey_hex, profile) in profiles {
        stmt.execute(params![
            owner_pubkey_hex,
            profile.name,
            profile.display_name,
            profile.picture,
            profile.updated_at_secs as i64,
        ])?;
    }
    Ok(())
}

fn load_chat_ttls(conn: &rusqlite::Connection) -> anyhow::Result<BTreeMap<String, u64>> {
    let mut stmt = conn.prepare("SELECT chat_id, ttl_seconds FROM chat_message_ttls")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
    })?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (chat_id, ttl) = row?;
        map.insert(chat_id, ttl);
    }
    Ok(map)
}

fn write_chat_ttls(tx: &Transaction, ttls: &BTreeMap<String, u64>) -> anyhow::Result<()> {
    tx.execute("DELETE FROM chat_message_ttls", [])?;
    let mut stmt =
        tx.prepare_cached("INSERT INTO chat_message_ttls(chat_id, ttl_seconds) VALUES (?1, ?2)")?;
    for (chat_id, ttl) in ttls {
        stmt.execute(params![chat_id, *ttl as i64])?;
    }
    Ok(())
}

fn load_app_keys(conn: &rusqlite::Connection) -> anyhow::Result<Vec<KnownAppKeys>> {
    let mut stmt =
        conn.prepare("SELECT owner_pubkey_hex, created_at_secs, devices_json FROM app_keys")?;
    let rows = stmt.query_map([], |row| {
        let owner_pubkey_hex: String = row.get(0)?;
        let created_at_secs: i64 = row.get(1)?;
        let devices_json: String = row.get(2)?;
        Ok((owner_pubkey_hex, created_at_secs, devices_json))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (owner_pubkey_hex, created_at_secs, devices_json) = row?;
        let devices: Vec<KnownAppKeyDevice> =
            serde_json::from_str(&devices_json).unwrap_or_default();
        entries.push(KnownAppKeys {
            owner_pubkey_hex,
            created_at_secs: created_at_secs as u64,
            devices,
        });
    }
    Ok(entries)
}

fn write_app_keys(tx: &Transaction, app_keys: &BTreeMap<String, KnownAppKeys>) -> anyhow::Result<()> {
    tx.execute("DELETE FROM app_keys", [])?;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO app_keys(owner_pubkey_hex, created_at_secs, devices_json)
         VALUES (?1, ?2, ?3)",
    )?;
    for entry in app_keys.values() {
        let devices_json = serde_json::to_string(&entry.devices)?;
        stmt.execute(params![
            entry.owner_pubkey_hex,
            entry.created_at_secs as i64,
            devices_json,
        ])?;
    }
    Ok(())
}

fn load_groups(conn: &rusqlite::Connection) -> anyhow::Result<Vec<GroupData>> {
    let mut stmt = conn.prepare("SELECT group_json FROM groups")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut groups = Vec::new();
    for row in rows {
        let json = row?;
        if let Ok(group) = serde_json::from_str::<GroupData>(&json) {
            groups.push(group);
        }
    }
    Ok(groups)
}

fn write_groups(tx: &Transaction, groups: &BTreeMap<String, GroupData>) -> anyhow::Result<()> {
    tx.execute("DELETE FROM groups", [])?;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO groups(group_id, name, picture, created_at_ms, updated_at_secs, group_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for group in groups.values() {
        let group_json = serde_json::to_string(group)?;
        stmt.execute(params![
            group.id,
            group.name,
            group.picture,
            group.created_at as i64,
            group.created_at as i64 / 1000,
            group_json,
        ])?;
    }
    Ok(())
}

/// Single-pass message load: one SELECT for thread metadata, one for
/// every message in the database, then group in Rust. Avoids one
/// round-trip per chat (the previous loop was N+1 prepared queries).
fn load_threads(conn: &rusqlite::Connection) -> anyhow::Result<Vec<PersistedThread>> {
    let mut threads_stmt =
        conn.prepare("SELECT chat_id, unread_count, updated_at_secs FROM threads")?;
    let thread_rows = threads_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
        ))
    })?;

    let mut by_chat: HashMap<String, PersistedThread> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for row in thread_rows {
        let (chat_id, unread_count, updated_at_secs) = row?;
        order.push(chat_id.clone());
        by_chat.insert(
            chat_id.clone(),
            PersistedThread {
                chat_id,
                unread_count,
                updated_at_secs,
                messages: Vec::new(),
            },
        );
    }

    let mut messages_stmt = conn.prepare(
        "SELECT chat_id, id, kind, author, body, is_outgoing, created_at_secs, expires_at_secs,
                delivery, attachments_json, reactions_json, reactors_json
         FROM messages
         ORDER BY chat_id ASC, created_at_secs ASC, id ASC",
    )?;
    let rows = messages_stmt.query_map([], |row| {
        let chat_id: String = row.get(0)?;
        Ok((
            chat_id.clone(),
            PersistedMessage {
                id: row.get(1)?,
                chat_id,
                kind: parse_message_kind(&row.get::<_, String>(2)?),
                author: row.get(3)?,
                body: row.get(4)?,
                attachments: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                reactions: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
                reactors: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
                is_outgoing: row.get::<_, i64>(5)? != 0,
                created_at_secs: row.get::<_, i64>(6)? as u64,
                expires_at_secs: row.get::<_, Option<i64>>(7)?.map(|secs| secs as u64),
                delivery: parse_delivery(&row.get::<_, String>(8)?),
            },
        ))
    })?;

    for row in rows {
        let (chat_id, message) = row?;
        if let Some(thread) = by_chat.get_mut(&chat_id) {
            thread.messages.push(message);
        }
    }

    Ok(order.into_iter().filter_map(|chat_id| by_chat.remove(&chat_id)).collect())
}

fn load_seen_events(conn: &rusqlite::Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT event_id FROM seen_events ORDER BY sequence ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

fn write_seen_events(tx: &Transaction, seen_event_order: &VecDeque<String>) -> anyhow::Result<()> {
    tx.execute("DELETE FROM seen_events", [])?;
    let mut stmt =
        tx.prepare_cached("INSERT INTO seen_events(event_id, sequence) VALUES (?1, ?2)")?;
    for (sequence, event_id) in seen_event_order.iter().enumerate() {
        stmt.execute(params![event_id, sequence as i64])?;
    }
    Ok(())
}

fn parse_message_kind(raw: &str) -> ChatMessageKind {
    match raw {
        "system" => ChatMessageKind::System,
        _ => ChatMessageKind::User,
    }
}

fn serialize_message_kind(kind: &ChatMessageKind) -> &'static str {
    match kind {
        ChatMessageKind::User => "user",
        ChatMessageKind::System => "system",
    }
}

fn parse_delivery(raw: &str) -> PersistedDeliveryState {
    match raw {
        "queued" => PersistedDeliveryState::Queued,
        "pending" => PersistedDeliveryState::Pending,
        "received" => PersistedDeliveryState::Received,
        "seen" => PersistedDeliveryState::Seen,
        "failed" => PersistedDeliveryState::Failed,
        _ => PersistedDeliveryState::Sent,
    }
}

fn serialize_delivery(state: &DeliveryState) -> &'static str {
    match state {
        DeliveryState::Queued => "queued",
        DeliveryState::Pending => "pending",
        DeliveryState::Sent => "sent",
        DeliveryState::Received => "received",
        DeliveryState::Seen => "seen",
        DeliveryState::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::super::open_database;
    use super::*;
    use crate::state::ChatMessageSnapshot;

    fn fresh_store() -> (tempfile::TempDir, AppStore) {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(tmp.path()).unwrap();
        (tmp, AppStore::new(conn))
    }

    fn empty_snapshot<'a>(
        active_chat_id: Option<&'a str>,
        next_message_id: u64,
        preferences: &'a PreferencesSnapshot,
        owner_profiles: &'a BTreeMap<String, OwnerProfileRecord>,
        chat_ttls: &'a BTreeMap<String, u64>,
        app_keys: &'a BTreeMap<String, KnownAppKeys>,
        groups: &'a BTreeMap<String, GroupData>,
        threads: &'a BTreeMap<String, ThreadRecord>,
        seen_events: &'a VecDeque<String>,
    ) -> SaveSnapshot<'a> {
        SaveSnapshot {
            active_chat_id,
            next_message_id,
            authorization_state: None,
            preferences,
            owner_profiles,
            chat_message_ttl_seconds: chat_ttls,
            app_keys,
            groups,
            threads,
            seen_event_order: seen_events,
        }
    }

    fn sample_message(id: &str, body: &str, ts: u64) -> ChatMessageSnapshot {
        ChatMessageSnapshot {
            id: id.to_string(),
            chat_id: "chat".to_string(),
            kind: ChatMessageKind::User,
            author: "alice".to_string(),
            body: body.to_string(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing: false,
            created_at_secs: ts,
            expires_at_secs: None,
            delivery: DeliveryState::Received,
        }
    }

    fn count(conn: &SharedConnection, table: &str) -> i64 {
        conn.lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn empty_database_returns_none() {
        let (_tmp, mut store) = fresh_store();
        assert!(store.load_state().unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips_a_thread_with_messages() {
        let (tmp, mut store) = fresh_store();
        let mut threads = BTreeMap::new();
        let chat_id = "abc123".to_string();
        threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 2,
                updated_at_secs: 100,
                messages: vec![sample_message("m1", "hi", 99)],
            },
        );
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let mut seen_events = VecDeque::new();
        seen_events.push_back("evt1".to_string());
        seen_events.push_back("evt2".to_string());

        let snapshot = SaveSnapshot {
            active_chat_id: Some(&chat_id),
            next_message_id: 42,
            authorization_state: Some(PersistedAuthorizationState::Authorized),
            preferences: &preferences,
            owner_profiles: &owner_profiles,
            chat_message_ttl_seconds: &chat_ttls,
            app_keys: &app_keys,
            groups: &groups,
            threads: &threads,
            seen_event_order: &seen_events,
        };
        store.save_state(&snapshot).unwrap();

        // Drop the store and re-open the database to simulate a restart.
        drop(store);
        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        let loaded = store.load_state().unwrap().expect("state present");
        assert_eq!(loaded.active_chat_id.as_deref(), Some(chat_id.as_str()));
        assert_eq!(loaded.next_message_id, 42);
        assert_eq!(loaded.threads.len(), 1);
        assert_eq!(loaded.threads[0].messages.len(), 1);
        assert_eq!(loaded.threads[0].messages[0].body, "hi");
        assert_eq!(loaded.seen_event_ids, vec!["evt1", "evt2"]);
        assert!(matches!(
            loaded.authorization_state,
            Some(PersistedAuthorizationState::Authorized)
        ));
    }

    #[test]
    fn second_save_with_unchanged_snapshot_is_a_noop() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let mut threads = BTreeMap::new();
        threads.insert(
            "chat".to_string(),
            ThreadRecord {
                chat_id: "chat".to_string(),
                unread_count: 0,
                updated_at_secs: 1,
                messages: vec![sample_message("m1", "hello", 1)],
            },
        );
        let seen_events = VecDeque::new();
        let snapshot = empty_snapshot(
            None,
            1,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );

        store.save_state(&snapshot).unwrap();
        let plan = SavePlan::compute(&store.cache, &snapshot);
        assert!(
            plan.is_empty(),
            "second save with identical snapshot should plan nothing"
        );
    }

    #[test]
    fn changing_only_one_thread_does_not_rewrite_other_threads() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();

        let mut threads = BTreeMap::new();
        threads.insert(
            "chat-a".to_string(),
            ThreadRecord {
                chat_id: "chat-a".to_string(),
                unread_count: 0,
                updated_at_secs: 1,
                messages: vec![sample_message("m1", "hello", 1)],
            },
        );
        threads.insert(
            "chat-b".to_string(),
            ThreadRecord {
                chat_id: "chat-b".to_string(),
                unread_count: 0,
                updated_at_secs: 2,
                messages: vec![sample_message("m2", "world", 2)],
            },
        );

        let snapshot = empty_snapshot(
            None, 1, &preferences, &owner_profiles, &chat_ttls,
            &app_keys, &groups, &threads, &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        // Change only chat-a; chat-b unchanged.
        threads.get_mut("chat-a").unwrap().messages[0].body = "edited".to_string();
        let snapshot = empty_snapshot(
            None, 1, &preferences, &owner_profiles, &chat_ttls,
            &app_keys, &groups, &threads, &seen_events,
        );
        let plan = SavePlan::compute(&store.cache, &snapshot);
        assert_eq!(plan.threads_to_write.len(), 1);
        assert!(plan.threads_to_write.contains_key("chat-a"));
        assert!(plan.threads_to_delete.is_empty());
        assert!(plan.preferences.is_none());
        assert!(plan.meta.is_none());
    }

    #[test]
    fn removing_a_thread_deletes_only_that_chat() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let conn_handle = store.shared();

        let mut threads = BTreeMap::new();
        threads.insert(
            "chat-a".to_string(),
            ThreadRecord {
                chat_id: "chat-a".to_string(),
                unread_count: 0,
                updated_at_secs: 1,
                messages: vec![sample_message("m1", "stay", 1)],
            },
        );
        threads.insert(
            "chat-b".to_string(),
            ThreadRecord {
                chat_id: "chat-b".to_string(),
                unread_count: 0,
                updated_at_secs: 2,
                messages: vec![sample_message("m2", "go", 2)],
            },
        );

        let snapshot = empty_snapshot(
            None, 1, &preferences, &owner_profiles, &chat_ttls,
            &app_keys, &groups, &threads, &seen_events,
        );
        store.save_state(&snapshot).unwrap();
        assert_eq!(count(&conn_handle, "threads"), 2);
        assert_eq!(count(&conn_handle, "messages"), 2);

        threads.remove("chat-b");
        let snapshot = empty_snapshot(
            None, 1, &preferences, &owner_profiles, &chat_ttls,
            &app_keys, &groups, &threads, &seen_events,
        );
        store.save_state(&snapshot).unwrap();
        assert_eq!(count(&conn_handle, "threads"), 1);
        assert_eq!(count(&conn_handle, "messages"), 1);
    }

    #[test]
    fn clear_drops_all_rows_and_resets_cache() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let threads = BTreeMap::new();
        let seen_events = VecDeque::new();
        let snapshot = empty_snapshot(
            None, 7, &preferences, &owner_profiles, &chat_ttls,
            &app_keys, &groups, &threads, &seen_events,
        );
        store.save_state(&snapshot).unwrap();
        assert!(store.load_state().unwrap().is_some());
        store.clear().unwrap();
        assert!(store.load_state().unwrap().is_none());

        // After clear the cache is empty, so the same snapshot becomes
        // a real write again rather than a no-op.
        let plan = SavePlan::compute(&store.cache, &snapshot);
        assert!(!plan.is_empty(), "cache must be reset on clear");
    }
}
