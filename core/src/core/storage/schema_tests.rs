use super::*;
use nostr::Keys;
use nostr_double_ratchet::DeviceEntry;

#[test]
fn migrates_v29_image_proxy_fallback_to_disabled_without_changing_other_preferences() {
    let mut conn = Connection::open_in_memory().unwrap();
    ensure_schema(&mut conn).unwrap();
    // Recreate the previous version's preferences table and an existing row.
    conn.execute_batch(
        "ALTER TABLE preferences DROP COLUMN image_proxy_fallback_enabled;
         INSERT INTO preferences (
            id, send_typing_indicators, send_read_receipts, desktop_notifications_enabled,
            startup_at_login_enabled, nostr_relay_urls_json, image_proxy_enabled,
            image_proxy_url, image_proxy_key_hex, image_proxy_salt_hex, mobile_push_server_url
         ) VALUES (1, 0, 0, 1, 1, '[]', 1, 'https://custom.example', 'key', 'salt', '');
         PRAGMA user_version = 29;",
    )
    .unwrap();

    ensure_schema(&mut conn).unwrap();

    let values: (i64, String) = conn
        .query_row(
            "SELECT image_proxy_fallback_enabled, image_proxy_url FROM preferences WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(values, (0, "https://custom.example".to_string()));
    conn.execute(
        "UPDATE preferences SET image_proxy_fallback_enabled = 1",
        [],
    )
    .unwrap();
    ensure_schema(&mut conn).unwrap();
    let enabled: i64 = conn
        .query_row(
            "SELECT image_proxy_fallback_enabled FROM preferences WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, 1, "reopening must preserve explicit opt-in");
}

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

#[test]
fn migrates_v26_to_current_user_discovery_cache() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 26;").unwrap();

    ensure_schema(&mut conn).unwrap();

    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    assert!(connection_table_exists(&conn, "user_discovery_state"));
    assert!(connection_table_exists(&conn, "user_discovery_users"));
    assert!(connection_table_exists(&conn, "user_discovery_social"));
    assert!(connection_table_exists(&conn, "profile_search_candidates"));
    conn.execute(
        "INSERT INTO user_discovery_state(id, follow_event_id, follow_created_at_secs)
         VALUES (1, 'head', 42)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO user_discovery_users(owner_pubkey_hex, follow_position, petname)
         VALUES ('owner', 3, 'Friend')",
        [],
    )
    .unwrap();
}

#[test]
fn migrates_v27_discovery_users_without_losing_social_fields() {
    let owner = Keys::generate();
    let device = Keys::generate().public_key();
    let app_keys_event = AppKeys::new(vec![DeviceEntry::new(device, 41)])
        .get_event_at(owner.public_key(), 41)
        .sign_with_keys(&owner)
        .unwrap();
    let owner_hex = owner.public_key().to_hex();
    let invalid_owner = Keys::generate();
    let mut invalid_event = AppKeys::new(vec![DeviceEntry::new(device, 42)])
        .get_event_at(invalid_owner.public_key(), 42)
        .sign_with_keys(&invalid_owner)
        .unwrap();
    invalid_event.content.push('x');
    let invalid_owner_hex = invalid_owner.public_key().to_hex();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE user_discovery_users (
             owner_pubkey_hex TEXT PRIMARY KEY,
             follow_position INTEGER NOT NULL,
             petname TEXT,
             app_keys_created_at_secs INTEGER NOT NULL,
             app_keys_event_id TEXT NOT NULL,
             app_keys_event_json TEXT NOT NULL
         );
         PRAGMA user_version = 27;",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO user_discovery_users
         VALUES (?1, 4, NULL, 42, ?2, ?3)",
        params![
            invalid_owner_hex,
            invalid_event.id.to_hex(),
            serde_json::to_string(&invalid_event).unwrap()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO user_discovery_users
         VALUES (?1, 3, 'Friend', 41, ?2, ?3)",
        params![
            owner_hex,
            app_keys_event.id.to_hex(),
            serde_json::to_string(&app_keys_event).unwrap()
        ],
    )
    .unwrap();

    ensure_schema(&mut conn).unwrap();

    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    let preserved: (String, i64, Option<String>) = conn
        .query_row(
            "SELECT owner_pubkey_hex, follow_position, petname
             FROM user_discovery_users
             WHERE owner_pubkey_hex = ?1",
            [&owner_hex],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        preserved,
        (owner_hex.clone(), 3, Some("Friend".to_string()))
    );
    let migrated_roster: (i64, String) = conn
        .query_row(
            "SELECT created_at_secs, devices_json FROM app_keys
             WHERE owner_pubkey_hex = ?1",
            [&owner_hex],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(migrated_roster.0, 41);
    assert!(migrated_roster.1.contains(&device.to_hex()));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM app_keys WHERE owner_pubkey_hex = ?1",
            [&invalid_owner_hex],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM user_discovery_users", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        2
    );
    for removed in [
        "app_keys_created_at_secs",
        "app_keys_event_id",
        "app_keys_event_json",
    ] {
        assert!(!connection_column_exists(
            &conn,
            "user_discovery_users",
            removed
        ));
    }
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

fn connection_table_exists(conn: &Connection, table_name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table_name],
        |_| Ok(()),
    )
    .is_ok()
}
