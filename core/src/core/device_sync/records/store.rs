use super::*;
use rusqlite::OptionalExtension;

impl AppCore {
    pub(in crate::core) fn sync_record_prefix(&self) -> Option<String> {
        let login = self.logged_in.as_ref()?;
        Some(format!(
            "iris-chat-sync-record-v1:{}:{}:",
            login.owner_pubkey.to_hex(),
            login.device_keys.public_key().to_hex()
        ))
    }
    // Account state survives replacing this installation's device key. Other
    // record projections retain their existing device/history ownership rules.
    fn record_storage_path(&self, key: &str) -> Option<String> {
        let parts: Vec<String> = serde_json::from_str(key).ok()?;
        if parts.first().is_some_and(|kind| kind == "privateBlock") {
            return Some(format!(
                "iris-chat-sync-record-v1:{}:private-state:{key}",
                self.logged_in.as_ref()?.owner_pubkey.to_hex()
            ));
        }
        Some(format!("{}{key}", self.sync_record_prefix()?))
    }
    pub(super) fn store_sync_record(&self, record: &DeviceSyncRecord) -> anyhow::Result<bool> {
        let key = self
            .record_storage_path(
                &record
                    .storage_key()
                    .ok_or_else(|| anyhow::anyhow!("Not a durable head"))?,
            )
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let intervals = if let DeviceSyncRecord::PrivateBlock { event } = record {
            Some(self.block_intervals_with_event(event)?)
        } else {
            None
        };
        let shared = self.app_store.shared();
        let mut conn = shared.lock().map_err(|_| anyhow::anyhow!("Storage lock"))?;
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
        }
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key,serde_json::to_string(record)?])?;
        if let Some(intervals) = intervals {
            let owner = self
                .logged_in
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No account"))?
                .owner_pubkey
                .to_hex();
            tx.execute("INSERT INTO app_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![format!("iris-chat-private-block-intervals-v1:{owner}"),intervals])?;
        }
        tx.commit()?;
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
    pub(in crate::core::device_sync) fn sync_record_page(
        &self,
        after: &str,
    ) -> anyhow::Result<Vec<(String, DeviceSyncRecord)>> {
        let prefix = self
            .sync_record_prefix()
            .ok_or_else(|| anyhow::anyhow!("No account"))?;
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?
            .owner_pubkey
            .to_hex();
        let owner_start = format!("iris-chat-sync-record-v1:{owner}:");
        let owner_end = format!("iris-chat-sync-record-v1:{owner};");
        let device_end = format!("{};", prefix.trim_end_matches(':'));
        let shared = self.app_store.shared();
        let conn = shared.lock().map_err(|_| anyhow::anyhow!("Storage lock"))?;
        let mut query = conn.prepare(
            "SELECT key,value FROM app_meta
             WHERE key>=?1 AND key<?2 AND key>?3
               AND ((key>=?4 AND key<?5)
                    OR CASE WHEN json_valid(value)
                        THEN json_extract(value, '$.type') IN ('messageMutation', 'privateBlock') END)
             ORDER BY key LIMIT 256",
        )?;
        let rows = query.query_map(
            rusqlite::params![owner_start, owner_end, after, prefix, device_end],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        rows.map(|row| {
            let (key, json) = row?;
            Ok((key, serde_json::from_str(&json)?))
        })
        .collect()
    }
    pub(in crate::core::device_sync) fn visit_sync_records(
        &self,
        mut visit: impl FnMut(DeviceSyncRecord) -> bool,
    ) -> anyhow::Result<()> {
        let mut after = String::new();
        loop {
            let page = self.sync_record_page(&after)?;
            if page.is_empty() {
                return Ok(());
            }
            for (key, record) in page {
                after = key;
                if !visit(record) {
                    return Ok(());
                }
            }
        }
    }
    pub(in crate::core::device_sync) fn load_sync_record(
        &self,
        locator: &RecordLocator,
    ) -> Option<DeviceSyncRecord> {
        match locator {
            RecordLocator::Group(id) => Some(DeviceSyncRecord::Group {
                group: DeviceSyncGroup::from_current(self, self.groups.get(id)?),
            }),
            RecordLocator::Head(key) => {
                let path = self.record_storage_path(key)?;
                let shared = self.app_store.shared();
                let conn = shared.lock().ok()?;
                let json: Option<String> = conn
                    .query_row("SELECT value FROM app_meta WHERE key=?1", [path], |row| {
                        row.get(0)
                    })
                    .optional()
                    .ok()?;
                if let Some(json) = json {
                    return serde_json::from_str(&json).ok();
                }
                drop(conn);
                let parts: Vec<String> = serde_json::from_str(key).ok()?;
                let [kind, chat, target, _] = parts.as_slice() else {
                    return None;
                };
                if kind != "messageMutation" {
                    return None;
                }
                self.message_mutation_records_result(chat, target)
                    .ok()?
                    .into_iter()
                    .map(|mutation| DeviceSyncRecord::MessageMutation { mutation })
                    .find(|record| record.storage_key().as_deref() == Some(key.as_str()))
            }
        }
    }
    pub(in crate::core::device_sync) fn sync_record_is_stored(
        &self,
        record: &DeviceSyncRecord,
    ) -> bool {
        let Some(path) = record
            .storage_key()
            .and_then(|key| self.record_storage_path(&key))
        else {
            return false;
        };
        let shared = self.app_store.shared();
        let Ok(conn) = shared.lock() else {
            return false;
        };
        let json: Option<String> = conn
            .query_row("SELECT value FROM app_meta WHERE key=?1", [path], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten();
        json.and_then(|json| serde_json::from_str::<DeviceSyncRecord>(&json).ok())
            .is_some_and(|stored| stored.id() == record.id() || stored.wins(record))
    }

    #[cfg(test)]
    pub(in crate::core) fn export_sync_record_values_for_test(
        &self,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let mut records = Vec::new();
        self.visit_sync_records(|record| {
            records.push(record);
            true
        })?;
        records
            .into_iter()
            .map(|record| {
                let locator = record
                    .locator()
                    .ok_or_else(|| anyhow::anyhow!("Missing locator"))?;
                anyhow::ensure!(
                    self.load_sync_record(&locator).as_ref() == Some(&record),
                    "History locator must resolve the inventoried record"
                );
                Ok(serde_json::to_value(record)?)
            })
            .collect()
    }
}

impl AppCore {
    pub(in crate::core) fn message_mutation_records(
        &self,
        chat: &str,
        message: &str,
    ) -> Vec<MessageMutation> {
        self.message_mutation_records_result(chat, message)
            .unwrap_or_default()
    }
    pub(in crate::core) fn message_mutation_records_result(
        &self,
        chat: &str,
        message: &str,
    ) -> anyhow::Result<Vec<MessageMutation>> {
        let owner = self
            .logged_in
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No account"))?
            .owner_pubkey
            .to_hex();
        // Restoring the account from its secret key creates a fresh device
        // identity while retaining this account's history and control records.
        let start = format!("iris-chat-sync-record-v1:{owner}:");
        let end = format!("iris-chat-sync-record-v1:{owner};");
        let shared = self.app_store.shared();
        let conn = shared.lock().map_err(|_| anyhow::anyhow!("Storage lock"))?;
        let mut query = conn.prepare(
            "SELECT value FROM app_meta WHERE key>=?1 AND key<?2
               AND json_extract(CASE WHEN json_valid(value) THEN value END, '$.type') = 'messageMutation'
               AND json_type(CASE WHEN json_valid(value) THEN value END, '$.mutation') = 'object'
               AND json_extract(CASE WHEN json_valid(value) THEN value END, '$.mutation.chatId') = ?3
               AND json_extract(CASE WHEN json_valid(value) THEN value END, '$.mutation.messageId') = ?4
             ORDER BY key",
        )?;
        let rows = query.query_map(rusqlite::params![start, end, chat, message], |row| {
            row.get::<_, String>(0)
        })?;
        let mut records = std::collections::BTreeMap::new();
        for row in rows {
            if let DeviceSyncRecord::MessageMutation { mutation } = serde_json::from_str(&row?)? {
                if let Some(previous) = records.insert(mutation.id.clone(), mutation.clone()) {
                    anyhow::ensure!(previous == mutation, "Conflicting stored message mutation");
                }
            }
        }
        Ok(records.into_values().collect())
    }
}
