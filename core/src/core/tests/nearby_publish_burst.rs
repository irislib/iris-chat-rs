#[test]
fn relay_publish_burst_does_not_replay_acknowledged_nearby_backlog() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("nearby-publish-burst", &owner, &device);
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    let endpoint = Arc::new(core.runtime.block_on(async {
        fips_core::FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap()
    }));
    let (tcp, _records) = DeviceSyncTcpSender::test_channel(4, 1024);
    core.install_device_sync_sender_for_test(endpoint.clone(), tcp, Vec::new());
    core.device_sync.as_mut().unwrap().nearby_enabled = true;
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_mailbag_enabled = true;
    // Keep server work in flight while new protocol responses are enqueued.
    core.logged_in.as_mut().unwrap().relay_urls =
        vec![RelayUrl::parse("ws://127.0.0.1:1").unwrap()];
    core.relay_transport_runtime.publish_drain_in_flight = true;
    core.relay_transport_runtime.publish_drain_started_at = Some(Instant::now());
    let event = EventBuilder::new(Kind::Custom(1060), "earlier control")
        .sign_with_keys(&device)
        .unwrap();
    assert!(core.publish_runtime_event(event.clone(), APPCORE_PROTOCOL_LABEL, None));
    let outbox = core.device_sync.as_ref().unwrap().nearby_outbox.clone();
    outbox
        .write()
        .unwrap()
        .acknowledge("test-peer", &event.id.to_hex());
    for index in 0..20 {
        let next = EventBuilder::new(Kind::Custom(1060), format!("new control {index}"))
            .sign_with_keys(&device)
            .unwrap();
        assert!(core.publish_runtime_event(next, APPCORE_PROTOCOL_LABEL, None));
    }
    let pending = outbox.read().unwrap().pending_for_link("test-peer", 1);
    assert_eq!(
        pending.len(),
        20,
        "new packets must still be sent immediately"
    );
    assert!(
        !pending.iter().any(|(id, _)| id == &event.id.to_hex()),
        "each new publication must not resurrect the entire nearby backlog"
    );
    assert_eq!(
        core.pending_relay_publishes.len(),
        21,
        "nearby delivery must preserve outstanding server persistence"
    );
    core.relay_transport_runtime.nearby_replay_started_at =
        Some(Instant::now() - Duration::from_secs(3));
    core.retry_pending_relay_publishes("scheduled_retry");
    assert_eq!(
        outbox
            .read()
            .unwrap()
            .pending_for_link("test-peer", 1)
            .len(),
        20,
        "server persistence retries must not resend to a peer that acknowledged the event"
    );
    assert_eq!(
        outbox
            .read()
            .unwrap()
            .pending_for_link("later-peer", 1)
            .len(),
        21,
        "server persistence retries must retain events for later peers"
    );
    core.runtime.block_on(endpoint.shutdown()).unwrap();
}
