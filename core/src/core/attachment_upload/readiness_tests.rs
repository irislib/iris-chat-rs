use super::*;
use hashtree_core::MemoryStore;

#[derive(Default)]
struct MissingHttp {
    attempted: tokio::sync::Notify,
    calls: AtomicU64,
}

#[async_trait]
impl Store for MissingHttp {
    async fn get(&self, _: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.attempted.notify_one();
        Ok(None)
    }
    async fn put(&self, _: Hash, _: Vec<u8>) -> Result<bool, StoreError> {
        Err(StoreError::Other("read-only fixture".into()))
    }
    async fn has(&self, _: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete(&self, _: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
}

async fn fixture() -> (Cid, Arc<MemoryStore>) {
    let memory = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(memory.clone()));
    let (cid, _) = tree.put(b"late peer attachment").await.unwrap();
    (cid, memory)
}

fn route(
    local: Arc<AttachmentChunkStore>,
    source: Arc<MemoryStore>,
) -> Arc<RoutedStore<AttachmentChunkStore>> {
    let router = Arc::new(
        BlobRouter::new(
            vec![BlobRouteEntry::new(
                "delayed-fixture-peer",
                Arc::new(StoreBlobRoute::new(source)),
            )],
            Some(local.clone()),
            BlobRouterConfig::default(),
        )
        .unwrap(),
    );
    Arc::new(RoutedStore::new(local, router))
}

#[tokio::test]
async fn http_miss_waits_for_exact_store_transport_event_then_caches() {
    let directory = tempfile::tempdir().unwrap();
    let local = AttachmentChunkStore::open(directory.path(), local_store::DEFAULT_BUDGET).unwrap();
    let (cid, memory) = fixture().await;
    let cached_cid = cid.clone();
    let http = Arc::new(MissingHttp::default());
    let waiting_local = local.clone();
    let waiting_http = http.clone();
    let task = tokio::spawn(async move {
        read(
            &cid,
            Some(waiting_local),
            waiting_http,
            MAX_ATTACHMENT_BYTES,
            Duration::from_secs(2),
        )
        .await
    });
    http.attempted.notified().await;
    let peer = route(local.clone(), memory);
    local.transport_ready(&peer).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        b"late peer attachment"
    );
    assert_eq!(http.calls.load(Ordering::SeqCst), 1);
    // The successful routed retry wrote into this account's persistent cache.
    assert_eq!(
        read_hashtree_attachment(&cached_cid, local, MAX_ATTACHMENT_BYTES)
            .await
            .unwrap(),
        b"late peer attachment"
    );
}

#[tokio::test]
async fn healthy_http_or_cache_read_does_not_wait_for_transport() {
    let directory = tempfile::tempdir().unwrap();
    let local = AttachmentChunkStore::open(directory.path(), local_store::DEFAULT_BUDGET).unwrap();
    let (cid, memory) = fixture().await;
    let bytes = tokio::time::timeout(
        Duration::from_millis(250),
        read(
            &cid,
            Some(local.clone()),
            memory.clone(),
            MAX_ATTACHMENT_BYTES,
            Duration::from_secs(60),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(bytes, b"late peer attachment");
    local
        .put(cid.hash, memory.get(&cid.hash).await.unwrap().unwrap())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(250),
            read(
                &cid,
                Some(local.clone()),
                local,
                MAX_ATTACHMENT_BYTES,
                Duration::from_secs(60),
            )
        )
        .await
        .unwrap()
        .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn startup_wait_is_bounded_and_releases_its_reader_slot() {
    let directory = tempfile::tempdir().unwrap();
    let local = AttachmentChunkStore::open(directory.path(), local_store::DEFAULT_BUDGET).unwrap();
    let (cid, _) = fixture().await;
    let http = Arc::new(MissingHttp::default());
    assert!(tokio::time::timeout(
        Duration::from_secs(1),
        read(
            &cid,
            Some(local.clone()),
            http.clone(),
            MAX_ATTACHMENT_BYTES,
            Duration::from_millis(10),
        )
    )
    .await
    .unwrap()
    .is_err());
    assert_eq!(http.calls.load(Ordering::SeqCst), 1);
    let readers = (0..4).map(|_| local.reading().unwrap()).collect::<Vec<_>>();
    assert!(local.reading().is_err());
    drop(readers);
}

#[tokio::test]
async fn logout_cancels_waiters_without_borrowing_another_accounts_route() {
    let directory = tempfile::tempdir().unwrap();
    let local =
        AttachmentChunkStore::open(&directory.path().join("old"), local_store::DEFAULT_BUDGET)
            .unwrap();
    let replacement =
        AttachmentChunkStore::open(&directory.path().join("new"), local_store::DEFAULT_BUDGET)
            .unwrap();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (cid, _) = fixture().await;
        let http = Arc::new(MissingHttp::default());
        let local = local.clone();
        let attempted = http.clone();
        tasks.push(tokio::spawn(async move {
            read(
                &cid,
                Some(local),
                http,
                MAX_ATTACHMENT_BYTES,
                Duration::from_secs(60),
            )
            .await
        }));
        attempted.attempted.notified().await;
    }
    let (cid, memory) = fixture().await;
    let http = Arc::new(MissingHttp::default());
    assert!(read(
        &cid,
        Some(local.clone()),
        http.clone(),
        MAX_ATTACHMENT_BYTES,
        Duration::from_secs(60)
    )
    .await
    .is_err());
    assert_eq!(
        http.calls.load(Ordering::SeqCst),
        0,
        "fifth request is rejected before HTTP"
    );
    let peer = route(replacement.clone(), memory);
    replacement.transport_ready(&peer).unwrap();
    assert!(
        local.transport_ready(&peer).is_err(),
        "cross-account route is rejected"
    );
    local.close();
    for task in tasks {
        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("closed"));
    }
    let (_, memory) = fixture().await;
    assert!(
        local
            .transport_ready(&route(local.clone(), memory))
            .is_err(),
        "late old-account bind is rejected"
    );
}
