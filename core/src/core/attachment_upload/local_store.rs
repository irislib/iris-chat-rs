//! App-owned ciphertext storage. No nhash decryption keys or signing keys live here.
use super::*;
use futures_util::FutureExt;
use hashtree_lmdb::LmdbBlobStore;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Mutex, MutexGuard};

pub(super) const DEFAULT_BUDGET: u64 = 1024 * 1024 * 1024;
const MAX_BLOCKS: usize = 8192;
const MAX_TREES: usize = 256;
pub(super) const MAX_TREE_BLOCKS: usize = 128;
pub(super) const MAX_SERVERS: usize = 8;
pub(super) const MAX_UPLOADS: usize = 32;

struct State {
    db: Connection,
    blobs: Option<LmdbBlobStore>,
    pins: HashMap<Hash, usize>,
    uploads: usize,
    readers: usize,
    closed: bool,
}

pub(in crate::core) struct AttachmentChunkStore {
    pub(in crate::core) id: u64,
    blob_directory: PathBuf,
    map_budget: usize,
    data_budget: u64,
    state: Mutex<State>,
    pub(super) wake: tokio::sync::Notify,
    pub(super) readiness: tokio::sync::watch::Sender<super::readiness::State>,
}

pub(super) struct UploadSession {
    store: Arc<AttachmentChunkStore>,
    hashes: Mutex<HashSet<Hash>>,
    progress: Option<Arc<AtomicU64>>,
}

pub(super) struct ReadLease {
    store: Arc<AttachmentChunkStore>,
    references: Mutex<HashMap<Hash, read_protection::ReadReference>>,
}

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

#[derive(Debug)]
pub(super) struct StorageSuspended;
impl std::fmt::Display for StorageSuspended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("attachment storage is suspended")
    }
}
impl std::error::Error for StorageSuspended {}

impl State {
    fn blobs(&self) -> anyhow::Result<&LmdbBlobStore> {
        self.blobs.as_ref().ok_or_else(|| StorageSuspended.into())
    }
}

impl AttachmentChunkStore {
    #[cfg(any(target_os = "ios", test))]
    pub(in crate::core) fn suspend(&self) -> anyhow::Result<()> {
        // LMDB keeps a shared file lock for the lifetime of an environment,
        // even with no transaction running. Drain operations under their usual
        // mutex, then close the environment before iOS releases background time.
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment store mutex poisoned"))?;
        let blobs = state.blobs.take();
        let result = blobs.as_ref().map(|blobs| blobs.force_sync()).transpose();
        drop(blobs);
        result?;
        Ok(())
    }

    fn reopen_locked(&self, state: &mut State) -> anyhow::Result<()> {
        anyhow::ensure!(!state.closed, "attachment account is closed");
        if state.blobs.is_none() {
            state.blobs = Some(
                LmdbBlobStore::with_exact_map_size_and_external_blob_options(
                    &self.blob_directory,
                    self.map_budget,
                    None,
                )?,
            );
            // Leases may be dropped while suspended. Reconcile their durable
            // vetoes before admitting reads, writes, or quota eviction again.
            let result = (|| {
                for hash in state.blobs()?.scan_hashes_after(None, MAX_BLOCKS + 1)? {
                    self.sync_pin(state, &hash)?;
                }
                state.blobs()?.force_sync()?;
                Ok::<_, anyhow::Error>(())
            })();
            if result.is_err() {
                state.blobs.take();
            }
            result?;
        }
        Ok(())
    }

    #[cfg(any(target_os = "ios", test))]
    pub(in crate::core) fn resume(&self) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment store mutex poisoned"))?;
        self.reopen_locked(&mut state)?;
        self.wake.notify_one();
        Ok(())
    }

    pub(super) fn is_suspended(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| !state.closed && state.blobs.is_none())
    }

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
             CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS blocks(hash TEXT PRIMARY KEY, allocated INTEGER NOT NULL,
                 accessed INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS trees(root TEXT PRIMARY KEY, replicated INTEGER NOT NULL DEFAULT 0,
                 created INTEGER NOT NULL, committed INTEGER NOT NULL DEFAULT 0, chat_id TEXT, message_id TEXT);
             CREATE TABLE IF NOT EXISTS tree_blocks(root TEXT NOT NULL REFERENCES trees(root) ON DELETE CASCADE,
                 hash TEXT NOT NULL, PRIMARY KEY(root,hash));
             CREATE INDEX IF NOT EXISTS tree_blocks_hash ON tree_blocks(hash);
             CREATE TABLE IF NOT EXISTS replicas(root TEXT NOT NULL REFERENCES trees(root) ON DELETE CASCADE,
                 server TEXT NOT NULL, complete INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0,
                 next_retry INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(root,server));",
            metadata_budget / 4096
        ))?;
        let metadata_pages: u64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        anyhow::ensure!(
            metadata_pages * 4096 <= metadata_budget,
            "existing attachment metadata exceeds configured quota"
        );
        let has_committed: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('trees') WHERE name='committed')",
            [],
            |r| r.get(0),
        )?;
        if !has_committed {
            // Preserve unknown legacy protection; unowned stores cannot be
            // adopted by an account or start a replication worker.
            db.execute_batch("ALTER TABLE trees ADD COLUMN committed INTEGER NOT NULL DEFAULT 1;
                ALTER TABLE trees ADD COLUMN chat_id TEXT; ALTER TABLE trees ADD COLUMN message_id TEXT;")?;
        }
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let map_budget = budget - metadata_budget * 4;
        let blobs = LmdbBlobStore::with_exact_map_size_and_external_blob_options(
            directory.join("blobs"),
            map_budget as usize,
            None,
        )?;
        anyhow::ensure!(
            blobs.map_size_bytes() as u64 <= map_budget,
            "existing attachment map exceeds configured quota"
        );
        let store = Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            blob_directory: directory.join("blobs"),
            map_budget: map_budget as usize,
            // The fixed LMDB map is the physical bound; reserve internal COW,
            // indexes and pin pages rather than promise every map byte as data.
            data_budget: map_budget * 3 / 4,
            state: Mutex::new(State {
                db,
                blobs: Some(blobs),
                pins: HashMap::new(),
                uploads: 0,
                readers: 0,
                closed: false,
            }),
            wake: tokio::sync::Notify::new(),
            readiness: tokio::sync::watch::channel(super::readiness::State::Pending).0,
        });
        let state = store.lock()?;
        let hashes = state.blobs()?.scan_hashes_after(None, MAX_BLOCKS + 1)?;
        anyhow::ensure!(
            hashes.len() <= MAX_BLOCKS,
            "attachment block count exceeds configured bound"
        );
        for hash in hashes {
            let tracked: bool = state.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM blocks WHERE hash=?1)",
                [to_hex(&hash)],
                |r| r.get(0),
            )?;
            if tracked {
                store.sync_pin(&state, &hash)?;
            } else {
                state.blobs()?.delete_sync(&hash)?;
            }
        }
        // Unreleased earlier file-backed candidates are not adopted or erased.
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            anyhow::ensure!(
                hashtree_core::from_hex(&entry.file_name().to_string_lossy()).is_err(),
                "legacy attachment files require explicit migration"
            );
        }
        state.blobs()?.force_sync()?;
        drop(state);
        Ok(store)
    }

    fn lock(&self) -> anyhow::Result<MutexGuard<'_, State>> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment store mutex poisoned"))?;
        anyhow::ensure!(!state.closed, "attachment account is closed");
        if state.blobs.is_none() {
            return Err(StorageSuspended.into());
        }
        Ok(state)
    }

    // The pinned backend's Store pin/eviction methods are synchronous LMDB
    // transactions wrapped in async_trait. Do not nest a runtime or silently
    // proceed if a future backend version changes that contract.
    fn immediate<T>(
        future: impl std::future::Future<Output = Result<T, StoreError>>,
    ) -> anyhow::Result<T> {
        future
            .now_or_never()
            .ok_or_else(|| anyhow::anyhow!("attachment backend unexpectedly yielded"))?
            .map_err(Into::into)
    }

    fn sync_pin(&self, state: &State, hash: &Hash) -> anyhow::Result<()> {
        let pending: bool = state.db.query_row("SELECT EXISTS(SELECT 1 FROM tree_blocks t JOIN trees a ON a.root=t.root WHERE t.hash=?1 AND a.replicated=0)", [to_hex(hash)], |r| r.get(0))?;
        // Iris owns overlapping references; the backend needs one durable veto.
        // Missing referenced blocks need only the in-process admission veto.
        // put_locked restores the backend pin before exposing a newly cached
        // block; never leave durable pins for absent hashes after a crash.
        let desired =
            u32::from((pending || state.pins.contains_key(hash)) && state.blobs()?.exists(hash)?);
        let current = state.blobs()?.pin_count(hash);
        anyhow::ensure!(
            current <= MAX_UPLOADS as u32 + MAX_TREES as u32 + 2,
            "attachment pin count is invalid"
        );
        for _ in current..desired {
            Self::immediate(state.blobs()?.pin(hash))?;
        }
        for _ in desired..current {
            Self::immediate(state.blobs()?.unpin(hash))?;
        }
        Ok(())
    }

    fn read_locked(&self, state: &State, hash: &Hash) -> anyhow::Result<Option<Vec<u8>>> {
        let tracked: bool = state.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM blocks WHERE hash=?1)",
            [to_hex(hash)],
            |r| r.get(0),
        )?;
        if !tracked {
            return Ok(None);
        }
        anyhow::ensure!(
            state
                .blobs()?
                .blob_size_sync(hash)?
                .is_none_or(|len| len <= MAX_ATTACHMENT_BYTES + 64),
            "invalid attachment block size"
        );
        let Some(bytes) = state.blobs()?.get_sync(hash)? else {
            return Ok(None);
        };
        anyhow::ensure!(
            hashtree_core::sha256(&bytes) == *hash,
            "corrupt attachment block"
        );
        let accessed = state
            .blobs()?
            .last_accessed_at_sync(hash)?
            .unwrap_or(0)
            .max(now())
            .saturating_add(1);
        // Read data remains valid if optional LRU bookkeeping runs out of map
        // space. Owned bytes outlive the lock and any later cache eviction.
        let _ = state.blobs()?.touch_accessed_sync(hash, accessed);
        Ok(Some(bytes))
    }

    fn make_room(&self, state: &State, additional: u64, new_block: bool) -> anyhow::Result<()> {
        let (used, count): (u64, usize) = state.db.query_row(
            "SELECT COALESCE(SUM(allocated),0),COUNT(*) FROM blocks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if used.saturating_add(additional) <= self.data_budget && (!new_block || count < MAX_BLOCKS)
        {
            return Ok(());
        }
        let need = used
            .saturating_add(additional)
            .saturating_sub(self.data_budget)
            .max(4096);
        self.evict_bytes(state, need)?;
        let (used, count): (u64, usize) = state.db.query_row(
            "SELECT COALESCE(SUM(allocated),0),COUNT(*) FROM blocks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        anyhow::ensure!(
            used.saturating_add(additional) <= self.data_budget
                && (!new_block || count < MAX_BLOCKS),
            "attachment storage is full; pending files are protected"
        );
        Ok(())
    }

    fn evict_bytes(&self, state: &State, need: u64) -> anyhow::Result<()> {
        // Failed drop-time unpins are conservative until the next admission.
        // Reconcile only this bounded index before asking the engine to evict.
        let keys = {
            let mut stmt = state.db.prepare("SELECT hash FROM blocks LIMIT 8192")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for key in &keys {
            self.sync_pin(state, &hashtree_core::from_hex(key)?)?;
        }
        let bytes = state.blobs()?.stats()?.total_bytes;
        // Eviction is the backend's LRU and rechecks durable pins in its writer
        // transaction. max_bytes is temporary: raw put must never evict behind
        // the application's pre-admission and read-protection rules.
        state
            .blobs()?
            .set_max_bytes(bytes.saturating_sub(need).max(1));
        let result = Self::immediate(state.blobs()?.evict_if_needed());
        state.blobs()?.set_max_bytes(0);
        result?;
        for key in keys {
            if !state.blobs()?.exists(&hashtree_core::from_hex(&key)?)? {
                state
                    .db
                    .execute("DELETE FROM blocks WHERE hash=?1", [key])?;
            }
        }
        Ok(())
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
        if old.is_some() {
            state.blobs()?.delete_sync(&hash)?;
        }
        if let Err(error) = state.blobs()?.put_sync(hash, data) {
            // Fragmentation/COW can fill a fixed map before the logical charge.
            // One bounded extra LRU eviction is allowed, still honoring pins.
            if !error.to_string().contains("MDB_MAP_FULL") {
                return Err(error.into());
            }
            self.evict_bytes(state, charged)?;
            state.blobs()?.put_sync(hash, data)?;
        }
        state.blobs()?.force_sync()?;
        state.db.execute("INSERT INTO blocks(hash,allocated,accessed) VALUES(?1,?2,0) ON CONFLICT(hash) DO UPDATE SET allocated=excluded.allocated", params![key, charged])?;
        // Repairing a corrupt pending block deletes its old backend pin too.
        self.sync_pin(state, &hash)?;
        state.blobs()?.force_sync()?;
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
        Ok(ReadLease {
            store: self.clone(),
            references: Mutex::new(HashMap::new()),
        })
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

    pub(super) fn bind_owner(&self, owner: &str) -> anyhow::Result<()> {
        nostr::PublicKey::from_hex(owner)?;
        let mut state = self.lock()?;
        let tx = state.db.transaction()?;
        let resetting: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='resetting')",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            !resetting,
            "attachment cleanup is incomplete; log out again to finish"
        );
        let previous: Option<String> = tx
            .query_row("SELECT value FROM metadata WHERE key='owner'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(previous) = previous {
            anyhow::ensure!(
                previous == owner,
                "attachment storage belongs to another account"
            );
        } else {
            let occupied: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM trees) OR EXISTS(SELECT 1 FROM blocks)",
                [],
                |r| r.get(0),
            )?;
            anyhow::ensure!(!occupied, "attachment storage ownership is unknown");
            tx.execute(
                "INSERT INTO metadata(key,value) VALUES('owner',?1)",
                [owner],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.blobs.take();
            self.readiness.send_replace(super::readiness::State::Closed);
            self.wake.notify_one();
        }
    }

    pub(super) fn transport_ready(&self, route: &Arc<RoutedStore<Self>>) -> anyhow::Result<()> {
        let _state = self.lock()?;
        anyhow::ensure!(
            std::ptr::eq(self, route.primary().as_ref()),
            "attachment account changed"
        );
        self.readiness
            .send_replace(super::readiness::State::Ready(Arc::downgrade(route)));
        Ok(())
    }

    pub(super) fn configure_pending(&self, servers: &[String]) -> anyhow::Result<()> {
        let servers = super::replication::bounded_servers(servers);
        let mut state = self.lock()?;
        let tx = state.db.transaction()?;
        // Successful receipts remain historical. Remove obsolete failed
        // targets so a new configuration can rescue an entirely offline set.
        let previous = {
            let mut stmt = tx.prepare("SELECT DISTINCT server FROM replicas WHERE complete=0")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for server in previous {
            if !servers.contains(&server) {
                tx.execute(
                    "DELETE FROM replicas WHERE complete=0 AND server=?1",
                    [server],
                )?;
            }
        }
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
                "SELECT r.root,r.server FROM replicas r JOIN trees t ON t.root=r.root WHERE r.complete=0 AND t.committed=1 AND r.next_retry<=?1
             ORDER BY r.next_retry,r.root,r.server LIMIT 1",
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
        for (index, hash) in hashes.iter().enumerate() {
            *state.pins.entry(*hash).or_default() += 1;
            if let Err(error) = self.sync_pin(&state, hash) {
                drop(state);
                unpin(self, hashes.iter().take(index + 1).copied());
                return Err(error);
            }
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
            if tx.execute(
                "UPDATE replicas SET complete=1 WHERE root=?1 AND server=?2",
                params![to_hex(root), server],
            )? == 0
            {
                // Configuration replaced this in-flight target. Do not make a
                // tree evictable without retaining its complete receipt.
                return Ok(());
            }
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
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("attachment store mutex poisoned"))?;
        self.reopen_locked(&mut state)?;
        state.closed = true;
        self.readiness.send_replace(super::readiness::State::Closed);
        state.db.execute(
            "INSERT OR REPLACE INTO metadata(key,value) VALUES('resetting','1')",
            [],
        )?;
        let keys = {
            let mut stmt = state.db.prepare("SELECT hash FROM blocks")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        for key in keys {
            state
                .blobs()?
                .delete_sync(&hashtree_core::from_hex(&key)?)?;
        }
        state.blobs()?.force_sync()?;
        let tx = state.db.transaction()?;
        tx.execute("DELETE FROM trees", [])?;
        tx.execute("DELETE FROM blocks", [])?;
        tx.execute("DELETE FROM metadata", [])?;
        tx.commit()?;
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
        for (_, hashes) in &trees {
            for hash in hashes {
                self.sync_pin(&state, hash)?;
            }
        }
        state.blobs()?.force_sync()?;
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
                if !state.closed && state.blobs.is_some() {
                    let _ = store.sync_pin(&state, &hash);
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
        if let Ok(references) = self.references.lock() {
            unpin(&self.store, references.keys().copied());
        }
        if let Ok(mut state) = self.store.state.lock() {
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
        let result = self.read_locked(&state, hash).map_err(error);
        if let Ok(bytes) = &result {
            crate::core::support::offline_lan::local_read(hash, bytes.as_deref());
        }
        result
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self.get(hash).await?.is_some())
    }
    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        let state = self.lock().map_err(error)?;
        let protected: bool = state.db.query_row("SELECT EXISTS(SELECT 1 FROM tree_blocks t JOIN trees a ON a.root=t.root WHERE t.hash=?1 AND a.replicated=0)", [to_hex(hash)], |r| r.get(0)).map_err(error)?;
        if protected || state.pins.contains_key(hash) {
            return Err(error("attachment block is protected"));
        }
        state.blobs().map_err(error)?.delete_sync(hash)?;
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
        self.store.sync_pin(&state, &hash).map_err(error)?;
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

#[path = "local_store_commit.rs"]
mod message_commit;

#[path = "local_store_reads.rs"]
mod read_protection;
