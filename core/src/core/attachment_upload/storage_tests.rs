use super::*;
use local_store::DEFAULT_BUDGET;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct BlossomFixture {
    url: String,
    task: tokio::task::JoinHandle<()>,
    uploads: Arc<std::sync::atomic::AtomicUsize>,
}
impl Drop for BlossomFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl BlossomFixture {
    async fn start(allow: usize, wrong_hash: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let uploads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = uploads.clone();
        let task = tokio::spawn(async move {
            let mut blocks = HashMap::<String, Vec<u8>>::new();
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0u8; 8192];
                let (header_end, length) = loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..n]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..end]);
                        let len = header
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        assert!(len < 3 * 1024 * 1024);
                        break (end + 4, len);
                    }
                    assert!(request.len() < 16384);
                };
                while request.len() < header_end + length {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let header = String::from_utf8_lossy(&request[..header_end]);
                let mut words = header.split_whitespace();
                let method = words.next().unwrap();
                let path = words.next().unwrap();
                let (status, body) = if method == "PUT" {
                    let attempt = count.fetch_add(1, Ordering::SeqCst);
                    if attempt >= allow {
                        (503, Vec::new())
                    } else {
                        let data = request[header_end..header_end + length].to_vec();
                        let hash = to_hex(&hashtree_core::sha256(&data));
                        blocks.insert(format!("/{hash}.bin"), data);
                        let ack = if wrong_hash { "0".repeat(64) } else { hash };
                        (201, format!(r#"{{"sha256":"{ack}"}}"#).into_bytes())
                    }
                } else if let Some(data) = blocks.get(path) {
                    (200, data.clone())
                } else {
                    (404, Vec::new())
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        Self { url, task, uploads }
    }
}

async fn stage(
    store: &Arc<AttachmentChunkStore>,
    dir: &Path,
    bytes: &[u8],
) -> (Arc<local_store::UploadSession>, String) {
    let file = dir.join("fixture.bin");
    std::fs::write(&file, bytes).unwrap();
    let session = store.begin(None).unwrap();
    let nhash = upload_file_to_store(&file, session.clone()).await.unwrap();
    (session, nhash)
}
fn cid(nhash: &str) -> Cid {
    let data = nhash_decode(nhash).unwrap();
    Cid {
        hash: data.hash,
        key: data.decrypt_key,
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn offline_attachment_is_durable_and_pending_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("blobs");
    let store = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    let bytes = b"local-first encrypted attachment survives server outage";
    let staged = stage(&store, dir.path(), bytes).await;
    let nhash = staged.1.clone();
    let root = cid(&nhash);
    let server = BlossomFixture::start(0, false).await;
    store
        .finish_batch(&[staged], std::slice::from_ref(&server.url))
        .unwrap();
    assert!(replication::replicate_tree(
        &store,
        &client(),
        &nostr::Keys::generate(),
        root.hash,
        &server.url
    )
    .await
    .is_err());
    assert!(store.uploaded_urls(&nhash).is_empty());
    drop(store);
    let reopened = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    assert_eq!(
        read_hashtree_attachment(&root, reopened.clone(), MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        bytes
    );
    reopened.resolve_staged(&root.hash, true).unwrap();
    assert!(reopened.next_replica().unwrap().is_some());
    assert!(reopened.delete(&root.hash).await.is_err());
    let metadata = std::fs::read(storage.join("index.sqlite3")).unwrap();
    assert!(!metadata.windows(nhash.len()).any(|w| w == nhash.as_bytes()));
    let key = root.key.unwrap();
    assert!(!metadata.windows(key.len()).any(|w| w == key));
}

#[tokio::test]
async fn only_whole_tree_correct_ack_exposes_ciphertext_root_url() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(&dir.path().join("blobs"), DEFAULT_BUDGET).unwrap();
    let data: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|i| (i ^ (i >> 13)) as u8)
        .collect();
    let staged = stage(&store, dir.path(), &data).await;
    let nhash = staged.1.clone();
    let root = cid(&nhash).hash;
    let partial = BlossomFixture::start(1, false).await;
    let wrong = BlossomFixture::start(usize::MAX, true).await;
    let complete = BlossomFixture::start(usize::MAX, false).await;
    store
        .finish_batch(
            &[staged],
            &[partial.url.clone(), wrong.url.clone(), complete.url.clone()],
        )
        .unwrap();
    let keys = nostr::Keys::generate();
    for fixture in [&partial, &wrong] {
        assert!(
            replication::replicate_tree(&store, &client(), &keys, root, &fixture.url)
                .await
                .is_err()
        );
        assert!(store.uploaded_urls(&nhash).is_empty());
    }
    replication::replicate_tree(&store, &client(), &keys, root, &complete.url)
        .await
        .unwrap();
    assert!(complete.uploads.load(Ordering::SeqCst) >= 3);
    assert_eq!(
        store.uploaded_urls(&nhash),
        vec![format!("{}/{}.bin", complete.url, to_hex(&root))]
    );
    store.delete(&root).await.unwrap(); // Entire tree is now recoverable.
}

#[tokio::test]
async fn quota_protects_pending_but_reclaims_failed_staging_and_read_cache() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(&dir.path().join("blobs"), 32 * 1024 * 1024).unwrap();
    let pending_bytes: Vec<u8> = (0..7 * 1024 * 1024)
        .map(|i| (7 + i / (512 * 1024)) as u8)
        .collect();
    let staged = stage(&store, dir.path(), &pending_bytes).await;
    let nhash = staged.1.clone();
    let pending = cid(&nhash);
    store.finish_batch(&[staged], &[]).unwrap();
    let next = store.begin(None).unwrap();
    let data = vec![8; 7 * 1024 * 1024];
    assert!(next.put(hashtree_core::sha256(&data), data).await.is_err());
    drop(next);
    assert_eq!(
        read_hashtree_attachment(&pending, store.clone(), MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        pending_bytes
    );

    let other = AttachmentChunkStore::open(&dir.path().join("failed"), 32 * 1024 * 1024).unwrap();
    let first_file = dir.path().join("first.bin");
    let second_file = dir.path().join("second.bin");
    std::fs::write(&first_file, &pending_bytes).unwrap();
    std::fs::write(
        &second_file,
        pending_bytes
            .iter()
            .map(|b| b.wrapping_add(41))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let prepared = [first_file, second_file]
        .into_iter()
        .map(|file_path| PreparedOutgoingAttachment {
            file_path,
            filename: "fixture.bin".into(),
            size_bytes: pending_bytes.len() as u64,
        })
        .collect::<Vec<_>>();
    assert!(upload_files_locally(other.clone(), &prepared, None)
        .await
        .is_err());
    assert!(other.staged_references().unwrap().is_empty());
    // The failed production multi-file batch leaves no permanent protection.
    let replacement: Vec<u8> = pending_bytes.iter().map(|b| b.wrapping_add(80)).collect();
    let replaced = stage(&other, dir.path(), &replacement).await;
    drop(replaced);
    let read = other.reading().unwrap();
    let replacement: Vec<u8> = pending_bytes.iter().map(|b| b.wrapping_add(120)).collect();
    let replaced = stage(&other, dir.path(), &replacement).await;
    assert_eq!(
        read_hashtree_attachment(&cid(&replaced.1), other.clone(), MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        replacement
    );
    drop(read); // Reads hold owned bytes, not a veto on unrelated cache eviction.
}

#[tokio::test]
async fn routed_download_replaces_full_evictable_cache_while_reading() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), 32 * 1024 * 1024).unwrap();
    let old = vec![3; 7 * 1024 * 1024];
    let old_hash = hashtree_core::sha256(&old);
    store.put(old_hash, old).await.unwrap();
    let remote = Arc::new(hashtree_core::MemoryStore::new());
    let expected: Vec<u8> = (0..7 * 1024 * 1024)
        .map(|i| (9 + i / (512 * 1024)) as u8)
        .collect();
    let tree = HashTree::new(HashTreeConfig::new(remote.clone()));
    let (cid, _) = tree.put(&expected).await.unwrap();
    let router = Arc::new(
        BlobRouter::new(
            vec![BlobRouteEntry::new(
                "fixture",
                Arc::new(StoreBlobRoute::new(remote)),
            )],
            Some(store.clone()),
            BlobRouterConfig::default(),
        )
        .unwrap(),
    );
    let routed = Arc::new(RoutedStore::new(store.clone(), router));
    assert_eq!(
        readiness::read(
            &cid,
            Some(store.clone()),
            routed,
            MAX_ATTACHMENT_BYTES,
            Duration::ZERO
        )
        .await
        .unwrap(),
        expected
    );
    assert!(store.get(&old_hash).await.unwrap().is_none());
    // The complete newly received file is now readable without a remote route.
    assert_eq!(
        read_hashtree_attachment(&cid, store.clone(), MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn restart_repairs_uncommitted_files_and_corruption_never_returns_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), DEFAULT_BUDGET).unwrap();
    let bytes = b"ciphertext fixture".to_vec();
    let hash = hashtree_core::sha256(&bytes);
    store.put(hash, bytes.clone()).await.unwrap();
    store.raw_put_for_test(hash, b"corrupt");
    assert!(store.get(&hash).await.is_err());
    store.put(hash, bytes.clone()).await.unwrap();
    let orphan = hashtree_core::sha256(b"orphan");
    store.raw_put_for_test(orphan, b"orphan");
    drop(store);
    let reopened = AttachmentChunkStore::open(dir.path(), DEFAULT_BUDGET).unwrap();
    assert!(reopened.get(&orphan).await.unwrap().is_none());
    assert_eq!(reopened.get(&hash).await.unwrap(), Some(bytes));
}

#[tokio::test]
async fn repaired_pending_block_restores_backend_pin_before_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), 32 * 1024 * 1024).unwrap();
    let staged = stage(&store, dir.path(), b"pending repair").await;
    let root = cid(&staged.1).hash;
    let bytes = store.get(&root).await.unwrap().unwrap();
    store.finish_batch(&[staged], &[]).unwrap();
    store.raw_put_for_test(root, b"broken");
    store.put(root, bytes.clone()).await.unwrap();
    // Enough eligible cache churn to exercise the backend's durable pin veto.
    for value in 0..20 {
        let data = vec![value; 1024 * 1024];
        store.put(hashtree_core::sha256(&data), data).await.unwrap();
    }
    assert_eq!(store.get(&root).await.unwrap(), Some(bytes));
    assert!(store.delete(&root).await.is_err());
}

#[tokio::test]
async fn retry_state_is_durable_bounded_and_logout_invalidates_writers() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("blobs");
    let store = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    let staged = stage(&store, dir.path(), b"pending").await;
    let root = cid(&staged.1).hash;
    let server = "https://example.invalid".to_string();
    store
        .finish_batch(&[staged], std::slice::from_ref(&server))
        .unwrap();
    store.resolve_staged(&root, true).unwrap();
    assert!(store.next_replica().unwrap().is_some());
    store.finish_replica(&root, &server, false).unwrap();
    assert!(store.next_replica().unwrap().is_none());
    let old_id = store.id;
    drop(store);
    let store = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    assert_ne!(old_id, store.id);
    assert!(store.next_replica().unwrap().is_none());
    let writer = store.begin(None).unwrap();
    store.close_and_clear().unwrap();
    let bytes = b"stale upload".to_vec();
    assert!(writer
        .put(hashtree_core::sha256(&bytes), bytes)
        .await
        .is_err());
    let new_account = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    assert!(new_account.next_replica().unwrap().is_none());
    assert!(new_account.get(&root).await.unwrap().is_none());
    let invalid = vec![
        "https://user:secret@example.invalid".into(),
        "https://example.invalid/?key=secret".into(),
        "ftp://example.invalid".into(),
    ];
    assert!(replication::bounded_servers(&invalid).is_empty());
    let many = (0..100)
        .map(|i| format!("https://s{i}.invalid"))
        .collect::<Vec<_>>();
    assert_eq!(replication::bounded_servers(&many).len(), 8);
}

#[test]
fn active_account_worker_emits_receipt_refresh_and_ignores_stale_completion() {
    let dir = tempfile::tempdir().unwrap();
    let (updates, _update_rx) = flume::unbounded();
    let (sender, receiver) = flume::unbounded();
    let mut core = AppCore::new(
        updates,
        sender,
        dir.path().to_string_lossy().into_owned(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let owner = nostr::Keys::generate();
    let device = nostr::Keys::generate();
    core.logged_in = Some(LoggedInState {
        owner_pubkey: owner.public_key(),
        owner_keys: Some(owner),
        device_keys: device.clone(),
        client: nostr_sdk::Client::new(device),
        relay_urls: Vec::new(),
        authorization_state: LocalAuthorizationState::Authorized,
    });
    let store = core.ensure_attachment_service().unwrap();
    let server = core
        .runtime
        .block_on(BlossomFixture::start(usize::MAX, false));
    let staged = core
        .runtime
        .block_on(stage(&store, dir.path(), b"worker receipt"));
    let nhash = staged.1.clone();
    store
        .finish_batch(&[staged], std::slice::from_ref(&server.url))
        .unwrap();
    let chat = nostr::Keys::generate().public_key().to_hex();
    core.handle_attachment_upload_finished(
        chat,
        Ok(format_attachment_links_message(
            "",
            &[(nhash.clone(), "worker.bin".into())],
        )),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let event = receiver
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .expect("worker receipt notification");
        if matches!(event, CoreMsg::Internal(ref event) if matches!(**event, InternalEvent::AttachmentReceiptsUpdated))
        {
            break;
        }
    }
    assert_eq!(store.uploaded_urls(&nhash).len(), 1);
    let old_id = store.id;
    core.clear_attachment_service();
    core.ensure_attachment_service().unwrap();
    core.state.busy.uploading_attachment = true;
    core.handle_internal(InternalEvent::AttachmentUploadFinished {
        store_id: old_id,
        chat_id: "stale".into(),
        result: Err("old account".into()),
    });
    assert!(
        core.state.busy.uploading_attachment,
        "old account completion must not change the current draft"
    );
    core.clear_attachment_service();
    drop(server);
}

#[tokio::test]
async fn metadata_and_parallel_work_are_bounded_across_reconfiguration() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("blobs");
    let store = AttachmentChunkStore::open(&storage, DEFAULT_BUDGET).unwrap();
    let staged = stage(&store, dir.path(), b"bounded metadata").await;
    store.finish_batch(&[staged], &[]).unwrap();
    for i in 0..20 {
        store
            .configure_pending(&[format!("https://s{i}.invalid")])
            .unwrap();
    }
    let db = rusqlite::Connection::open(storage.join("index.sqlite3")).unwrap();
    let count: usize = db
        .query_row("SELECT COUNT(*) FROM replicas", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "only the current failed target remains");
    let writers = (0..32)
        .map(|_| store.begin(None).unwrap())
        .collect::<Vec<_>>();
    assert!(store.begin(None).is_err());
    drop(writers);
    assert!(store.begin(None).is_ok());
    let readers = (0..4).map(|_| store.reading().unwrap()).collect::<Vec<_>>();
    assert!(store.reading().is_err());
    drop(readers);
    assert!(store.reading().is_ok());
}

#[tokio::test]
async fn replacing_all_failed_servers_rescues_pending_tree_and_keeps_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(&dir.path().join("blobs"), DEFAULT_BUDGET).unwrap();
    let staged = stage(&store, dir.path(), b"rescue from dead upload targets").await;
    let nhash = staged.1.clone();
    let root = cid(&nhash).hash;
    let old = (0..8)
        .map(|i| format!("https://old-{i}.invalid"))
        .collect::<Vec<_>>();
    store.finish_batch(&[staged], &old).unwrap();
    store.resolve_staged(&root, true).unwrap();
    for server in &old {
        store.finish_replica(&root, server, false).unwrap();
    }
    let healthy = BlossomFixture::start(usize::MAX, false).await;
    let mut replacement = vec![healthy.url.clone()];
    replacement.extend((0..7).map(|i| format!("https://new-{i}.invalid")));
    store.configure_pending(&replacement).unwrap();
    // A superseded in-flight ACK must not erase the new pending protection.
    store.finish_replica(&root, &old[0], true).unwrap();
    assert!(store.uploaded_urls(&nhash).is_empty());
    assert!(store.delete(&root).await.is_err());
    assert_eq!(
        store.next_replica().unwrap(),
        Some((root, healthy.url.clone()))
    );
    replication::replicate_tree(
        &store,
        &client(),
        &nostr::Keys::generate(),
        root,
        &healthy.url,
    )
    .await
    .unwrap();
    let receipt = store.uploaded_urls(&nhash);
    store
        .configure_pending(&["https://later.invalid".into()])
        .unwrap();
    assert_eq!(
        store.uploaded_urls(&nhash),
        receipt,
        "confirmed history survives configuration edits"
    );
    let db = rusqlite::Connection::open(dir.path().join("blobs/index.sqlite3")).unwrap();
    let count: usize = db
        .query_row("SELECT COUNT(*) FROM replicas", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "old incomplete jobs are retired");
}

// A separate process is essential: POSIX does not report a process's own
// record locks. Probe LMDB's environment lock even when no transaction is live.
#[cfg(unix)]
fn environment_lock_is_held(directory: &Path) -> bool {
    let status = std::process::Command::new("python3")
        .args([
            "-c",
            r#"import fcntl, os, sys
with open(sys.argv[1], 'r+b') as f:
    try:
        fcntl.lockf(f, fcntl.LOCK_EX | fcntl.LOCK_NB, 1, 0, os.SEEK_SET)
    except BlockingIOError:
        sys.exit(42)
"#,
        ])
        .arg(directory.join("blobs/lock.mdb"))
        .status()
        .expect("run independent file-lock observer");
    assert!(
        matches!(status.code(), Some(0 | 42)),
        "lock observer failed: {status}"
    );
    status.code() == Some(42)
}

#[cfg(unix)]
#[tokio::test]
async fn suspend_releases_idle_environment_lock_with_live_owners_and_preserves_data() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), 8 * 1024 * 1024).unwrap();
    let retained_owner = store.clone();
    let bytes = b"cached ciphertext survives iOS suspension".to_vec();
    let hash = hashtree_core::sha256(&bytes);
    store.put(hash, bytes.clone()).await.unwrap();
    assert!(environment_lock_is_held(dir.path()));
    store.suspend().unwrap();
    assert!(!environment_lock_is_held(dir.path()));
    assert!(retained_owner.get(&hash).await.is_err());
    assert!(retained_owner.put(hash, bytes.clone()).await.is_err());
    assert!(store.begin(None).is_err());
    assert!(
        !environment_lock_is_held(dir.path()),
        "late work must not reopen LMDB"
    );
    store.suspend().unwrap();
    store.resume().unwrap();
    assert!(environment_lock_is_held(dir.path()));
    assert_eq!(retained_owner.get(&hash).await.unwrap(), Some(bytes));
    store.close();
    assert!(!environment_lock_is_held(dir.path()));
    assert!(
        store.resume().is_err(),
        "logout must not revive an old account"
    );
}

#[tokio::test]
async fn suspended_dropped_upload_pins_are_reconciled_before_quota_eviction() {
    let dir = tempfile::tempdir().unwrap();
    // Keep room for LMDB's COW pages while still exceeding the 12 MiB
    // logical data quota. A 4 MiB map cannot reliably churn 1 MiB records.
    let store = AttachmentChunkStore::open(dir.path(), 32 * 1024 * 1024).unwrap();
    let abandoned = store.begin(None).unwrap();
    let old = vec![13; 1024 * 1024];
    let old_hash = hashtree_core::sha256(&old);
    abandoned.put(old_hash, old).await.unwrap();
    store.suspend().unwrap();
    drop(abandoned);
    store.resume().unwrap();
    for i in 0..32 {
        let bytes = vec![20 + i; 1024 * 1024];
        store
            .put(hashtree_core::sha256(&bytes), bytes)
            .await
            .unwrap();
    }
    assert!(store.get(&old_hash).await.unwrap().is_none());
}

#[tokio::test]
async fn logout_clears_a_suspended_cache_without_reopening_it_to_old_owners() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), 8 * 1024 * 1024).unwrap();
    let bytes = b"account-owned ciphertext".to_vec();
    let hash = hashtree_core::sha256(&bytes);
    store.put(hash, bytes).await.unwrap();
    store.suspend().unwrap();
    store.close_and_clear().unwrap();
    store.close();
    assert!(store.get(&hash).await.is_err());
    assert!(store.resume().is_err());
    drop(store);
    let reopened = AttachmentChunkStore::open(dir.path(), 8 * 1024 * 1024).unwrap();
    assert!(reopened.get(&hash).await.unwrap().is_none());
}
