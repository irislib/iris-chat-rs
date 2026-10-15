#[test]
fn group_outer_event_source_count_excludes_unrelated_queued_recoveries() {
    let (mut engine, _, _) = queued_group_history(3);
    let author = Keys::generate();
    let mut snapshot = engine.group_manager.snapshot();
    snapshot.sender_keys[0].sender_event_pubkey = ndr_device(author.public_key());
    engine.group_manager = GroupEventManager::from_snapshot(snapshot).unwrap();
    for candidate in engine.pending_group_sender_key_messages.iter_mut() {
        candidate.sender_event_pubkey = ndr_device(author.public_key());
    }
    let current = engine.pending_group_sender_key_messages.remove(0);
    let outer = nostr_double_ratchet::wire::group_sender_key_message_event(
        &nostr_double_ratchet::GroupSenderKeyMessageEnvelope {
            group_id: "queued-group".into(),
            sender_event_pubkey: ndr_device(author.public_key()),
            signer_secret_key: author.secret_key().to_secret_bytes(),
            key_id: current.key_id,
            message_number: current.message_number,
            encrypted_header: current.encrypted_header,
            created_at: current.created_at,
            ciphertext: current.ciphertext,
        },
    )
    .unwrap();
    let result = engine.process_group_outer_event(&outer).unwrap();
    assert_eq!(
        result.source_event_count, 1,
        "only the actual outer event gets its source ID"
    );
    assert_eq!(
        result.events.len(),
        3,
        "same turn also recovers two queued messages"
    );
    let bodies = result
        .events
        .iter()
        .map(|event| match event {
            GroupIncomingEvent::Message(message) => message.body.as_slice(),
            _ => panic!("expected user message"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bodies,
        [
            b"history-0".as_slice(),
            b"history-1".as_slice(),
            b"history-2".as_slice()
        ]
    );
}
