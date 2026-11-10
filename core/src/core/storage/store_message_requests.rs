use super::*;

impl AppStore {
    pub(crate) fn record_request_arrival(&self, chat: &str, now: u64) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Storage lock"))?;
        // Local receive order, not the sender's clock. An existing sender never
        // bumps its position by sending more messages.
        conn.execute(
            "INSERT OR IGNORE INTO message_request_arrivals(chat_id, received_at_ms)
            SELECT ?1, MAX(?2, COALESCE(MAX(received_at_ms), 0) + 1) FROM message_request_arrivals",
            params![chat, now],
        )?;
        Ok(())
    }

    pub(crate) fn request_arrivals(&self) -> anyhow::Result<Vec<(String, u64)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Storage lock"))?;
        let mut query =
            conn.prepare("SELECT chat_id, received_at_ms FROM message_request_arrivals")?;
        let rows = query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn request_message_usage(&self, chat: &str) -> anyhow::Result<(usize, usize)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Storage lock"))?;
        Ok(conn.query_row("SELECT COUNT(*), COALESCE(SUM(length(CAST(body AS BLOB)) + length(CAST(attachments_json AS BLOB))), 0)
            FROM messages WHERE chat_id=?1", [chat], |row| Ok((row.get(0)?, row.get(1)?)))?)
    }
}
