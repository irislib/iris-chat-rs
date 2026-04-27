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
    fn clear(&self);
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

    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}
