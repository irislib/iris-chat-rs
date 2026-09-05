use super::AppStore;
use crate::core::ProfileSearchCandidate;
use rusqlite::{params, OptionalExtension, Transaction};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_PROFILE_SEARCH_CANDIDATES: i64 = 4_096;

impl AppStore {
    /// Cache globally discovered profiles in one transaction. Metadata older
    /// than the cached event is ignored; the result only reports changes to
    /// fields visible in search results.
    pub(crate) fn upsert_profile_search_candidates(
        &self,
        candidates: &[ProfileSearchCandidate],
    ) -> anyhow::Result<bool> {
        if candidates.is_empty() {
            return Ok(false);
        }
        let cached_at_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("storage connection mutex poisoned"))?;
        let tx = conn.transaction()?;
        let mut visible_changed = false;

        for candidate in candidates {
            visible_changed |= upsert_profile_search_candidate(&tx, candidate, cached_at_secs)?;
        }
        prune_profile_search_candidates(&tx, MAX_PROFILE_SEARCH_CANDIDATES)?;
        tx.commit()?;
        Ok(visible_changed)
    }
}

fn upsert_profile_search_candidate(
    tx: &Transaction<'_>,
    candidate: &ProfileSearchCandidate,
    cached_at_secs: u64,
) -> anyhow::Result<bool> {
    let aliases_json = serde_json::to_string(&candidate.aliases)?;
    let existing = tx
        .query_row(
            "SELECT name, aliases_json, nip05, picture, created_at_secs
             FROM profile_search_candidates
             WHERE owner_pubkey_hex = ?1",
            [&candidate.owner_pubkey_hex],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;

    if existing.as_ref().is_some_and(|(_, _, _, _, created_at)| {
        *created_at > sqlite_integer(candidate.created_at_secs)
    }) {
        return Ok(false);
    }

    let visible_changed =
        existing
            .as_ref()
            .is_none_or(|(name, existing_aliases_json, nip05, picture, _)| {
                name != &candidate.name
                    || existing_aliases_json != &aliases_json
                    || nip05 != &candidate.nip05
                    || picture != &candidate.picture
            });
    tx.execute(
        "INSERT INTO profile_search_candidates(
             owner_pubkey_hex, name, aliases_json, nip05, picture,
             created_at_secs, cached_at_secs
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(owner_pubkey_hex) DO UPDATE SET
             name = excluded.name,
             aliases_json = excluded.aliases_json,
             nip05 = excluded.nip05,
             picture = excluded.picture,
             created_at_secs = excluded.created_at_secs,
             cached_at_secs = excluded.cached_at_secs",
        params![
            candidate.owner_pubkey_hex,
            candidate.name,
            aliases_json,
            candidate.nip05,
            candidate.picture,
            sqlite_integer(candidate.created_at_secs),
            sqlite_integer(cached_at_secs),
        ],
    )?;
    Ok(visible_changed)
}

fn prune_profile_search_candidates(tx: &Transaction<'_>, maximum: i64) -> anyhow::Result<()> {
    let count = tx.query_row(
        "SELECT COUNT(*) FROM profile_search_candidates",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let excess = count.saturating_sub(maximum);
    if excess > 0 {
        tx.execute(
            "DELETE FROM profile_search_candidates
             WHERE rowid IN (
                 SELECT rowid
                 FROM profile_search_candidates
                 ORDER BY cached_at_secs, rowid
                 LIMIT ?1
             )",
            [excess],
        )?;
    }
    Ok(())
}

fn sqlite_integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::open_database;

    fn create_table(store: &AppStore) {
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS profile_search_candidates (
                     owner_pubkey_hex TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     aliases_json TEXT NOT NULL,
                     nip05 TEXT,
                     picture TEXT,
                     created_at_secs INTEGER NOT NULL,
                     cached_at_secs INTEGER NOT NULL
                 );",
            )
            .unwrap();
    }

    fn candidate(name: &str, created_at_secs: u64) -> ProfileSearchCandidate {
        ProfileSearchCandidate {
            owner_pubkey_hex: "11".repeat(32),
            name: name.to_string(),
            aliases: vec!["alias".to_string()],
            nip05: Some("user@example.com".to_string()),
            picture: Some("https://example.com/picture.jpg".to_string()),
            created_at_secs,
        }
    }

    #[test]
    fn batch_upsert_reports_visible_changes_and_rejects_stale_profiles() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = AppStore::new(open_database(tmp.path()).unwrap());
        create_table(&store);

        assert!(store
            .upsert_profile_search_candidates(&[candidate("Alice", 2)])
            .unwrap());
        assert!(!store
            .upsert_profile_search_candidates(&[candidate("Alice", 3)])
            .unwrap());
        assert!(store
            .upsert_profile_search_candidates(&[candidate("Alicia", 4)])
            .unwrap());
        assert!(!store
            .upsert_profile_search_candidates(&[candidate("Old name", 1)])
            .unwrap());

        let stored = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT name, created_at_secs FROM profile_search_candidates",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(stored, ("Alicia".to_string(), 4));
    }

    #[test]
    fn pruning_removes_the_least_recent_candidates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = AppStore::new(open_database(tmp.path()).unwrap());
        create_table(&store);
        let mut conn = store.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for cached_at_secs in 1..=3 {
            tx.execute(
                "INSERT INTO profile_search_candidates(
                     owner_pubkey_hex, name, aliases_json, created_at_secs, cached_at_secs
                 ) VALUES (?1, ?1, '[]', 1, ?2)",
                params![cached_at_secs.to_string(), cached_at_secs],
            )
            .unwrap();
        }
        prune_profile_search_candidates(&tx, 2).unwrap();
        let oldest_remaining = tx
            .query_row(
                "SELECT MIN(cached_at_secs) FROM profile_search_candidates",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(oldest_remaining, 2);
    }
}
