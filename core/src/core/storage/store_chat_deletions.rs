use super::*;

const DELETION_PREFIX: &str = "chat_deleted_at:";

impl AppStore {
    pub(crate) fn load_chat_deletions(&self) -> anyhow::Result<BTreeMap<String, u64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let mut statement =
            conn.prepare("SELECT key, value FROM app_meta WHERE key LIKE 'chat_deleted_at:%'")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut deletions = BTreeMap::new();
        for row in rows {
            let (key, value) = row?;
            if let Some(chat_id) = key.strip_prefix(DELETION_PREFIX) {
                deletions.insert(chat_id.to_string(), value.parse()?);
            }
        }
        Ok(deletions)
    }

    pub(crate) fn apply_chat_deletion(
        &mut self,
        chat_id: &str,
        deleted_at: u64,
        keep_thread: bool,
        keep_group: bool,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("{DELETION_PREFIX}{chat_id}"), deleted_at.to_string()])?;
        tx.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND created_at_secs <= ?2",
            params![chat_id, deleted_at as i64],
        )?;
        if !keep_thread {
            tx.execute("DELETE FROM threads WHERE chat_id = ?1", [chat_id])?;
            tx.execute(
                "DELETE FROM chat_message_ttls WHERE chat_id = ?1",
                [chat_id],
            )?;
            tx.execute(
                "DELETE FROM app_meta WHERE key = ?1 AND value = ?2",
                params![META_ACTIVE_CHAT_ID, chat_id],
            )?;
        }
        if !keep_group {
            if let Some(group_id) = chat_id.strip_prefix("group:") {
                tx.execute("DELETE FROM groups WHERE group_id = ?1", [group_id])?;
            }
        }
        tx.commit()?;
        self.cache.threads.remove(chat_id);
        self.cache.meta = None;
        self.cache.groups = None;
        self.cache.chat_ttls = None;
        Ok(())
    }
}

pub(super) fn message_was_deleted(
    conn: &rusqlite::Connection,
    chat_id: &str,
    created_at: u64,
) -> anyhow::Result<bool> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [format!("{DELETION_PREFIX}{chat_id}")],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value
        .map(|at| at.parse::<u64>())
        .transpose()?
        .is_some_and(|at| created_at <= at))
}
