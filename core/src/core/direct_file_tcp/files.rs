use super::{TransferFile, CHUNK};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

pub(super) fn validate(files: &[TransferFile]) -> Result<(), String> {
    if files.is_empty() || files.len() > 32 {
        return Err("Choose between 1 and 32 files".into());
    }
    for file in files {
        if file.filename.is_empty()
            || file.filename.len() > 240
            || file.filename == "."
            || file.filename == ".."
            || file
                .filename
                .chars()
                .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
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
    directory: PathBuf,
    partial: Vec<PathBuf>,
    final_paths: Vec<PathBuf>,
    current: Option<File>,
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
        directory: PathBuf,
        id: &str,
    ) -> Result<Self, String> {
        validate(&files)?;
        // An exclusive directory prevents a replay or another transfer from
        // overwriting a previously saved file, even with duplicate filenames.
        fs::create_dir_all(&directory).map_err(|_| "Could not create file destination")?;
        let directory = directory.join(id);
        fs::create_dir(&directory).map_err(|_| "File destination is already in use")?;
        let total = files.iter().map(|f| f.size_bytes).sum();
        let mut result = Self {
            files,
            directory,
            partial: Vec::new(),
            final_paths: Vec::new(),
            current: None,
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
            if self.current.is_none() {
                let final_path =
                    self.directory
                        .join(format!("{}-{}", self.index + 1, file.filename));
                let partial_path = self.directory.join(format!("{}.part", self.index));
                let handle = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&partial_path)
                    .map_err(|_| "Could not save received file")?;
                self.current = Some(handle);
                self.partial.push(partial_path);
                self.final_paths.push(final_path);
            }
            if self.offset != file.size_bytes {
                break;
            }
            let computed = format!("{:x}", self.hash.clone().finalize());
            if !computed.eq_ignore_ascii_case(&file.sha256) {
                return Err("Received file did not match the offer".into());
            }
            self.current
                .take()
                .ok_or("File destination is closed")?
                .sync_all()
                .map_err(|_| "Could not save received file")?;
            self.index += 1;
            self.offset = 0;
            self.hash = Sha256::new();
        }
        Ok(())
    }
    pub(super) fn write(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Err("Empty file data frame".into());
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
            self.current
                .as_mut()
                .ok_or("File destination closed")?
                .write_all(chunk)
                .map_err(|_| "Could not save received file")?;
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
        for (partial, final_path) in self.partial.iter().zip(&self.final_paths) {
            fs::rename(partial, final_path)
                .map_err(|_| "Could not finish saving received files")?;
        }
        self.committed = true;
        Ok(self
            .final_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
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
        if self.committed {
            return;
        }
        self.current = None;
        for path in self.partial.iter().chain(&self.final_paths) {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&self.directory);
    }
}
