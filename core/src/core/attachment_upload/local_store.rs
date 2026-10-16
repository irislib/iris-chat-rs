//! App-owned ciphertext storage. No nhash decryption keys or signing keys live here.
use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::io::Write;
use std::sync::{Mutex, MutexGuard};

pub(super) const DEFAULT_BUDGET: u64 = 256 * 1024 * 1024;
const MAX_BLOCKS: usize = 8192;
const MAX_TREES: usize = 256;
pub(super) const MAX_TREE_BLOCKS: usize = 128;
pub(super) const MAX_SERVERS: usize = 8;
pub(super) const MAX_UPLOADS: usize = 32;

struct State {
    db: Connection,
    pins: HashMap<Hash, usize>,
    uploads: usize,
    readers: usize,
    closed: bool,
}

pub(in crate::core) struct AttachmentChunkStore {
    pub(in crate::core) id: u64,
    directory: PathBuf,
    data_budget: u64,
    state: Mutex<State>,
    pub(super) wake: tokio::sync::Notify,
}

pub(super) struct UploadSession {
    store: Arc<AttachmentChunkStore>,
    hashes: Mutex<HashSet<Hash>>,
    progress: Option<Arc<AtomicU64>>,
}

pub(super) struct ReadLease(Arc<AttachmentChunkStore>);

pub(super) struct TreeLease {
    store: Arc<AttachmentChunkStore>,
    pub(super) hashes: Vec<Hash>,
}

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn error(value: impl std::fmt::Display) -> StoreError {
    StoreError::Other(format!("attachment storage: {value}"))
}

impl AttachmentChunkStore {
    pub(super) fn open(directory: &Path, budget: u64) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(budget >= 1024 * 1024, "attachment budget is too small");
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        // Reserve metadata, its rollback journal and filesystem bookkeeping.
        // Fixed page/count limits keep even many tiny attachments bounded.
        let metadata_budget = (budget / 8).min(8 * 1024 * 1024);
        let db = Connection::open(directory.join("index.sqlite3"))?;
        db.execute_batch(&format!(
            "PRAGMA page_size=4096; PRAGMA max_page_count={};
             PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS blocks(hash TEXT PRIMARY KEY, allocated INTEGER NOT NULL,
                 accessed INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS trees(root TEXT PRIMARY KEY, replicated INTEGER NOT NULL DEFAULT 0,
                 created INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS tree_blocks(root TEXT NOT NULL REFERENCES trees(root) ON DELETE CASCADE,
                 hash TEXT NOT NULL, PRIMARY KEY(root,hash));
             CREATE INDEX IF NOT EXISTS tree_blocks_hash ON tree_blocks(hash);
             CREATE TABLE IF NOT EXISTS replicas(root TEXT NOT NULL REFERENCES trees(root) ON DELETE CASCADE,
                 server TEXT NOT NULL, complete INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0,
                 next_retry INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(root,server));",
            metadata_budget / 4096
        ))?;
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let store = Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            directory: directory.to_path_buf(),
            data_budget: budget - metadata_budget * 4,
            state: Mutex::new(State {
                db,
                pins: HashMap::new(),
                uploads: 0,
                readers: 0,
                closed: false,
            }),
            wake: tokio::sync::Notify::new(),
        });
        // Only an uncommitted ciphertext write may be discarded after a crash.
        let temporary = directory.join("incoming.tmp");
        if temporary.exists() {
            std::fs::remove_file(temporary)?;
        }
        let state = store.lock()?;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if hashtree_core::from_hex(&name).is_ok()
                && !state.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM blocks WHERE hash=?1)",
                    [&name],
                    |r| r.get::<_, bool>(0),
                )?
            {
                std::fs::remove_file(entry.path())?;
            }
        }
        drop(state);
        Ok(store)
    }

    fn lock(&self) -> anyhow::Result<MutexGuard<'_, State>> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment store mutex poisoned"))?;
        anyhow::ensure!(!state.closed, "attachment account is closed");
        Ok(state)
    }

    fn read_locked(&self, state: &State, hash: &Hash) -> anyhow::Result<Option<Vec<u8>>> {
        let key = to_hex(hash);
        let tracked: bool = state.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM blocks WHERE hash=?1)",
            [&key],
            |r| r.get(0),
        )?;
        if !tracked {
            return Ok(None);
        }
        let path = self.directory.join(&key);
        match std::fs::metadata(&path) {
            Ok(meta) => anyhow::ensure!(
                meta.is_file() && meta.len() <= MAX_ATTACHMENT_BYTES + 64,
                "invalid attachment block size"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            hashtree_core::sha256(&bytes) == *hash,
            "corrupt attachment block"
        );
        state.db.execute(
            "UPDATE blocks SET accessed=(SELECT COALESCE(MAX(accessed),0)+1 FROM blocks) WHERE hash=?1",
            [&key],
        )?;
        Ok(Some(bytes))
    }

    fn make_room(&self, state: &State, additional: u64, new_block: bool) -> anyhow::Result<()> {
        loop {
            let (used, count): (u64, usize) = state.db.query_row(
                "SELECT COALESCE(SUM(allocated),0),COUNT(*) FROM blocks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if used.saturating_add(additional) <= self.data_budget
                && (!new_block || count < MAX_BLOCKS)
            {
                return Ok(());
            }
            anyhow::ensure!(
                state.readers == 0,
                "attachment storage is busy reading protected files"
            );
            let mut stmt = state.db.prepare(
                "SELECT b.hash FROM blocks b WHERE NOT EXISTS(SELECT 1 FROM tree_blocks t
                 JOIN trees a ON a.root=t.root WHERE t.hash=b.hash AND a.replicated=0)
                 ORDER BY b.accessed,b.hash",
            )?;
            let candidate = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(Result::ok)
                .find(|key| {
                    hashtree_core::from_hex(key).is_ok_and(|h| !state.pins.contains_key(&h))
                });
            let Some(key) = candidate else {
                anyhow::bail!("attachment storage is full; pending files are protected");
            };
            match std::fs::remove_file(self.directory.join(&key)) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            state
                .db
                .execute("DELETE FROM blocks WHERE hash=?1", [key])?;
        }
    }

    fn put_locked(&self, state: &State, hash: Hash, data: &[u8]) -> anyhow::Result<bool> {
        anyhow::ensure!(
            data.len() as u64 <= MAX_ATTACHMENT_BYTES + 64,
            "attachment block is too large"
        );
        anyhow::ensure!(
            hashtree_core::sha256(data) == hash,
            "attachment block hash mismatch"
        );
        if self.read_locked(state, &hash).is_ok_and(|v| v.is_some()) {
            return Ok(false);
        }
        let key = to_hex(&hash);
        let old: Option<u64> = state
            .db
            .query_row("SELECT allocated FROM blocks WHERE hash=?1", [&key], |r| {
                r.get(0)
            })
            .optional()?;
        let charged = (data.len() as u64).max(1).div_ceil(4096) * 4096;
        self.make_room(
            state,
            charged.saturating_sub(old.unwrap_or(0)),
            old.is_none(),
        )?;
        // Replace missing/corrupt ciphertext without temporarily counting two full blocks.
        if old.is_some() && self.directory.join(&key).exists() {
            std::fs::remove_file(self.directory.join(&key))?;
        }
        let temporary = self.directory.join("incoming.tmp");
        let result = (|| -> anyhow::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(data)?;
            file.sync_all()?;
            std::fs::rename(&temporary, self.directory.join(&key))?;
            #[cfg(unix)]
            std::fs::File::open(&self.directory)?.sync_all()?;
            state.db.execute("INSERT INTO blocks(hash,allocated,accessed) VALUES(?1,?2,(SELECT COALESCE(MAX(accessed),0)+1 FROM blocks))
                ON CONFLICT(hash) DO UPDATE SET allocated=excluded.allocated,accessed=excluded.accessed",
                params![key, charged])?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
            if old.is_none() {
                let _ = std::fs::remove_file(self.directory.join(&key));
            }
        }
        result?;
        Ok(true)
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        progress: Option<Arc<AtomicU64>>,
    ) -> anyhow::Result<Arc<UploadSession>> {
        let mut state = self.lock()?;
        anyhow::ensure!(state.uploads < MAX_UPLOADS, "too many attachment writes");
        state.uploads += 1;
        Ok(Arc::new(UploadSession {
            store: self.clone(),
            hashes: Mutex::new(HashSet::new()),
            progress,
        }))
    }

    pub(super) fn reading(self: &Arc<Self>) -> anyhow::Result<ReadLease> {
        let mut state = self.lock()?;
        anyhow::ensure!(state.readers < 4, "too many attachment readers");
        state.readers += 1;
        Ok(ReadLease(self.clone()))
    }

    pub(in crate::core) fn uploaded_urls(&self, nhash: &str) -> Vec<String> {
        let Ok(decoded) = nhash_decode(nhash) else {
            return Vec::new();
        };
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        let Ok(mut stmt) = state.db.prepare(
            "SELECT server FROM replicas WHERE root=?1 AND complete=1 ORDER BY server LIMIT 8",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([to_hex(&decoded.hash)], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .map(|s| format!("{}/{}.bin", s.trim_end_matches('/'), to_hex(&decoded.hash)))
            .collect()
    }

    pub(super) fn configure_pending(&self, servers: &[String]) -> anyhow::Result<()> {
        let servers = super::replication::bounded_servers(servers);
        let mut state = self.lock()?;
        let tx = state.db.transaction()?;
        for server in servers {
            tx.execute("INSERT OR IGNORE INTO replicas(root,server) SELECT root,?1 FROM trees WHERE replicated=0 AND (SELECT COUNT(*) FROM replicas WHERE replicas.root=trees.root)<8", [server])?;
        }
        tx.commit()?;
        self.wake.notify_one();
        Ok(())
    }

    pub(super) fn next_replica(&self) -> anyhow::Result<Option<(Hash, String)>> {
        let state = self.lock()?;
        let row: Option<(String, String)> = state
            .db
            .query_row(
                "SELECT root,server FROM replicas WHERE complete=0 AND next_retry<=?1
             ORDER BY next_retry,root,server LIMIT 1",
                [now()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(root, server)| Ok((hashtree_core::from_hex(&root)?, server)))
            .transpose()
    }

    pub(super) fn lease(self: &Arc<Self>, root: &Hash) -> anyhow::Result<TreeLease> {
        let mut state = self.lock()?;
        let hashes: Vec<Hash> = {
            let mut stmt = state
                .db
                .prepare("SELECT hash FROM tree_blocks WHERE root=?1 ORDER BY hash LIMIT 129")?;
            let rows = stmt.query_map([to_hex(root)], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(hashtree_core::from_hex(&row?)?))
                .collect::<anyhow::Result<_>>()?
        };
        anyhow::ensure!(
            !hashes.is_empty() && hashes.len() <= MAX_TREE_BLOCKS,
            "invalid attachment tree"
        );
        for hash in &hashes {
            *state.pins.entry(*hash).or_default() += 1;
        }
        Ok(TreeLease {
            store: self.clone(),
            hashes,
        })
    }

    pub(super) fn confirmed_servers(&self, root: &Hash) -> anyhow::Result<Vec<String>> {
        let state = self.lock()?;
        let mut stmt = state
            .db
            .prepare("SELECT server FROM replicas WHERE root=?1 AND complete=1 LIMIT 8")?;
        let rows = stmt.query_map([to_hex(root)], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub(super) fn finish_replica(
        &self,
        root: &Hash,
        server: &str,
        complete: bool,
    ) -> anyhow::Result<()> {
        let mut state = self.lock()?;
        let tx = state.db.transaction()?;
        if complete {
            tx.execute(
                "UPDATE replicas SET complete=1 WHERE root=?1 AND server=?2",
                params![to_hex(root), server],
            )?;
            tx.execute(
                "UPDATE trees SET replicated=1 WHERE root=?1",
                [to_hex(root)],
            )?;
        } else {
            let attempts: Option<u32> = tx
                .query_row(
                    "SELECT attempts FROM replicas WHERE root=?1 AND server=?2",
                    params![to_hex(root), server],
                    |r| r.get(0),
                )
                .optional()?;
            // A bounded metadata eviction can retire an already-replicated
            // tree while its optional second-server retry is in flight.
            let Some(attempts) = attempts else {
                return Ok(());
            };
            let delay = (30u64 * (1u64 << attempts.min(7))).min(3600);
            tx.execute(
                "UPDATE replicas SET attempts=?3,next_retry=?4 WHERE root=?1 AND server=?2",
                params![
                    to_hex(root),
                    server,
                    attempts.saturating_add(1).min(8),
                    now() + delay
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn close_and_clear(&self) -> anyhow::Result<()> {
        let mut state = self.lock()?;
        state.closed = true;
        let keys = {
            let mut stmt = state.db.prepare("SELECT hash FROM blocks")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        state
            .db
            .execute_batch("DELETE FROM trees; DELETE FROM blocks;")?;
        for key in keys {
            let path = self.directory.join(key);
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

impl AttachmentChunkStore {
    pub(super) fn finish_batch(
        &self,
        staged: &[(Arc<UploadSession>, String)],
        servers: &[String],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !staged.is_empty() && staged.len() <= MAX_UPLOADS,
            "invalid attachment batch"
        );
        let servers = super::replication::bounded_servers(servers);
        let mut trees = Vec::with_capacity(staged.len());
        for (session, nhash) in staged {
            anyhow::ensure!(
                std::ptr::eq(self, session.store.as_ref()),
                "attachment store changed"
            );
            let root = nhash_decode(nhash)?.hash;
            let hashes = session
                .hashes
                .lock()
                .map_err(|_| anyhow::anyhow!("attachment upload mutex poisoned"))?;
            anyhow::ensure!(
                hashes.contains(&root) && hashes.len() <= MAX_TREE_BLOCKS,
                "incomplete attachment tree"
            );
            trees.push((root, hashes.iter().copied().collect::<Vec<_>>()));
        }
        let mut state = self.lock()?;
        for (_, hashes) in &trees {
            for hash in hashes {
                anyhow::ensure!(
                    self.read_locked(&state, hash)?.is_some(),
                    "attachment block disappeared before acceptance"
                );
            }
        }
        // Either every selected file becomes a protected pending tree, or none
        // does. Dropping a failed batch releases only its temporary pins.
        let tx = state.db.transaction()?;
        for (root, hashes) in &trees {
            let existing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM trees WHERE root=?1)",
                [to_hex(root)],
                |r| r.get(0),
            )?;
            if !existing {
                let count: usize = tx.query_row("SELECT COUNT(*) FROM trees", [], |r| r.get(0))?;
                if count >= MAX_TREES {
                    tx.execute("DELETE FROM trees WHERE root=(SELECT root FROM trees WHERE replicated=1 ORDER BY created LIMIT 1)", [])?;
                    let count: usize =
                        tx.query_row("SELECT COUNT(*) FROM trees", [], |r| r.get(0))?;
                    anyhow::ensure!(count < MAX_TREES, "pending attachment metadata is full");
                }
            }
            tx.execute(
                "INSERT OR IGNORE INTO trees(root,created) VALUES(?1,?2)",
                params![to_hex(root), now()],
            )?;
            for hash in hashes {
                tx.execute(
                    "INSERT OR IGNORE INTO tree_blocks(root,hash) VALUES(?1,?2)",
                    params![to_hex(root), to_hex(hash)],
                )?;
            }
            for server in &servers {
                tx.execute(
                    "INSERT OR IGNORE INTO replicas(root,server) SELECT ?1,?2 WHERE (SELECT COUNT(*) FROM replicas WHERE root=?1)<8",
                    params![to_hex(root), server],
                )?;
            }
        }
        tx.commit()?;
        self.wake.notify_one();
        Ok(())
    }
}

fn unpin(store: &AttachmentChunkStore, hashes: impl Iterator<Item = Hash>) {
    if let Ok(mut state) = store.state.lock() {
        for hash in hashes {
            if let Some(count) = state.pins.get_mut(&hash) {
                *count -= 1;
                if *count == 0 {
                    state.pins.remove(&hash);
                }
            }
        }
    }
}

impl Drop for UploadSession {
    fn drop(&mut self) {
        if let Ok(hashes) = self.hashes.lock() {
            unpin(&self.store, hashes.iter().copied());
        }
        if let Ok(mut state) = self.store.state.lock() {
            state.uploads = state.uploads.saturating_sub(1);
        }
    }
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.readers = state.readers.saturating_sub(1);
        }
    }
}

impl Drop for TreeLease {
    fn drop(&mut self) {
        unpin(&self.store, self.hashes.iter().copied());
    }
}

#[async_trait]
impl Store for AttachmentChunkStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        let state = self.lock().map_err(error)?;
        self.put_locked(&state, hash, &data).map_err(error)
    }
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let state = self.lock().map_err(error)?;
        self.read_locked(&state, hash).map_err(error)
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self.get(hash).await?.is_some())
    }
    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        let state = self.lock().map_err(error)?;
        let protected: bool = state.db.query_row("SELECT EXISTS(SELECT 1 FROM tree_blocks t JOIN trees a ON a.root=t.root WHERE t.hash=?1 AND a.replicated=0)", [to_hex(hash)], |r| r.get(0)).map_err(error)?;
        if protected || state.readers != 0 || state.pins.contains_key(hash) {
            return Err(error("attachment block is protected"));
        }
        let path = self.directory.join(to_hex(hash));
        if path.exists() {
            std::fs::remove_file(path).map_err(error)?;
        }
        Ok(state
            .db
            .execute("DELETE FROM blocks WHERE hash=?1", [to_hex(hash)])
            .map_err(error)?
            != 0)
    }
}

#[async_trait]
impl Store for UploadSession {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        let mut hashes = self.hashes.lock().map_err(error)?;
        if hashes.contains(&hash) {
            return Ok(false);
        }
        if hashes.len() >= MAX_TREE_BLOCKS {
            return Err(error("too many attachment blocks"));
        }
        let mut state = self.store.lock().map_err(error)?;
        let inserted = self.store.put_locked(&state, hash, &data).map_err(error)?;
        *state.pins.entry(hash).or_default() += 1;
        hashes.insert(hash);
        if let Some(progress) = &self.progress {
            progress.fetch_add(data.len() as u64, Ordering::Relaxed);
        }
        Ok(inserted)
    }
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        self.store.get(hash).await
    }
    async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.store.delete(hash).await
    }
}
