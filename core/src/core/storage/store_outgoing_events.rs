use super::*;
use nostr::UnsignedEvent;

impl AppStore {
    pub(crate) fn retire_pending_direct_publishes(
        &mut self,
        owner: &str,
        chat: &str,
        through: Option<u64>,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        // Ciphertext can be prepared long after its plaintext was authored.
        // Legacy controls without either timestamp retire conservatively;
        // their envelope creation time cannot prove post-unblock authorship.
        let ids = {
            let mut stmt = tx.prepare("SELECT event_id FROM pending_relay_publishes AS pending
                WHERE owner_pubkey_hex = ?1 AND chat_id = ?2 AND (?3 IS NULL OR
                    COALESCE(pending.authored_at_secs, (SELECT created_at_secs FROM messages
                        WHERE messages.chat_id = pending.chat_id AND messages.id = pending.inner_event_id),
                        0) <= ?3)")?;
            let rows = stmt.query_map(
                params![
                    owner,
                    chat,
                    through.map(|at| at.min(i64::MAX as u64) as i64)
                ],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<String>, _>>()?
        };
        for id in &ids {
            tx.execute(
                "DELETE FROM pending_relay_publishes WHERE event_id = ?1",
                [id],
            )?;
        }
        // Plaintext can still be waiting for device discovery. Retire it in
        // the same transaction so an unblock cannot prepare it anew.
        tx.execute(
            "UPDATE messages SET delivery = 'failed', outgoing_event_json = NULL
            WHERE chat_id = ?1 AND is_outgoing != 0 AND delivery = 'queued'
                AND (?2 IS NULL OR created_at_secs <= ?2)",
            params![chat, through.map(|at| at.min(i64::MAX as u64) as i64)],
        )?;
        tx.commit()?;
        self.cache.threads.remove(chat);
        Ok(ids)
    }

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
