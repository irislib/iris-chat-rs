use rusqlite::Connection;

// Bump when a non-additive change to the schema lands and migrate
// inside `ensure_schema` below. Greenfield: version 1 is the initial
// shape and there is no previous JSON layout to migrate from.
const SCHEMA_VERSION: u32 = 26;

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
    tx.pragma_update(None, "user_version", SCHEMA_VERSION as i64)?;
    tx.commit()?;
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
mod tests {
    use super::*;

    #[test]
    fn migrates_v25_pending_relay_publish_target_columns_removed() {
        const OLD_TARGET_OWNER_COLUMN: &str = concat!("target_owner_pubkey_", "hex");
        const OLD_TARGET_DEVICE_COLUMN: &str = concat!("target_device_", "id");
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            r#"
            CREATE TABLE pending_relay_publishes (
                event_id TEXT PRIMARY KEY,
                owner_pubkey_hex TEXT NOT NULL,
                label TEXT NOT NULL,
                event_json TEXT NOT NULL,
                inner_event_id TEXT,
                {OLD_TARGET_OWNER_COLUMN} TEXT,
                {OLD_TARGET_DEVICE_COLUMN} TEXT,
                chat_id TEXT,
                created_at_secs INTEGER NOT NULL,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT
            );
            INSERT INTO pending_relay_publishes(
                event_id, owner_pubkey_hex, label, event_json, inner_event_id,
                {OLD_TARGET_OWNER_COLUMN}, {OLD_TARGET_DEVICE_COLUMN}, chat_id, created_at_secs,
                attempt_count, last_error
            )
            VALUES(
                'outer', 'owner', 'appcore-protocol', '{{}}', 'inner',
                'peer', 'device', 'chat', 42, 1, 'retry'
            );
            PRAGMA user_version = 25;
            "#,
        ))
        .unwrap();

        ensure_schema(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(!connection_column_exists(
            &conn,
            "pending_relay_publishes",
            OLD_TARGET_OWNER_COLUMN
        ));
        assert!(!connection_column_exists(
            &conn,
            "pending_relay_publishes",
            OLD_TARGET_DEVICE_COLUMN
        ));
        let row = conn
            .query_row(
                "SELECT event_id, inner_event_id, chat_id, attempt_count, last_error
                 FROM pending_relay_publishes",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "outer".to_string(),
                Some("inner".to_string()),
                Some("chat".to_string()),
                1,
                Some("retry".to_string())
            )
        );
    }

    #[test]
    fn migrates_v9_preferences_pinned_chat_ids_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE preferences (
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
                image_proxy_url TEXT NOT NULL,
                image_proxy_key_hex TEXT NOT NULL,
                image_proxy_salt_hex TEXT NOT NULL,
                mobile_push_server_url TEXT NOT NULL,
                muted_chat_ids_json TEXT NOT NULL DEFAULT '[]'
            );
            PRAGMA user_version = 9;
            "#,
        )
        .unwrap();

        ensure_schema(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(connection_column_exists(
            &conn,
            "preferences",
            "pinned_chat_ids_json"
        ));
    }

    #[test]
    fn migrates_v10_to_v11_backfills_messages_fts() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE threads (chat_id TEXT PRIMARY KEY);
            CREATE TABLE messages (
                chat_id TEXT NOT NULL REFERENCES threads(chat_id) ON DELETE CASCADE,
                id TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'user',
                author TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL,
                is_outgoing INTEGER NOT NULL DEFAULT 0,
                created_at_secs INTEGER NOT NULL DEFAULT 0,
                expires_at_secs INTEGER,
                delivery TEXT NOT NULL DEFAULT 'sent',
                attachments_json TEXT NOT NULL DEFAULT '[]',
                reactions_json TEXT NOT NULL DEFAULT '[]',
                reactors_json TEXT NOT NULL DEFAULT '[]',
                source_event_id TEXT,
                recipient_deliveries_json TEXT NOT NULL DEFAULT '[]',
                delivery_trace_json TEXT NOT NULL DEFAULT '{}',
                PRIMARY KEY (chat_id, id)
            );
            INSERT INTO threads(chat_id) VALUES ('chat-1');
            INSERT INTO messages(chat_id, id, body) VALUES ('chat-1', '1', 'hello world');
            INSERT INTO messages(chat_id, id, body) VALUES ('chat-1', '2', 'goodbye moon');
            PRAGMA user_version = 10;
            "#,
        )
        .unwrap();

        ensure_schema(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        let hits: Vec<(String, String)> = conn
            .prepare("SELECT chat_id, message_id FROM messages_fts WHERE body MATCH 'hello'")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits, vec![("chat-1".to_string(), "1".to_string())]);

        // The triggers seeded by INITIAL_SCHEMA must keep the FTS index
        // in sync for rows inserted after migration.
        conn.execute(
            "INSERT INTO messages(chat_id, id, body) VALUES ('chat-1', '3', 'hello again')",
            [],
        )
        .unwrap();
        let hits: Vec<String> = conn
            .prepare(
                "SELECT message_id FROM messages_fts WHERE body MATCH 'hello'
                 ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits, vec!["1".to_string(), "3".to_string()]);
    }

    #[test]
    fn migrates_v19_owner_profiles_adds_nickname_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE owner_profiles (
                owner_pubkey_hex TEXT PRIMARY KEY,
                name TEXT,
                display_name TEXT,
                picture TEXT,
                updated_at_secs INTEGER NOT NULL
            );
            INSERT INTO owner_profiles
                (owner_pubkey_hex, name, display_name, picture, updated_at_secs)
            VALUES
                ('peer', 'alice', 'Alice', NULL, 1);
            PRAGMA user_version = 19;
            "#,
        )
        .unwrap();

        ensure_schema(&mut conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(connection_column_exists(
            &conn,
            "owner_profiles",
            "nickname"
        ));
        let row: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT nickname, display_name FROM owner_profiles WHERE owner_pubkey_hex = 'peer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (None, Some("Alice".to_string())));
    }

    fn user_version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap() as u32
    }

    fn connection_column_exists(conn: &Connection, table_name: &str, column_name: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table_name})"))
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
        for row in rows {
            if row.unwrap() == column_name {
                return true;
            }
        }
        false
    }
}
