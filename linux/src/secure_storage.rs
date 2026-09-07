use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoredAccountBundle {
    pub owner_nsec: Option<String>,
    pub owner_pubkey_hex: String,
    pub device_nsec: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoredPendingDeviceLink {
    pub device_nsec: String,
    pub approval_bootstrap_json: String,
}

pub trait SecretStore: Send + Sync {
    fn load(&self) -> Option<StoredAccountBundle>;
    fn save(&self, bundle: &StoredAccountBundle);
    fn load_pending_device_link(&self) -> Option<StoredPendingDeviceLink>;
    fn save_pending_device_link(&self, link: &StoredPendingDeviceLink);
    fn clear_pending_device_link(&self) -> bool;
    fn clear(&self) -> bool;
}

// File-backed store with mode 0600. Placeholder until libsecret/oo7 is wired.
pub struct FileSecretStore {
    path: PathBuf,
    pending_link_path: PathBuf,
}

impl FileSecretStore {
    pub fn new(secrets_dir: &Path) -> Self {
        let _ = fs::create_dir_all(secrets_dir);
        Self {
            path: secrets_dir.join("account.json"),
            pending_link_path: secrets_dir.join("pending-device-link.json"),
        }
    }
}

impl SecretStore for FileSecretStore {
    fn load(&self) -> Option<StoredAccountBundle> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save(&self, bundle: &StoredAccountBundle) {
        write_secret(&self.path, bundle);
    }

    fn load_pending_device_link(&self) -> Option<StoredPendingDeviceLink> {
        let bytes = fs::read(&self.pending_link_path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_pending_device_link(&self, link: &StoredPendingDeviceLink) {
        write_secret(&self.pending_link_path, link);
    }

    fn clear_pending_device_link(&self) -> bool {
        remove_secret(&self.pending_link_path)
    }

    fn clear(&self) -> bool {
        remove_secret(&self.path) && self.clear_pending_device_link()
    }
}

fn write_secret<T: Serialize>(path: &Path, value: &T) {
    let json = match serde_json::to_vec(value) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some((tmp, mut file)) = create_secret_temp_file(path) else {
        return;
    };
    let written = file.write_all(&json).and_then(|()| file.sync_all());
    drop(file);
    if written.is_err() || fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn create_secret_temp_file(path: &Path) -> Option<(PathBuf, fs::File)> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    for _ in 0..8 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!(
            "json.{}.{timestamp}.{sequence}.tmp",
            std::process::id()
        ));
        // Exclusive creation never follows a preexisting symlink and never
        // inherits the permissions of an old temporary file.
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(file) => return Some((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

fn remove_secret(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => !path.exists(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            eprintln!("Iris Chat file secret clear failed: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

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

    #[test]
    fn secret_save_does_not_follow_a_preexisting_temporary_symlink() {
        let dir = temp_dir();
        let store = FileSecretStore::new(&dir);
        let victim = dir.join("unrelated-file");
        fs::write(&victim, b"keep this file").unwrap();
        symlink(&victim, dir.join("account.json.tmp")).unwrap();

        store.save(&StoredAccountBundle {
            owner_nsec: Some("private owner key".into()),
            owner_pubkey_hex: "owner".into(),
            device_nsec: "private device key".into(),
        });

        assert_eq!(fs::read(&victim).unwrap(), b"keep this file");
        assert!(store.load().is_some());
        assert!(!fs::symlink_metadata(&store.path).unwrap().is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn secret_save_does_not_reuse_a_world_readable_temporary_file() {
        let dir = temp_dir();
        let store = FileSecretStore::new(&dir);
        let stale = dir.join("pending-device-link.json.tmp");
        fs::write(&stale, b"unrelated data").unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();

        store.save_pending_device_link(&StoredPendingDeviceLink {
            device_nsec: "private device key".into(),
            approval_bootstrap_json: "private bootstrap".into(),
        });

        assert_eq!(fs::read(&stale).unwrap(), b"unrelated data");
        assert_eq!(
            fs::metadata(&store.pending_link_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(store.load_pending_device_link().is_some());
        fs::remove_dir_all(dir).unwrap();
    }
}
