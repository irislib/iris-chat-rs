use super::AppStore;
use crate::core::{DiscoveredUserRecord, UserDiscoveryCache};
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeMap;

impl AppStore {
    pub(crate) fn load_user_discovery(&self) -> anyhow::Result<UserDiscoveryCache> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let (follow_event_id, follow_created_at_secs) = conn
            .query_row(
                "SELECT follow_event_id, follow_created_at_secs
                 FROM user_discovery_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)? as u64,
                    ))
                },
            )
            .optional()?
            .unwrap_or((None, 0));
        let mut users = BTreeMap::new();
        let mut stmt = conn.prepare(
            "SELECT owner_pubkey_hex, follow_position, petname,
                    app_keys_created_at_secs, app_keys_event_id, app_keys_event_json
             FROM user_discovery_users
             ORDER BY follow_position, owner_pubkey_hex",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DiscoveredUserRecord {
                owner_pubkey_hex: row.get(0)?,
                follow_position: row.get::<_, i64>(1)? as u32,
                petname: row.get(2)?,
                app_keys_created_at_secs: row.get::<_, i64>(3)? as u64,
                app_keys_event_id: row.get(4)?,
                app_keys_event_json: row.get(5)?,
            })
        })?;
        for row in rows {
            let row = row?;
            users.insert(row.owner_pubkey_hex.clone(), row);
        }
        Ok(UserDiscoveryCache {
            follow_event_id,
            follow_created_at_secs,
            users,
        })
    }

    pub(crate) fn replace_user_discovery(
        &mut self,
        cache: &UserDiscoveryCache,
    ) -> anyhow::Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM user_discovery_users", [])?;
        tx.execute("DELETE FROM user_discovery_state", [])?;
        tx.execute(
            "INSERT INTO user_discovery_state(
                 id, follow_event_id, follow_created_at_secs
             ) VALUES (1, ?1, ?2)",
            params![cache.follow_event_id, cache.follow_created_at_secs as i64],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO user_discovery_users(
                     owner_pubkey_hex, follow_position, petname,
                     app_keys_created_at_secs, app_keys_event_id, app_keys_event_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for user in cache.users.values() {
                stmt.execute(params![
                    user.owner_pubkey_hex,
                    user.follow_position as i64,
                    user.petname,
                    user.app_keys_created_at_secs as i64,
                    user.app_keys_event_id,
                    user.app_keys_event_json,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
