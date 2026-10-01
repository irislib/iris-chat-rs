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
