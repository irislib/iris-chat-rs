use super::*;

// This database belongs to the active account. These markers stay on this
// install; they neither retract published events nor sync deletion to siblings.
fn deletion_key(chat_id: &str, kind: &str, id: &str) -> String {
    format!("message_deleted:{}:{chat_id}:{kind}:{id}", chat_id.len())
}

pub(super) fn contains(
    conn: &rusqlite::Connection,
    chat_id: &str,
    message_id: Option<&str>,
    source_event_id: Option<&str>,
) -> anyhow::Result<bool> {
    let message_key = message_id.map(|id| deletion_key(chat_id, "id", id));
    let source_key = source_event_id.map(|id| deletion_key(chat_id, "source", id));
    Ok(conn
        .prepare_cached("SELECT 1 FROM app_meta WHERE key IN (?1, ?2) LIMIT 1")?
        .query_row(params![message_key, source_key], |_| Ok(()))
        .optional()?
        .is_some())
}

// Restrict cleanup to typed, well-formed message controls. Other app metadata
// (including malformed legacy values) must not break deletion or be removed.
const MESSAGE_CONTROL_RECORDS: &str = "WITH message_controls AS (
    SELECT key, CASE WHEN json_valid(value) THEN CASE
        WHEN json_extract(value, '$.type') = 'messageMutation'
             AND json_type(value, '$.mutation') = 'object'
            THEN json_extract(value, '$.mutation')
        WHEN json_extract(value, '$.type') = 'reaction'
             AND json_type(value, '$.reaction') = 'object'
            THEN json_extract(value, '$.reaction')
        END END AS record,
        CASE WHEN json_valid(value) THEN json_extract(value, '$.type') END AS kind
    FROM app_meta
    WHERE key >= 'iris-chat-sync-record-v1:' AND key < 'iris-chat-sync-record-v1;'
) ";

fn purge_message_controls(
    conn: &rusqlite::Connection,
    chat_id: &str,
    message_id: &str,
    source_event_id: Option<&str>,
    stored_source_event_id: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "{MESSAGE_CONTROL_RECORDS} DELETE FROM app_meta WHERE key IN (
            SELECT key FROM message_controls
            WHERE json_extract(record, '$.chatId') = ?1
              AND json_extract(record, '$.messageId') IN (?2, ?3, ?4)
        )"
        ),
        params![chat_id, message_id, source_event_id, stored_source_event_id],
    )?;
    Ok(())
}

pub(super) fn next_control_expiration_after(
    conn: &rusqlite::Connection,
    now_secs: u64,
) -> anyhow::Result<Option<u64>> {
    let expires_at = conn.query_row(
        &format!(
            "{MESSAGE_CONTROL_RECORDS}
            SELECT MIN(json_extract(record, '$.expiresAt')) FROM message_controls
            WHERE json_type(record, '$.expiresAt') = 'integer'
              AND json_extract(record, '$.expiresAt') > ?1"
        ),
        [now_secs as i64],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    Ok(expires_at.map(|seconds| seconds as u64))
}

pub(super) fn purge_expired_controls(
    conn: &rusqlite::Connection,
    now_secs: u64,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "{MESSAGE_CONTROL_RECORDS} DELETE FROM app_meta WHERE key IN (
            SELECT key FROM message_controls
            WHERE (json_type(record, '$.expiresAt') = 'integer'
                   AND json_extract(record, '$.expiresAt') <= ?1)
               OR EXISTS (
                   SELECT 1 FROM messages
                   WHERE messages.chat_id = json_extract(record, '$.chatId')
                     AND (messages.id = json_extract(record, '$.messageId')
                          OR messages.source_event_id = json_extract(record, '$.messageId'))
                     AND messages.expires_at_secs IS NOT NULL
                     AND messages.expires_at_secs <= ?1
               )
        )"
        ),
        [now_secs as i64],
    )?;
    Ok(())
}

pub(super) fn purge_deleted_chat_controls(
    conn: &rusqlite::Connection,
    chat_id: &str,
    deleted_at: u64,
    keep_thread: bool,
) -> anyhow::Result<()> {
    // A delayed chat deletion can leave newer messages in the same chat.
    // Keep their versions, but remove edits of deleted targets even when the
    // edits themselves arrived after the deletion cutoff.
    conn.execute(
        &format!(
            "{MESSAGE_CONTROL_RECORDS} DELETE FROM app_meta WHERE key IN (
            SELECT key FROM message_controls
            WHERE json_extract(record, '$.chatId') = ?1
              AND (EXISTS (
                  SELECT 1 FROM messages
                  WHERE messages.chat_id = ?1
                    AND (messages.id = json_extract(record, '$.messageId')
                         OR messages.source_event_id = json_extract(record, '$.messageId'))
                    AND (NOT ?3 OR messages.created_at_secs <= ?2)
              ) OR (json_extract(record, '$.createdAt') <= ?2 AND NOT EXISTS (
                  SELECT 1 FROM messages
                  WHERE messages.chat_id = ?1
                    AND (messages.id = json_extract(record, '$.messageId')
                         OR messages.source_event_id = json_extract(record, '$.messageId'))
              )))
        )"
        ),
        params![chat_id, deleted_at as i64, keep_thread],
    )?;
    Ok(())
}

pub(super) fn mark_expired(conn: &rusqlite::Connection, now_secs: u64) -> anyhow::Result<()> {
    // Match deletion_key's UTF-8 byte length, including non-ASCII group IDs.
    for (column, kind) in [("id", "id"), ("source_event_id", "source")] {
        conn.execute(&format!("INSERT OR IGNORE INTO app_meta(key,value)
            SELECT 'message_deleted:' || length(CAST(chat_id AS BLOB)) || ':' || chat_id || ':{kind}:' || {column}, '1'
            FROM messages WHERE expires_at_secs IS NOT NULL AND expires_at_secs <= ?1 AND {column} IS NOT NULL"), [now_secs as i64])?;
    }
    Ok(())
}

impl AppStore {
    pub(crate) fn deleted_message_ids(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut query =
            conn.prepare("SELECT key FROM app_meta WHERE key LIKE 'message_deleted:%' LIMIT ?1")?;
        let keys = query.query_map([limit as i64], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for key in keys {
            let key = key?;
            let Some((len, suffix)) = key
                .strip_prefix("message_deleted:")
                .and_then(|value| value.split_once(':'))
            else {
                continue;
            };
            let Ok(len) = len.parse::<usize>() else {
                continue;
            };
            let Some(chat) = suffix.get(..len) else {
                continue;
            };
            let Some(id) = suffix
                .get(len..)
                .and_then(|value| value.strip_prefix(":id:"))
            else {
                continue;
            };
            result.push((chat.to_string(), id.to_string()));
        }
        Ok(result)
    }

    pub(crate) fn delete_message_locally(
        &mut self,
        chat_id: &str,
        message_id: &str,
        source_event_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        let stored_source: Option<String> = tx
            .query_row(
                "SELECT source_event_id FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let mut keys = vec![deletion_key(chat_id, "id", message_id)];
        for source in source_event_id.into_iter().chain(stored_source.as_deref()) {
            keys.push(deletion_key(chat_id, "source", source));
        }
        for key in keys {
            tx.execute(
                "INSERT OR IGNORE INTO app_meta(key, value) VALUES (?1, '1')",
                [key],
            )?;
        }
        purge_message_controls(
            &tx,
            chat_id,
            message_id,
            source_event_id,
            stored_source.as_deref(),
        )?;
        tx.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )?;
        tx.commit()?;
        self.cache.threads.remove(chat_id);
        Ok(())
    }

    pub(crate) fn message_was_locally_deleted(
        &self,
        chat_id: &str,
        message_id: Option<&str>,
        source_event_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        contains(&conn, chat_id, message_id, source_event_id)
    }

    /// Replay is already handled when its row exists or the user removed it.
    /// Keep `message_exists` as a physical-row query for storage consumers.
    pub(crate) fn message_exists_or_deleted(
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
        let message_key = message_id.map(|id| deletion_key(chat_id, "id", id));
        let source_key = source_event_id.map(|id| deletion_key(chat_id, "source", id));
        let exists = conn
            .prepare_cached(
                "SELECT 1 FROM app_meta WHERE key IN (?4, ?5)
             UNION ALL SELECT 1 FROM messages WHERE chat_id = ?1
               AND ((?2 IS NOT NULL AND id = ?2)
                    OR (?3 IS NOT NULL AND source_event_id = ?3)) LIMIT 1",
            )?
            .query_row(
                params![
                    chat_id,
                    message_id,
                    source_event_id,
                    message_key,
                    source_key
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }
}

impl AppStore {
    /// Keep the visible row, FTS, and removal of old plaintext versions atomic.
    pub(crate) fn save_message_mutation_projection(
        &mut self,
        message: &ChatMessageSnapshot,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Storage lock"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO threads(chat_id,updated_at_secs) VALUES (?1,?2)",
            params![message.chat_id, message.created_at_secs as i64],
        )?;
        upsert_message_row(&tx, &message.chat_id, message)?;
        if message.deleted_for_everyone {
            tx.execute(
                "UPDATE messages SET outgoing_event_json=NULL WHERE chat_id=?1 AND id=?2",
                params![message.chat_id, message.id],
            )?;
            // Account databases can retain several device namespaces. Purge
            // every copy of this target's plaintext while keeping deletion
            // heads so stale history cannot resurrect the content.
            tx.execute(
                &format!(
                    "{MESSAGE_CONTROL_RECORDS} DELETE FROM app_meta WHERE key IN (
                    SELECT key FROM message_controls
                    WHERE json_extract(record, '$.chatId') = ?1
                      AND json_extract(record, '$.messageId') IN (?2, ?3)
                      AND (kind = 'reaction' OR json_extract(record, '$.operation') = 'edit')
                )"
                ),
                params![message.chat_id, message.id, message.source_event_id],
            )?;
        }
        tx.commit()?;
        self.cache.threads.remove(&message.chat_id);
        Ok(())
    }
}
