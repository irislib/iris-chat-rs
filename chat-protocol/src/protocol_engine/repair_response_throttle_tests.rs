#[test]
fn refreshed_sender_key_repair_requests_share_response_backoff() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let requester = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let group = engine
        .create_group(
            "Repair burst".to_string(),
            vec![requester.public_key()],
            UnixSeconds(100),
        )
        .unwrap()
        .snapshot
        .unwrap();
    engine.pending_group_fanouts.clear();
    let mut request = SenderKeyRepairRequest {
        group_id: group.group_id,
        sender_event_pubkey: ndr_device(Keys::generate().public_key()),
        key_id: None,
        message_number: None,
        required_revision: Some(group.revision),
        created_at: NdrUnixSeconds(101),
    };
    let requester_owner = ndr_owner(requester.public_key());
    engine
        .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(102))
        .unwrap();
    let pending = engine.pending_group_fanouts.clone();
    let answered = engine.answered_group_sender_key_repairs.clone();
    for now in 103..112 {
        request.created_at = NdrUnixSeconds(now);
        let effects = engine
            .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(now))
            .unwrap();
        assert!(effects.is_empty());
        assert_eq!(engine.pending_group_fanouts, pending);
        assert_eq!(
            engine.answered_group_sender_key_repairs, answered,
            "refreshing a request timestamp must not bypass the response cooldown"
        );
    }
    request.created_at = NdrUnixSeconds(112);
    engine
        .sender_key_repair_response_effects(requester_owner, &request, NdrUnixSeconds(112))
        .unwrap();
    assert_eq!(engine.answered_group_sender_key_repairs.len(), 1);
    let answered = &engine.answered_group_sender_key_repairs[0];
    assert_eq!(answered.last_responded_at_secs, 112);
    assert_eq!(answered.response_count, 2);
    assert_eq!(answered.next_response_at_secs, 142);

    // A different missing key/revision is independent work, not the same retry.
    request.required_revision = Some(group.revision + 1);
    assert!(!engine.group_sender_key_repair_response_throttled(
        requester_owner,
        &request,
        NdrUnixSeconds(113)
    ));
}
