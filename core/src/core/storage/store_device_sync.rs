use super::*;

impl AppStore {
    pub(crate) fn load_device_sync_messages_page(
        &self,
        cutoff_secs: u64,
        now_secs: u64,
        after_created_at: u64,
        after_chat_id: &str,
        after_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<PersistedMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT chat_id, id, kind, author, author_owner_pubkey_hex, body, is_outgoing,
                    created_at_secs, expires_at_secs, delivery, attachments_json, reactions_json,
                    reactors_json, source_event_id, recipient_deliveries_json, delivery_trace_json, call_json, system_notice_owner_pubkey_hex
             FROM messages
             WHERE kind = 'user' AND created_at_secs >= ?1
               AND (delivery NOT IN ('queued', 'pending', 'failed')
                    OR (delivery IN ('queued', 'pending') AND is_outgoing = 1
                        AND chat_id = author_owner_pubkey_hex AND body LIKE 'iris-direct-file-v1:%'))
               AND (expires_at_secs IS NULL OR expires_at_secs > ?2)
               AND (created_at_secs > ?3
                    OR (created_at_secs = ?3 AND chat_id > ?4)
                    OR (created_at_secs = ?3 AND chat_id = ?4 AND id > ?5))
             ORDER BY created_at_secs, chat_id, id
             LIMIT ?6",
        )?;
        let rows = stmt.query_map(
            params![
                cutoff_secs as i64,
                now_secs as i64,
                after_created_at as i64,
                after_chat_id,
                after_id,
                limit as i64,
            ],
            persisted_message_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
