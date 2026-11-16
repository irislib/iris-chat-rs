//! Small files that let the updater remember the last check and tell, after
//! a restart, whether the update it started actually finished.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const STATUS_FILE: &str = "update-status.json";
const PENDING_FILE: &str = "update-pending.json";
/// A restart that has not happened after this long no longer explains anything.
const PENDING_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CheckOutcome {
    UpToDate,
    Available,
    CheckFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SavedStatus {
    pub(super) checked_at: u64,
    pub(super) outcome: CheckOutcome,
    #[serde(default)]
    pub(super) version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PendingUpdate {
    pub(super) version: String,
    pub(super) from_version: String,
    pub(super) started_at: u64,
}

pub(super) struct UpdateStore {
    dir: PathBuf,
}

impl UpdateStore {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(super) fn load_status(&self) -> Option<SavedStatus> {
        read_json(&self.dir.join(STATUS_FILE))
    }

    pub(super) fn save_status(&self, status: &SavedStatus) {
        write_json(&self.dir, STATUS_FILE, status);
    }

    pub(super) fn mark_pending(&self, pending: &PendingUpdate) {
        write_json(&self.dir, PENDING_FILE, pending);
    }

    pub(super) fn clear_pending(&self) {
        let _ = fs::remove_file(self.dir.join(PENDING_FILE));
    }

    /// Read and forget the update started before the last restart.
    pub(super) fn take_pending(&self, now: u64) -> Option<PendingUpdate> {
        let path = self.dir.join(PENDING_FILE);
        let pending: Option<PendingUpdate> = read_json(&path);
        let _ = fs::remove_file(&path);
        pending.filter(|pending| now.saturating_sub(pending.started_at) <= PENDING_MAX_AGE_SECS)
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) {
    let Ok(bytes) = serde_json::to_vec(value) else {
        return;
    };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(name);
    let temporary = dir.join(format!(".{name}.{}", std::process::id()));
    if fs::write(&temporary, bytes).is_ok() && fs::rename(&temporary, &path).is_err() {
        let _ = fs::remove_file(&temporary);
    }
}
