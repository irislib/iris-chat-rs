use super::schema;
use super::SharedConnection;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) const CORE_DB_FILENAME: &str = "core.sqlite3";

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
