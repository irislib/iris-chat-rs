use super::SharedConnection;
use nostr_double_ratchet::{Error as NdrError, Result as NdrResult, StorageAdapter};

/// SQLite-backed implementation of `nostr_double_ratchet::StorageAdapter`.
/// Keys are namespaced by (owner_pubkey_hex, device_pubkey_hex) so a
/// single database serves multiple owner accounts and devices without
/// keyspace collisions — matching the per-(owner, device) directory
/// scoping the previous file-backed adapter used.
pub(crate) struct SqliteStorageAdapter {
    conn: SharedConnection,
    owner_pubkey_hex: String,
    device_pubkey_hex: String,
}

impl SqliteStorageAdapter {
    pub(crate) fn new(
        conn: SharedConnection,
        owner_pubkey_hex: String,
        device_pubkey_hex: String,
    ) -> Self {
        Self {
            conn,
            owner_pubkey_hex,
            device_pubkey_hex,
        }
    }

    fn map_err<E: std::fmt::Display>(error: E) -> NdrError {
        NdrError::Storage(error.to_string())
    }
}

impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> NdrResult<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| NdrError::Storage("ndr_kv connection mutex poisoned".to_string()))?;
        conn.query_row(
            "SELECT value FROM ndr_kv WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key = ?3",
            (&self.owner_pubkey_hex, &self.device_pubkey_hex, key),
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(Self::map_err(other)),
        })
    }

    fn put(&self, key: &str, value: String) -> NdrResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| NdrError::Storage("ndr_kv connection mutex poisoned".to_string()))?;
        conn.execute(
            "INSERT INTO ndr_kv (owner_pubkey_hex, device_pubkey_hex, key, value)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_pubkey_hex, device_pubkey_hex, key) DO UPDATE SET value = excluded.value",
            (&self.owner_pubkey_hex, &self.device_pubkey_hex, key, &value),
        )
        .map_err(Self::map_err)?;
        Ok(())
    }

    fn del(&self, key: &str) -> NdrResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| NdrError::Storage("ndr_kv connection mutex poisoned".to_string()))?;
        conn.execute(
            "DELETE FROM ndr_kv WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key = ?3",
            (&self.owner_pubkey_hex, &self.device_pubkey_hex, key),
        )
        .map_err(Self::map_err)?;
        Ok(())
    }

    fn list(&self, prefix: &str) -> NdrResult<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| NdrError::Storage("ndr_kv connection mutex poisoned".to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT key FROM ndr_kv
                 WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key LIKE ?3 ESCAPE '\\'",
            )
            .map_err(Self::map_err)?;
        let pattern = format!("{}%", escape_like(prefix));
        let rows = stmt
            .query_map(
                (&self.owner_pubkey_hex, &self.device_pubkey_hex, &pattern),
                |row| row.get::<_, String>(0),
            )
            .map_err(Self::map_err)?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(Self::map_err)?);
        }
        Ok(keys)
    }
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::open_database;
    use super::*;

    fn fresh_adapter() -> (tempfile::TempDir, SqliteStorageAdapter) {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(tmp.path()).unwrap();
        let adapter = SqliteStorageAdapter::new(conn, "owner".to_string(), "device".to_string());
        (tmp, adapter)
    }

    #[test]
    fn put_get_del_round_trip() {
        let (_tmp, adapter) = fresh_adapter();
        assert!(adapter.get("k").unwrap().is_none());
        adapter.put("k", "v".to_string()).unwrap();
        assert_eq!(adapter.get("k").unwrap(), Some("v".to_string()));
        adapter.put("k", "v2".to_string()).unwrap();
        assert_eq!(adapter.get("k").unwrap(), Some("v2".to_string()));
        adapter.del("k").unwrap();
        assert!(adapter.get("k").unwrap().is_none());
    }

    #[test]
    fn list_returns_only_matching_prefix() {
        let (_tmp, adapter) = fresh_adapter();
        adapter.put("user/alice", "1".to_string()).unwrap();
        adapter.put("user/bob", "2".to_string()).unwrap();
        adapter.put("invite/charlie", "3".to_string()).unwrap();
        let mut keys = adapter.list("user/").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["user/alice".to_string(), "user/bob".to_string()]);
    }

    #[test]
    fn keys_are_isolated_per_owner_device() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(tmp.path()).unwrap();
        let alice = SqliteStorageAdapter::new(conn.clone(), "owner_a".into(), "device_a".into());
        let bob = SqliteStorageAdapter::new(conn, "owner_b".into(), "device_b".into());
        alice.put("shared-key", "alice".to_string()).unwrap();
        bob.put("shared-key", "bob".to_string()).unwrap();
        assert_eq!(alice.get("shared-key").unwrap(), Some("alice".to_string()));
        assert_eq!(bob.get("shared-key").unwrap(), Some("bob".to_string()));
    }
}
