use super::*;

fn fixture() -> (AppCore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        dir.path().to_string_lossy().into(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(Keys::generate(), Keys::generate(), false, false)
        .unwrap();
    (core, dir)
}

fn receive(core: &mut AppCore, source: &Keys, data: Vec<u8>) {
    core.handle_internal(InternalEvent::FipsNearbyPacket {
        source_pubkey_hex: source.public_key().to_hex(),
        source_port: FIPS_NEARBY_PORT,
        data,
    });
}

fn packet(event_id: &str, event_json: &str, version: u8) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "type": "event", "v": version, "eventId": event_id, "eventJson": event_json
    }))
    .unwrap()
}

#[test]
fn real_internal_packet_path_reports_only_bounded_receiver_outcomes() {
    let (mut core, _dir) = fixture();
    let source = Keys::generate();
    let secret_text = "private payload must never appear in diagnostic logs";
    let event = EventBuilder::new(Kind::TextNote, secret_text)
        .sign_with_keys(&source)
        .unwrap();
    let json = serde_json::to_string(&event).unwrap();
    receive(&mut core, &source, packet(&event.id.to_hex(), &json, 1));
    assert!(core.has_seen_event(&event.id.to_hex()));
    receive(&mut core, &source, secret_text.as_bytes().to_vec());
    receive(&mut core, &source, packet(&event.id.to_hex(), &json, 2));
    receive(&mut core, &source, packet("not-an-id", &json, 1));
    receive(
        &mut core,
        &source,
        packet(&event.id.to_hex(), secret_text, 1),
    );
    receive(&mut core, &source, packet(&"ab".repeat(32), &json, 1));

    let mut forged = event.clone();
    forged.content.push_str("altered");
    receive(
        &mut core,
        &source,
        encode_fips_nearby_event(&forged).unwrap(),
    );
    let mut forged = event.clone();
    forged.sig = EventBuilder::new(Kind::TextNote, "another event")
        .sign_with_keys(&Keys::generate())
        .unwrap()
        .sig;
    receive(
        &mut core,
        &source,
        encode_fips_nearby_event(&forged).unwrap(),
    );

    let elsewhere = EventBuilder::new(Kind::Custom(MESSAGE_EVENT_KIND as u16), secret_text)
        .tags([
            nostr::Tag::public_key(source.public_key()),
            nostr::Tag::parse(["v", "2"]).unwrap(),
        ])
        .sign_with_keys(&source)
        .unwrap();
    receive(
        &mut core,
        &source,
        encode_fips_nearby_event(&elsewhere).unwrap(),
    );
    assert!(!core.has_seen_event(&elsewhere.id.to_hex()));
    for outcome in [
        "accepted",
        "packet_json",
        "packet_version",
        "packet_id",
        "event_json",
        "event_id",
        "event_hash",
        "signature",
        "recipient",
    ] {
        assert_eq!(
            core.fips_nearby_receive_diagnostics.outcomes.get(outcome),
            Some(&1),
            "{outcome}"
        );
    }
    assert_eq!(core.fips_nearby_receive_diagnostics.arrivals, 9);
    let first_flush = core.fips_nearby_receive_diagnostics.last_flush_scheduled;
    assert!(first_flush.is_some());
    core.handle_internal(InternalEvent::DebugLog {
        category: "fips_nearby.rx.flush".into(),
        detail: String::new(),
    });
    let snapshot = core.build_runtime_debug_snapshot();
    let logs = snapshot
        .recent_log
        .iter()
        .filter(|entry| entry.category.starts_with("fips_nearby.rx."))
        .map(|entry| entry.detail.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(logs.contains("packets=9"));
    for private in [
        secret_text,
        &source.public_key().to_hex(),
        &event.id.to_hex(),
        &source.secret_key().to_secret_hex(),
    ] {
        assert!(!logs.contains(private));
    }
    assert_eq!(
        core.fips_nearby_receive_diagnostics.last_flush_scheduled,
        first_flush
    );
}

#[test]
fn listener_and_carrier_refresh_once_without_idle_rescheduling() {
    let (mut core, _dir) = fixture();
    core.handle_internal(InternalEvent::DebugLog {
        category: "fips_nearby.rx.listener".into(),
        detail: "registered=true snapshot=true perf=true".into(),
    });
    let first_flush = core.fips_nearby_receive_diagnostics.last_flush_scheduled;
    assert!(first_flush.is_some());
    core.handle_internal(InternalEvent::DebugLog {
        category: "fips_nearby.tx.carrier".into(),
        detail: "discarded_outputs=0".into(),
    });
    assert_eq!(
        core.fips_nearby_receive_diagnostics.last_flush_scheduled,
        first_flush
    );
    core.handle_internal(InternalEvent::DebugLog {
        category: "fips_nearby.rx.flush".into(),
        detail: "debug snapshot refresh".into(),
    });
    assert_eq!(
        core.fips_nearby_receive_diagnostics.last_flush_scheduled,
        first_flush
    );
    assert!(core.debug_log.iter().any(|entry| {
        entry.category == "fips_nearby.rx.flush" && entry.detail.contains("packets=0")
    }));
}

#[test]
fn suspended_packet_counts_stay_in_memory_without_a_snapshot_write_or_flush() {
    let (mut core, _dir) = fixture();
    let builds = core.debug_snapshot_build_count();
    core.suspended = true;
    core.fips_nearby_receive_diagnostics.last_flush_scheduled = None;
    receive(
        &mut core,
        &Keys::generate(),
        b"private undecoded body".to_vec(),
    );
    assert_eq!(core.fips_nearby_receive_diagnostics.arrivals, 1);
    assert_eq!(core.fips_nearby_receive_diagnostics.deferred, 1);
    assert!(core.fips_nearby_receive_diagnostics.outcomes.is_empty());
    assert!(core
        .fips_nearby_receive_diagnostics
        .last_flush_scheduled
        .is_none());
    assert_eq!(core.debug_snapshot_build_count(), builds);
    assert!(core
        .debug_log
        .iter()
        .any(|entry| entry.category == "fips_nearby.rx.core"
            && entry.detail.contains("deferred=true")));
    assert!(!core
        .debug_log
        .iter()
        .any(|entry| entry.detail.contains("private undecoded body")));
}
