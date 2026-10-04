use super::{TransferFile, CHUNK};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, sync::Arc};

pub(super) fn validate(files: &[TransferFile]) -> Result<(), String> {
    if files.is_empty() || files.len() > 32 {
        return Err("Choose between 1 and 32 files".into());
    }
    for file in files {
        if !crate::direct_files::valid_filename(&file.filename)
            || file.size_bytes > 100 * 1024 * 1024 * 1024
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid file details".into());
        }
    }
    Ok(())
}

pub(super) struct SendFiles {
    files: Vec<TransferFile>,
    current: Option<File>,
    index: usize,
    offset: u64,
    transferred: u64,
    total: u64,
}
impl SendFiles {
    pub(super) fn new(files: Vec<TransferFile>) -> Result<Self, String> {
        validate(&files)?;
        let total = files.iter().map(|f| f.size_bytes).sum();
        Ok(Self {
            files,
            current: None,
            index: 0,
            offset: 0,
            transferred: 0,
            total,
        })
    }
    pub(super) fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        while let Some(file) = self.files.get(self.index) {
            if self.current.is_none() {
                let handle =
                    File::open(&file.path).map_err(|_| "Selected file is no longer available")?;
                let metadata = handle
                    .metadata()
                    .map_err(|_| "Could not read selected file")?;
                if !metadata.is_file() || metadata.len() != file.size_bytes {
                    return Err("Selected file changed".into());
                }
                self.current = Some(handle);
            }
            if self.offset == file.size_bytes {
                self.index += 1;
                self.offset = 0;
                self.current = None;
                continue;
            }
            let length = (file.size_bytes - self.offset).min(CHUNK as u64) as usize;
            let mut bytes = vec![0; length];
            self.current
                .as_mut()
                .ok_or("Selected file is closed")?
                .read_exact(&mut bytes)
                .map_err(|_| "Could not read selected file")?;
            self.offset += length as u64;
            self.transferred += length as u64;
            return Ok(Some(bytes));
        }
        Ok(None)
    }
    pub(super) fn transferred(&self) -> u64 {
        self.transferred
    }
    pub(super) fn total(&self) -> u64 {
        self.total
    }
}

pub(super) struct ReceiveFiles {
    files: Vec<TransferFile>,
    destination: Arc<dyn crate::DirectFileDestination>,
    hash: Sha256,
    index: usize,
    offset: u64,
    transferred: u64,
    total: u64,
    committed: bool,
}
impl ReceiveFiles {
    pub(super) fn new(
        files: Vec<TransferFile>,
        destination: Arc<dyn crate::DirectFileDestination>,
        id: &str,
    ) -> Result<Self, String> {
        validate(&files)?;
        let manifest = files
            .iter()
            .map(|f| crate::DirectFileSnapshot {
                filename: f.filename.clone(),
                size_bytes: f.size_bytes,
                local_path: None,
            })
            .collect();
        destination
            .prepare(id.into(), manifest)
            .map_err(|error| error.to_string())?;
        let total = files.iter().map(|f| f.size_bytes).sum();
        let mut result = Self {
            files,
            destination,
            hash: Sha256::new(),
            index: 0,
            offset: 0,
            transferred: 0,
            total,
            committed: false,
        };
        result.advance()?;
        Ok(result)
    }
    fn advance(&mut self) -> Result<(), String> {
        while let Some(file) = self.files.get(self.index) {
            if self.offset != file.size_bytes {
                break;
            }
            if !format!("{:x}", self.hash.clone().finalize()).eq_ignore_ascii_case(&file.sha256) {
                return Err("Received file did not match the offer".into());
            }
            self.destination
                .finish_file(self.index as u32)
                .map_err(|e| e.to_string())?;
            self.index += 1;
            self.offset = 0;
            self.hash = Sha256::new();
        }
        Ok(())
    }
    pub(super) fn write(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err("Invalid file data frame".into());
        }
        while !bytes.is_empty() {
            let file = self
                .files
                .get(self.index)
                .ok_or("Received more data than offered")?;
            let length = (file.size_bytes - self.offset).min(bytes.len() as u64) as usize;
            let (chunk, rest) = bytes
                .split_at_checked(length)
                .ok_or("Invalid file data length")?;
            self.destination
                .write(self.index as u32, chunk.to_vec())
                .map_err(|e| e.to_string())?;
            self.hash.update(chunk);
            self.offset += length as u64;
            self.transferred += length as u64;
            bytes = rest;
            self.advance()?;
        }
        Ok(())
    }
    pub(super) fn commit(&mut self) -> Result<Vec<String>, String> {
        if self.index != self.files.len() {
            return Err("File transfer ended before all files arrived".into());
        }
        let paths = self.destination.commit().map_err(|e| e.to_string())?;
        if paths.len() != self.files.len() || paths.iter().any(String::is_empty) {
            return Err("Could not finish saving received files".into());
        }
        self.committed = true;
        Ok(paths)
    }
    pub(super) fn transferred(&self) -> u64 {
        self.transferred
    }
    pub(super) fn total(&self) -> u64 {
        self.total
    }
}
impl Drop for ReceiveFiles {
    fn drop(&mut self) {
        if !self.committed {
            self.destination.abort();
        }
    }
}

#[cfg(test)]
mod destination_tests {
    use super::*;
    use std::fs;

    fn manifest(name: &str, bytes: &[u8]) -> TransferFile {
        TransferFile {
            filename: name.into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            path: Default::default(),
        }
    }
    #[test]
    fn selected_destination_commits_only_after_every_file_hash_and_preserves_existing_files() {
        let folder = tempfile::tempdir().unwrap();
        let existing = folder.path().join("one.txt");
        fs::write(&existing, b"keep").unwrap();
        let id = "ab".repeat(16);
        let batch = folder.path().join(format!("Iris files {id}"));
        let output =
            crate::direct_file_directory_destination(folder.path().to_string_lossy().into_owned());
        let mut receive = ReceiveFiles::new(
            vec![manifest("one.txt", b"one"), manifest("two.txt", b"two")],
            output,
            &id,
        )
        .unwrap();
        receive.write(b"one").unwrap();
        assert!(!batch.join("1-one.txt").exists());
        assert!(receive.commit().is_err());
        receive.write(b"two").unwrap();
        assert!(!batch.join("1-one.txt").exists());
        let saved = receive.commit().unwrap();
        drop(receive);
        assert_eq!(fs::read(&saved[0]).unwrap(), b"one");
        assert_eq!(fs::read(&saved[1]).unwrap(), b"two");
        assert_eq!(fs::read(existing).unwrap(), b"keep");
        assert!(!batch.join("0.part").exists());
    }
    #[test]
    fn failed_hash_and_cancellation_remove_only_the_new_batch() {
        for corrupt in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let existing = folder.path().join("one.txt");
            fs::write(&existing, b"keep").unwrap();
            let id = "cd".repeat(16);
            let batch = folder.path().join(format!("Iris files {id}"));
            let output = crate::direct_file_directory_destination(
                folder.path().to_string_lossy().into_owned(),
            );
            let mut receive = ReceiveFiles::new(
                vec![manifest("one.txt", b"one"), manifest("two.txt", b"two")],
                output,
                &id,
            )
            .unwrap();
            receive.write(b"one").unwrap();
            if corrupt {
                assert!(receive.write(b"bad").is_err());
            }
            drop(receive);
            assert!(!batch.exists());
            assert_eq!(fs::read(existing).unwrap(), b"keep");
        }
    }
    #[test]
    fn partial_commit_failure_rolls_back_the_new_batch() {
        let folder = tempfile::tempdir().unwrap();
        let id = "ef".repeat(16);
        let batch = folder.path().join(format!("Iris files {id}"));
        let output =
            crate::direct_file_directory_destination(folder.path().to_string_lossy().into_owned());
        let mut receive = ReceiveFiles::new(
            vec![manifest("one.txt", b"one"), manifest("two.txt", b"two")],
            output,
            &id,
        )
        .unwrap();
        receive.write(b"onetwo").unwrap();
        fs::remove_file(batch.join("1.part")).unwrap();
        assert!(receive.commit().is_err());
        drop(receive);
        assert!(
            !batch.exists(),
            "a first rename followed by a failed rename must not leave partial success"
        );
    }
    #[test]
    fn large_sparse_sender_reads_only_bounded_chunks() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("large.bin");
        let file = File::create(&path).unwrap();
        file.set_len(4 * 1024 * 1024 * 1024).unwrap();
        drop(file);
        let mut send = SendFiles::new(vec![TransferFile {
            filename: "large.bin".into(),
            size_bytes: 4 * 1024 * 1024 * 1024,
            sha256: "ab".repeat(32),
            path,
        }])
        .unwrap();
        assert!(
            send.current.is_none(),
            "no file read until transport requests a chunk"
        );
        for i in 1..=64 {
            let chunk = send.next_chunk().unwrap().unwrap();
            assert_eq!(chunk.len(), CHUNK);
            assert_eq!(send.transferred(), i * CHUNK as u64);
        }
    }
}
