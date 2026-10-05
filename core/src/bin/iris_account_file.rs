use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Account file needs a filename")
    })?;
    let (mut file, temporary) = loop {
        let mut name = OsString::from(".");
        name.push(filename);
        name.push(format!(
            ".{}-{}.tmp",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = path.with_file_name(name);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&temporary) {
            Ok(file) => break (file, temporary),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Readers keep the old complete file until the new complete file is published.
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            loop {
                let path = std::env::temp_dir().join(format!(
                    "iris-account-file-test-{}-{}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create isolated test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn replacement_preserves_open_reader_and_publishes_complete_new_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("cli-account.json");
        let old = br#"{"owner":"before","device":"original"}"#;
        let new = br#"{"owner":"after","device":"replacement"}"#;
        fs::write(&path, old).unwrap();
        let mut reader = fs::File::open(&path).unwrap();

        replace(&path, new).unwrap();

        let mut retained = Vec::new();
        reader.read_to_end(&mut retained).unwrap();
        assert_eq!(
            retained, old,
            "An existing reader keeps the complete old inode"
        );
        assert_eq!(fs::read(&path).unwrap(), new);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn failed_rename_keeps_destination_and_removes_temporary_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("cli-account.json");
        fs::create_dir(&path).unwrap();
        let retained = path.join("original");
        fs::write(&retained, b"unchanged").unwrap();

        assert!(replace(&path, b"replacement").is_err());

        assert_eq!(fs::read(&retained).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}
