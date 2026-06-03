use super::SharedConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    message: String,
}

impl StorageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for StorageError {}

pub type StorageResult<T> = Result<T, StorageError>;

pub trait StorageAdapter: Send + Sync {
    fn get(&self, key: &str) -> StorageResult<Option<String>>;
    fn put(&self, key: &str, value: String) -> StorageResult<()>;
    fn del(&self, key: &str) -> StorageResult<()>;
    fn list(&self, prefix: &str) -> StorageResult<Vec<String>>;
}

#[derive(Clone)]
pub struct InMemoryStorage {
    store: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageAdapter for InMemoryStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let store = self
            .store
            .lock()
            .map_err(|_| StorageError::new("storage mutex poisoned"))?;
        Ok(store.get(key).cloned())
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| StorageError::new("storage mutex poisoned"))?;
        store.insert(key.to_string(), value);
        Ok(())
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| StorageError::new("storage mutex poisoned"))?;
        store.remove(key);
        Ok(())
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let store = self
            .store
            .lock()
            .map_err(|_| StorageError::new("storage mutex poisoned"))?;
        Ok(store
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// SQLite-backed implementation of `iris_chat_protocol::StorageAdapter`.
/// Keys are namespaced by (owner_pubkey_hex, device_pubkey_hex) so a
/// single database serves multiple owner accounts and devices without
/// keyspace collisions, matching the per-(owner, device) directory
/// scoping the previous file-backed adapter used.
pub struct SqliteStorageAdapter {
    conn: SharedConnection,
    owner_pubkey_hex: String,
    device_pubkey_hex: String,
}

impl SqliteStorageAdapter {
    pub fn new(
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

    fn map_err<E: std::fmt::Display>(error: E) -> StorageError {
        StorageError::new(error.to_string())
    }
}

impl StorageAdapter for SqliteStorageAdapter {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StorageError::new("ndr_kv connection mutex poisoned"))?;
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

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StorageError::new("ndr_kv connection mutex poisoned"))?;
        conn.execute(
            "INSERT INTO ndr_kv (owner_pubkey_hex, device_pubkey_hex, key, value)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_pubkey_hex, device_pubkey_hex, key) DO UPDATE SET value = excluded.value",
            (&self.owner_pubkey_hex, &self.device_pubkey_hex, key, &value),
        )
        .map_err(Self::map_err)?;
        Ok(())
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StorageError::new("ndr_kv connection mutex poisoned"))?;
        conn.execute(
            "DELETE FROM ndr_kv WHERE owner_pubkey_hex = ?1 AND device_pubkey_hex = ?2 AND key = ?3",
            (&self.owner_pubkey_hex, &self.device_pubkey_hex, key),
        )
        .map_err(Self::map_err)?;
        Ok(())
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| StorageError::new("ndr_kv connection mutex poisoned"))?;
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
    use super::*;
    use std::sync::{Arc, Mutex};

    fn fresh_connection() -> SharedConnection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ndr_kv (
                owner_pubkey_hex TEXT NOT NULL,
                device_pubkey_hex TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (owner_pubkey_hex, device_pubkey_hex, key)
            );",
        )
        .unwrap();
        Arc::new(Mutex::new(conn))
    }

    fn fresh_adapter() -> SqliteStorageAdapter {
        SqliteStorageAdapter::new(
            fresh_connection(),
            "owner".to_string(),
            "device".to_string(),
        )
    }

    #[test]
    fn put_get_del_round_trip() {
        let adapter = fresh_adapter();
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
        let adapter = fresh_adapter();
        adapter.put("user/alice", "1".to_string()).unwrap();
        adapter.put("user/bob", "2".to_string()).unwrap();
        adapter.put("invite/charlie", "3".to_string()).unwrap();
        let mut keys = adapter.list("user/").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["user/alice".to_string(), "user/bob".to_string()]);
    }

    #[test]
    fn keys_are_isolated_per_owner_device() {
        let conn = fresh_connection();
        let alice = SqliteStorageAdapter::new(conn.clone(), "owner_a".into(), "device_a".into());
        let bob = SqliteStorageAdapter::new(conn, "owner_b".into(), "device_b".into());
        alice.put("shared-key", "alice".to_string()).unwrap();
        bob.put("shared-key", "bob".to_string()).unwrap();
        assert_eq!(alice.get("shared-key").unwrap(), Some("alice".to_string()));
        assert_eq!(bob.get("shared-key").unwrap(), Some("bob".to_string()));
    }
}
