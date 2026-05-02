use anyhow::{anyhow, Context};
use nostr_double_ratchet::{FileStorageAdapter, StorageAdapter, StoredUserRecord};
use nostr_sdk::prelude::{Keys, PublicKey};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const LEGACY_NDR_SESSION_MANAGER_DIR_ENV: &str = "IRIS_LEGACY_NDR_SESSION_MANAGER_DIR";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LegacyNdrImportSummary {
    pub(crate) imported: usize,
    pub(crate) replaced_empty: usize,
    pub(crate) skipped_existing: usize,
    pub(crate) skipped_invalid: usize,
}

pub(crate) fn import_legacy_ndr_storage(
    storage: &dyn StorageAdapter,
    owner_pubkey: PublicKey,
) -> anyhow::Result<LegacyNdrImportSummary> {
    let Some(source_dir) = legacy_ndr_session_manager_dir() else {
        return Ok(LegacyNdrImportSummary::default());
    };
    import_legacy_ndr_storage_from_dir(storage, owner_pubkey, &source_dir)
}

fn import_legacy_ndr_storage_from_dir(
    storage: &dyn StorageAdapter,
    owner_pubkey: PublicKey,
    source_dir: &Path,
) -> anyhow::Result<LegacyNdrImportSummary> {
    if !source_dir.is_dir() {
        return Ok(LegacyNdrImportSummary::default());
    }

    if let Some(legacy_owner) = legacy_config_owner(source_dir)? {
        if legacy_owner != owner_pubkey {
            return Ok(LegacyNdrImportSummary::default());
        }
    }

    let legacy_storage = FileStorageAdapter::new(source_dir.to_path_buf())
        .map_err(|error| anyhow!("open legacy NDR storage: {error}"))?;
    import_legacy_user_records(storage, &legacy_storage)
}

fn import_legacy_user_records(
    storage: &dyn StorageAdapter,
    legacy_storage: &FileStorageAdapter,
) -> anyhow::Result<LegacyNdrImportSummary> {
    let mut summary = LegacyNdrImportSummary::default();
    let mut keys = legacy_storage
        .list("user/")
        .map_err(|error| anyhow!("list legacy user records: {error}"))?;
    keys.sort();
    keys.dedup();

    for key in keys {
        if !is_valid_user_record_key(&key) {
            summary.skipped_invalid += 1;
            continue;
        }
        let Some(raw) = legacy_storage
            .get(&key)
            .map_err(|error| anyhow!("read legacy NDR record {key}: {error}"))?
        else {
            summary.skipped_invalid += 1;
            continue;
        };
        let Ok(source_record) = serde_json::from_str::<StoredUserRecord>(&raw) else {
            summary.skipped_invalid += 1;
            continue;
        };
        if !record_matches_key(&source_record, &key) {
            summary.skipped_invalid += 1;
            continue;
        }

        match storage.get(&key)? {
            None => {
                storage.put(&key, raw)?;
                summary.imported += 1;
            }
            Some(existing_raw) => {
                if should_replace_existing_record(&existing_raw, &source_record) {
                    storage.put(&key, raw)?;
                    summary.replaced_empty += 1;
                } else {
                    summary.skipped_existing += 1;
                }
            }
        }
    }

    Ok(summary)
}

fn legacy_ndr_session_manager_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(LEGACY_NDR_SESSION_MANAGER_DIR_ENV) {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("ndr")
                .join("session_manager")
        })
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(|base| PathBuf::from(base).join("ndr").join("session_manager"))
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|base| base.join("ndr").join("session_manager"))
    }
}

#[derive(Deserialize)]
struct LegacyNdrConfig {
    private_key: Option<String>,
}

fn legacy_config_owner(source_dir: &Path) -> anyhow::Result<Option<PublicKey>> {
    let Some(parent) = source_dir.parent() else {
        return Ok(None);
    };
    let config_path = parent.join("config.json");
    if !config_path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read legacy NDR config {}", config_path.display()))?;
    let config: LegacyNdrConfig = serde_json::from_str(&raw)
        .with_context(|| format!("parse legacy NDR config {}", config_path.display()))?;
    let Some(private_key) = config.private_key.as_deref() else {
        return Ok(None);
    };
    let keys = Keys::parse(private_key).context("parse legacy NDR owner key")?;
    Ok(Some(keys.public_key()))
}

fn is_valid_user_record_key(key: &str) -> bool {
    let Some(hex) = key.strip_prefix("user/") else {
        return false;
    };
    hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn record_matches_key(record: &StoredUserRecord, key: &str) -> bool {
    key.strip_prefix("user/")
        .is_some_and(|hex| record.user_id.eq_ignore_ascii_case(hex))
}

fn should_replace_existing_record(existing_raw: &str, source_record: &StoredUserRecord) -> bool {
    if active_session_count(source_record) == 0 {
        return false;
    }
    let Ok(existing_record) = serde_json::from_str::<StoredUserRecord>(existing_raw) else {
        return true;
    };
    active_session_count(&existing_record) == 0
}

fn active_session_count(record: &StoredUserRecord) -> usize {
    record
        .devices
        .iter()
        .filter(|device| !device.is_stale && device.active_session.is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::super::open_database;
    use super::super::SqliteStorageAdapter;
    use super::*;
    use nostr_double_ratchet::{
        DevicePubkey, FileStorageAdapter, SerializableKeyPair, SessionState, StoredDeviceRecord,
    };
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn adapter(tmp: &TempDir) -> SqliteStorageAdapter {
        SqliteStorageAdapter::new(
            open_database(tmp.path()).unwrap(),
            "owner".to_string(),
            "device".to_string(),
        )
    }

    fn write_legacy_config(root: &Path, owner: &Keys) {
        std::fs::write(
            root.join("config.json"),
            serde_json::json!({
                "private_key": owner.secret_key().to_secret_hex(),
                "relays": []
            })
            .to_string(),
        )
        .unwrap();
    }

    fn session_state() -> SessionState {
        let our_current = Keys::generate();
        let our_next = Keys::generate();
        let their_current = Keys::generate();
        let their_next = Keys::generate();
        SessionState {
            root_key: [1; 32],
            their_current_nostr_public_key: Some(DevicePubkey::from_bytes(
                their_current.public_key().to_bytes(),
            )),
            their_next_nostr_public_key: Some(DevicePubkey::from_bytes(
                their_next.public_key().to_bytes(),
            )),
            our_previous_nostr_key: None,
            our_current_nostr_key: Some(SerializableKeyPair {
                public_key: DevicePubkey::from_bytes(our_current.public_key().to_bytes()),
                private_key: our_current.secret_key().to_secret_bytes(),
            }),
            our_next_nostr_key: SerializableKeyPair {
                public_key: DevicePubkey::from_bytes(our_next.public_key().to_bytes()),
                private_key: our_next.secret_key().to_secret_bytes(),
            },
            receiving_chain_key: Some([2; 32]),
            sending_chain_key: Some([3; 32]),
            sending_chain_message_number: 1,
            receiving_chain_message_number: 1,
            previous_sending_chain_message_count: 0,
            skipped_keys: BTreeMap::new(),
        }
    }

    fn record(user_id: &str, active_session: bool) -> StoredUserRecord {
        StoredUserRecord {
            user_id: user_id.to_string(),
            devices: vec![StoredDeviceRecord {
                device_id: user_id.to_string(),
                active_session: active_session.then(session_state),
                inactive_sessions: Vec::new(),
                created_at: 1,
                is_stale: false,
                stale_timestamp: None,
                last_activity: Some(1),
            }],
            known_device_identities: vec![user_id.to_string()],
        }
    }

    #[test]
    fn imports_missing_legacy_user_records() {
        let legacy_root = TempDir::new().unwrap();
        let legacy_session_dir = legacy_root.path().join("session_manager");
        let legacy = FileStorageAdapter::new(legacy_session_dir.clone()).unwrap();
        let owner = Keys::generate();
        write_legacy_config(legacy_root.path(), &owner);
        let peer = Keys::generate().public_key().to_hex();
        let source = record(&peer, true);
        legacy
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&source).unwrap(),
            )
            .unwrap();

        let db = TempDir::new().unwrap();
        let storage = adapter(&db);
        let summary =
            import_legacy_ndr_storage_from_dir(&storage, owner.public_key(), &legacy_session_dir)
                .unwrap();

        assert_eq!(summary.imported, 1);
        assert_eq!(
            storage
                .get(&format!("user/{peer}"))
                .unwrap()
                .and_then(|raw| serde_json::from_str::<StoredUserRecord>(&raw).ok())
                .map(|record| record.user_id),
            Some(peer)
        );
    }

    #[test]
    fn does_not_clobber_active_sqlite_record() {
        let legacy_root = TempDir::new().unwrap();
        let legacy_session_dir = legacy_root.path().join("session_manager");
        let legacy = FileStorageAdapter::new(legacy_session_dir.clone()).unwrap();
        let owner = Keys::generate();
        write_legacy_config(legacy_root.path(), &owner);
        let peer = Keys::generate().public_key().to_hex();
        let mut existing = record(&peer, true);
        existing.devices[0].created_at = 99;
        let source = record(&peer, true);
        legacy
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&source).unwrap(),
            )
            .unwrap();

        let db = TempDir::new().unwrap();
        let storage = adapter(&db);
        storage
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&existing).unwrap(),
            )
            .unwrap();
        let summary =
            import_legacy_ndr_storage_from_dir(&storage, owner.public_key(), &legacy_session_dir)
                .unwrap();

        assert_eq!(summary.skipped_existing, 1);
        let stored = storage.get(&format!("user/{peer}")).unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<StoredUserRecord>(&stored)
                .unwrap()
                .devices[0]
                .created_at,
            99
        );
    }

    #[test]
    fn replaces_empty_sqlite_record_with_active_legacy_record() {
        let legacy_root = TempDir::new().unwrap();
        let legacy_session_dir = legacy_root.path().join("session_manager");
        let legacy = FileStorageAdapter::new(legacy_session_dir.clone()).unwrap();
        let owner = Keys::generate();
        write_legacy_config(legacy_root.path(), &owner);
        let peer = Keys::generate().public_key().to_hex();
        let source = record(&peer, true);
        legacy
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&source).unwrap(),
            )
            .unwrap();

        let db = TempDir::new().unwrap();
        let storage = adapter(&db);
        storage
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&record(&peer, false)).unwrap(),
            )
            .unwrap();
        let summary =
            import_legacy_ndr_storage_from_dir(&storage, owner.public_key(), &legacy_session_dir)
                .unwrap();

        assert_eq!(summary.replaced_empty, 1);
        let stored = storage.get(&format!("user/{peer}")).unwrap().unwrap();
        assert_eq!(
            active_session_count(&serde_json::from_str::<StoredUserRecord>(&stored).unwrap()),
            1
        );
    }

    #[test]
    fn ignores_legacy_storage_for_a_different_owner_config() {
        let legacy_root = TempDir::new().unwrap();
        let legacy_session_dir = legacy_root.path().join("session_manager");
        let legacy = FileStorageAdapter::new(legacy_session_dir.clone()).unwrap();
        write_legacy_config(legacy_root.path(), &Keys::generate());
        let owner = Keys::generate();
        let peer = Keys::generate().public_key().to_hex();
        legacy
            .put(
                &format!("user/{peer}"),
                serde_json::to_string(&record(&peer, true)).unwrap(),
            )
            .unwrap();

        let db = TempDir::new().unwrap();
        let storage = adapter(&db);
        let summary =
            import_legacy_ndr_storage_from_dir(&storage, owner.public_key(), &legacy_session_dir)
                .unwrap();

        assert_eq!(summary, LegacyNdrImportSummary::default());
        assert!(storage.get(&format!("user/{peer}")).unwrap().is_none());
    }
}
