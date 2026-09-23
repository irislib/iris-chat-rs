fn call_test_peer(core: &mut AppCore, owner: &Keys, device: &Keys) {
    let now = unix_now().get();
    core.app_keys.insert(
        owner.public_key().to_hex(),
        known_app_keys_from_ndr(
            owner.public_key(),
            &AppKeys::new(vec![DeviceEntry::new(device.public_key(), now)]),
            now,
        ),
    );
    core.ensure_thread_record(&owner.public_key().to_hex(), now);
    core.preferences
        .accepted_owner_pubkeys
        .push(owner.public_key().to_hex());
}
fn call_control(id: &str, kind: &str, video: bool) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"v":1,"type":kind,"call_id":id,"video":video,"codec":"pcm16-jpeg-v1"}),
    )
    .unwrap()
}
#[test]
fn calls_require_known_accepted_unblocked_device_and_matching_call() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let peer = Keys::generate();
    let device = Keys::generate();
    let stranger = Keys::generate();
    let (mut core, updates, _dir) = logged_in_test_core_with_updates("call-policy", &owner, &local);
    let id = "00112233445566778899aabbccddeeff";
    let offer = call_control(id, "offer", true);
    core.handle_call_packet(&device.public_key().to_hex(), 39511, &offer);
    assert!(core.state.call.is_none());
    call_test_peer(&mut core, &peer, &device);
    core.preferences.accepted_owner_pubkeys.clear();
    core.handle_call_packet(&device.public_key().to_hex(), 39511, &offer);
    assert!(core.state.call.is_none(), "unaccepted contacts cannot ring");
    core.preferences
        .accepted_owner_pubkeys
        .push(peer.public_key().to_hex());
    core.preferences
        .blocked_owner_pubkeys
        .push(peer.public_key().to_hex());
    core.handle_call_packet(&device.public_key().to_hex(), 39511, &offer);
    assert!(core.state.call.is_none());
    core.preferences.blocked_owner_pubkeys.clear();
    core.handle_call_packet(&stranger.public_key().to_hex(), 39511, &offer);
    core.handle_call_packet(&device.public_key().to_hex(), 12345, &offer);
    assert!(core.state.call.is_none());
    core.handle_call_packet(&device.public_key().to_hex(), 39511, &offer);
    assert_eq!(core.state.call.as_ref().unwrap().phase, "incoming");
    core.handle_action(AppAction::AnswerCallWithVoice { call_id: id.into() });
    assert_eq!(
        core.shared_state.read().unwrap().call,
        core.state.call,
        "call-only changes must reach the FFI snapshot"
    );
    let snapshot = core.state.call.as_ref().unwrap();
    assert_eq!(snapshot.phase, "connected");
    assert!(!snapshot.video_capable);
    assert!(!snapshot.video);
    core.handle_call_packet(
        &stranger.public_key().to_hex(),
        39511,
        &call_control(id, "end", false),
    );
    core.handle_call_packet(
        &device.public_key().to_hex(),
        39511,
        &call_control("ffeeddccbbaa99887766554433221100", "end", false),
    );
    assert_eq!(core.state.call.as_ref().unwrap().phase, "connected");
    core.handle_action(AppAction::SetCallVideoEnabled { enabled: true });
    assert!(!core.state.call.as_ref().unwrap().video);
    core.handle_action(AppAction::EndCall { call_id: id.into() });
    core.handle_call_packet(&device.public_key().to_hex(), 39511, &offer);
    assert_eq!(
        core.state.call.as_ref().unwrap().phase,
        "ended",
        "replayed offer cannot ring again"
    );
    assert!(!updates
        .try_iter()
        .any(|u| matches!(u, AppUpdate::CallMedia { .. })));
    core.handle_action(AppAction::EndCall { call_id: id.into() });
    assert!(core.state.call.is_none());
    assert!(
        core.shared_state.read().unwrap().call.is_none(),
        "dismissal reaches the UI"
    );
    let canceled = "0123456789abcdef0123456789abcdef";
    core.handle_call_packet(
        &device.public_key().to_hex(),
        39511,
        &call_control(canceled, "end", true),
    );
    core.handle_call_packet(
        &device.public_key().to_hex(),
        39511,
        &call_control(canceled, "offer", true),
    );
    assert!(
        core.state.call.is_none(),
        "late offer after cancellation must not ring"
    );
}
#[test]
fn call_settings_persist_and_allow_voice_answer_when_video_disabled() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let peer = Keys::generate();
    let device = Keys::generate();
    let (mut core, _updates, dir) =
        logged_in_test_core_with_updates("call-settings", &owner, &local);
    call_test_peer(&mut core, &peer, &device);
    core.handle_action(AppAction::SetVideoCallsEnabled { enabled: false });
    let id = "00112233445566778899aabbccddeeff";
    core.handle_call_packet(
        &device.public_key().to_hex(),
        39511,
        &call_control(id, "offer", true),
    );
    assert_eq!(core.state.call.as_ref().unwrap().phase, "incoming");
    assert!(!core.state.call.as_ref().unwrap().video_capable);
    core.handle_action(AppAction::AnswerCall { call_id: id.into() });
    assert_eq!(core.state.call.as_ref().unwrap().phase, "connected");
    assert!(!core.state.call.as_ref().unwrap().video);
    core.handle_action(AppAction::SetVoiceCallsEnabled { enabled: false });
    assert_eq!(core.state.call.as_ref().unwrap().phase, "ended");
    core.handle_action(AppAction::EndCall { call_id: id.into() });
    core.handle_call_packet(
        &device.public_key().to_hex(),
        39511,
        &call_control("ffeeddccbbaa99887766554433221100", "offer", true),
    );
    assert!(core.state.call.is_none());
    drop(core);
    let restarted = AppCore::new(
        flume::unbounded().0,
        flume::unbounded().0,
        dir.path().to_string_lossy().into(),
        Arc::new(RwLock::new(AppState::empty())),
    );
    assert!(!restarted.state.preferences.voice_calls_enabled);
    assert!(!restarted.state.preferences.video_calls_enabled);
    assert!(restarted.state.call.is_none());
}
fn pump_call_pair(
    a: &mut AppCore,
    ar: &flume::Receiver<CoreMsg>,
    b: &mut AppCore,
    br: &flume::Receiver<CoreMsg>,
) {
    for (core, rx) in [(a, ar), (b, br)] {
        for message in rx.try_iter().take(128) {
            core.handle_message(message);
        }
    }
}
fn wait_call_pair(
    a: &mut AppCore,
    ar: &flume::Receiver<CoreMsg>,
    b: &mut AppCore,
    br: &flume::Receiver<CoreMsg>,
    predicate: impl Fn(&AppCore, &AppCore) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        pump_call_pair(a, ar, b, br);
        if predicate(a, b) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "call did not converge: {:?} / {:?}",
            a.state.call,
            b.state.call
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn calls_e2e_without_internet_over_local_fips_udp() {
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let (mut a, au, _adir) = logged_in_test_core_with_updates("call-udp-a", &ao, &ad);
    let (mut b, bu, _bdir) = logged_in_test_core_with_updates("call-udp-b", &bo, &bd);
    for core in [&mut a, &mut b] {
        call_test_peer(core, &ao, &ad);
        call_test_peer(core, &bo, &bd);
    }
    let (at, ar) = flume::unbounded();
    a.core_sender = at.clone();
    a.priority_sender = at;
    let (bt, br) = flume::unbounded();
    b.core_sender = bt.clone();
    b.priority_sender = bt;
    let addr = || {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    };
    let aa = addr();
    let ba = addr();
    a.reconcile_calls_udp_for_test(aa, ba, &test_fips_peer(&bd).npub());
    b.reconcile_calls_udp_for_test(ba, aa, &test_fips_peer(&ad).npub());
    assert!(a.logged_in.as_ref().unwrap().relay_urls.is_empty());
    assert!(b.logged_in.as_ref().unwrap().relay_urls.is_empty());
    let ae = a.device_sync_endpoint_for_test().unwrap();
    let be = b.device_sync_endpoint_for_test().unwrap();
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, _| {
        a.runtime.block_on(device_sync_pair_is_connected([
            (&ae, &test_fips_peer(&bd)),
            (&be, &test_fips_peer(&ad)),
        ]))
    });
    assert_eq!(
        a.runtime
            .block_on(device_sync_peer_transport(&ae, &test_fips_peer(&bd)))
            .as_deref(),
        Some("udp")
    );
    a.handle_action(AppAction::StartCall {
        chat_id: bo.public_key().to_hex(),
        video: true,
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state.call.as_ref().is_some_and(|c| c.phase == "incoming")
    });
    let id = b.state.call.as_ref().unwrap().call_id.clone();
    a.handle_action(AppAction::SetCallMuted { muted: true });
    a.handle_action(AppAction::SetCallVideoEnabled { enabled: false });
    assert!(a.shared_state.read().unwrap().call.as_ref().unwrap().muted);
    b.handle_action(AppAction::AnswerCall {
        call_id: id.clone(),
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, b| {
        [a, b].iter().all(|c| {
            c.state
                .call
                .as_ref()
                .is_some_and(|s| s.phase == "connected")
        })
    });
    assert!(
        a.state.call.as_ref().unwrap().muted,
        "pre-answer mute persists"
    );
    assert!(
        !a.state.call.as_ref().unwrap().video,
        "pre-answer camera-off persists"
    );
    assert!(a.state.call.as_ref().unwrap().video_capable);
    a.handle_action(AppAction::SetCallMuted { muted: false });
    a.handle_action(AppAction::SetCallVideoEnabled { enabled: true });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state
            .call
            .as_ref()
            .is_some_and(|c| !c.remote_muted && c.remote_video)
    });
    au.try_iter().for_each(drop);
    bu.try_iter().for_each(drop);
    let audio = (0..320)
        .flat_map(|i| (((i as f64 * 0.17).sin() * 12000.0) as i16).to_le_bytes())
        .collect::<Vec<_>>();
    let mut video = vec![42u8; 12000];
    video[..2].copy_from_slice(&[255, 216]);
    video[11998..].copy_from_slice(&[255, 217]);
    for core in [&mut a, &mut b] {
        core.handle_action(AppAction::SendCallMedia {
            call_id: id.clone(),
            kind: 1,
            data: audio.clone(),
        });
        core.handle_action(AppAction::SendCallMedia {
            call_id: id.clone(),
            kind: 2,
            data: video.clone(),
        });
    }
    let mut seen_a = Vec::new();
    let mut seen_b = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while seen_a.len() < 2 || seen_b.len() < 2 {
        pump_call_pair(&mut a, &ar, &mut b, &br);
        for (updates, seen) in [(&au, &mut seen_a), (&bu, &mut seen_b)] {
            for update in updates.try_iter() {
                if let AppUpdate::CallMedia {
                    call_id,
                    kind,
                    data,
                } = update
                {
                    assert_eq!(call_id, id);
                    assert_eq!(
                        data,
                        if kind == 1 {
                            audio.clone()
                        } else {
                            video.clone()
                        }
                    );
                    seen.push(kind);
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "bidirectional media missing: {seen_a:?}/{seen_b:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    a.handle_action(AppAction::SetCallMuted { muted: true });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state.call.as_ref().unwrap().remote_muted
    });
    a.handle_action(AppAction::SetCallVideoEnabled { enabled: false });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        !b.state.call.as_ref().unwrap().remote_video
    });
    a.handle_action(AppAction::EndCall { call_id: id });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state.call.as_ref().unwrap().phase == "ended"
    });
    // A second video offer can be answered as voice. Neither peer may send video.
    a.handle_action(AppAction::StartCall {
        chat_id: bo.public_key().to_hex(),
        video: true,
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state.call.as_ref().unwrap().phase == "incoming"
    });
    let id = b.state.call.as_ref().unwrap().call_id.clone();
    b.handle_action(AppAction::AnswerCallWithVoice {
        call_id: id.clone(),
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, _| {
        a.state.call.as_ref().unwrap().phase == "connected"
    });
    assert!(!a.state.call.as_ref().unwrap().video);
    assert!(!a.state.call.as_ref().unwrap().video_capable);
    assert!(!b.state.call.as_ref().unwrap().video);
    b.handle_action(AppAction::SetVoiceCallsEnabled { enabled: false });
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, _| {
        a.state.call.as_ref().unwrap().phase == "ended"
    });
    a.stop_device_sync_now();
    b.stop_device_sync_now();
}
