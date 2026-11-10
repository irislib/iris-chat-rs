use super::*;

#[test]
fn notification_preview_does_not_initialize_missing_state() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let storage = Arc::new(InMemoryStorage::new());
    assert!(ProtocolEngine::load_for_notification_preview(
        storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap()
    .is_none());
    assert!(storage.list("").unwrap().is_empty());
}

#[test]
fn notification_preview_omits_outgoing_backlog_without_rewriting_or_consuming_state() {
    use nostr::JsonUtil;
    use std::sync::atomic::Ordering::Relaxed;

    let mut fixture = checkpoint_backlog_fixture(512, 4);
    fixture.sender.persist().unwrap();
    let before = fixture
        .store
        .get(PROTOCOL_ENGINE_STATE_KEY)
        .unwrap()
        .unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: fixture.store.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    let owner = fixture.owner.public_key();
    let send = fixture
        .ready
        .send_direct_text(
            owner,
            &owner.to_hex(),
            "unread notification after backgrounding",
            None,
            UnixSeconds(30),
        )
        .unwrap();
    let event = send
        .effects
        .into_iter()
        .find_map(|effect| match effect {
            ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() == 1060 => {
                Some(publish.event)
            }
            _ => None,
        })
        .expect("private message envelope");
    assert_ne!(event.pubkey, fixture.peer_owner.public_key());
    assert_ne!(event.pubkey, fixture.ready_device.public_key());
    assert!(!event
        .as_json()
        .contains(&fixture.peer_owner.public_key().to_hex()));

    // Repeat from the same saved generation: a preview must not consume the
    // ciphertext or copy/serialize the unrelated outgoing work on either pass.
    for _ in 0..2 {
        let mut preview =
            ProtocolEngine::load_for_notification_preview(storage.clone(), owner, &fixture.device)
                .unwrap()
                .unwrap();
        assert!(preview.pending_group_fanouts.is_empty());
        assert!(preview.ready_group_publications.is_empty());
        assert!(preview.pending_local_sibling_sends.is_empty());
        assert!(preview.pending_remote_sends.is_empty());
        assert_eq!(
            preview.pending_group_sender_key_messages.len(),
            4,
            "authenticated incoming recovery remains available"
        );
        let received = preview
            .process_direct_message_event(&event)
            .unwrap()
            .unwrap();
        assert_eq!(received.sender, fixture.peer_owner.public_key());
        assert!(received
            .content
            .contains("unread notification after backgrounding"));
        assert_eq!(
            storage.puts.load(Relaxed),
            0,
            "neither startup nor decryption may serialize a checkpoint"
        );
        assert_eq!(
            fixture
                .store
                .get(PROTOCOL_ENGINE_STATE_KEY)
                .unwrap()
                .unwrap(),
            before
        );
    }
    let mut restarted =
        ProtocolEngine::load_or_create_for_local_device(fixture.store, owner, &fixture.device)
            .unwrap();
    assert_eq!(
        restarted.pending_group_fanouts.len(),
        512,
        "the normal app must retain every pending send"
    );
    assert!(restarted
        .process_direct_message_event(&event)
        .unwrap()
        .is_some());
}
