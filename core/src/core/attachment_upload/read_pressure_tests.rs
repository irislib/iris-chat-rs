use super::*;

struct PausedCachedRead {
    local: Arc<AttachmentChunkStore>,
    root: Hash,
    root_read: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait]
impl Store for PausedCachedRead {
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let bytes = self.local.get(hash).await?;
        if *hash == self.root {
            self.root_read.notify_one();
            self.resume.notified().await;
        }
        Ok(bytes)
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.has(hash).await
    }
    async fn put(&self, _: Hash, _: Vec<u8>) -> Result<bool, StoreError> {
        Err(StoreError::Other("read-only fixture".into()))
    }
    async fn delete(&self, _: &Hash) -> Result<bool, StoreError> {
        Err(StoreError::Other("read-only fixture".into()))
    }
}

#[tokio::test]
async fn cached_tree_read_survives_concurrent_quota_eviction_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let local = AttachmentChunkStore::open(dir.path(), 32 * 1024 * 1024).unwrap();
    let expected: Vec<u8> = (0..7 * 1024 * 1024)
        .map(|i| (9 + i / (512 * 1024)) as u8)
        .collect();
    // Received/recoverable cache: no pending outgoing tree protects these blocks.
    let tree = HashTree::new(HashTreeConfig::new(local.clone()));
    let (cid, _) = tree.put(&expected).await.unwrap();
    let root = cid.hash;
    let mut old = Vec::new();
    for value in 40..44 {
        let bytes = vec![value; 1024 * 1024];
        let hash = hashtree_core::sha256(&bytes);
        local.put(hash, bytes).await.unwrap();
        old.push(hash);
    }
    let source = Arc::new(PausedCachedRead {
        local: local.clone(),
        root,
        root_read: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let reading_local = local.clone();
    let reading_source = source.clone();
    let read = tokio::spawn(async move {
        readiness::read(
            &cid,
            Some(reading_local),
            reading_source,
            MAX_ATTACHMENT_BYTES,
            Duration::ZERO,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), source.root_read.notified())
        .await
        .unwrap();
    // Each new block must be admitted by evicting unrelated eligible cache.
    // A global read veto fails this; unprotected tree leaves fail the final read.
    for value in 80..88 {
        let bytes = vec![value; 1024 * 1024];
        local
            .put(hashtree_core::sha256(&bytes), bytes)
            .await
            .unwrap();
    }
    assert!(local.get(&old[0]).await.unwrap().is_none());
    source.resume.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), read)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        expected
    );
    assert!(
        local.delete(&root).await.unwrap(),
        "read protection is released on completion"
    );
}
