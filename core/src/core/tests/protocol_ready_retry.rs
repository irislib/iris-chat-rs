fn ready_group_retry_core() -> (
    AppCore,
    flume::Receiver<CoreMsg>,
    flume::Receiver<CoreMsg>,
    Arc<SwitchableFailStorage>,
    String,
) {
    let storage = Arc::new(SwitchableFailStorage::new());
    let mut devices = sender_key_matrix_devices(2);
    devices[1].engine =
        test_protocol_engine_with_storage(&devices[1].owner, &devices[1].device, storage.clone());
    observe_sender_key_matrix_protocol_state(&mut devices);
    let recipient = devices[1].owner.public_key();
    let created = devices[0]
        .engine
        .create_group(
            "ready retry continuation".into(),
            vec![recipient],
            unix_now(),
        )
        .unwrap();
    let group_id = created.snapshot.unwrap().group_id;
    for index in 0..20 {
        let (rumor, id) = runtime_rumor_json(
            devices[0].owner.public_key(),
            CHAT_MESSAGE_KIND,
            &format!("ready message {index}"),
            unix_now().get(),
            vec![],
        );
        let sent = devices[0]
            .engine
            .send_group_payload(&group_id, rumor.into_bytes(), Some(id))
            .unwrap();
        let outer =
            sender_key_outer_events_for_engine(&devices[0].engine, &sent.effects, &sent.event_ids)
                [0]
            .clone();
        let waiting = devices[1].engine.process_group_outer_event(&outer).unwrap();
        assert!(waiting.consumed && waiting.events.is_empty());
    }
    assert_eq!(
        devices[1]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        20
    );
    devices[1].engine.enter_batch();
    let initial = deliver_protocol_effects_to_engine(&mut devices[1].engine, &created.effects);
    devices[1].engine.exit_batch().unwrap();
    let remaining = devices[1]
        .engine
        .debug_snapshot()
        .pending_group_sender_key_message_count;
    assert!(
        remaining > 0,
        "late metadata must leave a bounded ready remainder: {remaining}"
    );
    let receiver = devices.pop().unwrap();
    let mut core = logged_in_test_core_with_storage(
        "ready-group-retry",
        &receiver.owner,
        &receiver.device,
        storage.clone(),
    );
    core.preferences.send_read_receipts = false;
    core.protocol_engine = Some(receiver.engine);
    for event in initial {
        core.apply_group_decrypted_event(event);
    }
    let (background_tx, background_rx) = flume::unbounded();
    let (priority_tx, priority_rx) = flume::unbounded();
    core.core_sender = background_tx;
    core.priority_sender = priority_tx;
    (
        core,
        background_rx,
        priority_rx,
        storage,
        group_chat_id(&group_id),
    )
}

#[test]
fn protocol_ready_retry_continues_decryption_on_background_queue_without_network_wait() {
    let (mut core, background, priority, _, chat_id) = ready_group_retry_core();
    core.schedule_fast_protocol_retry_if_pending();
    let continuation = background
        .recv_timeout(Duration::from_millis(500))
        .expect("ready decryptions should resume on the background queue within one second");
    assert!(
        priority.is_empty(),
        "ready work must not bypass foreground queue fairness"
    );
    core.handle_messages(vec![continuation]);
    for _ in 0..20 {
        if core
            .protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count
            == 0
        {
            break;
        }
        core.handle_messages(vec![background
            .recv_timeout(Duration::from_millis(500))
            .unwrap()]);
    }
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        0
    );
    assert_eq!(
        core.threads[&chat_id]
            .messages
            .iter()
            .filter(|message| message.body.starts_with("ready message "))
            .count(),
        20
    );
    assert!(core
        .protocol_subscription_runtime
        .ready_retry_due_at
        .is_none());
    assert!(!core
        .protocol_engine
        .as_ref()
        .unwrap()
        .has_ready_group_sender_key_retry_work());
}

#[test]
fn protocol_ready_retry_coalesces_and_ignores_callbacks_after_runtime_reset() {
    let (mut core, _, _, _, _) = ready_group_retry_core();
    core.schedule_fast_protocol_retry_if_pending();
    let due_at = core
        .protocol_subscription_runtime
        .ready_retry_due_at
        .unwrap();
    assert!(due_at <= Instant::now() + Duration::from_secs(1));
    for _ in 0..10 {
        core.schedule_fast_protocol_retry_if_pending();
    }
    assert_eq!(
        core.protocol_subscription_runtime.ready_retry_due_at,
        Some(due_at)
    );
    assert!(core.logged_in.as_ref().unwrap().relay_urls.is_empty());
    core.request_protocol_subscription_refresh();
    assert_eq!(
        core.protocol_subscription_runtime.ready_retry_due_at,
        Some(due_at),
        "no-relay refresh must preserve the coalesced continuation"
    );

    let pending = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .debug_snapshot()
        .pending_group_sender_key_message_count;
    core.protocol_subscription_runtime = ProtocolSubscriptionRuntime::default();
    core.handle_internal(InternalEvent::RetryReadyProtocolWork { due_at });
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        pending
    );
    assert!(core
        .protocol_subscription_runtime
        .ready_retry_due_at
        .is_none());
    core.schedule_fast_protocol_retry_if_pending();
    let replacement = core
        .protocol_subscription_runtime
        .ready_retry_due_at
        .unwrap();
    assert_ne!(due_at, replacement);
    core.handle_internal(InternalEvent::RetryReadyProtocolWork { due_at });
    assert_eq!(
        core.protocol_subscription_runtime.ready_retry_due_at,
        Some(replacement)
    );
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        pending
    );
}

#[test]
fn protocol_ready_retry_storage_failure_keeps_slow_backoff_and_recovers() {
    let (mut core, _, _, storage, _) = ready_group_retry_core();
    core.schedule_fast_protocol_retry_if_pending();
    let cancelled = core
        .protocol_subscription_runtime
        .ready_retry_due_at
        .unwrap();
    let pending = core
        .protocol_engine
        .as_ref()
        .unwrap()
        .debug_snapshot()
        .pending_group_sender_key_message_count;
    storage.set_fail_puts(true);
    core.retry_protocol_engine_pending_work("ready_retry_failure");
    assert!(core
        .debug_log
        .iter()
        .any(|entry| entry.category == "appcore.protocol.retry.error"));
    assert!(
        core.protocol_subscription_runtime
            .ready_retry_due_at
            .is_none(),
        "failed storage must not create a 25 ms retry loop"
    );
    assert!(
        core.protocol_subscription_runtime.liveness_due_at.unwrap()
            > Instant::now() + Duration::from_secs(1)
    );
    core.handle_internal(InternalEvent::RetryReadyProtocolWork { due_at: cancelled });
    assert_eq!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        pending
    );
    storage.set_fail_puts(false);
    core.retry_protocol_engine_pending_work("ready_retry_recovery");
    assert!(
        core.protocol_engine
            .as_ref()
            .unwrap()
            .debug_snapshot()
            .pending_group_sender_key_message_count
            < pending
    );
}
