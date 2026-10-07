//! Private, opt-in synthetic identity receipt for an isolated control fixture.
#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
#[cfg(unix)]
use serde_json::json;
#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
pub fn restore(path: &Path, expected_device: &str) -> Result<iris_chat_core::AppAction> {
    use nostr::{Keys, ToBech32};
    use std::os::unix::fs::PermissionsExt;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Bundle {
        version: u8,
        owner_nsec: String,
        owner_pubkey_hex: String,
        device_nsec: String,
    }
    let metadata = std::fs::symlink_metadata(path).context("Read control receipt metadata")?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.permissions().mode() & 0o777 == 0o600
            && (1..=4096).contains(&metadata.len()),
        "Control receipt must be a private regular file"
    );
    let bytes = std::fs::read(path).context("Read private control identity")?;
    let bundle: Bundle = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid control identity receipt"))?;
    anyhow::ensure!(
        bundle.version == 1
            && bundle.owner_nsec.starts_with("nsec1")
            && bundle.device_nsec.starts_with("nsec1"),
        "Unsupported control identity receipt"
    );
    let owner = Keys::parse(&bundle.owner_nsec)
        .map_err(|_| anyhow::anyhow!("Invalid control owner identity"))?;
    let device = Keys::parse(&bundle.device_nsec)
        .map_err(|_| anyhow::anyhow!("Invalid control device identity"))?;
    anyhow::ensure!(
        owner.public_key().to_hex() == bundle.owner_pubkey_hex
            && device.public_key().to_bech32()? == expected_device,
        "Control identity does not match preserved pairing"
    );
    Ok(iris_chat_core::AppAction::RestoreAccountBundle {
        owner_nsec: Some(bundle.owner_nsec),
        owner_pubkey_hex: bundle.owner_pubkey_hex,
        device_nsec: bundle.device_nsec,
    })
}

#[cfg(not(unix))]
pub fn restore(_path: &Path, _expected_device: &str) -> Result<iris_chat_core::AppAction> {
    anyhow::bail!("Private control identity receipts require a Unix host")
}

#[cfg(unix)]
pub fn save(path: &Path, owner_nsec: Option<&str>, owner: &str, device_nsec: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let owner_nsec = owner_nsec.context("Control fixture requires its fresh owner identity")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
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

#[cfg(not(unix))]
pub fn save(
    _path: &Path,
    _owner_nsec: Option<&str>,
    _owner: &str,
    _device_nsec: &str,
) -> Result<()> {
    anyhow::bail!("Private control identity receipts require a Unix host")
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

    #[test]
    fn restore_requires_private_receipt_and_exact_paired_device() {
        use nostr::{Keys, ToBech32};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bundle.json");
        let owner = Keys::generate();
        let device = Keys::generate();
        let expected = device.public_key().to_bech32().unwrap();
        save(
            &path,
            Some(&owner.secret_key().to_bech32().unwrap()),
            &owner.public_key().to_hex(),
            &device.secret_key().to_bech32().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            restore(&path, &expected).unwrap(),
            iris_chat_core::AppAction::RestoreAccountBundle { .. }
        ));
        assert!(restore(&path, &owner.public_key().to_bech32().unwrap()).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(restore(&path, &expected).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(restore(&link, &expected).is_err());
    }
}
