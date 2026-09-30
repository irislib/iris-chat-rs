/// Real AppCore + authenticated FIPS UDP transport + shared Opus playback.
/// Synthetic Annex-B frames exercise video fragmentation/queue load, not H264
/// visual quality. No public network, microphone, camera or existing account.
/// Replay: cargo test --manifest-path core/Cargo.toml --locked --lib
/// calls_sustained_audio_playout_survives_video_bursts -- --ignored --nocapture
/// Set IRIS_CALL_SOAK_SECONDS=600 or 1800 for ten/thirty wall-clock minutes.
/// This does not cover hardware audio/video, echo cancellation, or a rate-limited WAN.
#[test]
#[ignore = "120-second real FIPS UDP audio/video queue stress"]
fn calls_sustained_audio_playout_survives_video_bursts() {
    let duration_seconds: u64 = std::env::var("IRIS_CALL_SOAK_SECONDS")
        .map(|value| {
            value
                .parse()
                .expect("IRIS_CALL_SOAK_SECONDS must be an integer")
        })
        .unwrap_or(120);
    assert!(
        (120..=3600).contains(&duration_seconds),
        "IRIS_CALL_SOAK_SECONDS must be 120..=3600"
    );
    let duration_us = duration_seconds * 1_000_000;
    let baseline_us = duration_us / 6;
    let recovery_us = duration_us * 3 / 4;
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let (mut a, au, _adir) = logged_in_test_core_with_updates("call-load-a", &ao, &ad);
    let (mut b, bu, _bdir) = logged_in_test_core_with_updates("call-load-b", &bo, &bd);
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
    let address = || {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    };
    let aa = address();
    let ba = address();
    a.reconcile_calls_udp_for_test(aa, ba, &test_fips_peer(&bd).npub());
    b.reconcile_calls_udp_for_test(ba, aa, &test_fips_peer(&ad).npub());
    let ae = a.device_sync_endpoint_for_test().unwrap();
    let be = b.device_sync_endpoint_for_test().unwrap();
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, _| {
        a.runtime.block_on(device_sync_pair_is_connected([
            (&ae, &test_fips_peer(&bd)),
            (&be, &test_fips_peer(&ad)),
        ]))
    });
    a.handle_action(AppAction::StartCall {
        chat_id: bo.public_key().to_hex(),
        video: true,
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state
            .call
            .as_ref()
            .is_some_and(|call| call.phase == "incoming")
    });
    let id = b.state.call.as_ref().unwrap().call_id.clone();
    b.handle_action(AppAction::AnswerCall {
        call_id: id.clone(),
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, b| {
        [a, b].iter().all(|core| {
            core.state
                .call
                .as_ref()
                .is_some_and(|call| call.phase == "connected")
        })
    });
    for core in [&mut a, &mut b] {
        core.handle_action(AppAction::SetCallMediaConnected {
            call_id: id.clone(),
            connected: true,
        });
    }
    au.try_iter().for_each(drop);
    bu.try_iter().for_each(drop);
    let encoder = crate::CallAudioCodec::new().unwrap();
    let receiver = crate::CallAudioCodec::new().unwrap();
    let mut video = vec![42; 262_144];
    video[..5].copy_from_slice(&[0, 0, 0, 1, 0x65]);
    let began = std::time::Instant::now();
    let mut next_frame = 0u64;
    let mut sent = 0usize;
    let mut received = [0usize; 3];
    let mut silent = [0usize; 3];
    let mut max_silent = [0usize; 3];
    let mut silent_run = 0usize;
    let mut latency = [Vec::<u64>::new(), Vec::new(), Vec::new()];
    let mut video_frames = 0usize;
    while began.elapsed() < Duration::from_secs(duration_seconds) {
        let now_us = began.elapsed().as_micros() as u64;
        let stage = if now_us < baseline_us {
            0
        } else if now_us < recovery_us {
            1
        } else {
            2
        };
        pump_call_pair(&mut a, &ar, &mut b, &br);
        for update in bu.try_iter() {
            if let AppUpdate::CallMedia {
                kind,
                sequence,
                timestamp_us,
                data,
                ..
            } = update
            {
                if kind == 1 {
                    received[stage] += 1;
                    latency[stage].push(now_us.saturating_sub(timestamp_us) / 1000);
                    receiver.queue(sequence, data);
                } else if kind == 2 {
                    video_frames += 1;
                }
            }
        }
        au.try_iter().for_each(drop);
        if now_us >= next_frame * 20_000 {
            // Burst eight maximum-sized keyframes before audio, every two
            // seconds. This deliberately exceeds normal camera burst size.
            if stage == 1 && next_frame.is_multiple_of(100) {
                for _ in 0..8 {
                    a.handle_action(AppAction::SendCallMedia {
                        call_id: id.clone(),
                        kind: 2,
                        timestamp_us: now_us,
                        key_frame: true,
                        data: video.clone(),
                    });
                }
            }
            let pcm = (0..960)
                .map(|i| {
                    let t = (next_frame * 960 + i) as f64 / 48_000.0;
                    let phase = (160.0 * t + 4.0 * (t * 5.0).sin()) * 2.0 * std::f64::consts::PI;
                    ((phase.sin() * 6500.0 + (phase * 2.0).sin() * 2200.0)
                        * (0.5 + 0.5 * (t * 11.0).sin().powi(2))) as i16
                })
                .collect();
            a.handle_action(AppAction::SendCallMedia {
                call_id: id.clone(),
                kind: 1,
                timestamp_us: now_us,
                key_frame: false,
                data: encoder.encode(pcm).unwrap(),
            });
            sent += 1;
            let output = receiver.playout();
            if now_us > 1_000_000 && output.iter().all(|value| value.unsigned_abs() <= 20) {
                silent_run += 1;
                silent[stage] += 1;
                max_silent[stage] = max_silent[stage].max(silent_run);
            } else {
                silent_run = 0;
            }
            next_frame += 1;
        }
        assert_eq!(a.state.call.as_ref().unwrap().phase, "connected");
        assert_eq!(b.state.call.as_ref().unwrap().phase, "connected");
        std::thread::sleep(Duration::from_millis(1));
    }
    for (stage, name) in ["baseline", "video-bursts", "recovery"]
        .into_iter()
        .enumerate()
    {
        latency[stage].sort_unstable();
        let samples = &latency[stage];
        let p99 = samples
            .get(samples.len().saturating_sub(1) * 99 / 100)
            .copied()
            .unwrap_or(0);
        println!("CALL_TRANSPORT_STRESS stage={name} audio_received={} silent_ms={} max_silent_ms={} p99_delivery_ms={p99} max_delivery_ms={}",
            received[stage], silent[stage] * 20, max_silent[stage] * 20, samples.last().copied().unwrap_or(0));
    }
    println!("CALL_TRANSPORT_STRESS audio_sent={sent} video_received={video_frames}");
    assert!(
        video_frames > 0,
        "must exercise actual chunked video transport"
    );
    assert!(
        received[0] as u64 >= baseline_us / 20_000 * 9 / 10
            && received[2] as u64 >= (duration_us - recovery_us) / 20_000 * 9 / 10,
        "clean windows must deliver audio"
    );
    assert!(
        max_silent[0] <= 10 && max_silent[2] <= 10,
        "baseline/recovery must not stall"
    );
    assert!(
        max_silent[1] <= 50,
        "video load must not starve audio for a second"
    );
    a.handle_action(AppAction::EndCall { call_id: id });
}
