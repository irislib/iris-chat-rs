fn pending_publish_changes(core: &AppCore) -> u64 {
    core.app_store
        .shared()
        .lock()
        .unwrap()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn pending_publish_replays_do_not_rewrite_offline_outbox() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("pending-publish-replay", &owner, &device);
    // A populated account can return the same signed bootstrap/control effects
    // from many protocol retry passes while its message servers remain offline.
    let effects = (0..32)
        .map(|index| {
            ProtocolEffect::Publish(ProtocolPublish {
                event: EventBuilder::new(
                    Kind::from(MESSAGE_EVENT_KIND as u16),
                    format!("pending control {index} {}", "x".repeat(4096)),
                )
                .sign_with_keys(&device)
                .unwrap(),
                chat_id: format!("pending-chat-{index}"),
                inner_event_id: None,
            })
        })
        .collect::<Vec<_>>();
    core.process_protocol_engine_effects(effects.clone());
    assert_eq!(core.pending_relay_publishes.len(), 32);
    let original = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    assert!(original.iter().all(|item| item.attempt_count == 1));
    let writes = pending_publish_changes(&core);
    let _ = updates.try_iter().count();

    for _ in 0..12 {
        core.process_protocol_engine_effects(effects.clone());
    }

    assert_eq!(
        pending_publish_changes(&core),
        writes,
        "already-durable retry effects must not rewrite the outbox or failure state"
    );
    assert_eq!(
        serde_json::to_value(
            core.app_store
                .load_pending_relay_publishes(&owner.public_key().to_hex())
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert!(
        !updates
            .try_iter()
            .any(|update| matches!(update, AppUpdate::NearbyPublishedEvent { .. })),
        "duplicate effects must use the existing paced nearby retry path"
    );
}

#[test]
fn pending_publish_replay_preserves_attempts_and_enriches_metadata() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, _, _dir) =
        logged_in_test_core_with_updates("pending-publish-metadata", &owner, &device);
    let event = EventBuilder::new(Kind::from(MESSAGE_EVENT_KIND as u16), "queued")
        .sign_with_keys(&device)
        .unwrap();
    let id = event.id.to_hex();
    core.publish_runtime_event(event.clone(), APPCORE_PROTOCOL_LABEL, None);
    let mut pending = core.pending_relay_publishes[&id].clone();
    pending.attempt_count = 7;
    pending.last_error = Some("publish attempt in progress".into());
    core.app_store
        .upsert_pending_relay_publish(&pending)
        .unwrap();
    core.pending_relay_publishes
        .insert(id.clone(), pending.clone());
    core.pending_relay_publish_inflight.insert(id.clone());
    core.relay_transport_runtime.publish_drain_in_flight = true;
    core.relay_transport_runtime.publish_drain_started_at = Some(Instant::now());
    core.logged_in.as_mut().unwrap().relay_urls =
        relay_urls_from_strings(&["ws://127.0.0.1:1".into()]);

    let chat = Keys::generate().public_key().to_hex();
    core.push_outgoing_message_with_id(
        "inner".into(),
        &chat,
        "body".into(),
        unix_now().get(),
        None,
        DeliveryState::Queued,
    );
    assert!(core.publish_protocol_event(ProtocolPublish {
        event: event.clone(),
        chat_id: chat.clone(),
        inner_event_id: Some("inner".into()),
    }));
    let enriched = &core.pending_relay_publishes[&id];
    assert_eq!(enriched.attempt_count, 7);
    assert_eq!(enriched.last_error, pending.last_error);
    assert_eq!(enriched.event_json, pending.event_json);
    assert_eq!(enriched.chat_id.as_deref(), Some(chat.as_str()));
    assert_eq!(enriched.inner_event_id.as_deref(), Some("inner"));
    assert!(core.pending_relay_publish_inflight.contains(&id));
    assert!(core.threads[&chat].messages[0]
        .delivery_trace
        .outer_event_ids
        .contains(&id));
    let writes = pending_publish_changes(&core);
    assert!(core.publish_runtime_event(event, APPCORE_PROTOCOL_LABEL, None));
    assert_eq!(pending_publish_changes(&core), writes);
    assert_eq!(
        core.pending_relay_publishes[&id].inner_event_id.as_deref(),
        Some("inner")
    );
    let restored = core
        .app_store
        .load_pending_relay_publishes(&owner.public_key().to_hex())
        .unwrap();
    assert_eq!(restored[0].attempt_count, 7);
    assert_eq!(restored[0].inner_event_id.as_deref(), Some("inner"));
}

#[test]
fn pending_identity_publication_advertises_without_resetting_durable_retries() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let (mut core, updates, _dir) =
        logged_in_test_core_with_updates("pending-identity-republish", &owner, &device);
    let now = unix_now().get();
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)]),
            now,
        ),
    );
    core.publish_local_identity_artifacts();
    let identities = core.build_local_identity_artifacts().1;
    assert_eq!(identities.len(), 2, "roster and local invitation");
    for (_, event) in &identities {
        let id = event.id.to_hex();
        let pending = core.pending_relay_publishes.get_mut(&id).unwrap();
        pending.attempt_count = 7;
        pending.last_error = Some("publish attempt in progress".into());
        core.app_store.upsert_pending_relay_publish(pending).unwrap();
        core.pending_relay_publish_inflight.insert(id);
    }
    core.relay_transport_runtime.publish_drain_in_flight = true;
    let started = Instant::now();
    core.relay_transport_runtime.publish_drain_started_at = Some(started);
    core.relay_transport_runtime.nearby_replay_started_at = Some(started);
    let original = core.pending_relay_publishes.clone();
    let inflight = core.pending_relay_publish_inflight.clone();
    let writes = pending_publish_changes(&core);
    drain_app_updates(&updates);

    // Startup must advertise the exact durable identity to newly attached
    // nearby transports even when the relay outbox already contains it.
    core.publish_local_identity_artifacts();
    let advertised = updates
        .try_iter()
        .filter_map(|update| match update {
            AppUpdate::NearbyPublishedEvent { event_json, .. } => {
                Some(serde_json::from_str::<Event>(&event_json).unwrap().id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(advertised.len(), identities.len());
    for (_, event) in &identities {
        assert!(advertised.contains(&event.id));
    }
    assert_eq!(pending_publish_changes(&core), writes);
    assert_eq!(core.pending_relay_publishes, original);
    assert_eq!(core.pending_relay_publish_inflight, inflight);
    assert!(core.relay_transport_runtime.publish_drain_in_flight);
    assert_eq!(core.relay_transport_runtime.publish_drain_started_at, Some(started));
    assert_eq!(core.relay_transport_runtime.nearby_replay_started_at, Some(started));

    // Routine replay of those same events must retain the CPU/write fix.
    for _ in 0..12 {
        for (label, event) in &identities {
            assert!(core.publish_runtime_event(event.clone(), label, None));
        }
    }
    assert_eq!(pending_publish_changes(&core), writes);
    assert_eq!(core.pending_relay_publishes, original);
    assert!(!updates.try_iter().any(|update| matches!(
        update,
        AppUpdate::NearbyPublishedEvent { .. }
    )));
}
