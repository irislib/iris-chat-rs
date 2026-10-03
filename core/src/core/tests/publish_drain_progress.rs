fn publish_drain_progress_fixture(count: usize) -> (AppCore, Vec<String>) {
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut core = logged_in_test_core("publish-drain-progress", &owner, &device);
    core.logged_in.as_mut().unwrap().relay_urls =
        vec![RelayUrl::parse("ws://127.0.0.1:1").unwrap()];
    let mut ids = Vec::new();
    for index in 0..count {
        let event = EventBuilder::new(Kind::TextNote, format!("queued {index}"))
            .sign_with_keys(&device)
            .unwrap();
        let event_id = event.id.to_hex();
        core.pending_relay_publishes.insert(
            event_id.clone(),
            PendingRelayPublish {
                owner_pubkey_hex: owner.public_key().to_hex(),
                event_id: event_id.clone(),
                label: "test".into(),
                event_json: event.as_json(),
                inner_event_id: None,
                chat_id: None,
                created_at_secs: event.created_at.as_secs(),
                attempt_count: 0,
                last_error: None,
            },
        );
        ids.push(event_id);
    }
    core.relay_transport_runtime.publish_drain_token = 7;
    core.relay_transport_runtime.publish_drain_in_flight = true;
    core.relay_transport_runtime.publish_drain_dirty = true;
    core.relay_transport_runtime.publish_drain_started_at = Some(Instant::now());
    (core, ids)
}

fn deliver_publish_drain_progress(core: &mut AppCore, event_id: &str, success: bool) {
    core.handle_internal(InternalEvent::RelayPublishDrainProgress {
        token: 7,
        result: RelayPublishDrainResult {
            event_id: event_id.into(),
            success,
            relay_urls: Vec::new(),
            detail: "test server result".into(),
        },
    });
}

#[test]
fn streamed_publish_failures_back_off_once_before_recovering_online() {
    // The production worker streams results, then finishes with an empty list.
    // A backlog larger than its 16-event batch keeps the coalesced flag set.
    let (mut core, ids) = publish_drain_progress_fixture(32);
    core.pending_relay_publish_inflight
        .extend(ids[..16].iter().cloned());
    for id in &ids[..16] {
        deliver_publish_drain_progress(&mut core, id, false);
    }
    core.handle_internal(InternalEvent::RelayPublishDrainFinished {
        token: 7,
        results: Vec::new(),
    });
    assert!(
        !core.relay_transport_runtime.publish_drain_in_flight,
        "failed streamed results must not immediately restart a full backlog"
    );
    assert_eq!(core.relay_transport_runtime.publish_drain_token, 7);
    assert_eq!(core.relay_transport_runtime.retry_backoff_attempt, 1);
    assert!(core.relay_transport_runtime.next_retry_due_at.unwrap() > Instant::now());
    assert_eq!(core.pending_relay_publishes.len(), 32);

    let relay = crate::local_relay::TestRelay::start();
    core.logged_in.as_mut().unwrap().relay_urls =
        vec![RelayUrl::parse(relay.url()).unwrap()];
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    core.retry_pending_relay_publishes("relay_transport_connected");
    pump_signer_core_until(&mut core, &rx, |core| {
        core.pending_relay_publishes.is_empty()
            && !core.relay_transport_runtime.publish_drain_in_flight
            && !core.relay_transport_runtime.connect_in_flight
    });
    assert_eq!(relay.events().len(), 32);
    assert_eq!(core.relay_transport_runtime.retry_backoff_attempt, 0);
}

#[test]
fn successful_streamed_result_does_not_hide_another_failed_result() {
    for success_first in [true, false] {
        let (mut core, ids) = publish_drain_progress_fixture(32);
        core.pending_relay_publish_inflight
            .extend(ids[..2].iter().cloned());
        deliver_publish_drain_progress(&mut core, &ids[0], success_first);
        deliver_publish_drain_progress(&mut core, &ids[1], !success_first);
        core.handle_internal(InternalEvent::RelayPublishDrainFinished {
            token: 7,
            results: Vec::new(),
        });
        assert!(!core.relay_transport_runtime.publish_drain_in_flight);
        assert_eq!(core.relay_transport_runtime.publish_drain_token, 7);
        assert_eq!(core.relay_transport_runtime.retry_backoff_attempt, 1);
        assert_eq!(core.pending_relay_publishes.len(), 31);
    }
}

#[test]
fn successful_streamed_drain_immediately_publishes_coalesced_new_work() {
    let relay = crate::local_relay::TestRelay::start();
    let (mut core, ids) = publish_drain_progress_fixture(2);
    core.logged_in.as_mut().unwrap().relay_urls =
        vec![RelayUrl::parse(relay.url()).unwrap()];
    core.pending_relay_publish_inflight.extend(ids.iter().cloned());
    core.relay_transport_runtime.publish_drain_dirty = false;
    let (tx, rx) = flume::unbounded();
    core.core_sender = tx.clone();
    core.priority_sender = tx;
    let new_event = EventBuilder::new(Kind::TextNote, "new work during the drain")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    assert!(core.publish_runtime_event(new_event.clone(), "test", None));
    assert!(core.relay_transport_runtime.publish_drain_dirty);
    for id in ids {
        deliver_publish_drain_progress(&mut core, &id, true);
    }
    core.handle_internal(InternalEvent::RelayPublishDrainFinished {
        token: 7,
        results: Vec::new(),
    });
    assert!(core.relay_transport_runtime.publish_drain_in_flight);
    assert_eq!(core.relay_transport_runtime.publish_drain_token, 8);
    pump_signer_core_until(&mut core, &rx, |core| {
        core.pending_relay_publishes.is_empty()
            && !core.relay_transport_runtime.publish_drain_in_flight
            && !core.relay_transport_runtime.connect_in_flight
    });
    assert!(relay.events().iter().any(|event| event["id"] == new_event.id.to_hex()));
}
