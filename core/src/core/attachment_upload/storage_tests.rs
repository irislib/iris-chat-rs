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
async fn quota_protects_pending_and_reads_but_reclaims_failed_staging() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(&dir.path().join("blobs"), 1024 * 1024).unwrap();
    let staged = stage(&store, dir.path(), &vec![7; 300 * 1024]).await;
    let nhash = staged.1.clone();
    let pending = cid(&nhash);
    store.finish_batch(&[staged], &[]).unwrap();
    let next = store.begin(None).unwrap();
    let data = vec![8; 300 * 1024];
    assert!(next.put(hashtree_core::sha256(&data), data).await.is_err());
    drop(next);
    assert_eq!(
        read_hashtree_attachment(&pending, store.clone(), MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        vec![7; 300 * 1024]
    );

    let other = AttachmentChunkStore::open(&dir.path().join("failed"), 1024 * 1024).unwrap();
    let first = stage(&other, dir.path(), &vec![1; 300 * 1024]).await;
    let second = other.begin(None).unwrap();
    let data = vec![2; 300 * 1024];
    assert!(second
        .put(hashtree_core::sha256(&data), data)
        .await
        .is_err());
    drop(first);
    drop(second); // Failed multi-file batch never committed persistent pins.
    let data = vec![3; 300 * 1024];
    other.put(hashtree_core::sha256(&data), data).await.unwrap();
    let read = other.reading().unwrap();
    let data = vec![4; 300 * 1024];
    assert!(other
        .put(hashtree_core::sha256(&data), data.clone())
        .await
        .is_err());
    drop(read);
    other.put(hashtree_core::sha256(&data), data).await.unwrap();
}

#[tokio::test]
async fn restart_repairs_uncommitted_files_and_corruption_never_returns_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = AttachmentChunkStore::open(dir.path(), DEFAULT_BUDGET).unwrap();
    let bytes = b"ciphertext fixture".to_vec();
    let hash = hashtree_core::sha256(&bytes);
    store.put(hash, bytes.clone()).await.unwrap();
    std::fs::write(dir.path().join(to_hex(&hash)), b"corrupt").unwrap();
    assert!(store.get(&hash).await.is_err());
    store.put(hash, bytes.clone()).await.unwrap();
    drop(store);
    let orphan = hashtree_core::sha256(b"orphan");
    std::fs::write(dir.path().join(to_hex(&orphan)), b"orphan").unwrap();
    std::fs::write(dir.path().join("incoming.tmp"), b"interrupted").unwrap();
    let reopened = AttachmentChunkStore::open(dir.path(), DEFAULT_BUDGET).unwrap();
    assert!(!dir.path().join(to_hex(&orphan)).exists());
    assert!(!dir.path().join("incoming.tmp").exists());
    assert_eq!(reopened.get(&hash).await.unwrap(), Some(bytes));
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
    assert_eq!(count, 8);
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
