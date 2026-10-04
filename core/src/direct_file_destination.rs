//! User-selected output streams. The transfer worker, never the UI thread,
//! calls these methods with bounded chunks and commits only after all hashes pass.
use crate::DirectFileSnapshot;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, uniffi::Error)]
pub enum DirectFileDestinationError {
    Failed { reason: String },
}
impl fmt::Display for DirectFileDestinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { reason } => f.write_str(reason),
        }
    }
}
impl std::error::Error for DirectFileDestinationError {}
impl From<uniffi::UnexpectedUniFFICallbackError> for DirectFileDestinationError {
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        failure("Could not save received files.")
    }
}

impl From<std::io::Error> for DirectFileDestinationError {
    fn from(_: std::io::Error) -> Self {
        failure("Could not save received files.")
    }
}
fn failure(message: &str) -> DirectFileDestinationError {
    DirectFileDestinationError::Failed {
        reason: message.into(),
    }
}
type Result<T> = std::result::Result<T, DirectFileDestinationError>;

/// Each selected destination belongs to one acceptance. `abort` must remove only
/// newly created partial outputs; it must never delete or truncate existing files.
/// `prepare` must clean its own partial preparation on error, without disturbing an
/// already-open destination if the same object is accidentally reused.
#[uniffi::export(with_foreign)]
pub trait DirectFileDestination: Send + Sync {
    fn prepare(&self, transfer_id: String, files: Vec<DirectFileSnapshot>) -> Result<()>;
    fn write(&self, file_index: u32, bytes: Vec<u8>) -> Result<()>;
    fn finish_file(&self, file_index: u32) -> Result<()>;
    fn commit(&self) -> Result<Vec<String>>;
    fn abort(&self);
}
impl fmt::Debug for dyn DirectFileDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Selected file destination")
    }
}

/// Save directly into a new batch folder within the folder selected by the user.
/// The exclusive batch folder keeps existing destination files untouched.
#[uniffi::export]
pub fn direct_file_directory_destination(directory: String) -> Arc<dyn DirectFileDestination> {
    Arc::new(DirectoryDestination {
        directory: PathBuf::from(directory),
        state: Mutex::new(None),
    })
}
struct DirectoryDestination {
    directory: PathBuf,
    state: Mutex<Option<DirectoryState>>,
}
struct DirectoryState {
    directory: PathBuf,
    partial: Vec<PathBuf>,
    paths: Vec<PathBuf>,
    handles: Vec<Option<File>>,
    committed: bool,
}
impl Drop for DirectoryState {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.handles.clear();
        for path in self.partial.iter().chain(&self.paths) {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&self.directory);
    }
}
impl DirectFileDestination for DirectoryDestination {
    fn prepare(&self, id: String, files: Vec<DirectFileSnapshot>) -> Result<()> {
        if !crate::direct_files::valid_transfer_id(&id)
            || files.is_empty()
            || files.len() > 32
            || files
                .iter()
                .any(|f| !crate::direct_files::valid_filename(&f.filename))
        {
            return Err(failure("Invalid file destination."));
        }
        let mut slot = self
            .state
            .lock()
            .map_err(|_| failure("File destination is unavailable."))?;
        if slot.is_some() {
            return Err(failure("File destination is already in use."));
        }
        if !self.directory.is_dir() {
            return Err(failure("Choose a folder to save these files."));
        }
        let directory = self.directory.join(format!("Iris files {id}"));
        fs::create_dir(&directory)?;
        let mut state = DirectoryState {
            directory,
            partial: vec![],
            paths: vec![],
            handles: vec![],
            committed: false,
        };
        for (i, file) in files.iter().enumerate() {
            let path = state.directory.join(format!("{}.part", i));
            let handle = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            state.partial.push(path);
            state
                .paths
                .push(state.directory.join(format!("{}-{}", i + 1, file.filename)));
            state.handles.push(Some(handle));
        }
        *slot = Some(state);
        Ok(())
    }
    fn write(&self, index: u32, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() > 32 * 1024 {
            return Err(failure("File chunk is too large."));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("File destination is unavailable."))?;
        state
            .as_mut()
            .and_then(|s| s.handles.get_mut(index as usize))
            .and_then(Option::as_mut)
            .ok_or_else(|| failure("File destination is closed."))?
            .write_all(&bytes)?;
        Ok(())
    }
    fn finish_file(&self, index: u32) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("File destination is unavailable."))?;
        state
            .as_mut()
            .and_then(|s| s.handles.get_mut(index as usize))
            .and_then(Option::take)
            .ok_or_else(|| failure("File destination is closed."))?
            .sync_all()?;
        Ok(())
    }
    fn commit(&self) -> Result<Vec<String>> {
        let mut slot = self
            .state
            .lock()
            .map_err(|_| failure("File destination is unavailable."))?;
        let state = slot
            .as_mut()
            .ok_or_else(|| failure("File destination is closed."))?;
        if state.committed || state.handles.iter().any(Option::is_some) {
            return Err(failure("Files are not ready to save."));
        }
        for (partial, path) in state.partial.iter().zip(&state.paths) {
            fs::rename(partial, path)?;
        }
        #[cfg(unix)]
        {
            File::open(&state.directory)?.sync_all()?;
            File::open(&self.directory)?.sync_all()?;
        }
        state.committed = true;
        Ok(state
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    }
    fn abort(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.take();
        }
    }
}
