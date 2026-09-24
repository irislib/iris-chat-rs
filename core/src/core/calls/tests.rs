use super::*;

#[path = "adaptation_tests.rs"]
mod adaptation;
#[path = "history_tests.rs"]
mod history;

const CALL_ID: &str = "00112233445566778899aabbccddeeff";
const NEXT_CALL_ID: &str = "112233445566778899aabbccddeeff00";

#[test]
fn media_batches_expire_and_preserve_peer_validation() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    f._updates.try_iter().for_each(drop);
    let slots = Arc::new(tokio::sync::Semaphore::new(4));
    let video = vec![0, 0, 0, 1, 0x65, 42];
    for (sequence, age_ms, source, port, delivered) in [
        (0, 200, f.devices[0].clone(), PORT, false),
        (1, 0, f.devices[1].clone(), PORT, false),
        (2, 0, f.devices[0].clone(), PORT + 1, false),
        (3, 0, f.devices[0].clone(), PORT, true),
    ] {
        let packets = wire::encode(CALL_ID, 2, sequence, 0, true, &video)
            .into_iter()
            .map(|data| (source.clone(), port, data))
            .collect();
        f.core
            .handle_message(CoreMsg::Internal(Box::new(InternalEvent::CallMediaBatch {
                packets,
                received_at: Clock::now() - Duration::from_millis(age_ms),
                _permit: slots.clone().try_acquire_owned().unwrap(),
            })));
        assert_eq!(
            f._updates
                .try_iter()
                .any(|update| matches!(update, AppUpdate::CallMedia { .. })),
            delivered
        );
        assert_eq!(
            slots.available_permits(),
            4,
            "discarded batches must release their queue budget"
        );
    }
}

struct Fixture {
    core: AppCore,
    _updates: flume::Receiver<AppUpdate>,
    owner: String,
    devices: Vec<String>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (sender, updates) = flume::unbounded();
        let mut core = AppCore::new(
            sender,
            flume::unbounded().0,
            directory.path().to_string_lossy().into(),
            Arc::new(RwLock::new(AppState::empty())),
        );
        let local_owner = Keys::generate();
        let local_device = Keys::generate();
        core.logged_in = Some(LoggedInState {
            owner_pubkey: local_owner.public_key(),
            owner_keys: Some(local_owner),
            device_keys: local_device.clone(),
            client: Client::new(local_device),
            relay_urls: Vec::new(),
            authorization_state: LocalAuthorizationState::Authorized,
        });
        let owner = Keys::generate().public_key().to_hex();
        let devices: Vec<_> = (0..2)
            .map(|_| Keys::generate().public_key().to_hex())
            .collect();
        core.app_keys.insert(
            owner.clone(),
            KnownAppKeys {
                owner_pubkey_hex: owner.clone(),
                created_at_secs: 1,
                devices: devices
                    .iter()
                    .map(|id| KnownAppKeyDevice {
                        identity_pubkey_hex: id.clone(),
                        created_at_secs: 1,
                        device_label: None,
                        client_label: None,
                        label_updated_at_secs: 0,
                    })
                    .collect(),
            },
        );
        core.preferences.accepted_owner_pubkeys.push(owner.clone());
        core.ensure_thread_record(&owner, unix_now().get());
        Self {
            core,
            _updates: updates,
            owner,
            devices,
            _directory: directory,
        }
    }

    fn receive(&mut self, device: usize, kind: &str, video: bool) {
        let signal = Signal::new(kind, CALL_ID, video, false);
        self.core.handle_call_packet(
            &self.devices[device],
            PORT,
            &serde_json::to_vec(&signal).unwrap(),
        );
    }

    fn outgoing(&mut self, video: bool) {
        // Install the same state as start_call after transport selection. These
        // lifecycle tests deliberately create no endpoint or network connection.
        self.core.install_call(
            CALL_ID.into(),
            self.owner.clone(),
            self.devices.clone(),
            None,
            video,
            video,
            true,
        );
    }

    fn connected_incoming(&mut self, video: bool) {
        self.receive(0, "offer", video);
        self.core.handle_action(AppAction::AnswerCall {
            call_id: CALL_ID.into(),
        });
    }

    fn snapshot(&self) -> &CallSnapshot {
        self.core.state.call.as_ref().unwrap()
    }
}

#[test]
fn calls_ringing_timeout_ends_both_directions_and_stale_tick_cannot_end_next_call() {
    for outgoing in [false, true] {
        let mut f = Fixture::new();
        if outgoing {
            f.outgoing(false);
        } else {
            f.receive(0, "offer", false);
        }
        f.core.call_tick(CALL_ID);
        assert!(f.core.calls.active.is_some());
        f.core.calls.active.as_mut().unwrap().started = Clock::now() - Duration::from_secs(31);
        f.core.call_tick(CALL_ID);
        assert!(f.core.calls.active.is_none());
        assert_eq!(f.snapshot().phase, "ended");
        assert_eq!(f.snapshot().end_reason.as_deref(), Some("No answer"));

        let next = Signal::new("offer", NEXT_CALL_ID, false, false);
        f.core
            .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&next).unwrap());
        f.core.call_tick(CALL_ID);
        assert_eq!(f.snapshot().call_id, NEXT_CALL_ID);
        assert_eq!(f.snapshot().phase, "incoming");
        assert!(f.core.calls.active.is_some());
    }
}

#[test]
fn calls_keepalive_refreshes_connected_deadline_then_silence_ends_call() {
    let mut f = Fixture::new();
    f.connected_incoming(false);
    f.core.calls.active.as_mut().unwrap().last_received = Clock::now() - Duration::from_secs(16);
    f.receive(0, "ping", false);
    f.core.call_tick(CALL_ID);
    assert_eq!(f.snapshot().phase, "connected");

    // A different device cannot keep the chosen peer's dead connection alive.
    f.core.calls.active.as_mut().unwrap().last_received = Clock::now() - Duration::from_secs(16);
    f.receive(1, "ping", false);
    f.core.call_tick(CALL_ID);
    assert!(f.core.calls.active.is_none());
    assert_eq!(f.snapshot().phase, "ended");
    assert_eq!(f.snapshot().end_reason.as_deref(), Some("Connection lost"));
}

#[test]
fn calls_pre_answer_mute_and_camera_choice_survive_both_answer_paths() {
    for outgoing in [false, true] {
        let mut f = Fixture::new();
        if outgoing {
            f.outgoing(true);
        } else {
            f.receive(0, "offer", true);
        }
        f.core
            .handle_action(AppAction::SetCallMuted { muted: true });
        f.core
            .handle_action(AppAction::SetCallVideoEnabled { enabled: false });
        assert!(f.snapshot().muted);
        assert!(!f.snapshot().video);
        if outgoing {
            f.receive(0, "answer", true);
        } else {
            f.core.handle_action(AppAction::AnswerCall {
                call_id: CALL_ID.into(),
            });
        }
        assert_eq!(f.snapshot().phase, "connected");
        assert!(
            f.snapshot().muted,
            "answer must preserve microphone consent"
        );
        assert!(
            !f.snapshot().video,
            "answer must not re-enable a disabled camera"
        );
        assert!(
            f.snapshot().video_capable,
            "camera-off does not renegotiate as voice"
        );
        // Platform engines enforce these consent flags before enabling tracks.
        f.core
            .handle_action(AppAction::SetCallMuted { muted: false });
        assert!(!f.snapshot().muted);
    }
}

#[test]
fn calls_duplicate_offer_keeps_negotiated_video_when_local_camera_is_off() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    f.core
        .handle_action(AppAction::SetCallVideoEnabled { enabled: false });
    let connected_at = f.snapshot().connected_at_secs;
    let revision = f.core.state.rev;
    f.receive(0, "offer", true);
    assert_eq!(f.snapshot().phase, "connected");
    assert_eq!(f.snapshot().connected_at_secs, connected_at);
    assert!(!f.snapshot().video);
    assert!(f.snapshot().video_capable);
    assert!(f.core.calls.active.as_ref().unwrap().video);
    assert_eq!(
        f.core.state.rev, revision,
        "retransmission must not restart the call UI"
    );
}

#[test]
fn calls_first_answer_wins_and_other_device_cannot_downgrade_or_end_call() {
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "answer", true);
    assert_eq!(
        f.core.calls.active.as_ref().unwrap().peer.as_ref(),
        Some(&f.devices[0])
    );
    let connected_at = f.snapshot().connected_at_secs;
    f.receive(1, "answer", false);
    f.receive(1, "end", false);
    f.receive(1, "reject", false);
    assert_eq!(f.snapshot().phase, "connected");
    assert!(f.snapshot().video_capable);
    assert_eq!(f.snapshot().connected_at_secs, connected_at);

    // Removing the sibling that did not answer does not revoke the chosen one.
    f.core
        .app_keys
        .get_mut(&f.owner)
        .unwrap()
        .devices
        .retain(|d| d.identity_pubkey_hex == f.devices[0]);
    f.core.call_tick(CALL_ID);
    assert_eq!(f.snapshot().phase, "connected");
    f.receive(0, "end", true);
    assert!(f.core.calls.active.is_none());
    assert_eq!(f.snapshot().phase, "ended");
}

#[test]
fn calls_revocation_or_blocking_drops_media_and_cleans_up_on_tick() {
    for change in [
        "remote-device-revoked",
        "contact-blocked",
        "local-device-revoked",
    ] {
        let mut f = Fixture::new();
        f.connected_incoming(false);
        match change {
            "remote-device-revoked" => f.core.app_keys.get_mut(&f.owner).unwrap().devices.clear(),
            "contact-blocked" => f
                .core
                .preferences
                .blocked_owner_pubkeys
                .push(f.owner.clone()),
            _ => {
                f.core.logged_in.as_mut().unwrap().authorization_state =
                    LocalAuthorizationState::Revoked
            }
        }
        let received = f.core.calls.active.as_ref().unwrap().last_received;
        for packet in wire::encode(CALL_ID, 1, 0, 0, true, &[42; 80]) {
            f.core.handle_call_packet(&f.devices[0], PORT, &packet);
        }
        assert_eq!(
            f.core.calls.active.as_ref().unwrap().last_received,
            received,
            "{change} must reject media"
        );
        f.core.call_tick(CALL_ID);
        assert!(
            f.core.calls.active.is_none(),
            "{change} must release the live call"
        );
        assert_eq!(f.snapshot().phase, "ended");
    }
}

#[test]
fn calls_feedback_adapts_bitrate_and_ignores_unauthenticated_or_replayed_feedback() {
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "answer", true);
    let initial = f.snapshot().target_bitrate_bps;
    let mut feedback = Signal::new("feedback", CALL_ID, true, false);
    feedback.feedback_seq = Some(0);
    feedback.video_seq = Some(29);
    feedback.received_frames = Some(15);
    feedback.received_bytes = Some(50000);
    feedback.interval_ms = Some(1000);
    f.core
        .handle_call_packet(&f.devices[1], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(f.snapshot().target_bitrate_bps, initial);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(f.snapshot().target_bitrate_bps, 340_000);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(f.snapshot().target_bitrate_bps, 340_000);
    feedback.feedback_seq = Some(1);
    feedback.video_seq = Some(59);
    feedback.received_frames = Some(30);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(
        f.snapshot().target_bitrate_bps,
        340_000,
        "hold after loss so queues can clear"
    );
    f.core.set_call_quality("custom".into(), 200_000);
    assert_eq!(f.snapshot().target_bitrate_bps, 200_000);
    f.receive(0, "keyframe", true);
    assert_eq!(f.snapshot().key_frame_generation, 1);
    f.receive(0, "keyframe", true);
    assert_eq!(f.snapshot().key_frame_generation, 1);
}

#[test]
fn calls_reduce_bitrate_when_only_control_packets_survive() {
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "answer", true);
    let initial = f.snapshot().target_bitrate_bps;
    f.core.calls.active.as_mut().unwrap().video_seq = 30;
    let mut feedback = Signal::new("feedback", CALL_ID, true, false);
    feedback.feedback_seq = Some(0);
    feedback.received_frames = Some(0);
    feedback.received_bytes = Some(0);
    feedback.interval_ms = Some(1000);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(f.snapshot().target_bitrate_bps, initial / 2);
    feedback.feedback_seq = Some(1);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&feedback).unwrap());
    assert_eq!(
        f.snapshot().target_bitrate_bps,
        initial / 2,
        "no further reduction when capture sends nothing"
    );
}

#[test]
fn calls_end_if_signaling_survives_but_media_never_connects() {
    let mut f = Fixture::new();
    f.connected_incoming(false);
    f.core
        .calls
        .active
        .as_mut()
        .unwrap()
        .media_disconnected_since = Some(Clock::now() - Duration::from_secs(31));
    f.receive(0, "ping", false);
    f.core.call_tick(CALL_ID);
    assert_eq!(f.snapshot().phase, "ended");
    assert!(f.core.calls.active.is_none());
}

#[test]
fn calls_explicit_decline_stops_all_targets_but_unavailable_device_does_not() {
    let mut f = Fixture::new();
    f.outgoing(true);
    f.receive(0, "reject", false);
    assert_eq!(f.snapshot().phase, "outgoing");
    let mut reject = Signal::new("reject", CALL_ID, false, false);
    reject.reason = Some("declined".into());
    f.core
        .handle_call_packet(&f.devices[1], PORT, &serde_json::to_vec(&reject).unwrap());
    assert_eq!(f.snapshot().phase, "ended");
    assert_eq!(f.snapshot().end_reason.as_deref(), Some("Call declined"));
}

#[test]
fn calls_incoming_stops_when_caller_disappears_but_repeated_offers_keep_it_alive() {
    let mut f = Fixture::new();
    f.receive(0, "offer", false);
    f.core.calls.active.as_mut().unwrap().last_received = Clock::now() - Duration::from_secs(11);
    f.receive(0, "offer", false);
    f.core.call_tick(CALL_ID);
    assert_eq!(f.snapshot().phase, "incoming");
    f.core.calls.active.as_mut().unwrap().last_received = Clock::now() - Duration::from_secs(11);
    f.core.call_tick(CALL_ID);
    assert_eq!(f.snapshot().phase, "ended");
    assert_eq!(f.snapshot().end_reason.as_deref(), Some("Connection lost"));
}
