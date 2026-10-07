//! Private, opt-in synthetic identity receipt for an isolated control fixture.
use anyhow::{Context, Result};
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub fn save(path: &Path, owner_nsec: Option<&str>, owner: &str, device_nsec: &str) -> Result<()> {
    let owner_nsec = owner_nsec.context("Control fixture requires its fresh owner identity")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    anyhow::bail!("Private control identity receipts require a Unix host");
    let mut file = options
        .open(path)
        .context("Create private control identity receipt")?;
    serde_json::to_writer(&mut file, &json!({
        "version": 1, "owner_nsec": owner_nsec, "owner_pubkey_hex": owner, "device_nsec": device_nsec,
    })).context("Write private control identity receipt")?;
    file.flush()
        .context("Flush private control identity receipt")?;
    file.sync_all()
        .context("Sync private control identity receipt")?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn receipt_is_private_complete_and_cannot_overwrite_an_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bundle.json");
        save(
            &path,
            Some("test-owner-secret"),
            "test-owner",
            "test-device-secret",
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let bytes = std::fs::read(&path).unwrap();
        let bundle: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            bundle,
            json!({"version":1,"owner_nsec":"test-owner-secret",
            "owner_pubkey_hex":"test-owner","device_nsec":"test-device-secret"})
        );
        assert!(save(&path, Some("replacement"), "replacement", "replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn missing_owner_fails_without_creating_a_partial_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bundle.json");
        assert!(save(&path, None, "test-owner", "test-device-secret").is_err());
        assert!(!path.exists());
    }
}
