use super::*;
use nostr::UnsignedEvent;

impl AppStore {
    /// Save the authored event with its visible row before attempting delivery.
    /// Ordinary snapshot writes preserve this column, so retries after restart
    /// use exactly the same event (including its millisecond timestamp and ID).
    pub(crate) fn save_outgoing_event(
        &mut self,
        thread: &ThreadRecord,
        message: &ChatMessageSnapshot,
        event: &UnsignedEvent,
    ) -> anyhow::Result<()> {
        let event_json = serde_json::to_string(event)?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO threads(chat_id, unread_count, updated_at_secs, draft)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat_id) DO UPDATE SET
                unread_count = excluded.unread_count,
                updated_at_secs = excluded.updated_at_secs,
                draft = excluded.draft",
            params![
                thread.chat_id,
                thread.unread_count as i64,
                thread.updated_at_secs as i64,
                thread.draft
            ],
        )?;
        upsert_message_row(&tx, &thread.chat_id, message)?;
        tx.execute(
            "UPDATE messages SET outgoing_event_json = ?3 WHERE chat_id = ?1 AND id = ?2",
            params![thread.chat_id, message.id, event_json],
        )?;
        tx.commit()?;
        self.cache.threads.remove(&thread.chat_id);
        Ok(())
    }

    pub(crate) fn load_outgoing_event(
        &self,
        chat_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Option<UnsignedEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let json: Option<String> = conn
            .query_row(
                "SELECT outgoing_event_json FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        json.map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }
}
