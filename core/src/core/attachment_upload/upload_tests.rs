use super::*;
use hashtree_core::MemoryStore;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

const IRIS_LOGO_PNG: &[u8] =
    include_bytes!("../../../../android/app/src/main/res/drawable-nodpi/iris_logo.png");
const IRIS_LOGO_SVG: &[u8] = include_bytes!("../../../../assets/iris-chat-logo.svg");

#[derive(Default)]
struct RecordedAttachmentStore(std::sync::Mutex<HashMap<Hash, Vec<u8>>>);

#[async_trait]
impl Store for RecordedAttachmentStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        Ok(self.0.lock().unwrap().insert(hash, data).is_none())
    }
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.0.lock().unwrap().get(hash).cloned())
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self.0.lock().unwrap().contains_key(hash))
    }
    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        Ok(self.0.lock().unwrap().remove(hash).is_some())
    }
}

#[tokio::test]
async fn oversized_attachment_is_rejected_before_uploading_any_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.bin");
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_ATTACHMENT_BYTES + 1)
        .unwrap();
    let store = Arc::new(RecordedAttachmentStore::default());

    let error = upload_file_to_store(&path, store.clone()).await.unwrap_err();

    assert!(error.to_string().contains("too large"), "{error}");
    assert!(store.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn attachment_upload_encrypts_every_stored_chunk_and_requires_the_key() {
    use hashtree_core::{decrypt_chk, try_decode_tree_node, DEFAULT_CHUNK_SIZE};
    let dir = tempfile::tempdir().unwrap();
    // Every native attachment, including voice messages, uses this upload path.
    let large: Vec<u8> = (0..DEFAULT_CHUNK_SIZE * 2 + 123)
        .map(|i| (i.wrapping_mul(37) ^ (i >> 8)) as u8)
        .collect();
    for (name, data) in [
        ("photo.png", IRIS_LOGO_PNG),
        ("drawing.svg", IRIS_LOGO_SVG),
        (
            "voice.m4a",
            include_bytes!("../../../../test-fixtures/voice-message.m4a").as_slice(),
        ),
        (
            "document.txt",
            b"Private attachment regression fixture".as_slice(),
        ),
        ("large-video.mp4", large.as_slice()),
        ("empty.txt", b"".as_slice()),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, data).unwrap();
        let store = Arc::new(RecordedAttachmentStore::default());
        let link = upload_file_to_store(&path, store.clone()).await.unwrap();
        let decoded = nhash_decode(&link).unwrap();
        let cid = Cid {
            hash: decoded.hash,
            key: decoded.decrypt_key,
        };
        assert!(
            cid.key.is_some(),
            "{name}: attachment link needs a decryption key"
        );
        assert_eq!(
            read_hashtree_attachment(&cid, store.clone(), MAX_ATTACHMENT_BYTES)
                .await
                .unwrap(),
            data
        );
        let ciphertext = store.0.lock().unwrap().clone();
        let mut pending = vec![cid.clone()];
        let mut visited = HashSet::new();
        while let Some(node) = pending.pop() {
            if !visited.insert(node.hash) {
                continue;
            }
            let key = node.key.expect("every child and root must be encrypted");
            let bytes = &ciphertext[&node.hash];
            let plain =
                decrypt_chk(bytes, &key).expect("stored bytes must authenticate with their key");
            assert_ne!(bytes, &plain, "{name}: plaintext reached the blob store");
            let mut wrong_key = key;
            wrong_key[0] ^= 1;
            assert!(decrypt_chk(bytes, &wrong_key).is_err());
            if let Some(tree) = try_decode_tree_node(&plain) {
                pending.extend(tree.links.iter().map(|link| link.to_cid()));
            }
        }
        assert_eq!(
            visited.len(),
            ciphertext.len(),
            "{name}: every stored chunk must be checked"
        );
        let without_key = Cid {
            hash: cid.hash,
            key: None,
        };
        let unkeyed = read_hashtree_attachment(&without_key, store, MAX_ATTACHMENT_BYTES).await;
        assert!(
            unkeyed.is_err() || unkeyed.unwrap() != data,
            "{name}: hash alone exposed attachment"
        );
    }
}

pub(super) fn serve_one_blossom_upload(status: &str) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local Blossom test server");
    let address = listener.local_addr().expect("local Blossom address");
    let status = status.to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept Blossom upload");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut expected_length = None;

        loop {
            let count = stream.read(&mut buffer).expect("read Blossom upload");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);

            if expected_length.is_none() {
                if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                    expected_length = content_length.map(|length| header_end + 4 + length);
                }
            }

            if expected_length.is_some_and(|length| request.len() >= length) {
                break;
            }
        }

        let response =
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream
            .write_all(response.as_bytes())
            .expect("write Blossom response");
        request
    });
    (format!("http://{address}"), server)
}

#[tokio::test]
async fn rejected_blossom_upload_is_not_reported_as_stored() {
    let hash = hashtree_core::sha256(IRIS_LOGO_PNG);
    let hash_hex = to_hex(&hash);
    shared_chunk_cache_write().remove(&hash_hex);
    let (server_url, server) = serve_one_blossom_upload("403 Forbidden");
    let store = UploadingBlossomStore::new(nostr::Keys::generate(), vec![], vec![server_url], None)
        .unwrap();

    let result = store.put(hash, IRIS_LOGO_PNG.to_vec()).await;

    let request = server.join().expect("join local Blossom test server");
    assert!(String::from_utf8_lossy(&request).starts_with("PUT /upload HTTP/1.1"));
    assert!(result.is_err(), "rejected remote upload must fail");
    assert!(!shared_chunk_cache_read().contains_key(&hash_hex));
}

#[tokio::test]
async fn retryable_blossom_failure_is_not_hidden_behind_long_retries() {
    let mut logo_with_marker = IRIS_LOGO_PNG.to_vec();
    logo_with_marker.extend_from_slice(b"-retry-test");
    let hash = hashtree_core::sha256(&logo_with_marker);
    let (server_url, server) = serve_one_blossom_upload("503 Service Unavailable");
    let store = UploadingBlossomStore::new(nostr::Keys::generate(), vec![], vec![server_url], None)
        .unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.put(hash, logo_with_marker),
    )
    .await
    .expect("a failed attachment upload must not enter a long retry loop");

    server.join().expect("join local Blossom test server");
    assert!(
        result.is_err(),
        "retryable remote failure must fail the send"
    );
}

#[tokio::test]
async fn confirmed_blossom_upload_is_cached_with_matching_bytes() {
    let hash = hashtree_core::sha256(IRIS_LOGO_SVG);
    let hash_hex = to_hex(&hash);
    shared_chunk_cache_write().remove(&hash_hex);
    let (server_url, server) = serve_one_blossom_upload("201 Created");
    let progress = Arc::new(AtomicU64::new(0));
    let store = UploadingBlossomStore::new(
        nostr::Keys::generate(),
        vec![],
        vec![server_url],
        Some(progress.clone()),
    )
    .unwrap();

    let result = store.put(hash, IRIS_LOGO_SVG.to_vec()).await;

    let request = server.join().expect("join local Blossom test server");
    let body_start = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .expect("upload headers")
        + 4;
    assert_eq!(&request[body_start..], IRIS_LOGO_SVG);
    assert!(result.expect("confirmed remote upload"));
    assert_eq!(
        shared_chunk_cache_read().get(&hash_hex).map(Vec::as_slice),
        Some(IRIS_LOGO_SVG)
    );
    assert_eq!(progress.load(Ordering::Relaxed), IRIS_LOGO_SVG.len() as u64);
}

#[tokio::test]
#[ignore = "publishes an encrypted fixture to the configured Blossom server"]
async fn real_blossom_round_trip_survives_sender_cache_clear() {
    let dir = tempfile::tempdir().expect("attachment tempdir");
    let path = dir.path().join("iris-logo.png");
    fs::write(&path, IRIS_LOGO_PNG).expect("write Iris logo fixture");
    let keys = nostr::Keys::generate();

    let nhash = upload_file_to_hashtree(keys.secret_key().to_secret_hex().as_str(), &path, None)
        .await
        .expect("upload Iris logo to configured Blossom server");

    let uploaded = nhash_decode(&nhash).expect("decode uploaded nhash");
    shared_chunk_cache_write().remove(&to_hex(&uploaded.hash));
    *attachment_blob_store()
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = None;

    let downloaded = download_hashtree_attachment_base64(&nhash)
        .await
        .expect("download Iris logo without sender cache");
    let downloaded = base64::engine::general_purpose::STANDARD
        .decode(downloaded)
        .expect("decode downloaded logo");
    assert_eq!(downloaded, IRIS_LOGO_PNG);
}

struct CountingDownloadStore {
    inner: MemoryStore,
    reads: AtomicU64,
}

#[async_trait]
impl Store for CountingDownloadStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.inner.put(hash, data).await
    }
    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.get(hash).await
    }
    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.has(hash).await
    }
    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.delete(hash).await
    }
}

#[tokio::test]
async fn attachment_limit_rejects_encrypted_tree_before_downloading_children() {
    let store = Arc::new(CountingDownloadStore {
        inner: MemoryStore::new(),
        reads: AtomicU64::new(0),
    });
    let tree = HashTree::new(HashTreeConfig::new(store.clone()).with_chunk_size(1024));
    let data = vec![7u8; 4096];
    let (cid, _) = tree.put(&data).await.unwrap();
    store.reads.store(0, Ordering::Relaxed);
    let error = read_hashtree_attachment(&cid, store.clone(), 4095)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("max_size"), "{error}");
    assert_eq!(
        store.reads.load(Ordering::Relaxed),
        1,
        "only the root may be read for an oversized attachment"
    );
    assert_eq!(
        read_hashtree_attachment(&cid, store, 4096).await.unwrap(),
        data
    );
}

#[tokio::test]
async fn attachment_limit_cannot_be_raised_by_a_preview_caller() {
    use hashtree_core::{encode_tree_node, Link, LinkType, TreeNode};
    let store = Arc::new(CountingDownloadStore {
        inner: MemoryStore::new(),
        reads: AtomicU64::new(0),
    });
    let node = TreeNode {
        node_type: LinkType::File,
        links: vec![Link::new([9; 32]).with_size(MAX_ATTACHMENT_BYTES + 1)],
    };
    let data = encode_tree_node(&node).unwrap();
    let hash = hashtree_core::sha256(&data);
    store.put(hash, data).await.unwrap();
    let error = read_hashtree_attachment(&Cid { hash, key: None }, store.clone(), u64::MAX)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("max_size"), "{error}");
    assert_eq!(store.reads.load(Ordering::Relaxed), 1);
}

fn read_download_request_headers(stream: &mut impl Read) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "connection closed before request headers");
        request.extend_from_slice(&buffer[..count]);
        assert!(
            request.len() <= 16 * 1024,
            "test request headers are too large"
        );
    }
}

#[tokio::test]
async fn attachment_limit_bounds_http_with_and_without_content_length() {
    for headers in ["Content-Length: 1024\r\n", ""] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            read_download_request_headers(&mut stream);
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n").as_bytes(),
            );
            let _ = stream.write_all(&[7u8; 1024]);
        });
        let hash = to_hex(&hashtree_core::sha256(&[7u8; 1024]));
        let error = download_attachment_blob(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            &hash,
            100,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("too large"), "{error}");
        server.join().unwrap();
    }
}

#[tokio::test]
async fn attachment_download_accepts_exact_limit_and_checks_hash() {
    for valid_hash in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            read_download_request_headers(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nclip")
                .unwrap();
        });
        let expected = if valid_hash { b"clip" } else { b"fake" };
        let hash = to_hex(&hashtree_core::sha256(expected));
        let result = download_attachment_blob(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            &hash,
            4,
        )
        .await;
        if valid_hash {
            assert_eq!(result.unwrap().unwrap(), b"clip");
        } else {
            assert!(result.unwrap_err().to_string().contains("hash mismatch"));
        }
        server.join().unwrap();
    }
}
