use rusqlite::Connection;

// Bump when a non-additive change to the schema lands and migrate
// inside `ensure_schema` below. Greenfield: version 1 is the initial
// shape and there is no previous JSON layout to migrate from.
const SCHEMA_VERSION: u32 = 3;

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
    nostr_relay_urls_json TEXT NOT NULL,
    image_proxy_enabled INTEGER NOT NULL,
    image_proxy_url TEXT NOT NULL,
    image_proxy_key_hex TEXT NOT NULL,
    image_proxy_salt_hex TEXT NOT NULL,
    mobile_push_server_url TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS owner_profiles (
    owner_pubkey_hex TEXT PRIMARY KEY,
    name TEXT,
    display_name TEXT,
    picture TEXT,
    updated_at_secs INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS app_keys (
    owner_pubkey_hex TEXT PRIMARY KEY,
    created_at_secs INTEGER NOT NULL,
    devices_json TEXT NOT NULL
);

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
    updated_at_secs INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS messages (
    chat_id TEXT NOT NULL REFERENCES threads(chat_id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'system')),
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    is_outgoing INTEGER NOT NULL,
    created_at_secs INTEGER NOT NULL,
    expires_at_secs INTEGER,
    delivery TEXT NOT NULL CHECK (delivery IN ('queued', 'pending', 'sent', 'received', 'seen', 'failed')),
    attachments_json TEXT NOT NULL DEFAULT '[]',
    reactions_json TEXT NOT NULL DEFAULT '[]',
    reactors_json TEXT NOT NULL DEFAULT '[]',
    source_event_id TEXT,
    PRIMARY KEY (chat_id, id)
);

CREATE INDEX IF NOT EXISTS messages_chat_order_idx
    ON messages(chat_id, created_at_secs, id);

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

CREATE TABLE IF NOT EXISTS ndr_kv (
    owner_pubkey_hex TEXT NOT NULL,
    device_pubkey_hex TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex, key)
);
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
    tx.pragma_update(None, "user_version", SCHEMA_VERSION as i64)?;
    tx.commit()?;
    Ok(())
}
