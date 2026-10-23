use super::{SqliteStorageAdapter, StorageError, StorageResult};
use rusqlite::{blob::Blob, types::ValueRef, Connection, DatabaseName, OptionalExtension};

const CHUNK_BYTES: usize = 64 * 1024;
const PAGE_BYTES: usize = 4096;
type Namespace<'a> = (&'a str, &'a str, &'a str);

#[cfg(test)]
thread_local! {
    static IO_BYTES: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
pub(super) fn io_bytes() -> (usize, usize) {
    IO_BYTES.with(std::cell::Cell::get)
}

pub(super) fn account_io(_read: usize, _written: usize) {
    #[cfg(test)]
    IO_BYTES.with(|count| {
        let (read, written) = count.get();
        count.set((read + _read, written + _written));
    });
}

fn invalid() -> StorageError {
    StorageError::new("invalid segmented storage record")
}

// No cached schema flag: creating these objects inside an outer transaction
// may be rolled back. Reads never create schema, including on read-only DBs.
fn schema(conn: &Connection, create: bool) -> StorageResult<bool> {
    let (objects, valid): (i64, i64) = conn.query_row(
        "SELECT count(*),coalesce(sum((name IN ('ndr_kv_segmented','ndr_kv_chunks') AND type='table') OR
         (name='ndr_kv_segmented_delete' AND type='trigger')),0) FROM sqlite_schema
         WHERE name IN ('ndr_kv_segmented','ndr_kv_chunks','ndr_kv_segmented_delete')", [], |r| Ok((r.get(0)?, r.get(1)?)),
    ).map_err(SqliteStorageAdapter::map_err)?;
    if objects == 3 && valid == 3 {
        return Ok(true);
    }
    if objects != 0 {
        return Err(invalid());
    }
    if !create {
        return Ok(false);
    }
    conn.execute_batch(
        "CREATE TABLE ndr_kv_segmented(
           owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL, key TEXT NOT NULL,
           format_version INTEGER NOT NULL, byte_length INTEGER NOT NULL, chunk_bytes INTEGER NOT NULL,
           PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key));
         CREATE TABLE ndr_kv_chunks(
           owner_pubkey_hex TEXT NOT NULL, device_pubkey_hex TEXT NOT NULL, key TEXT NOT NULL,
           chunk_index INTEGER NOT NULL, data BLOB NOT NULL,
           PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key,chunk_index));
         CREATE TRIGGER ndr_kv_segmented_delete AFTER DELETE ON ndr_kv BEGIN
           DELETE FROM ndr_kv_chunks WHERE owner_pubkey_hex=OLD.owner_pubkey_hex
             AND device_pubkey_hex=OLD.device_pubkey_hex AND key=OLD.key;
           DELETE FROM ndr_kv_segmented WHERE owner_pubkey_hex=OLD.owner_pubkey_hex
             AND device_pubkey_hex=OLD.device_pubkey_hex AND key=OLD.key;
         END;",
    ).map_err(SqliteStorageAdapter::map_err)?;
    Ok(true)
}

fn metadata(conn: &Connection, ns: Namespace<'_>) -> StorageResult<Option<usize>> {
    let row: Option<(i64, i64, i64)> = conn
        .query_row(
            "SELECT format_version,byte_length,chunk_bytes FROM ndr_kv_segmented
         WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3",
            ns,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(SqliteStorageAdapter::map_err)?;
    if let Some((version, bytes, chunk)) = row {
        let bytes = usize::try_from(bytes).map_err(|_| invalid())?;
        if version != 1 || chunk != CHUNK_BYTES as i64 || bytes < CHUNK_BYTES {
            return Err(invalid());
        }
        let anchor: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM ndr_kv WHERE owner_pubkey_hex=?1
             AND device_pubkey_hex=?2 AND key=?3 AND typeof(value)='text' AND length(CAST(value AS BLOB))=0)",
            ns, |r| r.get(0),
        ).map_err(SqliteStorageAdapter::map_err)?;
        if !anchor {
            return Err(invalid());
        }
        return Ok(Some(bytes));
    }
    let orphan: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3)",
        ns, |r| r.get(0),
    ).map_err(SqliteStorageAdapter::map_err)?;
    if orphan {
        return Err(invalid());
    }
    Ok(None)
}

fn read_chunks(
    conn: &Connection,
    ns: Namespace<'_>,
    bytes: usize,
    mut visit: impl FnMut(i64, usize, &[u8]) -> StorageResult<()>,
) -> StorageResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT rowid,chunk_index,data FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1
         AND device_pubkey_hex=?2 AND key=?3 ORDER BY chunk_index",
        )
        .map_err(SqliteStorageAdapter::map_err)?;
    let mut rows = stmt.query(ns).map_err(SqliteStorageAdapter::map_err)?;
    let count = bytes.div_ceil(CHUNK_BYTES);
    let mut index = 0;
    while let Some(row) = rows.next().map_err(SqliteStorageAdapter::map_err)? {
        let stored_index: i64 = row.get(1).map_err(SqliteStorageAdapter::map_err)?;
        if index >= count || stored_index != index as i64 {
            return Err(invalid());
        }
        let ValueRef::Blob(data) = row.get_ref(2).map_err(SqliteStorageAdapter::map_err)? else {
            return Err(invalid());
        };
        if data.len() != (bytes - index * CHUNK_BYTES).min(CHUNK_BYTES) {
            return Err(invalid());
        }
        account_io(data.len(), 0);
        visit(
            row.get(0).map_err(SqliteStorageAdapter::map_err)?,
            index,
            data,
        )?;
        index += 1;
    }
    if index != count {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn get(conn: &mut Connection, ns: Namespace<'_>) -> StorageResult<Option<String>> {
    get_with(conn, ns, || Ok(()))
}

// The hook lets a test commit on a second connection at the metadata/chunk
// boundary. Production monomorphizes the empty hook away.
pub(super) fn get_with(
    conn: &mut Connection,
    ns: Namespace<'_>,
    after_metadata: impl FnOnce() -> StorageResult<()>,
) -> StorageResult<Option<String>> {
    // The metadata and every chunk must see the same SQLite snapshot even if
    // a notification extension or another connection commits a new generation.
    let tx = conn.savepoint().map_err(SqliteStorageAdapter::map_err)?;
    if let Some(value) = super::checkpoint_sqlite::get_in_snapshot(&tx, ns)? {
        after_metadata()?;
        tx.commit().map_err(SqliteStorageAdapter::map_err)?;
        return Ok(Some(value));
    }
    let bytes = if schema(&tx, false)? {
        metadata(&tx, ns)?
    } else {
        None
    };
    after_metadata()?;
    let result = if let Some(bytes) = bytes {
        let mut result = Vec::new();
        read_chunks(&tx, ns, bytes, |_, _, chunk| {
            result
                .try_reserve(chunk.len())
                .map_err(SqliteStorageAdapter::map_err)?;
            result.extend_from_slice(chunk);
            Ok(())
        })?;
        Some(String::from_utf8(result).map_err(|_| invalid())?)
    } else {
        let value: Option<String> = tx.query_row(
            "SELECT value FROM ndr_kv WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3",
            ns, |r| r.get(0),
        ).optional().map_err(SqliteStorageAdapter::map_err)?;
        if let Some(value) = &value {
            account_io(value.len(), 0);
        }
        value
    };
    tx.commit().map_err(SqliteStorageAdapter::map_err)?;
    Ok(result)
}

enum Change {
    Replace(usize),
    Patch {
        rowid: i64,
        index: usize,
        pages: Vec<usize>,
    },
}

pub(super) fn put(conn: &mut Connection, ns: Namespace<'_>, value: &str) -> StorageResult<()> {
    put_with(
        conn,
        ns,
        value,
        |blob, bytes, offset| blob.write_at(bytes, offset),
        |blob| blob.close(),
    )
}

// Fault-injectable operations preserve close/write error handling in tests;
// production supplies the ordinary SQLite BLOB methods with no extra state.
pub(super) fn put_with(
    conn: &mut Connection,
    ns: Namespace<'_>,
    value: &str,
    mut write: impl FnMut(&mut Blob<'_>, &[u8], usize) -> rusqlite::Result<()>,
    mut close: impl FnMut(Blob<'_>) -> rusqlite::Result<()>,
) -> StorageResult<()> {
    let tx = conn.savepoint().map_err(SqliteStorageAdapter::map_err)?;
    super::checkpoint_sqlite::clear_in_snapshot(&tx, ns)?;
    schema(&tx, true)?;
    let previous = metadata(&tx, ns)?;
    let (owner, device, key) = ns;
    if value.len() < CHUNK_BYTES {
        if let Some(bytes) = previous {
            read_chunks(&tx, ns, bytes, |_, _, _| Ok(()))?;
            tx.execute("DELETE FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
            tx.execute("DELETE FROM ndr_kv_segmented WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
        }
        let changed = tx
            .execute(
            "INSERT INTO ndr_kv(owner_pubkey_hex,device_pubkey_hex,key,value) VALUES(?1,?2,?3,?4)
             ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key) DO UPDATE SET value=excluded.value
             WHERE value IS NOT excluded.value",
                (owner, device, key, value),
            )
            .map_err(SqliteStorageAdapter::map_err)?;
        if changed > 0 {
            account_io(0, value.len());
        }
        tx.commit().map_err(SqliteStorageAdapter::map_err)?;
        return Ok(());
    }
    // Retain only numeric locations, not another secret checkpoint snapshot.
    // Finish validation/comparison before changing the table under its cursor.
    let mut changes = Vec::new();
    if let Some(bytes) = previous {
        read_chunks(&tx, ns, bytes, |rowid, index, old| {
            let offset = index * CHUNK_BYTES;
            let Some(new) = value
                .as_bytes()
                .get(offset..value.len().min(offset + CHUNK_BYTES))
            else {
                return Ok(());
            };
            if new.is_empty() {
                return Ok(());
            }
            if old.len() != new.len() {
                changes.push(Change::Replace(index));
            } else {
                let pages = old
                    .chunks(PAGE_BYTES)
                    .zip(new.chunks(PAGE_BYTES))
                    .enumerate()
                    .filter_map(|(page, (old, new))| (old != new).then_some(page))
                    .collect::<Vec<_>>();
                if !pages.is_empty() {
                    changes.push(Change::Patch {
                        rowid,
                        index,
                        pages,
                    });
                }
            }
            Ok(())
        })?;
    }
    for index in previous.unwrap_or(0).div_ceil(CHUNK_BYTES)..value.len().div_ceil(CHUNK_BYTES) {
        changes.push(Change::Replace(index));
    }
    for change in changes {
        match change {
            Change::Replace(index) => {
                let offset = index * CHUNK_BYTES;
                let data = &value.as_bytes()[offset..value.len().min(offset + CHUNK_BYTES)];
                tx.execute(
                    "INSERT INTO ndr_kv_chunks(owner_pubkey_hex,device_pubkey_hex,key,chunk_index,data)
                     VALUES(?1,?2,?3,?4,?5) ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key,chunk_index)
                     DO UPDATE SET data=excluded.data", (owner, device, key, index as i64, data),
                ).map_err(SqliteStorageAdapter::map_err)?;
                account_io(0, data.len());
            }
            Change::Patch {
                rowid,
                index,
                pages,
            } => {
                let mut blob = tx
                    .blob_open(DatabaseName::Main, "ndr_kv_chunks", "data", rowid, false)
                    .map_err(SqliteStorageAdapter::map_err)?;
                for page in pages {
                    let offset = index * CHUNK_BYTES + page * PAGE_BYTES;
                    let data = &value.as_bytes()[offset..value.len().min(offset + PAGE_BYTES)];
                    write(&mut blob, data, page * PAGE_BYTES)
                        .map_err(SqliteStorageAdapter::map_err)?;
                    account_io(0, data.len());
                }
                close(blob).map_err(SqliteStorageAdapter::map_err)?;
            }
        }
    }
    if previous.is_some_and(|bytes| bytes > value.len()) {
        tx.execute("DELETE FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND chunk_index>=?4",
            (owner, device, key, value.len().div_ceil(CHUNK_BYTES) as i64)).map_err(SqliteStorageAdapter::map_err)?;
    }
    if previous.is_none() {
        // This is the sole full-value migration. Never retain a large mirror.
        tx.execute("INSERT INTO ndr_kv(owner_pubkey_hex,device_pubkey_hex,key,value) VALUES(?1,?2,?3,'') ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key) DO UPDATE SET value=''", ns).map_err(SqliteStorageAdapter::map_err)?;
    }
    if previous != Some(value.len()) {
        tx.execute("INSERT INTO ndr_kv_segmented(owner_pubkey_hex,device_pubkey_hex,key,format_version,byte_length,chunk_bytes) VALUES(?1,?2,?3,1,?4,?5)
            ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key) DO UPDATE SET byte_length=excluded.byte_length",
            (owner, device, key, value.len() as i64, CHUNK_BYTES as i64)).map_err(SqliteStorageAdapter::map_err)?;
    }
    tx.commit().map_err(SqliteStorageAdapter::map_err)
}

pub(super) fn delete(conn: &mut Connection, ns: Namespace<'_>) -> StorageResult<()> {
    let tx = conn.savepoint().map_err(SqliteStorageAdapter::map_err)?;
    super::checkpoint_sqlite::clear_in_snapshot(&tx, ns)?;
    if schema(&tx, false)? {
        tx.execute("DELETE FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
        tx.execute("DELETE FROM ndr_kv_segmented WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
    }
    // The trigger covers this path and direct namespace deletion at logout.
    tx.execute(
        "DELETE FROM ndr_kv WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3",
        ns,
    )
    .map_err(SqliteStorageAdapter::map_err)?;
    tx.commit().map_err(SqliteStorageAdapter::map_err)
}

pub(super) fn clear_in_snapshot(conn: &Connection, ns: Namespace<'_>) -> StorageResult<()> {
    if !schema(conn, false)? {
        return Ok(());
    }
    if let Some(bytes) = metadata(conn, ns)? {
        // A format transition validates the old value once before retirement.
        read_chunks(conn, ns, bytes, |_, _, _| Ok(()))?;
        conn.execute("DELETE FROM ndr_kv_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
        conn.execute("DELETE FROM ndr_kv_segmented WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns).map_err(SqliteStorageAdapter::map_err)?;
    }
    Ok(())
}

pub(super) fn has_value_in_snapshot(conn: &Connection, ns: Namespace<'_>) -> StorageResult<bool> {
    Ok(schema(conn, false)? && metadata(conn, ns)?.is_some())
}
