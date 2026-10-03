use super::*;
use rusqlite::OptionalExtension;

impl AppCore {
    pub(super) fn sync_record_prefix(&self) -> Option<String> {
        let login = self.logged_in.as_ref()?;
        Some(format!(
            "iris-chat-sync-record-v1:{}:{}:",
            login.owner_pubkey.to_hex(),
            login.device_keys.public_key().to_hex()
        ))
    }
    pub(super) fn store_sync_record(&self, record: &DeviceSyncRecord) -> anyhow::Result<bool> {
        let prefix = self
            .sync_record_prefix()
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let key = format!(
            "{prefix}{}",
            record
                .storage_key()
                .ok_or_else(|| anyhow::anyhow!("Not a durable head"))?
        );
        let shared = self.app_store.shared();
        let conn = shared.lock().map_err(|_| anyhow::anyhow!("Storage lock"))?;
        let previous: Option<String> = conn
            .query_row("SELECT value FROM app_meta WHERE key=?1", [&key], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(previous) = previous {
            let previous: DeviceSyncRecord = serde_json::from_str(&previous)?;
            if previous.id() == record.id() {
                return Ok(previous == *record);
            }
            if !record.wins(&previous) {
                return Ok(false);
            }
        } else {
            let count: u64 = conn.query_row(
                "SELECT count(*) FROM app_meta WHERE key LIKE ?1",
                [format!("{prefix}%")],
                |row| row.get(0),
            )?;
            anyhow::ensure!(count < 100_000, "Record cache full");
        }
        conn.execute("INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key,serde_json::to_string(record)?])?;
        Ok(true)
    }
    pub(super) fn reaction_records_for_message(
        &self,
        chat: &str,
        message: &str,
    ) -> Vec<DeviceSyncRecord> {
        let Some(prefix) = self.sync_record_prefix() else {
            return Vec::new();
        };
        let key = serde_json::json!(["reaction", chat, message]).to_string();
        let start = format!("{prefix}{},", key.trim_end_matches(']'));
        let end = format!("{start}\u{10ffff}");
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else {
            return Vec::new();
        };
        let Ok(mut query) = conn.prepare(
            "SELECT value FROM app_meta WHERE key>=?1 AND key<?2 ORDER BY key LIMIT 100001",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = query.query_map([start, end], |row| row.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect()
    }
    pub(in crate::core::device_sync) fn stored_sync_records(&self) -> Vec<DeviceSyncRecord> {
        let Some(prefix) = self.sync_record_prefix() else {
            return Vec::new();
        };
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else {
            return Vec::new();
        };
        let Ok(mut query) =
            conn.prepare("SELECT value FROM app_meta WHERE key LIKE ?1 ORDER BY key LIMIT 100001")
        else {
            return Vec::new();
        };
        let Ok(rows) = query.query_map([format!("{prefix}%")], |row| row.get::<_, String>(0))
        else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect()
    }
    pub(in crate::core::device_sync) fn sync_record_is_stored(
        &self,
        record: &DeviceSyncRecord,
    ) -> bool {
        let Some(prefix) = self.sync_record_prefix() else {
            return false;
        };
        let Some(key) = record.storage_key() else {
            return false;
        };
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else {
            return false;
        };
        let json: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key=?1",
                [format!("{prefix}{key}")],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        json.and_then(|json| serde_json::from_str::<DeviceSyncRecord>(&json).ok())
            .is_some_and(|stored| stored.id() == record.id() || stored.wins(record))
    }
}
