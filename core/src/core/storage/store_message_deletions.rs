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
