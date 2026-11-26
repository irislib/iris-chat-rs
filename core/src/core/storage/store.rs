use super::super::{
    ChatReadState, KnownAppKeyDevice, KnownAppKeys, OwnerProfileRecord,
    PersistedAuthorizationState, PersistedDeliveryState, PersistedMessage, PersistedPreferences,
    PersistedState, PersistedThread, ThreadRecord, PERSISTED_STATE_VERSION,
};
use super::SharedConnection;
use crate::state::{
    ChatMessageKind, ChatMessageSnapshot, DeliveryState, MessageDeliveryTraceSnapshot,
    MessageRecipientDeliverySnapshot, PreferencesSnapshot,
};
use nostr_double_ratchet::GroupSnapshot;
use rusqlite::{params, OptionalExtension, Row, Transaction};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

#[path = "store_message_requests.rs"]
mod store_message_requests;

#[path = "store_chat_deletions.rs"]
mod store_chat_deletions;

#[path = "store_device_sync.rs"]
mod store_device_sync;

#[path = "store_message_deletions.rs"]
mod store_message_deletions;

#[path = "store_chat_read_states.rs"]
mod store_chat_read_states;

#[path = "store_chat_mutes.rs"]
mod store_chat_mutes;
#[path = "store_chat_pins.rs"]
mod store_chat_pins;

#[path = "store_outgoing_events.rs"]
mod store_outgoing_events;

#[path = "store_message_pages.rs"]
mod store_message_pages;
#[path = "store_persist_cache.rs"]
mod store_persist_cache;
#[path = "store_preferences.rs"]
mod store_preferences;
#[cfg(test)]
use store_message_pages::load_messages_before;
pub(crate) use store_message_pages::{load_messages_around, load_recent_messages};
pub(crate) use store_message_pages::{
    load_messages_around_with_visibility, load_messages_before_with_visibility,
    load_recent_messages_with_visibility,
};
pub(crate) use store_persist_cache::SaveSnapshot;
use store_persist_cache::{
    hash_group_entry, hash_message, hash_meta_fields, hash_persisted_message,
    hash_seen_events_iter, hash_std, hash_thread_header, hash_thread_header_fields,
    plan_thread_write, MapDiff, PersistCache, SavePlan, SeenEventsPlan, ThreadPersistCache,
    ThreadWritePlan,
};
use store_preferences::{
    hash_persisted_preferences, hash_preferences, load_preferences, write_preferences,
};
#[path = "store_private_contacts.rs"]
mod store_private_contacts;
#[path = "store_profiles.rs"]
mod store_profiles;
use store_profiles::{load_owner_profiles, write_owner_profiles_diff};
#[path = "store_profile_search.rs"]
mod store_profile_search;
#[path = "store_user_discovery.rs"]
mod store_user_discovery;

const META_ACTIVE_CHAT_ID: &str = "active_chat_id";
const META_NEXT_MESSAGE_ID: &str = "next_message_id";
const META_AUTHORIZATION_STATE: &str = "authorization_state";
const RESTORED_MESSAGES_PER_THREAD: usize = 80;

pub(crate) struct AppStore {
    pub(super) conn: SharedConnection,
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

        let persisted_preferences = load_preferences(&conn)?;
        let owner_profiles = load_owner_profiles(&conn)?;
        let chat_message_ttl_seconds = load_chat_ttls(&conn)?;
        let app_keys = load_app_keys(&conn)?;
        let groups = load_groups(&conn)?;
        let group_pictures = load_group_pictures(&conn)?;
        let threads = load_threads(&conn, active_chat_id.as_deref())?;
        let (seen_event_ids, seen_events_max_seq) = load_seen_events(&conn)?;
        drop(conn);

        // Seed the diff cache from the raw rows we just read so the first save
        // after launch only writes slices/rows that actually changed (including
        // any legacy normalization applied in memory after load).
        self.cache.meta = Some(hash_meta_fields(
            active_chat_id.as_deref(),
            next_message_id,
            authorization_state.as_ref(),
        ));
        self.cache.preferences = persisted_preferences
            .as_ref()
            .map(hash_persisted_preferences);
        self.cache.owner_profiles = Some(
            owner_profiles
                .iter()
                .map(|(k, v)| (k.clone(), hash_std(v)))
                .collect(),
        );
        self.cache.chat_ttls = Some(
            chat_message_ttl_seconds
                .iter()
                .map(|(k, v)| (k.clone(), hash_std(v)))
                .collect(),
        );
        self.cache.app_keys = Some(
            app_keys
                .iter()
                .map(|entry| (entry.owner_pubkey_hex.clone(), hash_std(entry)))
                .collect(),
        );
        self.cache.groups = Some(
            groups
                .iter()
                .map(|group| {
                    (
                        group.group_id.clone(),
                        hash_group_entry(group, group_pictures.get(&group.group_id)),
                    )
                })
                .collect(),
        );
        self.cache.threads = threads
            .iter()
            .map(|thread| {
                (
                    thread.chat_id.clone(),
                    ThreadPersistCache {
                        header_hash: Some(hash_thread_header_fields(
                            thread.unread_count,
                            thread.updated_at_secs,
                            &thread.draft,
                        )),
                        messages: thread
                            .messages
                            .iter()
                            .map(|message| (message.id.clone(), hash_persisted_message(message)))
                            .collect(),
                    },
                )
            })
            .collect();
        self.cache.seen_events_persisted = seen_event_ids.iter().cloned().collect();
        self.cache.seen_events_hash = Some(hash_seen_events_iter(
            seen_event_ids.len(),
            seen_event_ids.iter().map(String::as_str),
        ));
        self.cache.seen_events_next_seq = seen_events_max_seq.saturating_add(1);

        let preferences = persisted_preferences.unwrap_or_default();
        Ok(Some(PersistedState {
            version: PERSISTED_STATE_VERSION,
            active_chat_id,
            next_message_id,
            owner_profiles,
            preferences,
            chat_message_ttl_seconds,
            app_keys,
            groups,
            group_pictures,
            threads,
            seen_event_ids,
            authorization_state,
        }))
    }

    pub(crate) fn has_app_key_device(
        &self,
        owner_pubkey_hex: &str,
        device_pubkey_hex: &str,
    ) -> anyhow::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let devices_json: Option<String> = conn
            .query_row(
                "SELECT devices_json FROM app_keys WHERE owner_pubkey_hex = ?1",
                [owner_pubkey_hex],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(devices_json) = devices_json else {
            return Ok(false);
        };
        let devices: Vec<KnownAppKeyDevice> =
            serde_json::from_str(&devices_json).unwrap_or_default();
        Ok(devices.iter().any(|device| {
            device
                .identity_pubkey_hex
                .eq_ignore_ascii_case(device_pubkey_hex)
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

    pub(crate) fn prepare_for_suspend(&mut self) -> anyhow::Result<()> {
        // Saves already commit their transactions. Never run PRAGMA optimize
        // here: it can start ANALYZE writes just as iOS is about to suspend us.
        // iOS uses DELETE journaling and has no WAL to checkpoint.
        // Direct FFI readers share this connection on iOS. Drain any reader
        // already holding it even though there is no WAL checkpoint to run.
        let _conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        anyhow::ensure!(
            _conn.is_autocommit(),
            "storage transaction did not finish before suspension"
        );
        #[cfg(not(target_os = "ios"))]
        _conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    pub(crate) fn delete_message(&mut self, chat_id: &str, message_id: &str) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        conn.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )?;
        if let Some(entry) = self.cache.threads.get_mut(chat_id) {
            entry.messages.remove(message_id);
        }
        Ok(())
    }

    pub(crate) fn delete_expired_messages(&mut self, now_secs: u64) -> anyhow::Result<usize> {
        let (deleted, chat_ids) = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
            let tx = conn.transaction()?;
            let chat_ids = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT chat_id FROM messages
                     WHERE expires_at_secs IS NOT NULL AND expires_at_secs <= ?1",
                )?;
                let rows = stmt.query_map([now_secs as i64], |row| row.get::<_, String>(0))?;
                let mut chat_ids = Vec::new();
                for row in rows {
                    chat_ids.push(row?);
                }
                chat_ids
            };
            tx.execute(
                "DELETE FROM pending_message_reads WHERE expires_at_secs <= ?1",
                [now_secs as i64],
            )?;
            store_message_deletions::mark_expired(&tx, now_secs)?;
            store_message_deletions::purge_expired_controls(&tx, now_secs)?;
            let deleted = tx.execute(
                "DELETE FROM messages
                 WHERE expires_at_secs IS NOT NULL AND expires_at_secs <= ?1",
                [now_secs as i64],
            )?;
            tx.commit()?;
            (deleted, chat_ids)
        };

        for chat_id in chat_ids {
            self.cache.threads.remove(&chat_id);
        }
        Ok(deleted)
    }

    pub(crate) fn next_message_expiration_after(
        &self,
        now_secs: u64,
    ) -> anyhow::Result<Option<u64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let expires_at = conn.query_row(
            "SELECT MIN(expires_at_secs) FROM messages
             WHERE expires_at_secs IS NOT NULL AND expires_at_secs > ?1",
            [now_secs as i64],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        let control_expiry =
            store_message_deletions::next_control_expiration_after(&conn, now_secs)?;
        Ok(expires_at
            .map(|secs| secs as u64)
            .into_iter()
            .chain(control_expiry)
            .min())
    }

    pub(crate) fn load_recent_messages(
        &self,
        chat_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<PersistedMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_recent_messages(&conn, chat_id, limit)
    }

    pub(crate) fn load_thread(
        &self,
        chat_id: &str,
        limit: usize,
    ) -> anyhow::Result<Option<PersistedThread>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_thread(&conn, chat_id, limit)
    }

    #[cfg(test)]
    pub(crate) fn load_messages_before(
        &self,
        chat_id: &str,
        before_message_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<PersistedMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_messages_before(&conn, chat_id, before_message_id, limit)
    }

    pub(crate) fn load_messages_around(
        &self,
        chat_id: &str,
        message_id: &str,
        before_limit: usize,
        after_limit: usize,
    ) -> anyhow::Result<Vec<PersistedMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        load_messages_around(&conn, chat_id, message_id, before_limit, after_limit)
    }

    pub(crate) fn seed_loaded_messages(&mut self, chat_id: &str, messages: &[PersistedMessage]) {
        if messages.is_empty() {
            return;
        }
        let entry = self.cache.threads.entry(chat_id.to_string()).or_default();
        for message in messages {
            entry
                .messages
                .insert(message.id.clone(), hash_persisted_message(message));
        }
    }

    pub(crate) fn seed_loaded_thread(&mut self, thread: &PersistedThread) {
        let entry = self
            .cache
            .threads
            .entry(thread.chat_id.clone())
            .or_default();
        entry.header_hash = Some(hash_thread_header_fields(
            thread.unread_count,
            thread.updated_at_secs,
            &thread.draft,
        ));
        for message in &thread.messages {
            entry
                .messages
                .insert(message.id.clone(), hash_persisted_message(message));
        }
    }

    pub(crate) fn save_message_receipts(
        &mut self,
        chat_id: &str,
        messages: &[ChatMessageSnapshot],
        unread_thread: Option<&ThreadRecord>,
    ) -> anyhow::Result<u64> {
        if messages.is_empty() {
            return Ok(0);
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        let mut newly_seen_unread = 0;
        let mut written_messages = Vec::new();
        if let Some(thread) = unread_thread {
            // A relay batch may have newer incoming rows only in memory. Include
            // unsaved ones before locating the last counted unread messages, or
            // an old receipt could incorrectly clear a newer message's badge.
            let cached = self.cache.threads.get(chat_id);
            for message in &thread.messages {
                let message_hash = hash_message(message);
                if cached.and_then(|entry| entry.messages.get(&message.id)) != Some(&message_hash) {
                    upsert_message_row(&tx, chat_id, message)?;
                    written_messages.push((message.id.clone(), message_hash));
                }
            }
            let mut statement = tx.prepare_cached(
                "SELECT id FROM messages WHERE chat_id = ?1
                 AND is_outgoing = 0 AND delivery != 'seen'
                 ORDER BY created_at_secs DESC, rowid DESC LIMIT ?2",
            )?;
            let ids = statement.query_map(params![chat_id, thread.unread_count as i64], |row| {
                row.get::<_, String>(0)
            })?;
            for id in ids {
                let id = id?;
                newly_seen_unread += u64::from(messages.iter().any(|message| message.id == id));
            }
        }
        {
            let mut statement = tx.prepare_cached(
                "UPDATE messages SET delivery = ?3, recipient_deliveries_json = ?4
                 WHERE chat_id = ?1 AND id = ?2",
            )?;
            for message in messages {
                statement.execute(params![
                    chat_id,
                    message.id,
                    serialize_delivery(&message.delivery),
                    serde_json::to_string(&message.recipient_deliveries)?
                ])?;
            }
        }
        tx.commit()?;
        if !written_messages.is_empty() || self.cache.threads.contains_key(chat_id) {
            let entry = self.cache.threads.entry(chat_id.to_string()).or_default();
            for (id, hash) in written_messages {
                entry.messages.insert(id, hash);
            }
            for message in messages {
                entry.messages.remove(&message.id);
            }
        }
        Ok(newly_seen_unread)
    }

    pub(crate) fn message_exists(
        &self,
        chat_id: &str,
        message_id: Option<&str>,
        source_event_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        if message_id.is_none() && source_event_id.is_none() {
            return Ok(false);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM messages
                 WHERE chat_id = ?1
                   AND ((?2 IS NOT NULL AND id = ?2)
                        OR (?3 IS NOT NULL AND source_event_id = ?3))
                 LIMIT 1",
                params![chat_id, message_id, source_event_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }

    #[cfg(test)]
    pub(crate) fn search_messages_fts(
        &self,
        query: &str,
        scope_chat_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<PersistedMessageSearchHit>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        search_messages_fts(&conn, query, scope_chat_id, limit)
    }

    pub(crate) fn upsert_notification_preview_message(
        &mut self,
        chat_id: &str,
        unread_count: u64,
        updated_at_secs: u64,
        message: &ChatMessageSnapshot,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        if store_chat_deletions::message_was_deleted(&tx, chat_id, message.created_at_secs)?
            || store_message_deletions::contains(
                &tx,
                chat_id,
                Some(&message.id),
                message.source_event_id.as_deref(),
            )?
        {
            return Ok(());
        }
        let message_exists = tx
            .query_row(
                "SELECT 1 FROM messages WHERE chat_id = ?1 AND id = ?2 LIMIT 1",
                params![chat_id, message.id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !message_exists {
            tx.execute(
                "INSERT INTO threads(chat_id, unread_count, updated_at_secs)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(chat_id) DO UPDATE SET
                    unread_count = excluded.unread_count,
                    updated_at_secs = MAX(threads.updated_at_secs, excluded.updated_at_secs)",
                params![chat_id, unread_count as i64, updated_at_secs as i64],
            )?;
        }
        upsert_notification_preview_message_row(&tx, chat_id, message)?;
        tx.commit()?;
        if let Some(entry) = self.cache.threads.get_mut(chat_id) {
            if !message_exists {
                entry.header_hash = None;
            }
            entry.messages.remove(&message.id);
        }
        Ok(())
    }
}

const TABLES_TO_CLEAR: &[&str] = &[
    "mesh_group_envelopes",
    "direct_file_transfers",
    "pending_message_reads",
    "messages",
    "threads",
    "seen_events",
    "groups",
    "app_keys",
    "user_discovery_social",
    "user_discovery_users",
    "user_discovery_state",
    "profile_search_candidates",
    "owner_profiles",
    "chat_message_ttls",
    "pending_relay_publishes",
    "preferences",
    "app_meta",
    "ndr_kv",
];

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

fn upsert_thread_header(
    tx: &Transaction,
    chat_id: &str,
    thread: &ThreadRecord,
) -> anyhow::Result<()> {
    tx.prepare_cached(
        "INSERT INTO threads(chat_id, unread_count, updated_at_secs, draft)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(chat_id) DO UPDATE SET
            unread_count = excluded.unread_count,
            updated_at_secs = excluded.updated_at_secs,
            draft = excluded.draft",
    )?
    .execute(params![
        chat_id,
        thread.unread_count as i64,
        thread.updated_at_secs as i64,
        thread.draft
    ])?;
    Ok(())
}

fn apply_thread_write_plan(
    tx: &Transaction,
    chat_id: &str,
    thread: &ThreadRecord,
    plan: &ThreadWritePlan,
) -> anyhow::Result<()> {
    if plan.header_hash.is_some() {
        upsert_thread_header(tx, chat_id, thread)?;
    }
    for &(index, _, _) in &plan.messages {
        // The row upsert preserves a previously stored incoming Seen.
        if let Some(message) = thread.messages.get(index) {
            upsert_message_row(tx, chat_id, message)?;
        }
    }
    Ok(())
}

fn upsert_unsaved_thread_rows(
    tx: &Transaction,
    cached: Option<&ThreadPersistCache>,
    chat_id: &str,
    thread: &ThreadRecord,
) -> anyhow::Result<Option<ThreadWritePlan>> {
    let Some(plan) = plan_thread_write(cached, thread) else {
        return Ok(None);
    };
    apply_thread_write_plan(tx, chat_id, thread, &plan)?;
    Ok(Some(plan))
}

fn upsert_message_row(
    tx: &Transaction,
    chat_id: &str,
    message: &ChatMessageSnapshot,
) -> anyhow::Result<()> {
    if store_message_deletions::contains(
        tx,
        chat_id,
        Some(&message.id),
        message.source_event_id.as_deref(),
    )? {
        return Ok(());
    }
    let pending_seen: bool = !message.is_outgoing
        && tx
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2 AND created_at_secs = ?3)",
            )?
            .query_row(
                params![chat_id, message.id, message.created_at_secs as i64],
                |row| row.get(0),
            )?;
    tx.prepare_cached(
        "INSERT INTO messages(
            chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing, created_at_secs,
            expires_at_secs, delivery, attachments_json, reactions_json, reactors_json,
            source_event_id, recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex, edit_history_json, deleted_for_everyone
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(chat_id, id) DO UPDATE SET
            kind = excluded.kind,
            author = excluded.author,
            author_owner_pubkey_hex = excluded.author_owner_pubkey_hex,
            body = excluded.body,
            is_outgoing = excluded.is_outgoing,
            created_at_secs = excluded.created_at_secs,
            expires_at_secs = excluded.expires_at_secs,
            delivery = CASE WHEN messages.is_outgoing = 0 AND messages.delivery = 'seen' THEN 'seen' ELSE excluded.delivery END,
            attachments_json = excluded.attachments_json,
            reactions_json = excluded.reactions_json,
            reactors_json = excluded.reactors_json,
            source_event_id = excluded.source_event_id,
            recipient_deliveries_json = excluded.recipient_deliveries_json,
            delivery_trace_json = excluded.delivery_trace_json,
            call_json = excluded.call_json,
            system_notice_owner_pubkey_hex = excluded.system_notice_owner_pubkey_hex,
            edit_history_json = excluded.edit_history_json,
            deleted_for_everyone = excluded.deleted_for_everyone",
    )?
    .execute(params![
        chat_id,
        message.id,
        serialize_message_kind(&message.kind),
        message.author,
        message.author_owner_pubkey_hex,
        message.body,
        message.is_outgoing as i64,
        message.created_at_secs as i64,
        message.expires_at_secs.map(|secs| secs as i64),
        if pending_seen {
            "seen"
        } else {
            serialize_delivery(&message.delivery)
        },
        serde_json::to_string(&message.attachments)?,
        serde_json::to_string(&message.reactions)?,
        serde_json::to_string(&message.reactors)?,
        message.source_event_id,
        serde_json::to_string(&message.recipient_deliveries)?,
        serde_json::to_string(&message.delivery_trace)?,
        message
            .call
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
        message.system_notice_owner_pubkey_hex,
        serde_json::to_string(&message.edit_history)?,
        message.deleted_for_everyone as i64,
    ])?;
    tx.prepare_cached("DELETE FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2")?
        .execute(params![chat_id, message.id])?;
    Ok(())
}

fn upsert_notification_preview_message_row(
    tx: &Transaction,
    chat_id: &str,
    message: &ChatMessageSnapshot,
) -> anyhow::Result<()> {
    let pending_seen: bool = !message.is_outgoing
        && tx
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2 AND created_at_secs = ?3)",
            )?
            .query_row(
                params![chat_id, message.id, message.created_at_secs as i64],
                |row| row.get(0),
            )?;
    tx.prepare_cached(
        "INSERT INTO messages(
            chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing, created_at_secs,
            expires_at_secs, delivery, attachments_json, reactions_json, reactors_json,
            source_event_id, recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex, edit_history_json, deleted_for_everyone
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(chat_id, id) DO UPDATE SET
            author_owner_pubkey_hex = COALESCE(messages.author_owner_pubkey_hex, excluded.author_owner_pubkey_hex),
            source_event_id = COALESCE(NULLIF(messages.source_event_id, ''), excluded.source_event_id),
            system_notice_owner_pubkey_hex = COALESCE(messages.system_notice_owner_pubkey_hex, excluded.system_notice_owner_pubkey_hex)",
    )?
    .execute(params![
        chat_id,
        message.id,
        serialize_message_kind(&message.kind),
        message.author,
        message.author_owner_pubkey_hex,
        message.body,
        message.is_outgoing as i64,
        message.created_at_secs as i64,
        message.expires_at_secs.map(|secs| secs as i64),
        if pending_seen {
            "seen"
        } else {
            serialize_delivery(&message.delivery)
        },
        serde_json::to_string(&message.attachments)?,
        serde_json::to_string(&message.reactions)?,
        serde_json::to_string(&message.reactors)?,
        message.source_event_id,
        serde_json::to_string(&message.recipient_deliveries)?,
        serde_json::to_string(&message.delivery_trace)?,
        message
            .call
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
        message.system_notice_owner_pubkey_hex,
        serde_json::to_string(&message.edit_history)?,
        message.deleted_for_everyone as i64,
    ])?;
    tx.prepare_cached("DELETE FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2")?
        .execute(params![chat_id, message.id])?;
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

fn write_chat_ttls_diff(
    tx: &Transaction,
    ttls: &BTreeMap<String, u64>,
    diff: &MapDiff,
) -> anyhow::Result<()> {
    if diff.full_replace {
        tx.execute("DELETE FROM chat_message_ttls", [])?;
    } else if !diff.deletes.is_empty() {
        let mut del = tx.prepare_cached("DELETE FROM chat_message_ttls WHERE chat_id = ?1")?;
        for chat_id in &diff.deletes {
            del.execute([chat_id])?;
        }
    }
    if diff.upserts.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "INSERT INTO chat_message_ttls(chat_id, ttl_seconds) VALUES (?1, ?2)
         ON CONFLICT(chat_id) DO UPDATE SET ttl_seconds = excluded.ttl_seconds",
    )?;
    for (chat_id, _) in &diff.upserts {
        if let Some(ttl) = ttls.get(chat_id) {
            stmt.execute(params![chat_id, *ttl as i64])?;
        }
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

fn write_app_keys_diff(
    tx: &Transaction,
    app_keys: &BTreeMap<String, KnownAppKeys>,
    diff: &MapDiff,
) -> anyhow::Result<()> {
    if diff.full_replace {
        tx.execute("DELETE FROM app_keys", [])?;
    } else if !diff.deletes.is_empty() {
        let mut del = tx.prepare_cached("DELETE FROM app_keys WHERE owner_pubkey_hex = ?1")?;
        for owner_pubkey_hex in &diff.deletes {
            del.execute([owner_pubkey_hex])?;
        }
    }
    if diff.upserts.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "INSERT INTO app_keys(owner_pubkey_hex, created_at_secs, devices_json)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(owner_pubkey_hex) DO UPDATE SET
            created_at_secs = excluded.created_at_secs,
            devices_json = excluded.devices_json",
    )?;
    for (owner_pubkey_hex, _) in &diff.upserts {
        let Some(entry) = app_keys.get(owner_pubkey_hex) else {
            continue;
        };
        let devices_json = serde_json::to_string(&entry.devices)?;
        stmt.execute(params![
            entry.owner_pubkey_hex,
            entry.created_at_secs as i64,
            devices_json,
        ])?;
    }
    Ok(())
}

fn load_groups(conn: &rusqlite::Connection) -> anyhow::Result<Vec<GroupSnapshot>> {
    let mut stmt = conn.prepare("SELECT group_json FROM groups")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut groups = Vec::new();
    for row in rows {
        let json = row?;
        if let Ok(group) = serde_json::from_str::<GroupSnapshot>(&json) {
            groups.push(group);
        }
    }
    Ok(groups)
}

fn load_group_pictures(conn: &rusqlite::Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT group_id, picture FROM groups WHERE picture IS NOT NULL AND picture != ''",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut pictures = BTreeMap::new();
    for row in rows {
        let (group_id, picture) = row?;
        pictures.insert(group_id, picture);
    }
    Ok(pictures)
}

fn write_groups_diff(
    tx: &Transaction,
    groups: &BTreeMap<String, GroupSnapshot>,
    group_pictures: &BTreeMap<String, String>,
    diff: &MapDiff,
) -> anyhow::Result<()> {
    if diff.full_replace {
        tx.execute("DELETE FROM groups", [])?;
    } else if !diff.deletes.is_empty() {
        let mut del = tx.prepare_cached("DELETE FROM groups WHERE group_id = ?1")?;
        for group_id in &diff.deletes {
            del.execute([group_id])?;
        }
    }
    if diff.upserts.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare_cached(
        "INSERT INTO groups(group_id, name, picture, created_at_ms, updated_at_secs, group_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(group_id) DO UPDATE SET
            name = excluded.name,
            picture = excluded.picture,
            created_at_ms = excluded.created_at_ms,
            updated_at_secs = excluded.updated_at_secs,
            group_json = excluded.group_json",
    )?;
    for (group_id, _) in &diff.upserts {
        let Some(group) = groups.get(group_id) else {
            continue;
        };
        let group_json = serde_json::to_string(group)?;
        stmt.execute(params![
            group.group_id,
            group.name,
            group_pictures.get(&group.group_id),
            group.created_at.get() as i64 * 1000,
            group.updated_at.get() as i64,
            group_json,
        ])?;
    }
    Ok(())
}

/// Load one preview message per inactive thread plus the newest page
/// for the active thread. Use the per-chat recent-message index so
/// restart cost follows chat count instead of total stored messages.
fn load_threads(
    conn: &rusqlite::Connection,
    active_chat_id: Option<&str>,
) -> anyhow::Result<Vec<PersistedThread>> {
    let mut threads_stmt =
        conn.prepare("SELECT chat_id, unread_count, updated_at_secs, draft FROM threads")?;
    let thread_rows = threads_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut by_chat: HashMap<String, PersistedThread> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for row in thread_rows {
        let (chat_id, unread_count, updated_at_secs, draft) = row?;
        order.push(chat_id.clone());
        by_chat.insert(
            chat_id.clone(),
            PersistedThread {
                chat_id,
                unread_count,
                updated_at_secs,
                messages: Vec::new(),
                draft,
            },
        );
    }

    drop(threads_stmt);

    for chat_id in &order {
        let limit = if active_chat_id == Some(chat_id.as_str()) {
            RESTORED_MESSAGES_PER_THREAD
        } else {
            1
        };
        let messages = load_recent_messages(conn, chat_id, limit)?;
        if let Some(thread) = by_chat.get_mut(chat_id) {
            thread.messages = messages;
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|chat_id| by_chat.remove(&chat_id))
        .collect())
}

fn load_thread(
    conn: &rusqlite::Connection,
    chat_id: &str,
    limit: usize,
) -> anyhow::Result<Option<PersistedThread>> {
    let metadata = conn
        .query_row(
            "SELECT unread_count, updated_at_secs, draft FROM threads WHERE chat_id = ?1",
            [chat_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let messages = load_recent_messages(conn, chat_id, limit)?;
    let Some((unread_count, updated_at_secs, draft)) = metadata else {
        if messages.is_empty() {
            return Ok(None);
        }
        let updated_at_secs = messages
            .iter()
            .map(|message| message.created_at_secs)
            .max()
            .unwrap_or(0);
        let unread_count = messages
            .iter()
            .filter(|message| !message.is_outgoing)
            .count() as u64;
        return Ok(Some(PersistedThread {
            chat_id: chat_id.to_string(),
            unread_count,
            updated_at_secs,
            messages,
            draft: String::new(),
        }));
    };
    Ok(Some(PersistedThread {
        chat_id: chat_id.to_string(),
        unread_count,
        updated_at_secs,
        messages,
        draft,
    }))
}

#[derive(Clone, Debug)]
pub(crate) struct PersistedMessageSearchHit {
    pub chat_id: String,
    pub message_id: String,
    pub author: String,
    pub author_owner_pubkey_hex: Option<String>,
    pub author_picture_url: Option<String>,
    pub kind: ChatMessageKind,
    pub body: String,
    pub is_outgoing: bool,
    pub created_at_secs: u64,
}

pub(crate) fn search_messages_fts(
    conn: &rusqlite::Connection,
    query: &str,
    scope_chat_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<PersistedMessageSearchHit>> {
    let Some(fts_query) = build_fts5_query(query) else {
        return Ok(Vec::new());
    };
    if limit == 0 {
        return Ok(Vec::new());
    }
    let owner: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'account_owner_pubkey_hex'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let intervals = owner
        .as_deref()
        .map(|owner| super::blocked_message_intervals(conn, owner))
        .transpose()?
        .unwrap_or_else(|| "[]".into());
    let map_row = |row: &Row<'_>| -> rusqlite::Result<PersistedMessageSearchHit> {
        Ok(PersistedMessageSearchHit {
            chat_id: row.get(0)?,
            message_id: row.get(1)?,
            author: row.get(2)?,
            body: super::super::attachments::chat_message_body_preview(&row.get::<_, String>(3)?),
            is_outgoing: row.get::<_, i64>(4)? != 0,
            created_at_secs: row.get::<_, i64>(5)?.max(0) as u64,
            author_owner_pubkey_hex: row.get(6)?,
            author_picture_url: row.get(7)?,
            kind: parse_message_kind(&row.get::<_, String>(8)?),
        })
    };
    let mut stmt = conn.prepare_cached(
        "SELECT messages.chat_id, messages.id,
                COALESCE(NULLIF(TRIM(author.nickname), ''),
                         NULLIF(TRIM(json_extract(author.contact_memory_json, '$.accepted_name')), ''),
                         NULLIF(TRIM(author.display_name), ''),
                         NULLIF(TRIM(author.name), ''), messages.author), messages.body,
                messages.is_outgoing, messages.created_at_secs,
                messages.author_owner_pubkey_hex, author.picture, messages.kind
         FROM messages_fts
         JOIN messages ON messages.rowid = messages_fts.rowid
         LEFT JOIN owner_profiles AS author
            ON author.owner_pubkey_hex = messages.author_owner_pubkey_hex
         WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR messages.chat_id = ?2)
         AND NOT EXISTS (
             SELECT 1 FROM preferences AS prefs,
                 json_each(prefs.blocked_owner_pubkeys_json) AS blocked
             WHERE prefs.id = 1 AND prefs.hide_blocked_group_messages != 0
               AND messages.chat_id LIKE 'group:%' AND messages.kind = 'user'
               AND messages.is_outgoing = 0
               AND blocked.value = messages.author_owner_pubkey_hex
         )
         AND NOT EXISTS (
             SELECT 1 FROM json_each(?4) AS blocked
             WHERE messages.chat_id LIKE 'group:%' AND messages.kind = 'user'
               AND messages.is_outgoing = 0
               AND json_extract(blocked.value, '$.author') = messages.author_owner_pubkey_hex
               AND messages.created_at_secs >= json_extract(blocked.value, '$.since')
               AND (json_extract(blocked.value, '$.until') IS NULL
                    OR messages.created_at_secs < json_extract(blocked.value, '$.until'))
         )
         ORDER BY messages.created_at_secs DESC, messages.rowid DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![&fts_query, scope_chat_id, limit as i64, &intervals],
        map_row,
    )?;
    let mut hits = Vec::new();
    for row in rows {
        hits.push(row?);
    }
    Ok(hits)
}

/// Translate a free-text user query into a safe FTS5 MATCH expression.
/// Each whitespace-separated token becomes a prefix-matched quoted
/// phrase so partial typing matches early ("hel" finds "hello"), and
/// punctuation/operator characters are folded into whitespace so a
/// query like `:" OR DROP TABLE` can never escape the quoted phrase.
fn build_fts5_query(input: &str) -> Option<String> {
    let cleaned: String = input
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|t| format!("\"{t}\"*"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}
fn persisted_message_from_row(row: &Row<'_>) -> rusqlite::Result<PersistedMessage> {
    let chat_id: String = row.get(0)?;
    Ok(PersistedMessage {
        edit_history: serde_json::from_str(&row.get::<_, String>(18)?).unwrap_or_default(),
        deleted_for_everyone: row.get::<_, i64>(19)? != 0,
        system_notice_owner_pubkey_hex: row.get(17)?,
        call: row
            .get::<_, Option<String>>(16)?
            .and_then(|json| serde_json::from_str(&json).ok()),
        id: row.get(1)?,
        chat_id,
        kind: parse_message_kind(&row.get::<_, String>(2)?),
        author: row.get(3)?,
        author_owner_pubkey_hex: row.get(4)?,
        body: row.get(5)?,
        attachments: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
        reactions: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
        reactors: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
        is_outgoing: row.get::<_, i64>(6)? != 0,
        created_at_secs: row.get::<_, i64>(7)? as u64,
        expires_at_secs: row.get::<_, Option<i64>>(8)?.map(|secs| secs as u64),
        delivery: parse_delivery(&row.get::<_, String>(9)?),
        source_event_id: row.get(13)?,
        recipient_deliveries: serde_json::from_str::<Vec<MessageRecipientDeliverySnapshot>>(
            &row.get::<_, String>(14)?,
        )
        .unwrap_or_default(),
        delivery_trace: serde_json::from_str::<MessageDeliveryTraceSnapshot>(
            &row.get::<_, String>(15)?,
        )
        .unwrap_or_default(),
    })
}

/// Returns the persisted event ids (in insertion order) and the highest
/// sequence number seen, used by the cache to assign monotonically growing
/// sequences to new inserts.
fn load_seen_events(conn: &rusqlite::Connection) -> anyhow::Result<(Vec<String>, i64)> {
    let mut stmt =
        conn.prepare("SELECT event_id, sequence FROM seen_events ORDER BY sequence ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut events = Vec::new();
    let mut max_seq: i64 = -1;
    for row in rows {
        let (event_id, sequence) = row?;
        if sequence > max_seq {
            max_seq = sequence;
        }
        events.push(event_id);
    }
    Ok((events, max_seq))
}

fn apply_seen_events_diff(tx: &Transaction, plan: &SeenEventsPlan) -> anyhow::Result<()> {
    if !plan.to_delete.is_empty() {
        let mut del_stmt = tx.prepare_cached("DELETE FROM seen_events WHERE event_id = ?1")?;
        for event_id in &plan.to_delete {
            del_stmt.execute([event_id])?;
        }
    }
    if !plan.to_insert.is_empty() {
        // ON CONFLICT DO NOTHING handles the case where a previous run
        // left rows we don't have cached — e.g. cache was reset on a
        // fresh AppStore. We never want a primary-key violation here.
        let mut ins_stmt = tx.prepare_cached(
            "INSERT INTO seen_events(event_id, sequence)
             VALUES (?1, ?2)
             ON CONFLICT(event_id) DO NOTHING",
        )?;
        for (seq, event_id) in (plan.first_insert_seq..).zip(plan.to_insert.iter()) {
            ins_stmt.execute(params![event_id, seq])?;
        }
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
#[path = "search_message_tests.rs"]
mod search_message_tests;

#[cfg(test)]
mod tests {
    use super::super::open_database;
    use super::*;
    use crate::core::{DiscoveredUserRecord, UserDiscoveryCache};
    use crate::state::{
        ChatMessageSnapshot, MessageAttachmentSnapshot, MessageReactionSnapshot, MessageReactor,
    };

    fn fresh_store() -> (tempfile::TempDir, AppStore) {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(tmp.path()).unwrap();
        (tmp, AppStore::new(conn))
    }

    #[allow(clippy::too_many_arguments)]
    fn empty_snapshot<'a>(
        active_chat_id: Option<&'a str>,
        next_message_id: u64,
        preferences: &'a PreferencesSnapshot,
        owner_profiles: &'a BTreeMap<String, OwnerProfileRecord>,
        chat_ttls: &'a BTreeMap<String, u64>,
        app_keys: &'a BTreeMap<String, KnownAppKeys>,
        groups: &'a BTreeMap<String, GroupSnapshot>,
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
            group_pictures: Box::leak(Box::new(BTreeMap::new())),
            threads,
            seen_event_order: seen_events,
        }
    }

    fn sample_message(id: &str, body: &str, ts: u64) -> ChatMessageSnapshot {
        ChatMessageSnapshot {
            edit_history: Vec::new(),
            deleted_for_everyone: false,
            system_notice_owner_pubkey_hex: None,
            direct_transfer: None,
            call: None,
            id: id.to_string(),
            chat_id: "chat".to_string(),
            kind: ChatMessageKind::User,
            author: "alice".to_string(),
            author_owner_pubkey_hex: None,
            author_picture_url: None,
            body: body.to_string(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            reactors: Vec::new(),
            is_outgoing: false,
            created_at_secs: ts,
            expires_at_secs: None,
            delivery: DeliveryState::Received,
            recipient_deliveries: Vec::new(),
            delivery_trace: Default::default(),
            source_event_id: None,
        }
    }

    fn sample_expiring_message(
        chat_id: &str,
        id: &str,
        body: &str,
        ts: u64,
        expires_at_secs: Option<u64>,
    ) -> ChatMessageSnapshot {
        let mut message = sample_message(id, body, ts);
        message.chat_id = chat_id.to_string();
        message.expires_at_secs = expires_at_secs;
        message
    }

    fn thread_from_messages(chat_id: &str, messages: Vec<ChatMessageSnapshot>) -> ThreadRecord {
        ThreadRecord {
            chat_id: chat_id.to_string(),
            unread_count: 0,
            updated_at_secs: messages
                .last()
                .map(|message| message.created_at_secs)
                .unwrap_or(0),
            messages,
            draft: String::new(),
        }
    }

    fn count(conn: &SharedConnection, table: &str) -> i64 {
        conn.lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn empty_database_returns_none() {
        let (_tmp, mut store) = fresh_store();
        assert!(store.load_state().unwrap().is_none());
    }

    /// Regression for iOS RUNNINGBOARD 0xdead10cc crashes during relay
    /// event processing: each save used to `DELETE FROM seen_events` and
    /// re-INSERT every entry. Verify we now only touch the actual diff:
    /// a single new event id gets exactly one new row and an evicted id
    /// is the only deletion.
    #[test]
    fn seen_events_writes_are_incremental() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let threads = BTreeMap::new();

        let mut window: VecDeque<String> = VecDeque::new();
        window.push_back("evt-a".to_string());
        window.push_back("evt-b".to_string());
        let snapshot = empty_snapshot(
            None,
            1,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &window,
        );
        store.save_state(&snapshot).unwrap();

        let conn = store.shared();
        let read_rows = || -> Vec<(String, i64)> {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT event_id, sequence FROM seen_events ORDER BY sequence ASC")
                .unwrap();
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        let rows = read_rows();
        assert_eq!(rows, vec![("evt-a".into(), 0_i64), ("evt-b".into(), 1_i64)]);

        // Append one new event id; the previous rows must keep their
        // sequences (no full rewrite) and the new row must get sequence 2.
        window.push_back("evt-c".to_string());
        let snapshot = empty_snapshot(
            None,
            1,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &window,
        );
        store.save_state(&snapshot).unwrap();
        let rows = read_rows();
        assert_eq!(
            rows,
            vec![
                ("evt-a".into(), 0_i64),
                ("evt-b".into(), 1_i64),
                ("evt-c".into(), 2_i64),
            ]
        );

        // Evict the oldest and append another. The middle row keeps its
        // sequence; only the head row is deleted and the tail row inserted.
        window.pop_front();
        window.push_back("evt-d".to_string());
        let snapshot = empty_snapshot(
            None,
            1,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &window,
        );
        store.save_state(&snapshot).unwrap();
        let rows = read_rows();
        assert_eq!(
            rows,
            vec![
                ("evt-b".into(), 1_i64),
                ("evt-c".into(), 2_i64),
                ("evt-d".into(), 3_i64),
            ]
        );

        // Re-saving the same window must be a no-op (no rewrite churn).
        store.save_state(&snapshot).unwrap();
        assert_eq!(read_rows().len(), 3);
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

                draft: String::new(),
            },
        );
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let group_pictures = BTreeMap::new();
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
            group_pictures: &group_pictures,
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
    fn message_exists_finds_stored_id_and_source_event() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let mut message = sample_message("m1", "hi", 99);
        message.source_event_id = Some("outer-1".to_string());
        let mut threads = BTreeMap::new();
        threads.insert(
            "chat".to_string(),
            ThreadRecord {
                chat_id: "chat".to_string(),
                unread_count: 0,
                updated_at_secs: 99,
                messages: vec![message],

                draft: String::new(),
            },
        );
        let snapshot = empty_snapshot(
            None,
            2,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        assert!(store.message_exists("chat", Some("m1"), None).unwrap());
        assert!(store.message_exists("chat", None, Some("outer-1")).unwrap());
        assert!(!store.message_exists("chat", Some("m2"), None).unwrap());
    }

    #[test]
    fn load_state_restores_newest_message_page_per_thread() {
        let (tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = (1..=RESTORED_MESSAGES_PER_THREAD + 10)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            Some("chat"),
            100,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        drop(store);
        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        let loaded = store.load_state().unwrap().expect("state present");
        let loaded_messages = &loaded.threads[0].messages;
        assert_eq!(loaded_messages.len(), RESTORED_MESSAGES_PER_THREAD);
        assert_eq!(loaded_messages.first().unwrap().body, "message 11");
        assert_eq!(loaded_messages.last().unwrap().body, "message 90");
    }

    #[test]
    fn load_state_restores_only_latest_message_for_inactive_threads() {
        let (tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = (1..=RESTORED_MESSAGES_PER_THREAD + 10)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            None,
            100,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        drop(store);
        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        let loaded = store.load_state().unwrap().expect("state present");
        let loaded_messages = &loaded.threads[0].messages;
        assert_eq!(loaded_messages.len(), 1);
        assert_eq!(loaded_messages[0].body, "message 90");

        let page = store
            .load_recent_messages("chat", RESTORED_MESSAGES_PER_THREAD)
            .unwrap();
        assert_eq!(page.len(), RESTORED_MESSAGES_PER_THREAD);
        assert_eq!(page.first().unwrap().body, "message 11");
        assert_eq!(page.last().unwrap().body, "message 90");
    }

    #[test]
    fn load_state_uses_insert_order_for_same_second_message_preview() {
        let (tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = vec![
            sample_message("z-local-random-id", "queued offline", 1_777_159_000),
            sample_message("a-local-random-id", "short lived", 1_777_159_000),
        ];
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            None,
            3,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        drop(store);
        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        let loaded = store.load_state().unwrap().expect("state present");
        let loaded_messages = &loaded.threads[0].messages;
        assert_eq!(loaded_messages.len(), 1);
        assert_eq!(loaded_messages[0].body, "short lived");
    }

    #[test]
    fn load_recent_messages_uses_recent_index_for_large_chat() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = (1..=20_000)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            None,
            20_001,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        let started = std::time::Instant::now();
        let page = store
            .load_recent_messages("chat", RESTORED_MESSAGES_PER_THREAD)
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(page.len(), RESTORED_MESSAGES_PER_THREAD);
        assert_eq!(page.first().unwrap().body, "message 19921");
        assert_eq!(page.last().unwrap().body, "message 20000");
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "large chat recent-message page took {elapsed:?}",
        );
    }

    #[test]
    fn load_messages_before_returns_older_page_for_large_chat() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = (1..=200)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            None,
            201,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        let page = store.load_messages_before("chat", "121", 40).unwrap();

        assert_eq!(page.len(), 40);
        assert_eq!(page.first().unwrap().body, "message 81");
        assert_eq!(page.last().unwrap().body, "message 120");
        assert!(!page.iter().any(|message| message.id == "121"));
    }

    #[test]
    fn load_messages_around_returns_search_hit_context_outside_recent_page() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let messages = (1..=200)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            None,
            201,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        let recent = store.load_recent_messages("chat", 80).unwrap();
        assert_eq!(recent.first().unwrap().body, "message 121");
        assert!(!recent.iter().any(|message| message.id == "25"));

        let page = store.load_messages_around("chat", "25", 10, 10).unwrap();

        assert_eq!(page.len(), 21);
        assert_eq!(page.first().unwrap().body, "message 15");
        assert_eq!(page.last().unwrap().body, "message 35");
        assert!(page.iter().any(|message| message.id == "25"));
    }

    #[test]
    fn saving_partially_loaded_thread_preserves_older_message_rows() {
        let (tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();
        let total = RESTORED_MESSAGES_PER_THREAD + 10;
        let messages = (1..=total)
            .map(|idx| sample_message(&idx.to_string(), &format!("message {idx}"), idx as u64))
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert("chat".to_string(), thread_from_messages("chat", messages));
        let snapshot = empty_snapshot(
            Some("chat"),
            100,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();
        drop(store);

        let conn = open_database(tmp.path()).unwrap();
        let mut store = AppStore::new(conn);
        let conn_handle = store.shared();
        let loaded = store.load_state().unwrap().expect("state present");
        let loaded_messages = loaded.threads[0]
            .messages
            .iter()
            .map(|message| {
                let mut snapshot =
                    sample_message(&message.id, &message.body, message.created_at_secs);
                snapshot.delivery = message.delivery.clone().into();
                snapshot
            })
            .collect::<Vec<_>>();
        let mut threads = BTreeMap::new();
        threads.insert(
            "chat".to_string(),
            thread_from_messages("chat", loaded_messages),
        );
        let snapshot = empty_snapshot(
            Some("chat"),
            loaded.next_message_id,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        assert_eq!(count(&conn_handle, "messages"), total as i64);
        store.delete_message("chat", "1").unwrap();
        assert_eq!(count(&conn_handle, "messages"), total as i64 - 1);
    }

    #[test]
    fn notification_preview_upsert_preserves_existing_message_decorations() {
        let (_tmp, mut store) = fresh_store();
        let preferences = PreferencesSnapshot::default();
        let owner_profiles = BTreeMap::new();
        let chat_ttls = BTreeMap::new();
        let app_keys = BTreeMap::new();
        let groups = BTreeMap::new();
        let seen_events = VecDeque::new();

        let mut existing = sample_message("m1", "original", 10);
        existing.delivery = DeliveryState::Seen;
        existing.attachments.push(MessageAttachmentSnapshot {
            uploaded_urls: Vec::new(),
            nhash: "nhash1abc".to_string(),
            filename: "photo.jpg".to_string(),
            filename_encoded: "photo.jpg".to_string(),
            htree_url: "htree://example".to_string(),
            is_image: true,
            is_video: false,
            is_audio: false,
        });
        existing.reactions.push(MessageReactionSnapshot {
            emoji: "+1".to_string(),
            count: 2,
            reacted_by_me: true,
        });
        existing.reactors.push(MessageReactor {
            author: "alice".to_string(),
            display_name: String::new(),
            picture_url: None,
            emoji: "+1".to_string(),
        });
        let mut threads = BTreeMap::new();
        threads.insert(
            "chat".to_string(),
            ThreadRecord {
                chat_id: "chat".to_string(),
                unread_count: 7,
                updated_at_secs: 10,
                messages: vec![existing],

                draft: String::new(),
            },
        );
        let snapshot = empty_snapshot(
            Some("chat"),
            100,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        let mut duplicate_preview = sample_message("m1", "replacement", 99);
        duplicate_preview.author = "mallory".to_string();
        duplicate_preview.delivery = DeliveryState::Received;
        duplicate_preview.source_event_id = Some("outer-event-id".to_string());
        store
            .upsert_notification_preview_message("chat", 99, 99, &duplicate_preview)
            .unwrap();

        let loaded = store.load_state().unwrap().expect("state present");
        assert_eq!(loaded.threads[0].unread_count, 7);
        assert_eq!(loaded.threads[0].updated_at_secs, 10);
        let message = &loaded.threads[0].messages[0];
        assert_eq!(message.body, "original");
        assert_eq!(message.author, "alice");
        assert!(matches!(message.delivery, PersistedDeliveryState::Seen));
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.reactions.len(), 1);
        assert_eq!(message.reactors.len(), 1);
        assert_eq!(message.source_event_id.as_deref(), Some("outer-event-id"));
    }

    #[test]
    fn delete_expired_messages_removes_rows_across_all_threads() {
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
            thread_from_messages(
                "chat-a",
                vec![
                    sample_expiring_message("chat-a", "old-a", "gone", 1, Some(10)),
                    sample_expiring_message("chat-a", "keep-a", "stays", 2, Some(200)),
                ],
            ),
        );
        threads.insert(
            "chat-b".to_string(),
            thread_from_messages(
                "chat-b",
                vec![
                    sample_expiring_message("chat-b", "old-b", "gone too", 3, Some(99)),
                    sample_expiring_message("chat-b", "keep-b", "plain", 4, None),
                ],
            ),
        );
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
        assert_eq!(count(&conn_handle, "messages"), 4);
        assert_eq!(store.next_message_expiration_after(0).unwrap(), Some(10));
        assert_eq!(store.next_message_expiration_after(100).unwrap(), Some(200));

        let deleted = store.delete_expired_messages(100).unwrap();

        assert_eq!(deleted, 2);
        assert_eq!(count(&conn_handle, "messages"), 2);
        assert_eq!(store.next_message_expiration_after(100).unwrap(), Some(200));
        assert_eq!(store.next_message_expiration_after(200).unwrap(), None);
        let loaded = store.load_state().unwrap().expect("state present");
        let mut loaded_bodies = loaded
            .threads
            .iter()
            .flat_map(|thread| thread.messages.iter().map(|message| message.body.as_str()))
            .collect::<Vec<_>>();
        loaded_bodies.sort_unstable();
        assert_eq!(loaded_bodies, vec!["plain", "stays"]);
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

                draft: String::new(),
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

                draft: String::new(),
            },
        );
        threads.insert(
            "chat-b".to_string(),
            ThreadRecord {
                chat_id: "chat-b".to_string(),
                unread_count: 0,
                updated_at_secs: 2,
                messages: vec![sample_message("m2", "world", 2)],

                draft: String::new(),
            },
        );

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

        // Change only chat-a; chat-b unchanged.
        threads.get_mut("chat-a").unwrap().messages[0].body = "edited".to_string();
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

                draft: String::new(),
            },
        );
        threads.insert(
            "chat-b".to_string(),
            ThreadRecord {
                chat_id: "chat-b".to_string(),
                unread_count: 0,
                updated_at_secs: 2,
                messages: vec![sample_message("m2", "go", 2)],

                draft: String::new(),
            },
        );

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
        assert_eq!(count(&conn_handle, "threads"), 2);
        assert_eq!(count(&conn_handle, "messages"), 2);

        threads.remove("chat-b");
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
        assert_eq!(count(&conn_handle, "threads"), 1);
        assert_eq!(count(&conn_handle, "messages"), 1);
    }

    #[test]
    fn search_messages_fts_returns_grouped_and_scoped_results() {
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
                updated_at_secs: 200,
                messages: vec![
                    sample_message_for_chat("chat-a", "m1", "Hello there, world", 100),
                    sample_message_for_chat("chat-a", "m2", "good morning sunshine", 200),
                ],

                draft: String::new(),
            },
        );
        threads.insert(
            "chat-b".to_string(),
            ThreadRecord {
                chat_id: "chat-b".to_string(),
                unread_count: 0,
                updated_at_secs: 300,
                messages: vec![sample_message_for_chat(
                    "chat-b",
                    "m3",
                    "Hello again from b",
                    300,
                )],

                draft: String::new(),
            },
        );
        let snapshot = empty_snapshot(
            None,
            10,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();

        // Unscoped search matches both chats, newest first.
        let hits = store.search_messages_fts("hello", None, 10).unwrap();
        let ordered: Vec<(&str, &str)> = hits
            .iter()
            .map(|h| (h.chat_id.as_str(), h.message_id.as_str()))
            .collect();
        assert_eq!(ordered, vec![("chat-b", "m3"), ("chat-a", "m1")]);

        // Scoped search only returns hits inside the scope chat.
        let hits = store
            .search_messages_fts("hello", Some("chat-a"), 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chat_id, "chat-a");
        assert_eq!(hits[0].message_id, "m1");

        // Empty / whitespace query returns nothing instead of every row.
        assert!(store
            .search_messages_fts("   ", None, 10)
            .unwrap()
            .is_empty());

        // Prefix matching (Signal-style live search): "hel" finds "Hello".
        let hits = store.search_messages_fts("hel", None, 10).unwrap();
        assert_eq!(hits.len(), 2);

        // Punctuation in the query must not break out of the quoted phrase.
        let hits = store.search_messages_fts("hello\"", None, 10).unwrap();
        assert_eq!(hits.len(), 2);

        // Delete propagates through the trigger.
        store.delete_message("chat-b", "m3").unwrap();
        let hits = store.search_messages_fts("hello", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chat_id, "chat-a");
    }

    fn sample_message_for_chat(
        chat_id: &str,
        id: &str,
        body: &str,
        ts: u64,
    ) -> ChatMessageSnapshot {
        let mut msg = sample_message(id, body, ts);
        msg.chat_id = chat_id.to_string();
        msg
    }

    #[test]
    fn user_discovery_cache_replaces_atomically_and_restores_offline() {
        let (_tmp, mut store) = fresh_store();
        assert_eq!(
            store.load_user_discovery().unwrap(),
            UserDiscoveryCache::default()
        );
        let first_user = DiscoveredUserRecord {
            owner_pubkey_hex: "owner-a".to_string(),
            follow_position: 0,
            petname: Some("Alice".to_string()),
        };
        let second_user = DiscoveredUserRecord {
            owner_pubkey_hex: "owner-b".to_string(),
            follow_position: 1,
            petname: None,
        };
        let initial = UserDiscoveryCache {
            follow_event_json: Some(r#"{"id":"follow-1","tags":[["p","owner-a"]]}"#.into()),
            owner_pubkey_hex: Some("root-a".to_string()),
            follow_event_id: Some("follow-1".to_string()),
            follow_created_at_secs: 10,
            users: BTreeMap::from([
                (first_user.owner_pubkey_hex.clone(), first_user),
                (second_user.owner_pubkey_hex.clone(), second_user.clone()),
            ]),
            social_rank_ready: true,
            social_friend_support: BTreeMap::from([("global-a".to_string(), 2)]),
            social_graph: Some(vec![0, 1, 127, 255]),
            social_opinion_events: vec![
                "public-follow-head".to_owned(),
                "public-mute-head".to_owned(),
            ],
        };
        store.replace_user_discovery(&initial).unwrap();
        assert_eq!(store.load_user_discovery().unwrap(), initial);

        let replacement = UserDiscoveryCache {
            follow_event_json: None,
            owner_pubkey_hex: Some("root-b".to_string()),
            follow_event_id: Some("follow-2".to_string()),
            follow_created_at_secs: 11,
            users: BTreeMap::from([(second_user.owner_pubkey_hex.clone(), second_user)]),
            social_rank_ready: true,
            social_friend_support: BTreeMap::from([("global-b".to_string(), 1)]),
            social_graph: None,
            social_opinion_events: Vec::new(),
        };
        store.replace_user_discovery(&replacement).unwrap();
        assert_eq!(store.load_user_discovery().unwrap(), replacement);
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
            None,
            7,
            &preferences,
            &owner_profiles,
            &chat_ttls,
            &app_keys,
            &groups,
            &threads,
            &seen_events,
        );
        store.save_state(&snapshot).unwrap();
        assert!(store.load_state().unwrap().is_some());
        store.clear().unwrap();
        assert!(store.load_state().unwrap().is_none());
        assert_eq!(
            store.load_user_discovery().unwrap(),
            UserDiscoveryCache::default()
        );

        // After clear the cache is empty, so the same snapshot becomes
        // a real write again rather than a no-op.
        let plan = SavePlan::compute(&store.cache, &snapshot);
        assert!(!plan.is_empty(), "cache must be reset on clear");
    }
}

#[cfg(test)]
mod suspension_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn suspend_does_not_start_database_analysis() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE suspend_probe(id INTEGER, body TEXT);
             CREATE INDEX suspend_probe_id ON suspend_probe(id);
             WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<1000)
             INSERT INTO suspend_probe SELECT n, 'message' FROM numbers;",
        )
        .unwrap();
        // Querying an unanalyzed index makes PRAGMA optimize run ANALYZE.
        let _: i64 = conn
            .query_row("SELECT count(*) FROM suspend_probe WHERE id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let db = Arc::new(Mutex::new(conn));
        let mut store = AppStore::new(db.clone());
        store.prepare_for_suspend().unwrap();
        let analyzed: bool = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sqlite_stat1')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !analyzed,
            "suspension must not start optional SQLite analysis writes"
        );
    }
}
