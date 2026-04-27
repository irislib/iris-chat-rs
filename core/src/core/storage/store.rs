use super::super::{
    KnownAppKeyDevice, KnownAppKeys, OwnerProfileRecord, PersistedAuthorizationState,
    PersistedDeliveryState, PersistedMessage, PersistedPreferences, PersistedState,
    PersistedThread, ThreadRecord, PERSISTED_STATE_VERSION,
};
use super::SharedConnection;
use crate::state::{ChatMessageKind, DeliveryState, PreferencesSnapshot};
use nostr_double_ratchet::GroupData;
use rusqlite::{params, OptionalExtension, Transaction};
use std::collections::{BTreeMap, VecDeque};

const META_ACTIVE_CHAT_ID: &str = "active_chat_id";
const META_NEXT_MESSAGE_ID: &str = "next_message_id";
const META_AUTHORIZATION_STATE: &str = "authorization_state";

pub(crate) struct AppStore {
    conn: SharedConnection,
}

impl AppStore {
    pub(crate) fn new(conn: SharedConnection) -> Self {
        Self { conn }
    }

    pub(crate) fn shared(&self) -> SharedConnection {
        self.conn.clone()
    }

    /// Load the durable app state. Returns `Ok(None)` when the database
    /// is empty (no `next_message_id` entry). The shape mirrors the
    /// previous JSON layout so the rest of the core can be left as-is.
    pub(crate) fn load_state(&self) -> anyhow::Result<Option<PersistedState>> {
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
            .and_then(|raw| match raw.as_str() {
                "authorized" => Some(PersistedAuthorizationState::Authorized),
                "awaiting_approval" => Some(PersistedAuthorizationState::AwaitingApproval),
                "revoked" => Some(PersistedAuthorizationState::Revoked),
                _ => None,
            });

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

    /// Replace all durable state in one transaction.
    pub(crate) fn save_state(&self, snapshot: &SaveSnapshot<'_>) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;

        write_meta(&tx, snapshot)?;
        write_preferences(&tx, snapshot.preferences)?;
        write_owner_profiles(&tx, snapshot.owner_profiles)?;
        write_chat_ttls(&tx, snapshot.chat_message_ttl_seconds)?;
        write_app_keys(&tx, snapshot.app_keys)?;
        write_groups(&tx, snapshot.groups)?;
        write_threads_and_messages(&tx, snapshot.threads)?;
        write_seen_events(&tx, snapshot.seen_event_order)?;

        tx.commit()?;
        Ok(())
    }

    /// Drop all durable state. Used by `logout`.
    pub(crate) fn clear(&self) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        for table in [
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
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

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
            let value = match state {
                PersistedAuthorizationState::Authorized => "authorized",
                PersistedAuthorizationState::AwaitingApproval => "awaiting_approval",
                PersistedAuthorizationState::Revoked => "revoked",
            };
            upsert.execute(params![META_AUTHORIZATION_STATE, value])?;
        }
        None => {
            delete.execute([META_AUTHORIZATION_STATE])?;
        }
    }
    Ok(())
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

fn write_app_keys(
    tx: &Transaction,
    app_keys: &BTreeMap<String, KnownAppKeys>,
) -> anyhow::Result<()> {
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

    let mut threads_meta: Vec<(String, u64, u64)> = Vec::new();
    for row in thread_rows {
        threads_meta.push(row?);
    }

    let mut messages_stmt = conn.prepare(
        "SELECT id, kind, author, body, is_outgoing, created_at_secs, expires_at_secs,
                delivery, attachments_json, reactions_json, reactors_json
         FROM messages
         WHERE chat_id = ?1
         ORDER BY created_at_secs ASC, id ASC",
    )?;

    let mut threads = Vec::with_capacity(threads_meta.len());
    for (chat_id, unread_count, updated_at_secs) in threads_meta {
        let messages = messages_stmt.query_map(params![chat_id], |row| {
            Ok(PersistedMessage {
                id: row.get(0)?,
                chat_id: chat_id.clone(),
                kind: parse_message_kind(&row.get::<_, String>(1)?),
                author: row.get(2)?,
                body: row.get(3)?,
                attachments: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
                reactions: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                reactors: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
                is_outgoing: row.get::<_, i64>(4)? != 0,
                created_at_secs: row.get::<_, i64>(5)? as u64,
                expires_at_secs: row.get::<_, Option<i64>>(6)?.map(|secs| secs as u64),
                delivery: parse_delivery(&row.get::<_, String>(7)?),
            })
        })?;

        let mut collected = Vec::new();
        for message in messages {
            collected.push(message?);
        }
        threads.push(PersistedThread {
            chat_id,
            unread_count,
            updated_at_secs,
            messages: collected,
        });
    }
    Ok(threads)
}

fn write_threads_and_messages(
    tx: &Transaction,
    threads: &BTreeMap<String, ThreadRecord>,
) -> anyhow::Result<()> {
    // Cascade also removes messages.
    tx.execute("DELETE FROM threads", [])?;
    let mut thread_stmt = tx.prepare_cached(
        "INSERT INTO threads(chat_id, unread_count, updated_at_secs) VALUES (?1, ?2, ?3)",
    )?;
    let mut message_stmt = tx.prepare_cached(
        "INSERT INTO messages(
            chat_id, id, kind, author, body, is_outgoing, created_at_secs,
            expires_at_secs, delivery, attachments_json, reactions_json, reactors_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;

    for (chat_id, thread) in threads {
        thread_stmt.execute(params![
            chat_id,
            thread.unread_count as i64,
            thread.updated_at_secs as i64,
        ])?;
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
    Ok(())
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

    #[test]
    fn empty_database_returns_none() {
        let (_tmp, store) = fresh_store();
        assert!(store.load_state().unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips_a_thread_with_messages() {
        let (tmp, store) = fresh_store();
        let mut threads = BTreeMap::new();
        let chat_id = "abc123".to_string();
        threads.insert(
            chat_id.clone(),
            ThreadRecord {
                chat_id: chat_id.clone(),
                unread_count: 2,
                updated_at_secs: 100,
                messages: vec![ChatMessageSnapshot {
                    id: "m1".to_string(),
                    chat_id: chat_id.clone(),
                    kind: ChatMessageKind::User,
                    author: "alice".to_string(),
                    body: "hi".to_string(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    reactors: Vec::new(),
                    is_outgoing: false,
                    created_at_secs: 99,
                    expires_at_secs: None,
                    delivery: DeliveryState::Received,
                }],
            },
        );
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_message_ttl_seconds = BTreeMap::new();
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
            chat_message_ttl_seconds: &chat_message_ttl_seconds,
            app_keys: &app_keys,
            groups: &groups,
            threads: &threads,
            seen_event_order: &seen_events,
        };
        store.save_state(&snapshot).unwrap();

        // Drop the store and re-open the database to simulate a restart.
        drop(store);
        let conn = open_database(tmp.path()).unwrap();
        let store = AppStore::new(conn);
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
    fn clear_drops_all_rows() {
        let (_tmp, store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_message_ttl_seconds = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let threads = BTreeMap::new();
        let seen_events = VecDeque::new();
        let snapshot = SaveSnapshot {
            active_chat_id: None,
            next_message_id: 7,
            authorization_state: None,
            preferences: &preferences,
            owner_profiles: &owner_profiles,
            chat_message_ttl_seconds: &chat_message_ttl_seconds,
            app_keys: &app_keys,
            groups: &groups,
            threads: &threads,
            seen_event_order: &seen_events,
        };
        store.save_state(&snapshot).unwrap();
        assert!(store.load_state().unwrap().is_some());
        store.clear().unwrap();
        assert!(store.load_state().unwrap().is_none());
    }
}
