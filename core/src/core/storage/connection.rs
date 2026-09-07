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
        ensure_private_data_dir(data_dir)?;
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
        ensure_private_data_dir(data_dir)?;
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
    ensure_private_data_dir(data_dir)?;
    let path = data_dir.join(CORE_DB_FILENAME);
    let mut conn = Connection::open(&path)?;
    apply_pragmas(&conn)?;
    schema::ensure_schema(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn ensure_private_data_dir(data_dir: &Path) -> anyhow::Result<()> {
    // The database, SQLite sidecars, and notification caches contain decrypted
    // messages and session keys. Secure their directory before opening any file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data_dir)?;
        // Also repair directories created by older versions with the process umask.
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(data_dir)?;
    Ok(())
}

fn apply_pragmas(conn: &Connection) -> anyhow::Result<()> {
    // foreign_keys is per-connection and must be set every open.
    conn.pragma_update(None, "foreign_keys", "ON")?;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // Mobile app databases have one foreground writer; auxiliary readers
        // are short-lived. WAL's shared-memory index has caused platform-level
        // crashes in that shape (RunningBoard lock kills on iOS, SIGBUS in
        // walIndexAppend on Android emulators), so mobile uses rollback
        // journaling instead of keeping a long-lived WAL mapping.
        conn.pragma_update(None, "journal_mode", "DELETE")?;
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    // WAL is persistent in the file header but is cheap to re-apply.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    // NORMAL keeps write latency low. In WAL mode durability is bounded by the
    // most recent checkpoint; on mobile DELETE mode avoids long-lived locks.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn assert_private_directory(path: &Path) {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn database_directory_is_private_on_creation_and_reopen() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("account");
        drop(open_database(&data_dir).unwrap());
        assert_private_directory(&data_dir);

        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        drop(open_database(&data_dir).unwrap());
        assert_private_directory(&data_dir);
    }

    #[test]
    fn acquiring_data_lock_secures_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("account");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _lock = DataDirLock::acquire(&data_dir).unwrap();
        assert_private_directory(&data_dir);
    }
}
