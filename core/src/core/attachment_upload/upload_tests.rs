use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

const IRIS_LOGO_PNG: &[u8] =
    include_bytes!("../../../../android/app/src/main/res/drawable-nodpi/iris_logo.png");
const IRIS_LOGO_SVG: &[u8] = include_bytes!("../../../../assets/iris-chat-logo.svg");

fn serve_one_blossom_upload(status: &str) -> (String, thread::JoinHandle<Vec<u8>>) {
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
    let store = UploadingBlossomStore::new(nostr::Keys::generate(), vec![], vec![server_url], None);

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
    let store = UploadingBlossomStore::new(nostr::Keys::generate(), vec![], vec![server_url], None);

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
    );

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
