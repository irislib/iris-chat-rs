use std::path::Path;
use std::sync::{Arc, Mutex};

use nostr::{Event, Filter, Keys, PublicKey, UnsignedEvent};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    is_app_keys_event, AppKeys, ProtocolDecryptedMessage, ProtocolEffect, ProtocolEngine,
    ProtocolRetryBatch, SharedConnection, SqliteStorageAdapter, UnixSeconds, APP_KEYS_EVENT_KIND,
    CHAT_MESSAGE_KIND, INVITE_EVENT_KIND, INVITE_RESPONSE_KIND, MESSAGE_EVENT_KIND,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS private_chat_threads (
    chat_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    avatar_seed TEXT NOT NULL,
    updated_at_secs INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS private_chat_messages (
    chat_id TEXT NOT NULL,
    id TEXT NOT NULL,
    body TEXT NOT NULL,
    is_outgoing INTEGER NOT NULL,
    created_at_secs INTEGER NOT NULL,
    delivery TEXT NOT NULL,
    source_event_id TEXT,
    PRIMARY KEY (chat_id, id)
);

CREATE INDEX IF NOT EXISTS private_chat_recent_idx
    ON private_chat_messages(chat_id, created_at_secs, id);

CREATE UNIQUE INDEX IF NOT EXISTS private_chat_source_event_idx
    ON private_chat_messages(source_event_id)
    WHERE source_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS private_chat_seen_events (
    event_id TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS ndr_kv (
    owner_pubkey_hex TEXT NOT NULL,
    device_pubkey_hex TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex, key)
);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectMessageDelivery {
    Pending,
    Sent,
    Received,
    Failed,
}

impl DirectMessageDelivery {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Received => "received",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "sent" => Self::Sent,
            "received" => Self::Received,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectMessageSnapshot {
    pub id: String,
    pub chat_id: String,
    pub body: String,
    pub is_outgoing: bool,
    pub created_at_secs: u64,
    pub delivery: DirectMessageDelivery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectChatSnapshot {
    pub chat_id: String,
    pub last_message_preview: String,
    pub last_message_at: u64,
    pub unread_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectThreadSnapshot {
    pub chat: DirectChatSnapshot,
    pub messages: Vec<DirectMessageSnapshot>,
}

#[derive(Clone, Debug)]
pub enum DirectMessageCommand {
    Publish(Event),
    Subscribe {
        subscription_id: String,
        filters: Vec<Filter>,
        durable: bool,
    },
}

pub struct DirectMessageService {
    conn: SharedConnection,
    protocol_engine: Option<ProtocolEngine>,
    owner_public_key: Option<PublicKey>,
    relay_subscription_key: Option<String>,
    fetch_subscription_counter: u64,
    last_error: Option<String>,
}

impl DirectMessageService {
    pub fn memory() -> Self {
        let service = Self {
            conn: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
            protocol_engine: None,
            owner_public_key: None,
            relay_subscription_key: None,
            fetch_subscription_counter: 0,
            last_error: None,
        };
        service.ensure_schema();
        service
    }

    pub fn open(data_dir: &Path, owner_keys: Option<&Keys>) -> Self {
        let path = data_dir.join("private-chat.sqlite3");
        let conn = Connection::open(path).or_else(|_| Connection::open_in_memory());
        let conn = match conn {
            Ok(conn) => conn,
            Err(error) => {
                return Self {
                    conn: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
                    protocol_engine: None,
                    owner_public_key: None,
                    relay_subscription_key: None,
                    fetch_subscription_counter: 0,
                    last_error: Some(format!("Direct message store open failed: {error}")),
                };
            }
        };
        let service = Self {
            conn: Arc::new(Mutex::new(conn)),
            protocol_engine: None,
            owner_public_key: None,
            relay_subscription_key: None,
            fetch_subscription_counter: 0,
            last_error: None,
        };
        service.ensure_schema();
        if let Some(keys) = owner_keys {
            service.with_protocol_engine(keys)
        } else {
            service
        }
    }

    pub fn activate(&mut self, keys: &Keys) -> Vec<DirectMessageCommand> {
        let next = Self {
            conn: Arc::clone(&self.conn),
            protocol_engine: None,
            owner_public_key: None,
            relay_subscription_key: self.relay_subscription_key.clone(),
            fetch_subscription_counter: self.fetch_subscription_counter,
            last_error: self.last_error.clone(),
        }
        .with_protocol_engine(keys);
        self.protocol_engine = next.protocol_engine;
        self.owner_public_key = next.owner_public_key;
        self.protocol_subscription_commands()
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.clone()
    }

    pub fn chats(&self) -> Vec<DirectChatSnapshot> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT t.chat_id,
                    COALESCE(m.body, ''), COALESCE(m.created_at_secs, t.updated_at_secs)
             FROM private_chat_threads t
             LEFT JOIN private_chat_messages m
               ON m.chat_id = t.chat_id
              AND m.created_at_secs = (
                    SELECT MAX(created_at_secs)
                    FROM private_chat_messages
                    WHERE chat_id = t.chat_id
              )
             ORDER BY COALESCE(m.created_at_secs, t.updated_at_secs) DESC, t.chat_id ASC",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map([], |row| {
            Ok(DirectChatSnapshot {
                chat_id: row.get(0)?,
                last_message_preview: row.get(1)?,
                last_message_at: row.get::<_, i64>(2)?.max(0) as u64,
                unread_count: 0,
            })
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(Result::ok).collect()
    }

    pub fn thread(&self, chat_id: &str) -> Option<DirectThreadSnapshot> {
        let chat_id = normalize_pubkey(chat_id).ok()?;
        let chat = self
            .chats()
            .into_iter()
            .find(|chat| chat.chat_id == chat_id)
            .unwrap_or_else(|| chat_snapshot_for_pubkey(&chat_id));
        let messages = self.messages(&chat_id, 160);
        Some(DirectThreadSnapshot { chat, messages })
    }

    pub fn open_chat(
        &mut self,
        peer_input: &str,
        _keys: &Keys,
    ) -> Result<(DirectThreadSnapshot, Vec<DirectMessageCommand>), String> {
        let public_key = PublicKey::parse(peer_input).map_err(|error| error.to_string())?;
        let chat_id = public_key.to_hex();
        self.ensure_thread(&chat_id, unix_now());
        let commands = self.protocol_subscription_commands();
        let thread = self
            .thread(&chat_id)
            .ok_or_else(|| "Chat open failed".to_string())?;
        Ok((thread, commands))
    }

    pub fn send_message(
        &mut self,
        chat_id: &str,
        body: &str,
        _keys: &Keys,
    ) -> Result<Vec<DirectMessageCommand>, String> {
        let body = body.trim();
        if body.is_empty() {
            return Ok(Vec::new());
        }
        let public_key = PublicKey::parse(chat_id).map_err(|error| error.to_string())?;
        let chat_id = public_key.to_hex();
        self.ensure_thread(&chat_id, unix_now());
        let engine = self
            .protocol_engine
            .as_mut()
            .ok_or_else(|| "Direct message runtime is not ready".to_string())?;
        let result = engine
            .send_direct_text(public_key, &chat_id, body, None, UnixSeconds(unix_now()))
            .map_err(|error| error.to_string())?;
        let delivery = if result.event_ids.is_empty() {
            DirectMessageDelivery::Pending
        } else {
            DirectMessageDelivery::Sent
        };
        self.insert_message(
            &chat_id,
            &result.message_id,
            body,
            true,
            unix_now(),
            delivery,
            None,
        );
        Ok(self.commands_from_effects(result.effects))
    }

    pub fn process_event(&mut self, event: Event, _keys: &Keys) -> Vec<DirectMessageCommand> {
        let event_id = event.id.to_hex();
        if self.seen_event(&event_id) {
            return Vec::new();
        }
        let Some(engine) = self.protocol_engine.as_mut() else {
            return Vec::new();
        };
        let kind = event.kind.as_u16() as u32;
        let mut effects = Vec::new();
        let mut retry_batch = ProtocolRetryBatch::default();
        let mut decrypted = None;

        let processed = match kind {
            APP_KEYS_EVENT_KIND if is_app_keys_event(&event) => match AppKeys::from_event(&event) {
                Ok(app_keys) => match engine.ingest_app_keys_snapshot(
                    event.pubkey,
                    app_keys,
                    event.created_at.as_secs(),
                ) {
                    Ok(batch) => {
                        retry_batch = batch;
                        true
                    }
                    Err(error) => {
                        self.last_error = Some(format!("Direct message app keys failed: {error}"));
                        false
                    }
                },
                Err(_) => false,
            },
            INVITE_EVENT_KIND => match engine.observe_invite_event(&event) {
                Ok(batch) => {
                    retry_batch = batch;
                    true
                }
                Err(_) => false,
            },
            INVITE_RESPONSE_KIND => match engine.observe_invite_response_event(&event) {
                Ok(batch) => {
                    retry_batch = batch;
                    true
                }
                Err(_) => false,
            },
            MESSAGE_EVENT_KIND => match engine.process_direct_message_event(&event) {
                Ok(message) => {
                    decrypted = message;
                    true
                }
                Err(_) => false,
            },
            _ => false,
        };

        if !processed {
            return Vec::new();
        }
        self.mark_seen_event(&event_id);
        if let Some(message) = decrypted {
            self.apply_decrypted_protocol_message(message);
        }
        effects.extend(self.effects_from_retry_batch(retry_batch));
        self.commands_from_effects(effects)
    }

    pub fn mobile_push_message_author_pubkeys(&self) -> Vec<String> {
        let Some(engine) = self.protocol_engine.as_ref() else {
            return Vec::new();
        };
        let mut authors = engine
            .known_message_author_pubkeys()
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect::<Vec<_>>();
        authors.sort();
        authors.dedup();
        authors
    }

    fn subscription_command(&mut self) -> Option<DirectMessageCommand> {
        let engine = self.protocol_engine.as_ref()?;
        let authors = engine
            .known_message_author_pubkeys()
            .into_iter()
            .chain(self.owner_public_key)
            .collect::<Vec<_>>();
        let mut author_hexes = authors.iter().map(PublicKey::to_hex).collect::<Vec<_>>();
        author_hexes.sort();
        author_hexes.dedup();
        let key = author_hexes.join(",");
        if key.is_empty() || self.relay_subscription_key.as_deref() == Some(key.as_str()) {
            return None;
        }
        self.relay_subscription_key = Some(key);

        let public_keys = author_hexes
            .iter()
            .filter_map(|hex| PublicKey::parse(hex).ok())
            .collect::<Vec<_>>();
        let filter = Filter::new()
            .authors(public_keys)
            .kinds([
                nostr::Kind::from(MESSAGE_EVENT_KIND as u16),
                nostr::Kind::from(INVITE_EVENT_KIND as u16),
                nostr::Kind::from(INVITE_RESPONSE_KIND as u16),
                nostr::Kind::from(APP_KEYS_EVENT_KIND as u16),
            ])
            .limit(500);
        Some(DirectMessageCommand::Subscribe {
            subscription_id: "iris-native-private-chat".to_string(),
            filters: vec![filter],
            durable: true,
        })
    }

    fn with_protocol_engine(mut self, keys: &Keys) -> Self {
        let owner = keys.public_key();
        let owner_hex = owner.to_hex();
        let storage = Arc::new(SqliteStorageAdapter::new(
            Arc::clone(&self.conn),
            owner_hex.clone(),
            owner_hex,
        ));
        match ProtocolEngine::load_or_create_for_local_device(storage, owner, keys) {
            Ok(engine) => {
                self.protocol_engine = Some(engine);
                self.owner_public_key = Some(owner);
            }
            Err(error) => self.last_error = Some(format!("Direct message init failed: {error}")),
        }
        self
    }

    fn protocol_subscription_commands(&mut self) -> Vec<DirectMessageCommand> {
        self.subscription_command().into_iter().collect()
    }

    fn commands_from_effects(&mut self, effects: Vec<ProtocolEffect>) -> Vec<DirectMessageCommand> {
        let mut commands = Vec::new();
        for effect in effects {
            match effect {
                ProtocolEffect::Publish(publish) => {
                    commands.push(DirectMessageCommand::Publish(publish.event));
                }
                ProtocolEffect::FetchProtocolState { filters, reason } => {
                    self.fetch_subscription_counter =
                        self.fetch_subscription_counter.saturating_add(1);
                    let subscription_id = format!(
                        "iris-native-private-chat-fetch-{reason}-{}",
                        self.fetch_subscription_counter
                    );
                    commands.push(DirectMessageCommand::Subscribe {
                        subscription_id,
                        filters,
                        durable: false,
                    });
                }
            }
        }
        commands
    }

    fn effects_from_retry_batch(&mut self, batch: ProtocolRetryBatch) -> Vec<ProtocolEffect> {
        let mut effects = batch.effects;
        for result in batch.direct_results {
            if !result.event_ids.is_empty() {
                self.mark_message_sent(&result.chat_id, &result.message_id);
            }
            effects.extend(result.effects);
        }
        effects.extend(batch.group_result.effects);
        for message in batch.direct_messages {
            self.apply_decrypted_protocol_message(message);
        }
        effects
    }

    fn apply_decrypted_protocol_message(&mut self, message: ProtocolDecryptedMessage) {
        self.apply_decrypted(
            message.sender,
            message.conversation_owner,
            &message.content,
            message.event_id,
        );
    }

    fn apply_decrypted(
        &mut self,
        sender: PublicKey,
        conversation_owner: Option<PublicKey>,
        content: &str,
        source_event_id: Option<String>,
    ) {
        let Some(rumor) = parse_runtime_rumor(content) else {
            return;
        };
        if rumor.kind != CHAT_MESSAGE_KIND {
            return;
        }
        let local_owner = self.owner_public_key;
        let peer = if local_owner == Some(sender) {
            conversation_owner.unwrap_or(sender)
        } else {
            sender
        };
        let chat_id = peer.to_hex();
        self.ensure_thread(&chat_id, rumor.created_at_secs);
        self.insert_message(
            &chat_id,
            &rumor.id,
            &rumor.content,
            local_owner == Some(sender),
            rumor.created_at_secs,
            if local_owner == Some(sender) {
                DirectMessageDelivery::Sent
            } else {
                DirectMessageDelivery::Received
            },
            source_event_id.as_deref(),
        );
    }

    fn ensure_schema(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute_batch(SCHEMA);
        }
    }

    fn ensure_thread(&self, chat_id: &str, updated_at: u64) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO private_chat_threads (chat_id, display_name, avatar_seed, updated_at_secs)
                 VALUES (?1, '', '', ?2)
                 ON CONFLICT(chat_id) DO UPDATE SET updated_at_secs = MAX(updated_at_secs, excluded.updated_at_secs)",
                params![chat_id, updated_at as i64],
            );
        }
    }

    fn messages(&self, chat_id: &str, limit: usize) -> Vec<DirectMessageSnapshot> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT id, body, is_outgoing, created_at_secs, delivery
             FROM private_chat_messages
             WHERE chat_id = ?1
             ORDER BY created_at_secs DESC, id DESC
             LIMIT ?2",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![chat_id, limit as i64], |row| {
            Ok(DirectMessageSnapshot {
                id: row.get(0)?,
                chat_id: chat_id.to_string(),
                body: row.get(1)?,
                is_outgoing: row.get::<_, i64>(2)? != 0,
                created_at_secs: row.get::<_, i64>(3)?.max(0) as u64,
                delivery: DirectMessageDelivery::from_str(&row.get::<_, String>(4)?),
            })
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        let mut messages = rows.filter_map(Result::ok).collect::<Vec<_>>();
        messages.reverse();
        messages
    }

    fn insert_message(
        &self,
        chat_id: &str,
        id: &str,
        body: &str,
        is_outgoing: bool,
        created_at: u64,
        delivery: DirectMessageDelivery,
        source_event_id: Option<&str>,
    ) {
        if id.is_empty() {
            return;
        }
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO private_chat_messages
                 (chat_id, id, body, is_outgoing, created_at_secs, delivery, source_event_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    chat_id,
                    id,
                    body,
                    is_outgoing as i64,
                    created_at as i64,
                    delivery.as_str(),
                    source_event_id,
                ],
            );
            let _ = conn.execute(
                "UPDATE private_chat_threads SET updated_at_secs = MAX(updated_at_secs, ?2)
                 WHERE chat_id = ?1",
                params![chat_id, created_at as i64],
            );
        }
    }

    fn seen_event(&self, event_id: &str) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return true;
        };
        conn.query_row(
            "SELECT 1 FROM private_chat_seen_events WHERE event_id = ?1",
            [event_id],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    }

    fn mark_seen_event(&self, event_id: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO private_chat_seen_events (event_id) VALUES (?1)",
                [event_id],
            );
        }
    }

    fn mark_message_sent(&self, chat_id: &str, id: &str) {
        if id.is_empty() {
            return;
        }
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "UPDATE private_chat_messages
                 SET delivery = ?3
                 WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, id, DirectMessageDelivery::Sent.as_str()],
            );
        }
    }
}

struct RuntimeRumor {
    id: String,
    kind: u32,
    content: String,
    created_at_secs: u64,
}

fn parse_runtime_rumor(content: &str) -> Option<RuntimeRumor> {
    let mut event = serde_json::from_str::<UnsignedEvent>(content).ok()?;
    event.ensure_id();
    event.verify_id().ok()?;
    Some(RuntimeRumor {
        id: event.id.as_ref()?.to_string(),
        kind: event.kind.as_u16() as u32,
        content: event.content,
        created_at_secs: event.created_at.as_secs(),
    })
}

fn chat_snapshot_for_pubkey(chat_id: &str) -> DirectChatSnapshot {
    DirectChatSnapshot {
        chat_id: chat_id.to_string(),
        last_message_preview: String::new(),
        last_message_at: 0,
        unread_count: 0,
    }
}

fn normalize_pubkey(input: &str) -> Result<String, String> {
    PublicKey::parse(input)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| error.to_string())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
