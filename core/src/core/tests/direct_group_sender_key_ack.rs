#[test]
fn direct_group_runtime_message_ack_clears_sender_key_candidate() {
    let mut devices = sender_key_matrix_devices(2);
    let alice = 0;
    let bob = 1;
    let alice_owner = devices[alice].owner.public_key();
    let alice_device = devices[alice].device.public_key();
    let bob_owner_keys = devices[bob].owner.clone();
    let bob_device_keys = devices[bob].device.clone();

    let created = devices[alice]
        .engine
        .create_group(
            "direct group ack cleanup".to_string(),
            vec![bob_owner_keys.public_key()],
            UnixSeconds(1_777_159_480),
        )
        .expect("create group");
    let group = created.snapshot.clone().expect("created group");
    let group_id = group.group_id.clone();
    let chat_id = group_chat_id(&group_id);
    deliver_protocol_effects_to_engine(&mut devices[bob].engine, &created.effects);

    let sent = devices[alice]
        .engine
        .send_group_payload(
            &group_id,
            b"group body from sender key".to_vec(),
            Some("direct-group-ack-cleanup-inner".to_string()),
        )
        .expect("send group payload");
    let outer =
        sender_key_outer_events_for_engine(&devices[alice].engine, &sent.effects, &sent.event_ids)
            .into_iter()
            .next()
            .expect("sender-key outer")
            .clone();

    assert!(devices[bob]
        .engine
        .process_direct_message_event(&outer)
        .expect("direct probing queues sender-key candidate")
        .is_none());
    assert_eq!(
        devices[bob]
            .engine
            .debug_snapshot()
            .pending_group_sender_key_message_count,
        1
    );

    let bob_engine = std::mem::replace(
        &mut devices[bob].engine,
        test_protocol_engine(&bob_owner_keys, &bob_device_keys),
    );
    let mut core = logged_in_test_core(
        "direct-group-runtime-ack-clears-candidate",
        &bob_owner_keys,
        &bob_device_keys,
    );
    core.preferences.send_read_receipts = false;
    core.groups.insert(group_id.clone(), group);
    core.protocol_engine = Some(bob_engine);

    let (payload, rumor_id) = runtime_rumor_json(
        alice_owner,
        CHAT_MESSAGE_KIND,
        "visible via sibling copy",
        outer.created_at.as_secs(),
        vec![vec!["l".to_string(), group_id.clone()]],
    );
    core.apply_decrypted_runtime_message_with_metadata(
        alice_owner,
        Some(alice_device),
        None,
        payload,
        Some("direct-copy-event".to_string()),
    );

    let snapshot = core
        .protocol_engine
        .as_ref()
        .expect("protocol engine")
        .debug_snapshot();
    assert_eq!(
        snapshot.pending_group_sender_key_message_count, 0,
        "visible direct group copy should clear a matching sender-key candidate"
    );
    let thread = core.threads.get(&chat_id).unwrap_or_else(|| {
        panic!(
            "group thread missing; known threads={:?}",
            core.threads.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].id, rumor_id);
    assert_eq!(thread.messages[0].body, "visible via sibling copy");
}
