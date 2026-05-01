use super::schema;
use super::SharedConnection;
use rusqlite::Connection;
#[cfg(not(target_os = "ios"))]
use rusqlite::ErrorCode;
use std::path::Path;
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "ios"))]
use std::time::Duration;

pub(crate) const CORE_DB_FILENAME: &str = "core.sqlite3";
#[cfg(not(target_os = "ios"))]
pub(crate) const CORE_LOCK_DB_FILENAME: &str = "core.lock.sqlite3";

#[cfg(not(target_os = "ios"))]
pub(crate) struct DataDirLock {
    _conn: Connection,
}

#[cfg(target_os = "ios")]
pub(crate) struct DataDirLock;

#[cfg(not(target_os = "ios"))]
impl DataDirLock {
    pub(crate) fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join(CORE_LOCK_DB_FILENAME);
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_millis(250))?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        match conn.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS core_lock (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 acquired_at_secs INTEGER NOT NULL
             );
             INSERT INTO core_lock(id, acquired_at_secs)
             VALUES (1, strftime('%s', 'now'))
             ON CONFLICT(id) DO UPDATE SET acquired_at_secs = excluded.acquired_at_secs;",
        ) {
            Ok(()) => Ok(Self { _conn: conn }),
            Err(error) if is_lock_busy(&error) => {
                Err(anyhow::anyhow!("Iris is already using this data folder."))
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(target_os = "ios")]
impl DataDirLock {
    pub(crate) fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        // iOS kills background-suspended apps that hold file or SQLite locks
        // (RunningBoard 0xdead10cc). The app has one foreground core process,
        // while the notification extension uses overlay storage and does not
        // own the live ratchet writer, so there is no long-lived OS lock here.
        std::fs::create_dir_all(data_dir)?;
        Ok(Self)
    }
}

#[cfg(not(target_os = "ios"))]
fn is_lock_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(inner.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

pub(crate) fn open_database(data_dir: &Path) -> anyhow::Result<SharedConnection> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(CORE_DB_FILENAME);
    let mut conn = Connection::open(&path)?;
    apply_pragmas(&conn)?;
    schema::ensure_schema(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn apply_pragmas(conn: &Connection) -> anyhow::Result<()> {
    // foreign_keys is per-connection and must be set every open.
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // WAL is persistent in the file header but is cheap to re-apply.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    // NORMAL is the usual mobile WAL trade-off. Power-loss durability
    // is bounded by the most-recent commit; regular WAL checkpoints
    // still flush to the main file.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}
