use super::*;

fn fixture() -> (AppCore, flume::Receiver<AppUpdate>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = flume::unbounded();
    let mut core = AppCore::new(
        tx,
        flume::unbounded().0,
        dir.path().to_string_lossy().into(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    core.preferences.nostr_relay_urls.clear();
    core.start_primary_session(Keys::generate(), Keys::generate(), false, false)
        .unwrap();
    (core, rx, dir)
}

#[test]
fn nearby_ingestion_validates_once_across_transport_replays() {
    let (mut core, _updates, _dir) = fixture();
    let author = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "already authenticated")
        .sign_with_keys(&author)
        .unwrap();
    let packet = encode_fips_nearby_event(&event).unwrap();
    let before = core.event_validation.signature_checks;
    core.handle_fips_nearby_packet(&author.public_key().to_hex(), FIPS_NEARBY_PORT, &packet);
    assert_eq!(
        core.event_validation.signature_checks - before,
        1,
        "one packet must not verify the same signature again at its relay handoff"
    );
    assert!(core.has_seen_event(&event.id.to_hex()));
    for _ in 0..128 {
        core.handle_fips_nearby_packet(&author.public_key().to_hex(), FIPS_NEARBY_PORT, &packet);
        core.handle_relay_event_with_channel(event.clone(), "FIPS mesh");
    }
    assert_eq!(
        core.event_validation.signature_checks - before,
        1,
        "authenticated retransmissions must not repeat expensive signature verification"
    );
}

#[test]
fn nearby_ingestion_replays_do_not_flood_peer_updates() {
    let (mut core, updates, _dir) = fixture();
    let author = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "does not change nearby peers")
        .sign_with_keys(&author)
        .unwrap();
    core.handle_relay_event(event.clone());
    while updates.try_recv().is_ok() {}
    let packet = encode_fips_nearby_event(&event).unwrap();
    for _ in 0..128 {
        core.handle_fips_nearby_packet(&author.public_key().to_hex(), FIPS_NEARBY_PORT, &packet);
    }
    assert_eq!(updates.try_iter().filter(|u| matches!(u, AppUpdate::NearbyPeersChanged { .. })).count(), 0,
        "event retransmissions must not enqueue unchanged Nearby views ahead of foreground feedback");
}

#[test]
fn nearby_ingestion_cached_proof_rejects_forged_event_variants() {
    let (mut core, updates, _dir) = fixture();
    let author = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "authenticated body")
        .sign_with_keys(&author)
        .unwrap();
    let other = EventBuilder::new(Kind::TextNote, "different body")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    assert!(core.handle_relay_event_with_channel(event.clone(), "FIPS nearby"));
    while updates.try_recv().is_ok() {}
    let mut forged_content = event.clone();
    forged_content.content.push_str("tampered");
    let mut forged_id = event.clone();
    forged_id.id = other.id;
    let mut forged_author = event.clone();
    forged_author.pubkey = other.pubkey;
    let mut forged_signature = event.clone();
    forged_signature.sig = other.sig;
    for forged in [forged_content, forged_id, forged_author, forged_signature] {
        assert!(!core.handle_relay_event_with_channel(forged, "FIPS nearby"));
    }
    assert!(
        updates.try_recv().is_err(),
        "forgeries must cause no app effects"
    );
    assert_eq!(core.event_validation.signature_checks, 2);
    assert!(core.handle_relay_event_with_channel(event, "FIPS nearby"));
    assert_eq!(core.event_validation.signature_checks, 2);
}
