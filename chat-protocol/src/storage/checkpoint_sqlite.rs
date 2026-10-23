use super::checkpoint::{append_padding, PartBytes, StorageCheckpoint};
use super::{segmented, SqliteStorageAdapter, StorageError, StorageResult};
use nostr::hashes::{sha256, Hash};
use rusqlite::{types::ValueRef, Connection, DatabaseName, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};

type Namespace<'a> = (&'a str, &'a str, &'a str);
const CHUNK: usize = 64 * 1024;
const PAGE: usize = 4096;
fn invalid() -> StorageError {
    StorageError::new("invalid record checkpoint")
}
fn err(error: impl std::fmt::Display) -> StorageError {
    SqliteStorageAdapter::map_err(error)
}
fn number(value: i64) -> StorageResult<usize> {
    usize::try_from(value).map_err(|_| invalid())
}

fn schema(conn: &Connection, create: bool) -> StorageResult<bool> {
    let (count, valid): (i64, i64) = conn.query_row(
        "SELECT count(*),coalesce(sum((name IN ('ndr_kv_checkpoint','ndr_kv_checkpoint_nodes','ndr_kv_checkpoint_chunks') AND type='table') OR (name='ndr_kv_checkpoint_delete' AND type='trigger')),0)
         FROM sqlite_schema WHERE name IN ('ndr_kv_checkpoint','ndr_kv_checkpoint_nodes','ndr_kv_checkpoint_chunks','ndr_kv_checkpoint_delete')", [], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(err)?;
    if count == 4 && valid == 4 {
        return Ok(true);
    }
    if count != 0 {
        return Err(invalid());
    }
    if !create {
        return Ok(false);
    }
    conn.execute_batch(
        "CREATE TABLE ndr_kv_checkpoint(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,
           format_version INTEGER NOT NULL,byte_length INTEGER NOT NULL,node_count INTEGER NOT NULL,head BLOB,
           PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key));
         CREATE TABLE ndr_kv_checkpoint_nodes(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,
           node_key BLOB NOT NULL,next_key BLOB,payload_key BLOB NOT NULL,byte_length INTEGER NOT NULL,
           prefix TEXT NOT NULL,suffix TEXT NOT NULL,padding INTEGER NOT NULL,
           PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key,node_key));
         CREATE TABLE ndr_kv_checkpoint_chunks(owner_pubkey_hex TEXT NOT NULL,device_pubkey_hex TEXT NOT NULL,key TEXT NOT NULL,
           payload_key BLOB NOT NULL,chunk_index INTEGER NOT NULL,data BLOB NOT NULL,
           PRIMARY KEY(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index));
         CREATE TRIGGER ndr_kv_checkpoint_delete AFTER DELETE ON ndr_kv BEGIN
           DELETE FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=OLD.owner_pubkey_hex AND device_pubkey_hex=OLD.device_pubkey_hex AND key=OLD.key;
           DELETE FROM ndr_kv_checkpoint_nodes WHERE owner_pubkey_hex=OLD.owner_pubkey_hex AND device_pubkey_hex=OLD.device_pubkey_hex AND key=OLD.key;
           DELETE FROM ndr_kv_checkpoint WHERE owner_pubkey_hex=OLD.owner_pubkey_hex AND device_pubkey_hex=OLD.device_pubkey_hex AND key=OLD.key;
         END;"
    ).map_err(err)?;
    Ok(true)
}

#[derive(PartialEq, Eq)]
struct Meta {
    bytes: usize,
    count: usize,
    head: Option<Vec<u8>>,
}
#[derive(PartialEq, Eq)]
struct Node {
    next: Option<Vec<u8>>,
    payload: Vec<u8>,
    bytes: usize,
    prefix: String,
    suffix: String,
    padding: usize,
}
impl Node {
    fn stored_bytes(&self) -> usize {
        self.next.as_ref().map_or(0, Vec::len)
            + self.payload.len()
            + self.prefix.len()
            + self.suffix.len()
            + 16
    }
    fn logical_bytes(&self) -> StorageResult<usize> {
        self.bytes
            .checked_add(self.prefix.len())
            .and_then(|n| n.checked_add(self.suffix.len()))
            .and_then(|n| n.checked_add(self.padding))
            .ok_or_else(invalid)
    }
}

fn metadata(conn: &Connection, ns: Namespace<'_>) -> StorageResult<Option<Meta>> {
    if !schema(conn, false)? {
        return Ok(None);
    }
    let row: Option<(i64,i64,i64,Option<Vec<u8>>)> = conn.query_row(
        "SELECT format_version,byte_length,node_count,head FROM ndr_kv_checkpoint WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3", ns,
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(err)?;
    if let Some((version, bytes, count, head)) = row {
        if version != 1 {
            return Err(invalid());
        }
        if segmented::has_value_in_snapshot(conn, ns)? {
            return Err(invalid());
        }
        let anchor: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM ndr_kv WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND typeof(value)='text' AND length(CAST(value AS BLOB))=0)",ns,|r|r.get(0)).map_err(err)?;
        if !anchor {
            return Err(invalid());
        }
        segmented::account_io(24 + head.as_ref().map_or(0, Vec::len), 0);
        return Ok(Some(Meta {
            bytes: number(bytes)?,
            count: number(count)?,
            head,
        }));
    }
    let orphan: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM ndr_kv_checkpoint_nodes WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3) OR EXISTS(SELECT 1 FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3)",ns,|r|r.get(0)).map_err(err)?;
    if orphan {
        return Err(invalid());
    }
    Ok(None)
}

fn nodes(
    conn: &Connection,
    ns: Namespace<'_>,
    meta: &Meta,
) -> StorageResult<BTreeMap<Vec<u8>, Node>> {
    let mut stmt = conn.prepare("SELECT node_key,next_key,payload_key,byte_length,prefix,suffix,padding FROM ndr_kv_checkpoint_nodes WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3").map_err(err)?;
    let mut rows = stmt.query(ns).map_err(err)?;
    let mut result = BTreeMap::new();
    while let Some(row) = rows.next().map_err(err)? {
        let key: Vec<u8> = row.get(0).map_err(err)?;
        let node = Node {
            next: row.get(1).map_err(err)?,
            payload: row.get(2).map_err(err)?,
            bytes: number(row.get(3).map_err(err)?)?,
            prefix: row.get(4).map_err(err)?,
            suffix: row.get(5).map_err(err)?,
            padding: number(row.get(6).map_err(err)?)?,
        };
        if !matches!(node.payload.first(), Some(0 | 1))
            || (node.payload[0] == 1 && node.payload.len() != 33)
        {
            return Err(invalid());
        }
        segmented::account_io(key.len() + node.stored_bytes(), 0);
        if result.insert(key, node).is_some() {
            return Err(invalid());
        }
    }
    if result.len() != meta.count {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut next = meta.head.as_ref();
    let mut total = 0usize;
    while let Some(key) = next {
        let node = result.get(key).ok_or_else(invalid)?;
        if !seen.insert(key) {
            return Err(invalid());
        }
        total = total
            .checked_add(node.logical_bytes()?)
            .ok_or_else(invalid)?;
        next = node.next.as_ref();
    }
    if seen.len() != result.len() || total != meta.bytes {
        return Err(invalid());
    }
    Ok(result)
}

// Validate references and chunk shape without reading immutable payload bytes.
// A full read verifies their hashes. Ordinary saves trust the once-computed
// SHA-256 identities, not a mutable caller-supplied content/version token.
fn payloads(
    conn: &Connection,
    ns: Namespace<'_>,
    nodes: &BTreeMap<Vec<u8>, Node>,
) -> StorageResult<BTreeMap<Vec<u8>, usize>> {
    let mut expected = BTreeMap::new();
    for node in nodes.values() {
        if expected
            .insert(node.payload.clone(), node.bytes)
            .is_some_and(|old| old != node.bytes)
        {
            return Err(invalid());
        }
    }
    let mut indices = BTreeMap::<Vec<u8>, usize>::new();
    let mut stmt = conn.prepare("SELECT payload_key,chunk_index,length(data),typeof(data) FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 ORDER BY payload_key,chunk_index").map_err(err)?;
    let mut rows = stmt.query(ns).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let key: Vec<u8> = row.get(0).map_err(err)?;
        let index = number(row.get(1).map_err(err)?)?;
        let bytes = number(row.get(2).map_err(err)?)?;
        let kind: String = row.get(3).map_err(err)?;
        let total = *expected.get(&key).ok_or_else(invalid)?;
        let next = indices.entry(key.clone()).or_default();
        if index != *next
            || index >= total.div_ceil(CHUNK)
            || bytes != (total - index * CHUNK).min(CHUNK)
            || kind != "blob"
        {
            return Err(invalid());
        }
        *next += 1;
        segmented::account_io(key.len() + 20, 0);
    }
    if expected
        .iter()
        .any(|(key, bytes)| indices.get(key).copied().unwrap_or(0) != bytes.div_ceil(CHUNK))
    {
        return Err(invalid());
    }
    Ok(expected)
}

pub(super) fn get_in_snapshot(
    conn: &Connection,
    ns: Namespace<'_>,
) -> StorageResult<Option<String>> {
    let Some(meta) = metadata(conn, ns)? else {
        return Ok(None);
    };
    let nodes = nodes(conn, ns, &meta)?;
    payloads(conn, ns, &nodes)?;
    let mut output = String::new();
    output.try_reserve_exact(meta.bytes).map_err(err)?;
    let mut next = meta.head.as_ref();
    let mut stmt = conn.prepare("SELECT data FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND payload_key=?4 ORDER BY chunk_index").map_err(err)?;
    while let Some(key) = next {
        let node = nodes.get(key).ok_or_else(invalid)?;
        output.push_str(&node.prefix);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(node.bytes).map_err(err)?;
        let mut rows = stmt.query((ns.0, ns.1, ns.2, &node.payload)).map_err(err)?;
        while let Some(row) = rows.next().map_err(err)? {
            let ValueRef::Blob(data) = row.get_ref(0).map_err(err)? else {
                return Err(invalid());
            };
            segmented::account_io(data.len(), 0);
            bytes.extend_from_slice(data);
        }
        if bytes.len() != node.bytes
            || (node.payload[0] == 1
                && sha256::Hash::hash(&bytes).to_byte_array().as_slice() != &node.payload[1..])
        {
            return Err(invalid());
        }
        output.push_str(std::str::from_utf8(&bytes).map_err(|_| invalid())?);
        output.push_str(&node.suffix);
        append_padding(&mut output, node.padding);
        next = node.next.as_ref();
    }
    if output.len() != meta.bytes {
        return Err(invalid());
    }
    Ok(Some(output))
}

pub(super) fn clear_in_snapshot(conn: &Connection, ns: Namespace<'_>) -> StorageResult<()> {
    let Some(meta) = metadata(conn, ns)? else {
        return Ok(());
    };
    let old = nodes(conn, ns, &meta)?;
    payloads(conn, ns, &old)?;
    for table in [
        "ndr_kv_checkpoint_chunks",
        "ndr_kv_checkpoint_nodes",
        "ndr_kv_checkpoint",
    ] {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3"
            ),
            ns,
        )
        .map_err(err)?;
    }
    Ok(())
}

fn put_payload(
    conn: &Connection,
    ns: Namespace<'_>,
    key: &[u8],
    bytes: &[u8],
    previous: Option<usize>,
) -> StorageResult<()> {
    let mut changes = Vec::<(usize, Option<i64>, Vec<usize>)>::new();
    if previous.is_some() {
        let mut stmt = conn.prepare("SELECT rowid,chunk_index,data FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND payload_key=?4 ORDER BY chunk_index").map_err(err)?;
        let mut rows = stmt.query((ns.0, ns.1, ns.2, key)).map_err(err)?;
        while let Some(row) = rows.next().map_err(err)? {
            let index = number(row.get(1).map_err(err)?)?;
            let ValueRef::Blob(old) = row.get_ref(2).map_err(err)? else {
                return Err(invalid());
            };
            segmented::account_io(old.len(), 0);
            let Some(new) = bytes.get(index * CHUNK..bytes.len().min((index + 1) * CHUNK)) else {
                continue;
            };
            if new.is_empty() {
                continue;
            }
            if new.len() != old.len() {
                changes.push((index, None, Vec::new()));
            } else {
                let pages = old
                    .chunks(PAGE)
                    .zip(new.chunks(PAGE))
                    .enumerate()
                    .filter_map(|(i, (a, b))| (a != b).then_some(i))
                    .collect::<Vec<_>>();
                if !pages.is_empty() {
                    changes.push((index, Some(row.get(0).map_err(err)?), pages));
                }
            }
        }
    }
    for index in previous.unwrap_or(0).div_ceil(CHUNK)..bytes.len().div_ceil(CHUNK) {
        changes.push((index, None, Vec::new()));
    }
    for (index, rowid, pages) in changes {
        let value = &bytes[index * CHUNK..bytes.len().min((index + 1) * CHUNK)];
        if let Some(rowid) = rowid {
            let mut blob = conn
                .blob_open(
                    DatabaseName::Main,
                    "ndr_kv_checkpoint_chunks",
                    "data",
                    rowid,
                    false,
                )
                .map_err(err)?;
            for page in pages {
                let value = &value[page * PAGE..value.len().min((page + 1) * PAGE)];
                blob.write_at(value, page * PAGE).map_err(err)?;
                segmented::account_io(0, value.len());
            }
            blob.close().map_err(err)?;
        } else {
            conn.execute("INSERT INTO ndr_kv_checkpoint_chunks(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index,data) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key,payload_key,chunk_index) DO UPDATE SET data=excluded.data",(ns.0,ns.1,ns.2,key,index as i64,value)).map_err(err)?;
            segmented::account_io(0, value.len());
        }
    }
    if previous.is_some_and(|old| old > bytes.len()) {
        conn.execute("DELETE FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND payload_key=?4 AND chunk_index>=?5",(ns.0,ns.1,ns.2,key,bytes.len().div_ceil(CHUNK) as i64)).map_err(err)?;
    }
    Ok(())
}

pub(super) fn put(
    conn: &mut Connection,
    ns: Namespace<'_>,
    value: &StorageCheckpoint,
) -> StorageResult<()> {
    value.validate()?;
    let tx = conn.savepoint().map_err(err)?;
    schema(&tx, true)?;
    let previous = metadata(&tx, ns)?;
    let old = previous
        .as_ref()
        .map(|meta| nodes(&tx, ns, meta))
        .transpose()?
        .unwrap_or_default();
    let old_payloads = payloads(&tx, ns, &old)?;
    let mut current = BTreeSet::new();
    let mut current_payloads = BTreeSet::new();
    for (index, part) in value.parts.iter().enumerate() {
        if !current.insert(part.key.clone()) {
            return Err(invalid());
        }
        let node = Node {
            next: value.parts.get(index + 1).map(|p| p.key.clone()),
            payload: part.payload_key.clone(),
            bytes: part.bytes.bytes().len(),
            prefix: part.prefix.clone(),
            suffix: part.suffix.clone(),
            padding: part.padding,
        };
        if current_payloads.insert(part.payload_key.clone()) {
            let old_bytes = old_payloads.get(&part.payload_key).copied();
            if matches!(&part.bytes, PartBytes::Inline(_)) || old_bytes.is_none() {
                put_payload(&tx, ns, &part.payload_key, part.bytes.bytes(), old_bytes)?;
            } else if old_bytes != Some(node.bytes) {
                return Err(invalid());
            }
        }
        if old.get(&part.key) != Some(&node) {
            tx.execute("INSERT INTO ndr_kv_checkpoint_nodes(owner_pubkey_hex,device_pubkey_hex,key,node_key,next_key,payload_key,byte_length,prefix,suffix,padding) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key,node_key) DO UPDATE SET next_key=excluded.next_key,payload_key=excluded.payload_key,byte_length=excluded.byte_length,prefix=excluded.prefix,suffix=excluded.suffix,padding=excluded.padding",rusqlite::params![ns.0,ns.1,ns.2,part.key,node.next,node.payload,node.bytes as i64,node.prefix,node.suffix,node.padding as i64]).map_err(err)?;
            segmented::account_io(0, part.key.len() + node.stored_bytes());
        }
    }
    for key in old.keys().filter(|key| !current.contains(*key)) {
        tx.execute("DELETE FROM ndr_kv_checkpoint_nodes WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND node_key=?4",(ns.0,ns.1,ns.2,key)).map_err(err)?;
    }
    for key in old_payloads
        .keys()
        .filter(|key| !current_payloads.contains(*key))
    {
        tx.execute("DELETE FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=?1 AND device_pubkey_hex=?2 AND key=?3 AND payload_key=?4",(ns.0,ns.1,ns.2,key)).map_err(err)?;
    }
    let meta = Meta {
        bytes: value.len(),
        count: value.parts.len(),
        head: value.parts.first().map(|p| p.key.clone()),
    };
    if previous.is_none() {
        // The one-time migration drops old chunks and the TEXT mirror in this
        // same transaction; a failed commit leaves the old checkpoint intact.
        segmented::clear_in_snapshot(&tx, ns)?;
        tx.execute("INSERT INTO ndr_kv(owner_pubkey_hex,device_pubkey_hex,key,value) VALUES(?1,?2,?3,'') ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key) DO UPDATE SET value=''",ns).map_err(err)?;
    }
    if previous.as_ref() != Some(&meta) {
        tx.execute("INSERT INTO ndr_kv_checkpoint(owner_pubkey_hex,device_pubkey_hex,key,format_version,byte_length,node_count,head) VALUES(?1,?2,?3,1,?4,?5,?6) ON CONFLICT(owner_pubkey_hex,device_pubkey_hex,key) DO UPDATE SET byte_length=excluded.byte_length,node_count=excluded.node_count,head=excluded.head",(ns.0,ns.1,ns.2,meta.bytes as i64,meta.count as i64,&meta.head)).map_err(err)?;
        segmented::account_io(0, 24 + meta.head.as_ref().map_or(0, Vec::len));
    }
    tx.commit().map_err(err)
}
