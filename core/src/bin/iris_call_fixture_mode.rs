//! Explicit fixture identity modes; normal saved fixtures never alter Nearby policy.
use anyhow::{bail, Result};
use std::path::Path;

pub struct Mode {
    pub normal_persistent: bool,
    pub persist_identity: bool,
    pub resume_device: Option<String>,
    pub resume_owner: Option<String>,
}

impl Mode {
    pub fn parse(args: &[String], isolated_control: bool) -> Result<Self> {
        let mut mode = Self {
            normal_persistent: false,
            persist_identity: isolated_control,
            resume_device: None,
            resume_owner: None,
        };
        match args {
            [] => (),
            [flag] if flag == "--persist-normal" && !isolated_control => {
                mode.normal_persistent = true;
                mode.persist_identity = true;
            }
            [flag, owner, device] if flag == "--resume-normal" && !isolated_control => {
                mode.normal_persistent = true;
                mode.persist_identity = true;
                mode.resume_owner = Some(owner.clone());
                mode.resume_device = Some(device.clone());
            }
            [flag, device] if flag == "--resume-control" && isolated_control => {
                mode.resume_device = Some(device.clone());
            }
            _ => bail!("Invalid fixture identity mode or control environment"),
        }
        Ok(mode)
    }
}

#[cfg(unix)]
pub fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !path.try_exists()? {
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700,
        "Persistent fixture directory must be a private real directory"
    );
    Ok(())
}

#[cfg(not(unix))]
pub fn private_directory(_path: &Path) -> Result<()> {
    bail!("Persistent fixture identities require a Unix host")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], control: bool) -> Result<Mode> {
        Mode::parse(
            &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            control,
        )
    }

    #[test]
    fn persistence_and_restore_are_explicit_and_cannot_mix_network_modes() {
        assert!(!parse(&[], false).unwrap().persist_identity);
        assert!(parse(&[], true).unwrap().persist_identity);
        assert!(
            parse(&["--persist-normal"], false)
                .unwrap()
                .normal_persistent
        );
        let saved = parse(&["--resume-normal", "owner", "device"], false).unwrap();
        assert_eq!(saved.resume_owner.as_deref(), Some("owner"));
        assert_eq!(saved.resume_device.as_deref(), Some("device"));
        assert!(
            !parse(&["--resume-control", "device"], true)
                .unwrap()
                .normal_persistent
        );
        for (args, control) in [
            (vec!["--persist-normal"], true),
            (vec!["--resume-normal", "owner", "device"], true),
            (vec!["--resume-control", "device"], false),
            (vec!["--resume-normal", "device"], false),
        ] {
            assert!(parse(&args, control).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_rejects_symlinks_and_public_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("fixture");
        private_directory(&directory).unwrap();
        std::os::unix::fs::symlink(&directory, temp.path().join("link")).unwrap();
        assert!(private_directory(&temp.path().join("link")).is_err());
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_directory(&directory).is_err());
    }
}
