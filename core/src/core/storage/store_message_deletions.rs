use super::*;

// Replay guards are account-local. Explicit deletions also store a private
// sync record. Expiry and legacy guards have no record to export.
fn deletion_key(chat_id: &str, kind: &str, id: &str) -> String {
    format!("message_deleted:{}:{chat_id}:{kind}:{id}", chat_id.len())
}

/// A verified device alias and its canonical owner share exact replay guards.
/// Retain the alias keys as well: an old queued delivery may still use that ID.
pub(super) fn copy_alias_markers(tx: &Transaction, from: &str, to: &str) -> anyhow::Result<()> {
    for kind in ["id", "source"] {
        let prefix = deletion_key(from, kind, "");
        let end = format!("{};", prefix.trim_end_matches(':'));
        tx.execute(
            "INSERT OR IGNORE INTO app_meta(key,value)
            SELECT ?3 || substr(key,length(?1)+1),value FROM app_meta
            WHERE key>=?1 AND key<?2",
            params![prefix, end, deletion_key(to, kind, "")],
        )?;
    }
    Ok(())
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

/// Reconcile copies which arrived under the owner before its device alias was
/// known. Keep all work in the caller's migration transaction and in SQLite.
pub(super) fn purge_marked_alias_copies(tx: &Transaction, chat_id: &str) -> anyhow::Result<()> {
    let prefix = format!("message_deleted:{}:{chat_id}:", chat_id.len());
    let marked = "EXISTS (SELECT 1 FROM app_meta WHERE key IN (
        ?2 || 'id:' || m.id, ?2 || 'source:' || m.source_event_id))";
    let present: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM messages AS m WHERE chat_id=?1 AND {marked})"),
        params![chat_id, prefix],
        |row| row.get(0),
    )?;
    if !present {
        return Ok(());
    }

    // A stored copy can have a different local ID or outer event. Guard both
    // identifiers before erasing it so re-encapsulation cannot bring it back.
    // Source guards first also find any other local IDs for that exact event.
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO app_meta(key,value)
        SELECT ?2 || 'source:' || m.source_event_id,'1' FROM messages AS m
        WHERE chat_id=?1 AND m.source_event_id IS NOT NULL AND {marked}"
        ),
        params![chat_id, prefix],
    )?;
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO app_meta(key,value)
        SELECT ?2 || 'id:' || m.id,'1' FROM messages AS m
        WHERE chat_id=?1 AND {marked}"
        ),
        params![chat_id, prefix],
    )?;
    let removed = format!(
        "removed AS (SELECT m.id,m.source_event_id FROM messages AS m
        WHERE chat_id=?1 AND {marked})"
    );
    tx.execute(
        &format!(
            "{MESSAGE_CONTROL_RECORDS}, {removed}
        DELETE FROM app_meta WHERE key IN (SELECT key FROM message_controls
            WHERE json_extract(record,'$.chatId')=?1 AND (
                json_extract(record,'$.messageId') IN (SELECT id FROM removed)
                OR json_extract(record,'$.messageId') IN (SELECT source_event_id FROM removed)
                OR EXISTS(SELECT 1 FROM app_meta AS guard WHERE guard.key IN (
                    ?2 || 'id:' || json_extract(record,'$.messageId'),
                    ?2 || 'source:' || json_extract(record,'$.messageId')))))"
        ),
        params![chat_id, prefix],
    )?;
    // Only the recent counted unread window contributes to the existing badge.
    tx.execute(
        &format!(
            "WITH {removed}
        UPDATE threads SET unread_count=MAX(0,unread_count-(SELECT count(*) FROM (
            SELECT id FROM messages WHERE chat_id=?1 AND is_outgoing=0 AND delivery!='seen'
            ORDER BY created_at_secs DESC,rowid DESC
            LIMIT (SELECT MAX(0,unread_count) FROM threads WHERE chat_id=?1)
        ) WHERE id IN (SELECT id FROM removed))) WHERE chat_id=?1"
        ),
        params![chat_id, prefix],
    )?;
    tx.execute(
        "DELETE FROM pending_message_reads WHERE chat_id=?1 AND EXISTS(
        SELECT 1 FROM app_meta WHERE key=?2 || 'id:' || pending_message_reads.id)",
        params![chat_id, prefix],
    )?;
    // Existing DELETE triggers erase the matching FTS and envelope rows.
    tx.execute(
        &format!("DELETE FROM messages AS m WHERE chat_id=?1 AND {marked}"),
        params![chat_id, prefix],
    )?;
    Ok(())
}

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

    #[cfg(test)]
    pub(crate) fn delete_message_locally(
        &mut self,
        chat_id: &str,
        message_id: &str,
        source_event_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.remove_message_for_me(chat_id, message_id, source_event_id, None, None)
            .map(|_| ())
    }

    pub(crate) fn remove_message_for_me(
        &mut self,
        chat_id: &str,
        message_id: &str,
        source_event_id: Option<&str>,
        thread: Option<&ThreadRecord>,
        sync_entry: Option<(String, String)>,
    ) -> anyhow::Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        // Pending UI/relay updates may still exist only in memory. Include
        // unsaved rows in the same transaction before deciding which row owns a badge.
        let thread_plan = match thread.filter(|thread| thread.unread_count > 0) {
            Some(thread) => {
                upsert_unsaved_thread_rows(&tx, self.cache.threads.get(chat_id), chat_id, thread)?
            }
            None => None,
        };
        if let Some((key, value)) = sync_entry {
            tx.execute(
                "INSERT OR IGNORE INTO app_meta(key,value) VALUES (?1,?2)",
                params![key, value],
            )?;
        }
        // The badge counts the most recent unread rows. Removing an older row
        // whose badge was already cleared must not decrement it again.
        let removed_unread = tx.execute(
            "UPDATE threads SET unread_count = unread_count - 1
             WHERE chat_id = ?1 AND unread_count > 0 AND ?2 IN (
                SELECT id FROM messages WHERE chat_id = ?1 AND is_outgoing = 0 AND delivery != 'seen'
                ORDER BY created_at_secs DESC, rowid DESC
                LIMIT (SELECT unread_count FROM threads WHERE chat_id = ?1))",
            params![chat_id, message_id],
        )? > 0;
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
            "DELETE FROM pending_message_reads WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )?;
        tx.commit()?;
        if let Some(plan) = thread_plan {
            let entry = self.cache.threads.entry(chat_id.to_string()).or_default();
            plan.apply_to_cache(entry);
        }
        if let Some(entry) = self.cache.threads.get_mut(chat_id) {
            entry.header_hash = None;
            entry.messages.remove(message_id);
        }
        Ok(removed_unread)
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
        let entry = self
            .cache
            .threads
            .entry(message.chat_id.clone())
            .or_default();
        entry
            .messages
            .insert(message.id.clone(), hash_message(message));
        Ok(())
    }
}
