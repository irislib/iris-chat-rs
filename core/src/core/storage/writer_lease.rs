//! Cross-process ownership for a core that can yield its storage while suspended.
//!
//! This is the handoff primitive, not permission to open another live core. A
//! caller must drain storage and stop all writers before dropping the lease.
//! Reacquisition runs the supplied reload before returning write authority when
//! another owner (including one that crashed) may have touched the account.

use rand::RngCore;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const LEASE_FILE: &str = "core.writer.lock";
const EPOCH_BYTES: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriterEpoch([u8; EPOCH_BYTES]);

pub(crate) struct CoreWriterLease {
    // Never rename or unlink this file: every process must lock the same inode.
    _file: File,
    epoch: WriterEpoch,
}

impl CoreWriterLease {
    /// A busy owner is an ordinary retry, never a reason to bypass the lock.
    /// `cached_epoch` belongs to the in-memory state being reused by this core.
    /// Pass None for a cold start; `reload` must then initialize from storage.
    pub(crate) fn try_acquire(
        data_dir: &Path,
        cached_epoch: Option<WriterEpoch>,
        reload: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<Self>> {
        super::connection::ensure_private_data_dir(data_dir)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(data_dir.join(LEASE_FILE))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let mut previous = [0; EPOCH_BYTES];
        let previous = if file.metadata()?.len() == EPOCH_BYTES as u64 {
            file.read_exact(&mut previous)?;
            Some(WriterEpoch(previous))
        } else {
            // An empty or torn marker never proves that retained state is fresh.
            None
        };

        // Persist a fresh token BEFORE reload or any account access. If this
        // owner dies, a retained core must reload even though no normal release
        // callback ran. The marker contains no identity, recipient or secret.
        let mut epoch = [0; EPOCH_BYTES];
        rand::rngs::OsRng.try_fill_bytes(&mut epoch)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&epoch)?;
        file.set_len(EPOCH_BYTES as u64)?;
        file.sync_all()?;

        if previous.is_none() || previous != cached_epoch {
            reload()?;
        }
        Ok(Some(Self {
            _file: file,
            epoch: WriterEpoch(epoch),
        }))
    }

    pub(crate) fn epoch(&self) -> WriterEpoch {
        self.epoch
    }
}

#[cfg(test)]
#[path = "writer_lease_tests.rs"]
mod tests;
