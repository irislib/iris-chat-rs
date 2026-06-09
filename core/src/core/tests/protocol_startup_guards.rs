#[test]
fn startup_sensitive_relay_paths_do_not_block_on_relay_status() {
    let protocol_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/protocol.rs"),
    )
    .expect("read protocol source");
    let publish_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/publishing.rs"),
    )
    .expect("read publishing source");

    for (source, fn_name, end_marker) in [
        (
            protocol_source.as_str(),
            "start_relay_status_watchers",
            "\n    pub(super) fn schedule_session_connect",
        ),
        (
            protocol_source.as_str(),
            "handle_protocol_subscription_liveness_check",
            "\n    pub(super) fn reconcile_protocol_subscriptions",
        ),
        (
            protocol_source.as_str(),
            "reconcile_protocol_subscriptions",
            "\n    pub(super) fn handle_protocol_subscription_reconcile_completed",
        ),
        (
            publish_source.as_str(),
            "retry_pending_relay_publishes",
            "\n    pub(super) fn handle_relay_publish_drain_finished",
        ),
    ] {
        let start = source
            .find(&format!("pub(super) fn {fn_name}"))
            .unwrap_or_else(|| panic!("missing {fn_name}"));
        let body = &source[start
            ..source[start..]
                .find(end_marker)
                .map(|offset| start + offset)
                .unwrap_or(source.len())];
        assert!(
            !body.contains("runtime.block_on")
                && !body.contains("refresh_relay_connection_status()"),
            "{fn_name} must use cached relay statuses instead of blocking the core thread"
        );
    }
}
