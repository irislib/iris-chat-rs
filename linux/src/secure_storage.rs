use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoredAccountBundle {
    pub owner_nsec: Option<String>,
    pub owner_pubkey_hex: String,
    pub device_nsec: String,
}

pub trait SecretStore: Send + Sync {
    fn load(&self) -> Option<StoredAccountBundle>;
    fn save(&self, bundle: &StoredAccountBundle);
    fn clear(&self) -> bool;
}

// File-backed store with mode 0600. Placeholder until libsecret/oo7 is wired.
pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(secrets_dir: &Path) -> Self {
        let _ = fs::create_dir_all(secrets_dir);
        Self {
            path: secrets_dir.join("account.json"),
        }
    }
}

impl SecretStore for FileSecretStore {
    fn load(&self) -> Option<StoredAccountBundle> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save(&self, bundle: &StoredAccountBundle) {
        let json = match serde_json::to_vec(bundle) {
            Ok(v) => v,
            Err(_) => return,
        };
        let tmp = self.path.with_extension("json.tmp");
        let mut opts = fs::OpenOptions::new();
        opts.create(true).truncate(true).write(true).mode(0o600);
        let Ok(mut file) = opts.open(&tmp) else {
            return;
        };
        if file.write_all(&json).is_err() {
            return;
        }
        if file.sync_all().is_err() {
            return;
        }
        drop(file);
        let _ = fs::rename(&tmp, &self.path);
    }

    fn clear(&self) -> bool {
        match fs::remove_file(&self.path) {
            Ok(()) => self.load().is_none(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                eprintln!("Iris Chat file secret clear failed: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("iris-chat-secret-store-{nanos}"))
    }

    #[test]
    fn file_secret_store_clear_removes_owner_and_device_bundle() {
        let dir = temp_dir();
        let store = FileSecretStore::new(&dir);
        store.save(&StoredAccountBundle {
            owner_nsec: Some("nsec1owner".to_string()),
            owner_pubkey_hex: "owner-hex".to_string(),
            device_nsec: "nsec1device".to_string(),
        });

        assert!(store.load().is_some());
        assert!(store.clear());
        assert!(store.load().is_none());

        let _ = fs::remove_dir_all(dir);
    }
}
