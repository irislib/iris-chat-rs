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
        &serde_json::json!({"v":3,"type":kind,"call_id":id,"video":video,"codec":"opus-h264-v3"}),
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
    let (mut core, _updates, _dir) = logged_in_test_core_with_updates("call-policy", &owner, &local);
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
    core.handle_action(AppAction::SetCallQuality { quality: "custom".into(), max_bitrate_bps: 777_000 });
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
    assert_eq!(restarted.state.preferences.call_quality, "custom");
    assert_eq!(restarted.state.preferences.call_max_bitrate_bps, 777_000);
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
    exercise_local_fips_call(false, false);
}

#[test]
fn calls_e2e_resuming_recipient_receives_still_ringing_call() {
    exercise_local_fips_call(true, false);
}

#[test]
fn calls_e2e_push_wakes_suspended_recipient_and_connects_over_fips() {
    exercise_local_fips_call(true, true);
}

fn exercise_local_fips_call(resume_recipient: bool, push_wakeup: bool) {
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
    if resume_recipient {
        b.prepare_for_suspend();
        assert!(b.suspended);
        assert!(b.device_sync.is_none());
    }
    let wake_server = if push_wakeup {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        a.preferences.mobile_push_server_url = format!("http://{}", listener.local_addr().unwrap());
        Some(std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new(); let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0); bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = headers.lines().find_map(|line| line.to_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse().unwrap())).unwrap();
                    if bytes.len() >= end + 4 + length {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
                        break serde_json::from_slice::<serde_json::Value>(&bytes[end+4..end+4+length]).unwrap();
                    }
                }
            }
        }))
    } else { None };
    a.handle_action(AppAction::StartCall {
        chat_id: bo.public_key().to_hex(),
        video: true,
    });
    if resume_recipient {
        let suspended_until = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < suspended_until {
            pump_call_pair(&mut a, &ar, &mut b, &br);
            assert!(b.state.call.is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(server) = wake_server {
            let event = server.join().unwrap();
            let payload = serde_json::json!({"event": event}).to_string();
            b.persist_best_effort();
            assert!(super::calls::push::resolve_call_push_invite(
                _bdir.path().to_string_lossy().into(), bd.secret_key().to_secret_hex(), payload.clone()).is_some());
            b.ingest_mobile_push_payload(&payload);
            assert_eq!(b.state.call.as_ref().unwrap().phase, "incoming");
            b.ingest_mobile_push_payload(&payload); // Duplicate push cannot create another call.
        } else {
            b.handle_app_foregrounded();
        }
        // Restore the test's local-only addressing after the production resume.
        b.reconcile_calls_udp_for_test(ba, aa, &test_fips_peer(&ad).npub());
        assert!(!b.suspended);
    }
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
    // The WAN-denial harness blocks STUN. Keep the call alive beyond ICE
    // gathering before proving codec delivery on the existing FIPS route.
    let gather_deadline = std::time::Instant::now()
        + Duration::from_millis(fips_core::WebRtcConfig::default().ice_gather_timeout_ms() + 500);
    while std::time::Instant::now() < gather_deadline {
        pump_call_pair(&mut a, &ar, &mut b, &br);
        for core in [&a, &b] {
            assert_eq!(core.state.call.as_ref().unwrap().phase, "connected");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    au.try_iter().for_each(drop);
    bu.try_iter().for_each(drop);
    let codec=crate::CallAudioCodec::new().unwrap();
    let audio=codec.encode((0..960).map(|i|((i as f32*0.1).sin()*10000.0) as i16).collect()).unwrap();
    let mut video=vec![42;12000];video[..4].copy_from_slice(&[0,0,0,1]);
    for core in [&mut a,&mut b] {
        core.handle_action(AppAction::SendCallMedia {call_id:id.clone(),kind:1,timestamp_us:0,key_frame:true,data:audio.clone()});
        core.handle_action(AppAction::SendCallMedia {call_id:id.clone(),kind:2,timestamp_us:0,key_frame:true,data:video.clone()});
    }
    let mut seen_a=Vec::new();let mut seen_b=Vec::new();let deadline=std::time::Instant::now()+Duration::from_secs(10);
    while seen_a.len()<2 || seen_b.len()<2 {
        pump_call_pair(&mut a,&ar,&mut b,&br);
        for (updates,seen) in [(&au,&mut seen_a),(&bu,&mut seen_b)] {
            for update in updates.try_iter() {
                if let AppUpdate::CallMedia{call_id,kind,sequence,timestamp_us,key_frame,data}=update {
                    assert_eq!(call_id,id);assert_eq!(sequence,0);assert_eq!(timestamp_us,0);assert!(key_frame);
                    assert_eq!(data,if kind==1 {audio.clone()}else {video.clone()});seen.push(kind);
                }
            }
        }
        assert!(std::time::Instant::now()<deadline,"Bidirectional codec packets missing: {seen_a:?}/{seen_b:?}");
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

#[test]
fn device_sync_contact_refresh_preserves_live_fips_sessions() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _updates, _temp) =
        logged_in_test_core_with_updates("websocket-contact-refresh", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let address = reserve_tcp_addr();
    let websocket = fips_core::config::WebSocketConfig {
        bind_addr: Some(address.to_string()),
        ..Default::default()
    };
    core.reconcile_device_sync_with_websocket_for_test(websocket.clone());
    let before = core.device_sync_endpoint_for_test().unwrap();
    let contact = Keys::generate().public_key().to_hex();
    let mut roster = core.app_keys.get(&owner.public_key().to_hex()).unwrap().clone();
    roster.owner_pubkey_hex = contact.clone();
    roster.devices[0].identity_pubkey_hex = Keys::generate().public_key().to_hex();
    roster.devices.truncate(1);
    core.app_keys.insert(contact, roster);
    core.reconcile_device_sync_with_websocket_for_test(websocket);
    let after = core.device_sync_endpoint_for_test().unwrap();
    assert!(Arc::ptr_eq(&before, &after), "learning a contact must not replace active FIPS sessions");
    core.stop_device_sync_now();
}

#[test]
fn device_sync_keeps_fixed_websocket_listener_after_roster_refresh() {
    let owner = Keys::generate();
    let local = Keys::generate();
    let sibling = Keys::generate();
    let (mut core, _updates, _temp) =
        logged_in_test_core_with_updates("websocket-roster-refresh", &owner, &local);
    configure_test_device_sync_profile(&mut core, &owner, &local, &sibling, None);
    let address = reserve_tcp_addr();
    for generation in 0..3 {
        core.app_keys.get_mut(&owner.public_key().to_hex()).unwrap().created_at_secs += 1;
        core.reconcile_device_sync_with_websocket_for_test(fips_core::config::WebSocketConfig {
            bind_addr: Some(address.to_string()),
            ..Default::default()
        });
        assert!(core.device_sync.is_some(), "endpoint lost on refresh {generation}");
        std::thread::sleep(Duration::from_millis(200));
        std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1))
            .expect("FIPS WebSocket listener must survive roster refresh");
    }
    core.stop_device_sync_now();
}
