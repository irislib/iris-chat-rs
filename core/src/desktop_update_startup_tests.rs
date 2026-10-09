use super::*;

/// Swift's cooperative executor has a smaller stack than a Rust test worker.
/// Keep the real FFI/bootstrap path in a child so an overflow is a test failure,
/// rather than aborting the rest of the Rust test suite.
#[test]
fn secure_update_check_survives_swift_sized_stack() {
    const CHILD: &str = "IRIS_TEST_UPDATE_SMALL_STACK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let result = std::thread::Builder::new()
            .name("swift-sized-update-caller".into())
            .stack_size(512 * 1024)
            .spawn(iris_desktop_update_check)
            .unwrap()
            .join()
            .unwrap();
        // An offline bootstrap must fail normally, without changing the
        // authenticity or availability requirements for an update.
        assert!(!result.ok);
        assert!(result
            .error
            .unwrap()
            .contains("failed to resolve signed release"));
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop_update::startup_tests::secure_update_check_survives_swift_sized_stack",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env_remove("IRIS_UPDATE_MANIFEST_URL")
        .env(
            "IRIS_UPDATE_HTREE_REF",
            crate::update_announcements::HTREE_UPDATE_REF,
        )
        .env("IRIS_UPDATE_RELAYS", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .env("IRIS_UPDATE_BLOSSOM_SERVERS", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "small-stack update child failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn verified_download_survives_discovery_outage() {
    const CHILD: &str = "IRIS_TEST_CACHED_UPDATE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "desktop_update::startup_tests::verified_download_survives_discovery_outage",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("IRIS_UPDATE_MANIFEST_URL")
            .env_remove("IRIS_UPDATE_HTREE_REF")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use hashtree_core::{DirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore, Store};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let store = Arc::new(MemoryStore::new());
        let tree = HashTree::new(HashTreeConfig::new(store.clone()).public());
        let bytes = b"authenticated app archive fixture";
        let (file, size) = tree.put_file(bytes).await.unwrap();
        let root = tree
            .put_directory(vec![DirEntry::from_cid("app", &file)
                .with_size(size)
                .with_link_type(LinkType::File)])
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::env::set_var(
            "IRIS_UPDATE_BLOSSOM_SERVERS",
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let served_root = root.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let cid = if request.contains(&hashtree_core::to_hex(&served_root.hash)) {
                    &served_root
                } else {
                    &file
                };
                let body = store.get(&cid.hash).await.unwrap().unwrap();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let provider = Arc::new(nostr_pubsub::InMemoryEventBus::new());
        let resolver =
            AvailableUpdateResolver::new(vec![provider], Duration::from_millis(1)).unwrap();
        let check = UpdateCheck {
            root_cid: root.clone(),
            release_cid: root,
            manifest_path: "release.json".into(),
            manifest: UpdateManifest {
                version: "2099.1.1".into(),
                tag: Some("v2099.1.1".into()),
                assets: vec![UpdateAsset {
                    name: "iris-chat-v2099.1.1-macos-arm64.app.tar.gz".into(),
                    path: "app".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            asset: None,
            update_available: true,
        };
        *VERIFIED_APP_UPDATE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some((secure_update_ref().unwrap(), resolver, check));
        let directory = tempfile::tempdir().unwrap();
        let result = run_secure_update_async(UpdateOperation::Download {
            download_dir: Some(directory.path().into()),
        })
        .await
        .unwrap();
        assert!(result.verified);
        assert_eq!(fs::read(result.path.unwrap()).unwrap(), bytes);
        // A different authority must never reuse the previous selection.
        let other = nostr::Keys::generate();
        use nostr::ToBech32;
        std::env::set_var(
            "IRIS_UPDATE_HTREE_REF",
            format!("htree://{}/other", other.public_key().to_bech32().unwrap()),
        );
        std::env::set_var("IRIS_UPDATE_RELAYS", "");
        std::env::set_var("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
        let result = run_secure_update_async(UpdateOperation::Download {
            download_dir: Some(directory.path().into()),
        })
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to resolve signed release"));
        server.abort();
    });
}
