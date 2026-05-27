use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

pub(crate) fn latest_message_keys(data_dir: &Path, chat: Option<&str>) -> Result<HashSet<String>> {
    let conn = open_existing_db(data_dir)?;
    let mut seen = HashSet::new();
    match chat {
        Some(chat_id) => {
            let mut stmt = conn.prepare("SELECT chat_id, id FROM messages WHERE chat_id = ?1")?;
            let rows = stmt.query_map([chat_id], |row| {
                Ok(format!(
                    "{}\0{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })?;
            for row in rows {
                seen.insert(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare("SELECT chat_id, id FROM messages")?;
            let rows = stmt.query_map([], |row| {
                Ok(format!(
                    "{}\0{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })?;
            for row in rows {
                seen.insert(row?);
            }
        }
    }
    Ok(seen)
}

pub(crate) fn new_message_rows(
    data_dir: &Path,
    chat: Option<&str>,
    seen: &HashSet<String>,
) -> Result<Vec<Value>> {
    let conn = open_existing_db(data_dir)?;
    let sql = match chat {
        Some(_) => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             WHERE chat_id = ?1
             ORDER BY created_at_secs ASC, id ASC"
        }
        None => {
            "SELECT chat_id, id, body, is_outgoing, created_at_secs, delivery
             FROM messages
             ORDER BY created_at_secs ASC, id ASC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
        Ok(json!({
            "chat_id": row.get::<_, String>(0)?,
            "id": row.get::<_, String>(1)?,
            "body": row.get::<_, String>(2)?,
            "is_outgoing": row.get::<_, i64>(3)? != 0,
            "created_at_secs": row.get::<_, i64>(4)?,
            "delivery": row.get::<_, String>(5)?,
        }))
    };
    let mut messages = Vec::new();
    match chat {
        Some(chat_id) => {
            let rows = stmt.query_map([chat_id], map_row)?;
            for row in rows {
                push_unseen_message(row?, seen, &mut messages);
            }
        }
        None => {
            let rows = stmt.query_map([], map_row)?;
            for row in rows {
                push_unseen_message(row?, seen, &mut messages);
            }
        }
    }
    Ok(messages)
}

pub(crate) fn latest_outgoing_message_row(
    data_dir: &Path,
    chat_id: &str,
    body: &str,
) -> Result<Option<Value>> {
    let conn = open_existing_db(data_dir)?;
    conn.query_row(
        "SELECT chat_id, id, body, is_outgoing, created_at_secs, expires_at_secs, delivery, source_event_id
         FROM messages
         WHERE chat_id = ?1 AND body = ?2 AND is_outgoing = 1
         ORDER BY created_at_secs DESC, id DESC
         LIMIT 1",
        (chat_id, body),
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(1)?,
                "chat_id": row.get::<_, String>(0)?,
                "author": Value::Null,
                "body": row.get::<_, String>(2)?,
                "is_outgoing": row.get::<_, i64>(3)? != 0,
                "created_at_secs": row.get::<_, i64>(4)?,
                "expires_at_secs": row.get::<_, Option<i64>>(5)?,
                "delivery": row.get::<_, String>(6)?,
                "source_event_id": row.get::<_, Option<String>>(7)?,
                "recipient_deliveries": [],
                "delivery_trace": {
                    "outer_event_ids": [],
                    "pending_relay_event_ids": [],
                    "queued_protocol_targets": [],
                    "target_device_ids": [],
                    "transport_channels": [],
                    "last_transport_error": Value::Null,
                },
                "attachments": [],
                "reactions": [],
            }))
        },
    )
    .optional()
    .map_err(Into::into)
}

fn push_unseen_message(message: Value, seen: &HashSet<String>, messages: &mut Vec<Value>) {
    let key = format!(
        "{}\0{}",
        message["chat_id"].as_str().unwrap_or_default(),
        message["id"].as_str().unwrap_or_default()
    );
    if !seen.contains(&key) {
        messages.push(message);
    }
}

fn open_existing_db(data_dir: &Path) -> Result<Connection> {
    let path = data_dir.join("core.sqlite3");
    Connection::open(path).context("Open Iris chat database")
}
