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
        let (owner_pubkey_hex, follow_event_id, follow_created_at_secs, social_rank_ready) = conn
            .query_row(
                "SELECT owner_pubkey_hex, follow_event_id, follow_created_at_secs,
                        social_rank_ready
                 FROM user_discovery_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?
            .unwrap_or((None, None, 0, false));
        let mut users = BTreeMap::new();
        let mut stmt = conn.prepare(
            "SELECT owner_pubkey_hex, follow_position, petname
             FROM user_discovery_users
             ORDER BY follow_position, owner_pubkey_hex",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DiscoveredUserRecord {
                owner_pubkey_hex: row.get(0)?,
                follow_position: row.get::<_, i64>(1)? as u32,
                petname: row.get(2)?,
            })
        })?;
        for row in rows {
            let row = row?;
            users.insert(row.owner_pubkey_hex.clone(), row);
        }
        let mut social_friend_support = BTreeMap::new();
        let mut stmt = conn.prepare(
            "SELECT target_owner_pubkey_hex, friend_support
             FROM user_discovery_social WHERE account_owner_pubkey_hex = ?1",
        )?;
        let rows = stmt.query_map([owner_pubkey_hex.as_deref().unwrap_or_default()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u16>(1)?))
        })?;
        for row in rows {
            let (owner, support) = row?;
            social_friend_support.insert(owner, support);
        }
        Ok(UserDiscoveryCache {
            owner_pubkey_hex,
            follow_event_id,
            follow_created_at_secs,
            users,
            social_rank_ready,
            social_friend_support,
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
        tx.execute("DELETE FROM user_discovery_social", [])?;
        tx.execute("DELETE FROM user_discovery_users", [])?;
        tx.execute("DELETE FROM user_discovery_state", [])?;
        tx.execute(
            "INSERT INTO user_discovery_state(
                 id, owner_pubkey_hex, follow_event_id, follow_created_at_secs,
                 social_rank_ready
             ) VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                cache.owner_pubkey_hex,
                cache.follow_event_id,
                cache.follow_created_at_secs as i64,
                cache.social_rank_ready,
            ],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO user_discovery_users(owner_pubkey_hex, follow_position, petname)
                 VALUES (?1, ?2, ?3)",
            )?;
            for user in cache.users.values() {
                stmt.execute(params![
                    user.owner_pubkey_hex,
                    user.follow_position as i64,
                    user.petname,
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO user_discovery_social(
                     account_owner_pubkey_hex, target_owner_pubkey_hex, friend_support
                 ) VALUES (?1, ?2, ?3)",
            )?;
            if let Some(account_owner) = &cache.owner_pubkey_hex {
                for (target_owner, support) in &cache.social_friend_support {
                    stmt.execute(params![account_owner, target_owner, support])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}
