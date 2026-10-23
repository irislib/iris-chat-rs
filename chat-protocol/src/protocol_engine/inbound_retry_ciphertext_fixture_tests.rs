#[derive(Clone, Copy)]
enum SyntheticInboundCiphertext {
    Malformed,
    CorruptMac,
}

// Retain the original malformed fixture for its existing allocation/deadline
// tests. The v3 CPU workload uses authentic encodings with a damaged MAC.
pub(super) fn synthetic_pending_inbound(
    receiver: &mut ProtocolEngine,
    count: usize,
    due: usize,
    now: u64,
) -> Vec<ProtocolPendingInbound> {
    synthetic_pending_inbound_with_ciphertext(
        receiver,
        count,
        due,
        now,
        SyntheticInboundCiphertext::Malformed,
    )
}

fn synthetic_pending_inbound_with_ciphertext(
    receiver: &mut ProtocolEngine,
    count: usize,
    due: usize,
    now: u64,
    ciphertext: SyntheticInboundCiphertext,
) -> Vec<ProtocolPendingInbound> {
    synthetic_pending_inbound_modified(
        receiver,
        count,
        due,
        now,
        |index, envelope| match ciphertext {
            SyntheticInboundCiphertext::Malformed => {
                envelope.ciphertext = format!("invalid ciphertext {index}:{}", "x".repeat(1024));
            }
            SyntheticInboundCiphertext::CorruptMac => corrupt_synthetic_inbound_mac(envelope),
        },
    )
}

fn corrupt_synthetic_inbound_mac(envelope: &mut MessageEnvelope) {
    use base64::Engine;
    let mut bytes = base64::engine::general_purpose::STANDARD
        .decode(&envelope.ciphertext)
        .unwrap();
    // NIP44 authentication is at the tail. Alter one emitted byte, preserving
    // the encoder's actual layout, size, version and all header/counter fields.
    *bytes.last_mut().expect("encrypted payload has a MAC") ^= 1;
    envelope.ciphertext = base64::engine::general_purpose::STANDARD.encode(bytes);
}

// Synthetic keys and payloads only. Keep the original sequence 0..count and
// deadline mix; do not turn far-ahead packets into cheaper first messages.
fn synthetic_pending_inbound_modified(
    receiver: &mut ProtocolEngine,
    count: usize,
    due: usize,
    now: u64,
    mut modify: impl FnMut(usize, &mut MessageEnvelope),
) -> Vec<ProtocolPendingInbound> {
    let sender = Keys::generate();
    receiver
        .ingest_app_keys_event(&signed_app_keys(&sender, &[sender.public_key()], 1))
        .unwrap();
    let (mut session, response) = receiver
        .local_invite()
        .unwrap()
        .accept_with_owner(
            sender.public_key(),
            sender.secret_key().to_secret_bytes(),
            Some(sender.public_key().to_hex()),
            Some(sender.public_key()),
        )
        .unwrap();
    receiver
        .observe_invite_response_event(&invite_response_event(&response).unwrap())
        .unwrap();
    (0..count)
        .map(|index| {
            let plan = session
                .plan_send(
                    format!("deferred-{index}:{}", "x".repeat(1024)).as_bytes(),
                    NdrUnixSeconds(1),
                )
                .unwrap();
            let mut envelope = session.apply_send(plan).envelope;
            modify(index, &mut envelope);
            let event = nostr_double_ratchet::message_event(&envelope).unwrap();
            let metadata =
                receiver.pending_inbound_metadata_for_event(&event, Some(&envelope), None);
            let mut pending = ProtocolPendingInbound {
                body: ProtocolPendingInboundBody {
                    event,
                    event_id: String::new(),
                    envelope: None,
                    sender_message_pubkey_hex: None,
                    resolved_owner_pubkey_hex: None,
                    claimed_owner_pubkey_hex: None,
                    metadata_verified: false,
                }
                .into(),
                created_at_secs: 1,
                next_retry_at_secs: if index < due { now } else { now + 52 },
            };
            apply_pending_inbound_metadata(&mut pending, metadata);
            pending
        })
        .collect()
}

fn assert_corrupt_mac_inbound_format(packets: &[ProtocolPendingInbound]) -> (usize, usize) {
    use base64::Engine;
    let mut minimum = usize::MAX;
    let mut maximum = 0;
    for packet in packets {
        let envelope = packet.envelope.as_ref().unwrap();
        assert_eq!(packet.event.content, envelope.ciphertext);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&envelope.ciphertext)
            .expect("synthetic pending ciphertext must be valid base64");
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(&bytes),
            envelope.ciphertext,
            "synthetic encoding must be canonical"
        );
        assert_eq!(
            bytes.first(),
            Some(&nostr::nips::nip44::Version::V2.as_u8())
        );
        // Aggregate input observation, not a duplicated NIP44 layout constant.
        assert!((387..=2115).contains(&bytes.len()));
        minimum = minimum.min(bytes.len());
        maximum = maximum.max(bytes.len());
    }
    assert!(!packets.is_empty());
    (minimum, maximum)
}

#[test]
fn corrupt_mac_inbound_fixture_reaches_authentication_and_preserves_state() {
    use base64::Engine;
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut receiver = test_engine(&owner, &device);
    let mut originals = Vec::new();
    let packets = synthetic_pending_inbound_modified(
        &mut receiver,
        4_214,
        102,
        20_000_000,
        |index, envelope| {
            if index == 0 || index == nostr_double_ratchet::MAX_SKIP {
                originals.push((index, envelope.clone()));
            }
            corrupt_synthetic_inbound_mac(envelope);
        },
    );
    let (minimum, maximum) = assert_corrupt_mac_inbound_format(&packets);
    let snapshot = receiver.session_manager.snapshot();
    let expected = serde_json::to_value(&snapshot).unwrap();
    for (index, original) in &originals {
        let sender = receiver.resolve_message_sender_owner(original);
        let sender_owner = match sender {
            ProtocolSenderOwnerResolution::Verified { owner }
            | ProtocolSenderOwnerResolution::ProvisionalDeviceOwner { owner } => owner,
            _ => panic!("fixture sender must have an authenticated session"),
        };
        let mut control =
            SessionManager::from_snapshot(snapshot.clone(), device.secret_key().to_secret_bytes())
                .unwrap();
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(1), &mut rng);
        let received = control
            .receive(&mut ctx, sender_owner, original)
            .unwrap()
            .unwrap();
        assert_eq!(
            received.payload,
            format!("deferred-{index}:{}", "x".repeat(1024)).as_bytes()
        );
        let mut expected_bytes = base64::engine::general_purpose::STANDARD
            .decode(&original.ciphertext)
            .unwrap();
        *expected_bytes.last_mut().unwrap() ^= 1;
        let damaged = packets[*index].envelope.as_ref().unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(expected_bytes),
            damaged.ciphertext
        );
        let mut restored = damaged.clone();
        restored.ciphertext = original.ciphertext.clone();
        assert!(
            restored == *original,
            "tampering changed more than the payload MAC"
        );
    }
    let mut authentication_failures = 0;
    let mut skip_limit_failures = 0;
    let mut event_ids = HashSet::new();
    for (index, packet) in packets.iter().enumerate() {
        packet.event.verify().unwrap();
        assert!(event_ids.insert(packet.event.id));
        assert_ne!(
            packet.event.pubkey.to_hex(),
            packet.resolved_owner_pubkey_hex.as_ref().unwrap().as_str()
        );
        assert!(packet.event.tags.public_keys().next().is_none());
        let error = match receiver.decrypt_pending_direct_message_event(packet) {
            Err(error) => error,
            Ok(_) => panic!("damaged packet {index} must fail decryption"),
        };
        if index <= nostr_double_ratchet::MAX_SKIP {
            assert!(
                matches!(
                    error.downcast_ref::<NdrError>(),
                    Some(NdrError::Nip44(nostr::nips::nip44::Error::V2(
                        nostr::nips::nip44::v2::ErrorV2::InvalidHmac
                    )))
                ),
                "eligible packet {index} did not reach MAC verification"
            );
            authentication_failures += 1;
        } else {
            assert!(
                matches!(
                    error.downcast_ref::<NdrError>(),
                    Some(NdrError::Domain(DomainError::TooManySkippedMessages))
                ),
                "far-ahead packet {index} changed its skip-limit branch"
            );
            skip_limit_failures += 1;
        }
    }
    assert!(serde_json::to_value(receiver.session_manager.snapshot()).unwrap() == expected);
    assert!(receiver.pending_decrypted_deliveries.is_empty());
    assert_eq!(authentication_failures, 1_001);
    assert_eq!(skip_limit_failures, 3_213);
    eprintln!("corrupt_mac_inbound_fixture packets=4214 format_checked=4214 decoded_min={minimum} decoded_max={maximum} authenticated_failures={authentication_failures} skip_limit_failures={skip_limit_failures} positive_controls=2 distinct_ids={}", event_ids.len());
}

#[test]
fn corrupt_mac_inbound_retry_preserves_backoff_and_restart() {
    use std::sync::atomic::Ordering::Relaxed;
    let owner = Keys::generate();
    let device = Keys::generate();
    let mut engine = test_engine(&owner, &device);
    let now = NdrUnixSeconds(20_000_000);
    engine.pending_inbound = synthetic_pending_inbound_with_ciphertext(
        &mut engine,
        16,
        3,
        now.get(),
        SyntheticInboundCiphertext::CorruptMac,
    );
    let mut expected = engine.pending_inbound.clone();
    let sessions = serde_json::to_value(engine.session_manager.snapshot()).unwrap();
    engine.persist().unwrap();
    let storage = Arc::new(GroupRetryCountingStorage {
        inner: engine.storage.clone(),
        puts: 0.into(),
        fail_next: false.into(),
    });
    engine.storage = storage.clone();
    assert!(engine.retry_pending_protocol(now).unwrap().is_empty());
    assert_eq!(storage.puts.load(Relaxed), 0);
    for packet in &mut expected[..3] {
        packet.next_retry_at_secs = now.get() + 60;
    }
    assert!(engine.pending_inbound == expected);
    assert!(engine.pending_decrypted_deliveries.is_empty());
    assert!(serde_json::to_value(engine.session_manager.snapshot()).unwrap() == sessions);
    engine.persist().unwrap();
    let mut restored = ProtocolEngine::load_or_create_for_local_device(
        storage.clone(),
        owner.public_key(),
        &device,
    )
    .unwrap();
    assert!(restored.pending_inbound == expected);
    assert_corrupt_mac_inbound_format(&restored.pending_inbound);
    let puts = storage.puts.load(Relaxed);
    assert!(restored
        .retry_pending_protocol(NdrUnixSeconds(now.get() + 60))
        .unwrap()
        .is_empty());
    assert_eq!(storage.puts.load(Relaxed), puts);
    for packet in &mut expected {
        packet.next_retry_at_secs = now.get() + 120;
    }
    assert!(restored.pending_inbound == expected);
    assert!(restored.pending_decrypted_deliveries.is_empty());
    assert!(serde_json::to_value(restored.session_manager.snapshot()).unwrap() == sessions);
    eprintln!("corrupt_mac_inbound_retry packets=16 initially_due=3 rounds=2 saves_on_failed_retry=0 restart_preserved=true");
}
