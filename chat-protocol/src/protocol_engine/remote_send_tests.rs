struct RemoteSendFixture {
    store: Arc<InMemoryStorage>,
    owner: Keys,
    device: Keys,
    peer_owner: Keys,
    ready_device: Keys,
    late_device: Keys,
    sender: ProtocolEngine,
    ready: ProtocolEngine,
    late: ProtocolEngine,
}

fn remote_send_fixture() -> RemoteSendFixture {
    let owner = Keys::generate();
    let device = Keys::generate();
    let peer_owner = Keys::generate();
    let ready_device = Keys::generate();
    let late_device = Keys::generate();
    let store = Arc::new(InMemoryStorage::new());
    let mut sender =
        ProtocolEngine::load_or_create_for_local_device(store.clone(), owner.public_key(), &device)
            .unwrap();
    let mut ready = test_engine(&peer_owner, &ready_device);
    let mut late = test_engine(&peer_owner, &late_device);
    let own = signed_app_keys(&owner, &[device.public_key()], 1);
    let peer = signed_app_keys(
        &peer_owner,
        &[ready_device.public_key(), late_device.public_key()],
        1,
    );
    for engine in [&mut sender, &mut ready, &mut late] {
        engine.ingest_app_keys_event(&own).unwrap();
        engine.ingest_app_keys_event(&peer).unwrap();
    }
    observe_sibling_invite(&mut sender, &ready, &ready_device);
    RemoteSendFixture {
        store,
        owner,
        device,
        peer_owner,
        ready_device,
        late_device,
        sender,
        ready,
        late,
    }
}

#[test]
fn remote_direct_readiness_needs_one_reachable_authorized_device() {
    let f = remote_send_fixture();
    assert_eq!(
        f.sender.direct_send_readiness(f.peer_owner.public_key()),
        DirectSendReadiness::Ready,
        "one undiscovered device must not prevent delivery to the contact's ready device"
    );
}

#[test]
fn remote_direct_partial_delivery_recovers_after_restart_without_resending_ready_device() {
    for peer_only in [false, true] {
        let mut f = remote_send_fixture();
        let peer = f.peer_owner.public_key();
        let rumor = own_seen_rumor(&f.owner);
        let sent = if peer_only {
            f.sender.send_direct_unsigned_event_to_peer_only(
                peer,
                &peer.to_hex(),
                rumor,
                UnixSeconds(10),
            )
        } else {
            f.sender
                .send_direct_unsigned_event(peer, &peer.to_hex(), rumor, UnixSeconds(10))
        }
        .expect("ready devices receive immediately while missing devices remain queued");
        assert_eq!(
            decrypt_own_sync_effects(&mut f.ready, sent.effects).len(),
            1
        );
        assert!(f.sender.has_pending_retry_work());
        assert!(f
            .sender
            .retry_pending_protocol(NdrUnixSeconds(20))
            .unwrap()
            .effects
            .is_empty());
        f.sender = ProtocolEngine::load_or_create_for_local_device(
            f.store,
            f.owner.public_key(),
            &f.device,
        )
        .unwrap();
        let retry = observe_sibling_invite(&mut f.sender, &f.late, &f.late_device);
        assert_eq!(retry.effects.iter().filter(|effect| matches!(effect, ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND)).count(), 1,
            "a completed recipient must not be encrypted and sent again");
        assert_eq!(
            decrypt_own_sync_effects(&mut f.late, retry.effects).len(),
            1
        );
        assert!(!f.sender.has_pending_retry_work());
    }
}

#[test]
fn remote_direct_pending_history_excludes_new_devices_and_prunes_revoked_devices() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "only originally authorized devices",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let added_device = Keys::generate();
    let added = test_engine(&f.peer_owner, &added_device);
    let expanded = signed_app_keys(
        &f.peer_owner,
        &[
            f.ready_device.public_key(),
            f.late_device.public_key(),
            added_device.public_key(),
        ],
        30,
    );
    assert!(f
        .sender
        .ingest_app_keys_event(&expanded)
        .unwrap()
        .effects
        .is_empty());
    assert!(
        observe_sibling_invite(&mut f.sender, &added, &added_device)
            .effects
            .is_empty(),
        "new device must not receive old pending content"
    );
    let revoked = signed_app_keys(
        &f.peer_owner,
        &[f.ready_device.public_key(), added_device.public_key()],
        40,
    );
    assert!(f
        .sender
        .ingest_app_keys_event(&revoked)
        .unwrap()
        .effects
        .is_empty());
    assert!(
        !f.sender.has_pending_retry_work(),
        "revoked pending target must be discarded"
    );
    let restored = signed_app_keys(
        &f.peer_owner,
        &[
            f.ready_device.public_key(),
            f.late_device.public_key(),
            added_device.public_key(),
        ],
        50,
    );
    f.sender.ingest_app_keys_event(&restored).unwrap();
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty(),
        "reauthorizing a device must not resurrect discarded history"
    );
}

#[test]
fn remote_direct_pending_payload_is_discarded_when_expired() {
    let mut f = remote_send_fixture();
    let now = unix_now().get();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "temporary",
            Some(now + 30),
            UnixSeconds(now),
        )
        .unwrap();
    assert!(f.sender.has_pending_retry_work());
    assert!(f
        .sender
        .retry_pending_protocol(NdrUnixSeconds(now + 31))
        .unwrap()
        .effects
        .is_empty());
    assert!(!f.sender.has_pending_retry_work());
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty()
    );
}

#[test]
fn remote_direct_pending_payload_is_discarded_when_local_device_is_revoked() {
    let mut f = remote_send_fixture();
    let peer = f.peer_owner.public_key();
    f.sender
        .send_direct_text(
            peer,
            &peer.to_hex(),
            "before removal",
            None,
            UnixSeconds(10),
        )
        .unwrap();
    let remaining = Keys::generate();
    let revoke = signed_app_keys(&f.owner, &[remaining.public_key()], 30);
    assert!(f
        .sender
        .ingest_app_keys_event(&revoke)
        .unwrap()
        .effects
        .is_empty());
    assert!(!f.sender.has_pending_retry_work());
    assert!(
        observe_sibling_invite(&mut f.sender, &f.late, &f.late_device)
            .effects
            .is_empty()
    );
}
