use super::SharedConnection;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

pub struct FileStorageAdapter {
    base_path: PathBuf,
}

impl FileStorageAdapter {
    pub fn new(base_path: PathBuf) -> StorageResult<Self> {
        fs::create_dir_all(&base_path)
            .map_err(|err| storage_io_error("failed to create storage directory", err))?;
        Ok(Self { base_path })
    }

    fn sanitize_key(key: &str) -> String {
        key.replace(['/', '\\', ':'], "_")
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        let sanitized = Self::sanitize_key(key);
        self.base_path.join(format!("{}.json", sanitized))
    }
}

impl StorageAdapter for FileStorageAdapter {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let path = self.key_to_path(key);
        match fs::read_to_string(&path) {
            Ok(contents) => Ok(Some(contents)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(storage_io_error("failed to read storage file", err)),
        }
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        let path = self.key_to_path(key);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                storage_io_error("failed to create storage parent directory", err)
            })?;
        }

        let tmp_path = path.with_extension(format!("json.{}.tmp", rand::random::<u128>()));
        fs::write(&tmp_path, value)
            .map_err(|err| storage_io_error("failed to write storage temp file", err))?;

        #[cfg(windows)]
        {
            if path.exists() {
                fs::remove_file(&path).map_err(|err| {
                    storage_io_error("failed to replace existing storage file", err)
                })?;
            }
        }

        fs::rename(&tmp_path, &path)
            .map_err(|err| storage_io_error("failed to commit storage file", err))?;

        Ok(())
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        let path = self.key_to_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(storage_io_error("failed to delete storage file", err)),
        }
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let mut keys = Vec::new();
        let sanitized_prefix = Self::sanitize_key(prefix);
        let entries = fs::read_dir(&self.base_path)
            .map_err(|err| storage_io_error("failed to read storage directory", err))?;

        for entry in entries {
            let entry =
                entry.map_err(|err| storage_io_error("failed to read storage entry", err))?;
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if !file_name_str.ends_with(".json") {
                continue;
            }

            let key = file_name_str
                .strip_suffix(".json")
                .unwrap_or(&file_name_str)
                .to_string();

            if prefix.is_empty() {
                keys.push(key);
                continue;
            }

            if key.starts_with(&sanitized_prefix) {
                let remainder = key.strip_prefix(&sanitized_prefix).unwrap_or("");
                keys.push(format!("{}{}", prefix, remainder));
            }
        }

        Ok(keys)
    }
}

pub struct DebouncedFileStorage {
    adapter: FileStorageAdapter,
    pending_writes: Mutex<HashMap<String, String>>,
    last_flush: Mutex<Instant>,
    flush_interval: Duration,
}

impl DebouncedFileStorage {
    pub fn new(base_path: PathBuf, flush_interval_ms: u64) -> StorageResult<Self> {
        Ok(Self {
            adapter: FileStorageAdapter::new(base_path)?,
            pending_writes: Mutex::new(HashMap::new()),
            last_flush: Mutex::new(Instant::now()),
            flush_interval: Duration::from_millis(flush_interval_ms),
        })
    }

    pub fn flush(&self) -> StorageResult<()> {
        let mut pending = self
            .pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?;
        for (key, value) in pending.drain() {
            self.adapter.put(&key, value)?;
        }
        *self
            .last_flush
            .lock()
            .map_err(|_| StorageError::new("file storage flush mutex poisoned"))? = Instant::now();
        Ok(())
    }

    fn maybe_flush(&self) -> StorageResult<()> {
        let last_flush = *self
            .last_flush
            .lock()
            .map_err(|_| StorageError::new("file storage flush mutex poisoned"))?;
        let pending_count = self
            .pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?
            .len();

        if last_flush.elapsed() >= self.flush_interval && pending_count > 0 {
            self.flush()?;
        }
        Ok(())
    }
}

impl StorageAdapter for DebouncedFileStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let pending = self
            .pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?;
        if let Some(value) = pending.get(key) {
            return Ok(Some(value.clone()));
        }
        drop(pending);
        self.adapter.get(key)
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        self.pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?
            .insert(key.to_string(), value);
        self.maybe_flush()
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        self.pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?
            .remove(key);
        self.adapter.del(key)
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let mut keys = self.adapter.list(prefix)?;
        let pending = self
            .pending_writes
            .lock()
            .map_err(|_| StorageError::new("pending file storage mutex poisoned"))?;

        for key in pending.keys() {
            if key.starts_with(prefix) && !keys.contains(key) {
                keys.push(key.clone());
            }
        }

        Ok(keys)
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

fn storage_io_error(context: &str, error: std::io::Error) -> StorageError {
    StorageError::new(format!("{}: {}", context, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

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

    #[test]
    fn file_storage_round_trips_values() {
        let temp_dir = TempDir::new().unwrap();
        let adapter = FileStorageAdapter::new(temp_dir.path().to_path_buf()).unwrap();

        assert!(adapter.get("test-key").unwrap().is_none());

        adapter.put("test-key", "test-value".to_string()).unwrap();
        assert_eq!(
            adapter.get("test-key").unwrap(),
            Some("test-value".to_string())
        );

        adapter.del("test-key").unwrap();
        assert!(adapter.get("test-key").unwrap().is_none());
    }

    #[test]
    fn file_storage_lists_sanitized_runtime_keys() {
        let temp_dir = TempDir::new().unwrap();
        let adapter = FileStorageAdapter::new(temp_dir.path().to_path_buf()).unwrap();

        adapter.put("user/alice", "1".to_string()).unwrap();
        adapter.put("user/bob", "2".to_string()).unwrap();
        adapter.put("invite/charlie", "3".to_string()).unwrap();

        let mut user_keys = adapter.list("user/").unwrap();
        user_keys.sort();
        assert_eq!(
            user_keys,
            vec!["user/alice".to_string(), "user/bob".to_string()]
        );

        let mut all_keys = adapter.list("").unwrap();
        all_keys.sort();
        assert_eq!(
            all_keys,
            vec![
                "invite_charlie".to_string(),
                "user_alice".to_string(),
                "user_bob".to_string()
            ]
        );
    }

    #[test]
    fn debounced_file_storage_reads_pending_writes_and_flushes() {
        let temp_dir = TempDir::new().unwrap();
        let storage = DebouncedFileStorage::new(temp_dir.path().to_path_buf(), 1000).unwrap();

        storage.put("key1", "value1".to_string()).unwrap();

        assert_eq!(storage.get("key1").unwrap(), Some("value1".to_string()));
        assert!(storage.pending_writes.lock().unwrap().contains_key("key1"));

        storage.flush().unwrap();

        assert!(storage.pending_writes.lock().unwrap().is_empty());
        assert_eq!(
            storage.adapter.get("key1").unwrap(),
            Some("value1".to_string())
        );
    }
}
