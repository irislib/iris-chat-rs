use async_trait::async_trait;
use hashtree_core::{sha256, to_hex, Hash, Store, StoreError};
use reqwest::StatusCode;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;

/// Read-only Blossom store whose owned response buffer cannot exceed its
/// per-blob cap. Every received body chunk also shares one traversal budget,
/// including chunks from failed servers and snapshot fallback attempts.
pub(super) struct BoundedBlossomStore {
    client: reqwest::Client,
    servers: Vec<String>,
    max_blob_bytes: usize,
    budget: ReadBudget,
    cache: RwLock<HashMap<Hash, Vec<u8>>>,
}

struct ReadBudget {
    reads_left: AtomicUsize,
    bytes_left: AtomicUsize,
}

impl ReadBudget {
    fn new(reads: usize, bytes: usize) -> Self {
        Self {
            reads_left: AtomicUsize::new(reads),
            bytes_left: AtomicUsize::new(bytes),
        }
    }

    fn consume(counter: &AtomicUsize, amount: usize) -> bool {
        let previous = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                Some(left.saturating_sub(amount))
            })
            .unwrap_or_else(|left| left);
        previous >= amount
    }

    fn consume_read(&self) -> bool {
        Self::consume(&self.reads_left, 1)
    }

    fn consume_bytes(&self, bytes: usize) -> bool {
        Self::consume(&self.bytes_left, bytes)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BlobFetchError {
    Request(String),
    Status(u16),
    TooLarge { buffered_bytes: usize, limit: usize },
    HashMismatch,
    BudgetExceeded,
}

impl std::fmt::Display for BlobFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(error) => formatter.write_str(error),
            Self::Status(status) => write!(formatter, "HTTP {status}"),
            Self::TooLarge {
                buffered_bytes,
                limit,
            } => write!(
                formatter,
                "blob exceeds {limit}-byte limit after buffering {buffered_bytes} bytes"
            ),
            Self::HashMismatch => formatter.write_str("downloaded blob hash does not match CID"),
            Self::BudgetExceeded => formatter.write_str("profile search byte budget exceeded"),
        }
    }
}

impl BoundedBlossomStore {
    pub(super) fn new(
        servers: Vec<String>,
        timeout: Duration,
        max_blob_bytes: usize,
        max_reads: usize,
        max_total_bytes: usize,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| format!("could not start profile blob client: {error}"))?;
        Ok(Self {
            client,
            servers,
            max_blob_bytes,
            budget: ReadBudget::new(max_reads, max_total_bytes),
            cache: RwLock::new(HashMap::new()),
        })
    }

    async fn fetch_from_server(
        &self,
        server: &str,
        hash: &Hash,
    ) -> Result<Option<Vec<u8>>, BlobFetchError> {
        let url = format!("{}/{}.bin", server.trim_end_matches('/'), to_hex(hash));
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| BlobFetchError::Request(error.to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(BlobFetchError::Status(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.max_blob_bytes as u64)
        {
            return Err(BlobFetchError::TooLarge {
                buffered_bytes: 0,
                limit: self.max_blob_bytes,
            });
        }

        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| BlobFetchError::Request(error.to_string()))?
        {
            if !self.budget.consume_bytes(chunk.len()) {
                return Err(BlobFetchError::BudgetExceeded);
            }
            if chunk.len() > self.max_blob_bytes.saturating_sub(data.len()) {
                return Err(BlobFetchError::TooLarge {
                    buffered_bytes: data.len(),
                    limit: self.max_blob_bytes,
                });
            }
            data.extend_from_slice(&chunk);
        }
        if sha256(&data) != *hash {
            return Err(BlobFetchError::HashMismatch);
        }
        Ok(Some(data))
    }
}

#[async_trait]
impl Store for BoundedBlossomStore {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Err(StoreError::Other(
            "profile search store is read-only".to_string(),
        ))
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if !self.budget.consume_read() {
            return Err(StoreError::Other(
                "profile search read budget exceeded".to_string(),
            ));
        }
        {
            let cache = self.cache.read().await;
            if let Some(data) = cache.get(hash) {
                if !self.budget.consume_bytes(data.len()) {
                    return Err(StoreError::Other(
                        "profile search byte budget exceeded".to_string(),
                    ));
                }
                return Ok(Some(data.clone()));
            }
        }

        let mut last_error = None;
        for server in &self.servers {
            match self.fetch_from_server(server, hash).await {
                Ok(Some(data)) => {
                    self.cache.write().await.insert(*hash, data.clone());
                    return Ok(Some(data));
                }
                Ok(None) => {}
                Err(BlobFetchError::BudgetExceeded) => {
                    return Err(StoreError::Other(
                        "profile search byte budget exceeded".to_string(),
                    ));
                }
                Err(error) => last_error = Some(format!("{server}: {error}")),
            }
        }
        match last_error {
            Some(error) => Err(StoreError::Other(format!(
                "profile search blob download failed: {error}"
            ))),
            None => Ok(None),
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        if !self.budget.consume_read() {
            return Err(StoreError::Other(
                "profile search read budget exceeded".to_string(),
            ));
        }
        if self.cache.read().await.contains_key(hash) {
            return Ok(true);
        }
        let hash = to_hex(hash);
        for server in &self.servers {
            let url = format!("{}/{}.bin", server.trim_end_matches('/'), hash);
            if self
                .client
                .head(url)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Err(StoreError::Other(
            "profile search store is read-only".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_chunked_blob(chunks: Vec<Vec<u8>>) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            for chunk in chunks {
                if write!(stream, "{:x}\r\n", chunk.len()).is_err()
                    || stream.write_all(&chunk).is_err()
                    || stream.write_all(b"\r\n").is_err()
                    || stream.flush().is_err()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let _ = stream.write_all(b"0\r\n\r\n");
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{address}"), server)
    }

    fn test_store(
        servers: Vec<String>,
        blob: usize,
        reads: usize,
        total: usize,
    ) -> BoundedBlossomStore {
        BoundedBlossomStore::new(servers, Duration::from_secs(1), blob, reads, total).unwrap()
    }

    fn assert_requested(server: thread::JoinHandle<String>, hash: &Hash) {
        assert!(server
            .join()
            .unwrap()
            .contains(&format!("/{}.bin", to_hex(hash))));
    }

    #[tokio::test]
    async fn chunked_response_never_grows_owned_buffer_past_blob_limit() {
        let body = vec![7; 48];
        let hash = sha256(&body);
        let (url, server) = serve_chunked_blob(vec![body[..16].to_vec(); 3]);
        let store = test_store(vec![url.clone()], 24, 1, 100);

        let error = store.fetch_from_server(&url, &hash).await.unwrap_err();
        assert!(matches!(
            error,
            BlobFetchError::TooLarge {
                buffered_bytes: 0..=24,
                limit: 24
            }
        ));
        assert_requested(server, &hash);
    }

    #[tokio::test]
    async fn oversized_server_falls_back_to_valid_hash_matching_blob() {
        let valid = b"bounded profile blob";
        let hash = sha256(valid);
        let (large_url, large_server) = serve_chunked_blob(vec![vec![9; 16], vec![9; 16]]);
        let (valid_url, valid_server) = serve_chunked_blob(vec![valid.to_vec()]);
        let store = test_store(vec![large_url, valid_url], valid.len(), 1, 100);

        assert_eq!(
            store.get(&hash).await.unwrap().as_deref(),
            Some(valid.as_slice())
        );
        assert_requested(large_server, &hash);
        assert_requested(valid_server, &hash);
    }

    #[tokio::test]
    async fn hash_mismatch_falls_back_to_valid_server() {
        let valid = b"bounded profile index blob";
        let hash = sha256(valid);
        let (bad_url, bad_server) = serve_chunked_blob(vec![b"wrong body".to_vec()]);
        let (valid_url, valid_server) = serve_chunked_blob(vec![valid.to_vec()]);
        let store = test_store(vec![bad_url, valid_url], valid.len(), 1, 100);

        assert_eq!(
            store.get(&hash).await.unwrap().as_deref(),
            Some(valid.as_slice())
        );
        assert_requested(bad_server, &hash);
        assert_requested(valid_server, &hash);
    }

    #[tokio::test]
    async fn failed_servers_share_and_exhaust_aggregate_byte_budget() {
        let hash = sha256(b"expected body");
        let (first_url, first_server) = serve_chunked_blob(vec![vec![1; 16]]);
        let (second_url, second_server) = serve_chunked_blob(vec![vec![2; 16]]);
        let store = test_store(vec![first_url, second_url], 32, 1, 24);

        let error = store.get(&hash).await.unwrap_err();
        assert!(error.to_string().contains("byte budget exceeded"));
        assert_requested(first_server, &hash);
        assert_requested(second_server, &hash);
        assert_eq!(store.budget.bytes_left.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn cache_hits_consume_read_and_byte_budgets() {
        let body = b"cached profile blob";
        let hash = sha256(body);
        let (url, server) = serve_chunked_blob(vec![body.to_vec()]);
        let store = test_store(vec![url], body.len(), 3, body.len() * 2);

        assert!(store.get(&hash).await.unwrap().is_some());
        assert!(store.get(&hash).await.unwrap().is_some());
        assert_eq!(store.budget.reads_left.load(Ordering::Relaxed), 1);
        assert_eq!(store.budget.bytes_left.load(Ordering::Relaxed), 0);
        assert!(store
            .get(&hash)
            .await
            .unwrap_err()
            .to_string()
            .contains("byte budget exceeded"));
        assert_requested(server, &hash);
    }
}
