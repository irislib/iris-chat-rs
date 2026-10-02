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

impl AppStore {
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
