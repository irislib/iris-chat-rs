use super::*;
use nostr::Tag;

fn fixture() -> (AppCore, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        directory.path().to_string_lossy().into(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.transports.udp =
        fips_core::config::TransportInstances::Single(fips_core::config::UdpConfig {
            bind_addr: Some("127.0.0.1:0".into()),
            advertise_on_nostr: Some(false),
            public: Some(false),
            ..Default::default()
        });
    let endpoint = Arc::new(
        core.runtime
            .block_on(
                fips_core::FipsEndpoint::builder()
                    .config(config)
                    .without_system_tun()
                    .bind(),
            )
            .unwrap(),
    );
    let (tcp, _records) =
        super::super::super::device_sync_tcp::DeviceSyncTcpSender::test_channel(1, 1024);
    core.install_device_sync_sender_for_test(endpoint, tcp, Vec::new());
    core.preferences.nearby_enabled = true;
    core.preferences.nearby_mailbag_enabled = true;
    core.device_sync.as_mut().unwrap().nearby_enabled = true;
    (core, directory)
}

#[test]
fn unrelated_nearby_receipt_preserves_delivery_to_a_late_peer() {
    let (mut core, _directory) = fixture();
    let unrelated = Keys::generate();
    let late = Keys::generate();
    let event = EventBuilder::new(Kind::GiftWrap, "opaque message for the later peer")
        .tag(Tag::public_key(late.public_key()))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let event_id = event.id.to_hex();
    // Exercise the real publication queue and transport receipt handler.
    // The first peer's transport receipt says nothing about the later peer.
    core.publish_fips_nearby(&event);
    let queue = core.device_sync.as_ref().unwrap().nearby_outbox.clone();
    queue.write().unwrap().mark_sent_on_link(
        &unrelated.public_key().to_bech32().unwrap(),
        7,
        std::slice::from_ref(&event_id),
    );
    core.handle_fips_nearby_packet(
        &unrelated.public_key().to_hex(),
        FIPS_NEARBY_PORT,
        &FipsNearbyPacket::receipt(event_id.clone())
            .unwrap()
            .encode()
            .unwrap(),
    );
    let pending = queue
        .read()
        .unwrap()
        .pending_for_link(&late.public_key().to_bech32().unwrap(), 8);
    core.stop_device_sync_now();
    assert_eq!(
        pending.len(),
        1,
        "another peer's receipt discarded the late peer's message"
    );
    assert_eq!(pending[0].0, event_id);
    assert_eq!(pending[0].1, encode_fips_nearby_event(&event).unwrap());
}

#[test]
fn nearby_receipts_are_idempotent_and_reject_invalid_sources() {
    let (mut core, _directory) = fixture();
    let recipient = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "queued event")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    core.publish_fips_nearby(&event);
    let queue = core.device_sync.as_ref().unwrap().nearby_outbox.clone();
    let receipt = FipsNearbyPacket::receipt(event.id.to_hex())
        .unwrap()
        .encode()
        .unwrap();
    let npub = recipient.public_key().to_bech32().unwrap();
    for source in ["", "not-a-device", &"ff".repeat(32)] {
        core.handle_fips_nearby_packet(source, FIPS_NEARBY_PORT, &receipt);
        assert_eq!(queue.read().unwrap().pending_for_link(&npub, 1).len(), 1);
    }
    for packet in [
        FipsNearbyPacket::Receipt {
            v: 2,
            event_id: event.id.to_hex(),
        },
        FipsNearbyPacket::Receipt {
            v: 1,
            event_id: "invalid".into(),
        },
        FipsNearbyPacket::receipt("cd".repeat(32)).unwrap(),
    ] {
        core.handle_fips_nearby_packet(
            &recipient.public_key().to_hex(),
            FIPS_NEARBY_PORT,
            &serde_json::to_vec(&packet).unwrap(),
        );
        assert_eq!(queue.read().unwrap().pending_for_link(&npub, 1).len(), 1);
    }
    for _ in 0..64 {
        core.handle_fips_nearby_packet(
            &recipient.public_key().to_hex(),
            FIPS_NEARBY_PORT,
            &receipt,
        );
    }
    // Replaying a pending server publication must preserve the recipient's ACK.
    core.publish_fips_nearby(&event);
    let queue = queue.read().unwrap();
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(queue.entries[0].acknowledged_peers.len(), 1);
    assert!(queue.pending_for_link(&npub, 2).is_empty());
    assert_eq!(queue.pending_for_link("later-peer", 2).len(), 1);
    drop(queue);
    core.stop_device_sync_now();
}

#[test]
fn nearby_receipt_retention_and_explicit_cancellation_stay_bounded() {
    let (mut core, _directory) = fixture();
    let author = Keys::generate();
    let mut ids = Vec::new();
    for index in 0..=FIPS_NEARBY_OUTBOX_MAX_EVENTS {
        let event = EventBuilder::new(Kind::TextNote, format!("queued {index}"))
            .sign_with_keys(&author)
            .unwrap();
        ids.push(event.id.to_hex());
        core.publish_fips_nearby(&event);
    }
    let newest = ids.last().unwrap();
    let receipt = FipsNearbyPacket::receipt(newest.clone())
        .unwrap()
        .encode()
        .unwrap();
    for _ in 0..=FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT {
        core.handle_fips_nearby_packet(
            &Keys::generate().public_key().to_hex(),
            FIPS_NEARBY_PORT,
            &receipt,
        );
    }
    let queue = core.device_sync.as_ref().unwrap().nearby_outbox.clone();
    let mut queue = queue.write().unwrap();
    assert_eq!(queue.entries.len(), FIPS_NEARBY_OUTBOX_MAX_EVENTS);
    assert_eq!(
        queue.entries.back().unwrap().acknowledged_peers.len(),
        FIPS_NEARBY_OUTBOX_MAX_LINKS_PER_EVENT
    );
    let pending = queue.pending_for_link("later-peer", 1);
    assert_eq!(pending.len(), FIPS_NEARBY_OUTBOX_MAX_EVENTS);
    assert!(!pending.iter().any(|(id, _)| id == &ids[0]));
    queue.forget(newest);
    assert!(!queue
        .pending_for_link("later-peer", 2)
        .iter()
        .any(|(id, _)| id == newest));
    drop(queue);
    core.stop_device_sync_now();
}
