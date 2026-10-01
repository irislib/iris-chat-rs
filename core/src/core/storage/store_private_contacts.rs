use super::super::account::validate_account_storage;
use super::AppStore;
use rusqlite::{params, OptionalExtension};

const STATE_KEY: &str = "private_contact_sync_v2";

impl AppStore {
    pub(crate) fn load_private_contact_sync(&self, owner: &str) -> anyhow::Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        validate_account_storage(&conn, owner)?;
        Ok(conn
            .query_row(
                "SELECT value FROM app_meta WHERE key IN (?1, 'private_contact_sync_v1') ORDER BY key DESC LIMIT 1",
                [STATE_KEY],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// This record is the authority. Save it before updating profile projections
    /// or transmitting any encrypted event, including retries after restart.
    pub(crate) fn save_private_contact_sync(&self, owner: &str, json: &str) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        let tx = conn.transaction()?;
        validate_account_storage(&tx, owner)?;
        tx.execute("INSERT INTO app_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![STATE_KEY, json])?;
        tx.execute(
            "DELETE FROM app_meta WHERE key = 'private_contact_sync_v1'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}
