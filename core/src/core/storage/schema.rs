use super::super::account_app_keys::known_app_keys_from_ndr;
use super::super::identity::unix_now;
use nostr::{Event, PublicKey};
use nostr_double_ratchet::AppKeys;
use rusqlite::{params, Connection, Transaction};

// Bump when a non-additive change to the schema lands and migrate
// inside `ensure_schema` below. Greenfield: version 1 is the initial
// shape and there is no previous JSON layout to migrate from.
const SCHEMA_VERSION: u32 = 30;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS app_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS preferences (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    send_typing_indicators INTEGER NOT NULL,
    send_read_receipts INTEGER NOT NULL,
    desktop_notifications_enabled INTEGER NOT NULL,
    invite_acceptance_notifications_enabled INTEGER NOT NULL DEFAULT 1,
    startup_at_login_enabled INTEGER NOT NULL,
    nearby_bluetooth_enabled INTEGER NOT NULL DEFAULT 0,
    nearby_lan_enabled INTEGER NOT NULL DEFAULT 0,
    nostr_relay_urls_json TEXT NOT NULL,
    image_proxy_enabled INTEGER NOT NULL,
    image_proxy_fallback_enabled INTEGER NOT NULL DEFAULT 0,
    image_proxy_url TEXT NOT NULL,
    image_proxy_key_hex TEXT NOT NULL,
    image_proxy_salt_hex TEXT NOT NULL,
    mobile_push_server_url TEXT NOT NULL,
    muted_chat_ids_json TEXT NOT NULL DEFAULT '[]',
    pinned_chat_ids_json TEXT NOT NULL DEFAULT '[]',
    debug_logging_enabled INTEGER NOT NULL DEFAULT 0,
    accept_unknown_direct_messages INTEGER NOT NULL DEFAULT 1,
    nearby_enabled INTEGER NOT NULL DEFAULT 1,
    blocked_owner_pubkeys_json TEXT NOT NULL DEFAULT '[]',
    accepted_owner_pubkeys_json TEXT NOT NULL DEFAULT '[]',
    nearby_mailbag_enabled INTEGER NOT NULL DEFAULT 1,
    nearby_show_in_chat_list INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS owner_profiles (
    owner_pubkey_hex TEXT PRIMARY KEY,
    nickname TEXT,
    name TEXT,
    display_name TEXT,
    picture TEXT,
    about TEXT,
    extra_metadata_json TEXT NOT NULL DEFAULT '{}',
    extra_tags_json TEXT NOT NULL DEFAULT '[]',
    updated_at_secs INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS app_keys (
    owner_pubkey_hex TEXT PRIMARY KEY,
    created_at_secs INTEGER NOT NULL,
    devices_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_discovery_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    owner_pubkey_hex TEXT,
    follow_event_id TEXT,
    follow_created_at_secs INTEGER NOT NULL DEFAULT 0,
    social_rank_ready INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS user_discovery_users (
    owner_pubkey_hex TEXT PRIMARY KEY,
    follow_position INTEGER NOT NULL,
    petname TEXT
);

CREATE INDEX IF NOT EXISTS user_discovery_users_position_idx
    ON user_discovery_users(follow_position, owner_pubkey_hex);

CREATE TABLE IF NOT EXISTS user_discovery_social (
    account_owner_pubkey_hex TEXT NOT NULL,
    target_owner_pubkey_hex TEXT NOT NULL,
    friend_support INTEGER NOT NULL,
    PRIMARY KEY(account_owner_pubkey_hex, target_owner_pubkey_hex)
);

CREATE TABLE IF NOT EXISTS profile_search_candidates (
    owner_pubkey_hex TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    aliases_json TEXT NOT NULL DEFAULT '[]',
    nip05 TEXT,
    picture TEXT,
    created_at_secs INTEGER NOT NULL DEFAULT 0,
    cached_at_secs INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS profile_search_candidates_cached_idx
    ON profile_search_candidates(cached_at_secs, owner_pubkey_hex);

CREATE TABLE IF NOT EXISTS groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    picture TEXT,
    created_at_ms INTEGER NOT NULL DEFAULT 0,
    updated_at_secs INTEGER NOT NULL DEFAULT 0,
    group_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_message_ttls (
    chat_id TEXT PRIMARY KEY,
    ttl_seconds INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS threads (
    chat_id TEXT PRIMARY KEY,
    unread_count INTEGER NOT NULL DEFAULT 0,
    updated_at_secs INTEGER NOT NULL DEFAULT 0,
    draft TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS messages (
    chat_id TEXT NOT NULL REFERENCES threads(chat_id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'system')),
    author TEXT NOT NULL,
    author_owner_pubkey_hex TEXT,
    body TEXT NOT NULL,
    is_outgoing INTEGER NOT NULL,
    created_at_secs INTEGER NOT NULL,
    expires_at_secs INTEGER,
    delivery TEXT NOT NULL CHECK (delivery IN ('queued', 'pending', 'sent', 'received', 'seen', 'failed')),
    attachments_json TEXT NOT NULL DEFAULT '[]',
    reactions_json TEXT NOT NULL DEFAULT '[]',
    reactors_json TEXT NOT NULL DEFAULT '[]',
    source_event_id TEXT,
    recipient_deliveries_json TEXT NOT NULL DEFAULT '[]',
    delivery_trace_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (chat_id, id)
);

CREATE INDEX IF NOT EXISTS messages_chat_order_idx
    ON messages(chat_id, created_at_secs, id);

CREATE INDEX IF NOT EXISTS messages_chat_recent_idx
    ON messages(
        chat_id,
        created_at_secs DESC,
        CASE
            WHEN id != '' AND id NOT GLOB '*[^0-9]*' THEN CAST(id AS INTEGER)
            ELSE 9223372036854775807
        END DESC,
        id DESC
    );

CREATE INDEX IF NOT EXISTS messages_expires_idx
    ON messages(expires_at_secs) WHERE expires_at_secs IS NOT NULL;

-- Used by the notification extension to find an already-decrypted
-- rumor by its outer relay event id.
CREATE INDEX IF NOT EXISTS messages_source_event_idx
    ON messages(source_event_id) WHERE source_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS seen_events (
    event_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS seen_events_sequence_idx
    ON seen_events(sequence);

CREATE TABLE IF NOT EXISTS pending_relay_publishes (
    event_id TEXT PRIMARY KEY,
    owner_pubkey_hex TEXT NOT NULL,
    label TEXT NOT NULL,
    event_json TEXT NOT NULL,
    inner_event_id TEXT,
    chat_id TEXT,
    created_at_secs INTEGER NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS pending_relay_publishes_owner_idx
    ON pending_relay_publishes(owner_pubkey_hex, created_at_secs);

CREATE TABLE IF NOT EXISTS ndr_kv (
    owner_pubkey_hex TEXT NOT NULL,
    device_pubkey_hex TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex, key)
);

-- Full-text index over the bodies of `messages`, kept in sync via the
-- triggers below. `unicode61` is the default tokenizer plus diacritic
-- stripping so "Schön" matches "schon"; the message_id/chat_id columns
-- are unindexed because we only need them for join-back, not for matching.
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    body,
    chat_id UNINDEXED,
    message_id UNINDEXED,
    tokenize = "unicode61 remove_diacritics 1"
);

-- Keep `messages_fts` synchronized with `messages`. The FTS table is
-- not external-content because the parent has a composite primary key
-- and no stable rowid alias to bind against; we mirror inserts/deletes
-- explicitly on the implicit rowid instead.
CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, body, chat_id, message_id)
    VALUES (new.rowid, new.body, new.chat_id, new.id);
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid = old.rowid;
    INSERT INTO messages_fts(rowid, body, chat_id, message_id)
    VALUES (new.rowid, new.body, new.chat_id, new.id);
END;
"#;

pub(super) fn ensure_schema(conn: &mut Connection) -> anyhow::Result<()> {
    let current: u32 =
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))? as u32;
    if current >= SCHEMA_VERSION {
        // Re-running CREATE TABLE IF NOT EXISTS on an established
        // database is cheap, but skipping it on the hot path keeps
        // cold-start fast.
        return Ok(());
    }

    let tx = conn.transaction()?;
    tx.execute_batch(INITIAL_SCHEMA)?;
    if current < 3 {
        let has_column = {
            let mut stmt = tx.prepare("PRAGMA table_info(preferences)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == "invite_acceptance_notifications_enabled" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_column {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN invite_acceptance_notifications_enabled INTEGER NOT NULL DEFAULT 1;",
            )?;
        }
    }
    if current < 4 {
        let has_column = {
            let mut stmt = tx.prepare("PRAGMA table_info(preferences)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == "muted_chat_ids_json" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_column {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN muted_chat_ids_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }
    }
    if current < 5 {
        let has_column = {
            let mut stmt = tx.prepare("PRAGMA table_info(preferences)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == "nearby_bluetooth_enabled" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_column {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN nearby_bluetooth_enabled INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
    }
    if current < 6 {
        let has_column = {
            let mut stmt = tx.prepare("PRAGMA table_info(preferences)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for row in rows {
                if row? == "nearby_lan_enabled" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_column {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN nearby_lan_enabled INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
    }
    if current < 8 {
        if !column_exists(&tx, "messages", "recipient_deliveries_json")? {
            tx.execute_batch(
                "ALTER TABLE messages
                 ADD COLUMN recipient_deliveries_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }
        if !column_exists(&tx, "messages", "delivery_trace_json")? {
            tx.execute_batch(
                "ALTER TABLE messages
                 ADD COLUMN delivery_trace_json TEXT NOT NULL DEFAULT '{}';",
            )?;
        }
        if !column_exists(&tx, "pending_relay_publishes", "inner_event_id")? {
            tx.execute_batch(
                "ALTER TABLE pending_relay_publishes
                 ADD COLUMN inner_event_id TEXT;",
            )?;
        }
        if !column_exists(&tx, "pending_relay_publishes", "chat_id")? {
            tx.execute_batch(
                "ALTER TABLE pending_relay_publishes
                 ADD COLUMN chat_id TEXT;",
            )?;
        }
        if !column_exists(&tx, "pending_relay_publishes", "attempt_count")? {
            tx.execute_batch(
                "ALTER TABLE pending_relay_publishes
                 ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !column_exists(&tx, "pending_relay_publishes", "last_error")? {
            tx.execute_batch(
                "ALTER TABLE pending_relay_publishes
                 ADD COLUMN last_error TEXT;",
            )?;
        }
    }
    if current < 10 && !column_exists(&tx, "preferences", "pinned_chat_ids_json")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN pinned_chat_ids_json TEXT NOT NULL DEFAULT '[]';",
        )?;
    }
    if current < 11 {
        // INITIAL_SCHEMA above already created `messages_fts` plus the
        // sync triggers via IF NOT EXISTS. Backfill any rows that pre-
        // date the FTS index. INSERT OR IGNORE so partial / re-run
        // migrations stay idempotent.
        tx.execute_batch(
            "INSERT OR IGNORE INTO messages_fts(rowid, body, chat_id, message_id)
             SELECT rowid, body, chat_id, id FROM messages;",
        )?;
    }
    if current < 12 && !column_exists(&tx, "threads", "draft")? {
        tx.execute_batch(
            "ALTER TABLE threads
             ADD COLUMN draft TEXT NOT NULL DEFAULT '';",
        )?;
    }
    if current < 13 && !column_exists(&tx, "preferences", "debug_logging_enabled")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN debug_logging_enabled INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    if current < 14 {
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS messages_chat_recent_idx
             ON messages(
                 chat_id,
                 created_at_secs DESC,
                 CASE
                     WHEN id != '' AND id NOT GLOB '*[^0-9]*' THEN CAST(id AS INTEGER)
                     ELSE 9223372036854775807
                 END DESC,
                 id DESC
             );",
        )?;
    }
    if current < 15 && !column_exists(&tx, "preferences", "accept_unknown_direct_messages")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN accept_unknown_direct_messages INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if current < 16 && !column_exists(&tx, "preferences", "nearby_enabled")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN nearby_enabled INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if current < 17 {
        // Signal-style per-peer state: a blocklist that the core uses
        // to drop blocked authors from the nostr + push subscriptions,
        // and an accepted-peers set that the chat-request gate (Signal
        // whitelist) reads to decide whether a thread shows the
        // Accept / Delete / Block bar. Both are JSON arrays of owner
        // pubkey hex, stored on the singleton `preferences` row so
        // they ride the existing load/persist path.
        if !column_exists(&tx, "preferences", "blocked_owner_pubkeys_json")? {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN blocked_owner_pubkeys_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }
        if !column_exists(&tx, "preferences", "accepted_owner_pubkeys_json")? {
            tx.execute_batch(
                "ALTER TABLE preferences
                 ADD COLUMN accepted_owner_pubkeys_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }
    }
    if current < 18 && !column_exists(&tx, "preferences", "nearby_mailbag_enabled")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN nearby_mailbag_enabled INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if current < 19 && !column_exists(&tx, "messages", "author_owner_pubkey_hex")? {
        tx.execute_batch(
            "ALTER TABLE messages
             ADD COLUMN author_owner_pubkey_hex TEXT;",
        )?;
    }
    if current < 20 && !column_exists(&tx, "owner_profiles", "nickname")? {
        tx.execute_batch(
            "ALTER TABLE owner_profiles
             ADD COLUMN nickname TEXT;",
        )?;
    }
    if current < 22 && !column_exists(&tx, "preferences", "nearby_show_in_chat_list")? {
        tx.execute_batch(
            "ALTER TABLE preferences
             ADD COLUMN nearby_show_in_chat_list INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if current < 21 && !column_exists(&tx, "owner_profiles", "about")? {
        tx.execute_batch(
            "ALTER TABLE owner_profiles
             ADD COLUMN about TEXT;",
        )?;
    }
    if current < 23 {
        if !column_exists(&tx, "owner_profiles", "extra_metadata_json")? {
            tx.execute_batch(
                "ALTER TABLE owner_profiles
                 ADD COLUMN extra_metadata_json TEXT NOT NULL DEFAULT '{}';",
            )?;
        }
        if !column_exists(&tx, "owner_profiles", "extra_tags_json")? {
            tx.execute_batch(
                "ALTER TABLE owner_profiles
                 ADD COLUMN extra_tags_json TEXT NOT NULL DEFAULT '[]';",
            )?;
        }
    }
    if current < 26 {
        tx.execute_batch(
            "DROP INDEX IF EXISTS pending_relay_publishes_owner_idx;
             ALTER TABLE pending_relay_publishes RENAME TO pending_relay_publishes_old;
             CREATE TABLE pending_relay_publishes (
                 event_id TEXT PRIMARY KEY,
                 owner_pubkey_hex TEXT NOT NULL,
                 label TEXT NOT NULL,
                 event_json TEXT NOT NULL,
                 inner_event_id TEXT,
                 chat_id TEXT,
                 created_at_secs INTEGER NOT NULL,
                 attempt_count INTEGER NOT NULL DEFAULT 0,
                 last_error TEXT
             );
             INSERT INTO pending_relay_publishes(
                 event_id, owner_pubkey_hex, label, event_json, inner_event_id,
                 chat_id, created_at_secs, attempt_count, last_error
             )
             SELECT event_id, owner_pubkey_hex, label, event_json, inner_event_id,
                    chat_id, created_at_secs, attempt_count, last_error
             FROM pending_relay_publishes_old;
             DROP TABLE pending_relay_publishes_old;
             CREATE INDEX IF NOT EXISTS pending_relay_publishes_owner_idx
                 ON pending_relay_publishes(owner_pubkey_hex, created_at_secs);",
        )?;
    }
    if current < 27 {
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_discovery_state (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 follow_event_id TEXT,
                 follow_created_at_secs INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS user_discovery_users (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 follow_position INTEGER NOT NULL,
                 petname TEXT,
                 app_keys_created_at_secs INTEGER NOT NULL,
                 app_keys_event_id TEXT NOT NULL,
                 app_keys_event_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS user_discovery_users_position_idx
                 ON user_discovery_users(follow_position, owner_pubkey_hex);",
        )?;
    }
    if current < 28 {
        if column_exists(&tx, "user_discovery_users", "app_keys_event_json")? {
            migrate_discovery_app_keys(&tx)?;
        }
        tx.execute_batch(
            "DROP INDEX IF EXISTS user_discovery_users_position_idx;
             ALTER TABLE user_discovery_users RENAME TO user_discovery_users_v27;
             CREATE TABLE user_discovery_users (
                 owner_pubkey_hex TEXT PRIMARY KEY,
                 follow_position INTEGER NOT NULL,
                 petname TEXT
             );
             INSERT INTO user_discovery_users(owner_pubkey_hex, follow_position, petname)
             SELECT owner_pubkey_hex, follow_position, petname
             FROM user_discovery_users_v27;
             DROP TABLE user_discovery_users_v27;
             CREATE INDEX user_discovery_users_position_idx
                 ON user_discovery_users(follow_position, owner_pubkey_hex);",
        )?;
    }
    if current < 29 {
        if !column_exists(&tx, "user_discovery_state", "owner_pubkey_hex")? {
            tx.execute_batch("ALTER TABLE user_discovery_state ADD COLUMN owner_pubkey_hex TEXT;")?;
        }
        if !column_exists(&tx, "user_discovery_state", "social_rank_ready")? {
            tx.execute_batch("ALTER TABLE user_discovery_state ADD COLUMN social_rank_ready INTEGER NOT NULL DEFAULT 0;")?;
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS user_discovery_social (account_owner_pubkey_hex TEXT NOT NULL, target_owner_pubkey_hex TEXT NOT NULL, friend_support INTEGER NOT NULL, PRIMARY KEY(account_owner_pubkey_hex, target_owner_pubkey_hex));")?;
    }
    if current < 30 && !column_exists(&tx, "preferences", "image_proxy_fallback_enabled")? {
        tx.execute_batch("ALTER TABLE preferences ADD COLUMN image_proxy_fallback_enabled INTEGER NOT NULL DEFAULT 0;")?;
    }

    tx.pragma_update(None, "user_version", SCHEMA_VERSION as i64)?;
    tx.commit()?;
    Ok(())
}

fn migrate_discovery_app_keys(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let cached_events = {
        let mut stmt = tx.prepare(
            "SELECT owner_pubkey_hex, app_keys_event_json
             FROM user_discovery_users",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let now_secs = unix_now().get();
    for (owner_hex, event_json) in cached_events {
        let (Ok(owner), Ok(event)) = (
            PublicKey::from_hex(&owner_hex),
            serde_json::from_str::<Event>(&event_json),
        ) else {
            continue;
        };
        if event.pubkey != owner
            || event.created_at.as_secs() > now_secs.saturating_add(600)
            || event.verify().is_err()
        {
            continue;
        }
        let Ok(app_keys) = AppKeys::from_event(&event) else {
            continue;
        };
        if app_keys.get_all_devices().is_empty() {
            continue;
        }
        let known = known_app_keys_from_ndr(owner, &app_keys, event.created_at.as_secs());
        tx.execute(
            "INSERT INTO app_keys(owner_pubkey_hex, created_at_secs, devices_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(owner_pubkey_hex) DO UPDATE SET
                 created_at_secs = excluded.created_at_secs,
                 devices_json = excluded.devices_json
             WHERE excluded.created_at_secs > app_keys.created_at_secs",
            params![
                known.owner_pubkey_hex,
                known.created_at_secs as i64,
                serde_json::to_string(&known.devices)?,
            ],
        )?;
    }
    Ok(())
}

fn column_exists(
    tx: &rusqlite::Transaction<'_>,
    table_name: &str,
    column_name: &str,
) -> anyhow::Result<bool> {
    let mut stmt = tx.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
