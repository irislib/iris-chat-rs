/// Real production call actions, encryption, UDP, reassembly, and feedback.
/// Payloads model compressed frame sizes; native codec quality is measured separately.
#[test]
fn calls_sustained_video_quality_over_local_fips_udp() {
    let ao = Keys::generate();
    let ad = Keys::generate();
    let bo = Keys::generate();
    let bd = Keys::generate();
    let (mut a, au, _adir) = logged_in_test_core_with_updates("quality-a", &ao, &ad);
    let (mut b, bu, _bdir) = logged_in_test_core_with_updates("quality-b", &bo, &bd);
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
    let sockets: Vec<_> = (0..2)
        .map(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap())
        .collect();
    let aa = sockets[0].local_addr().unwrap();
    let ba = sockets[1].local_addr().unwrap();
    drop(sockets);
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
    for (core, endpoint, peer) in [(&a, &ae, &bd), (&b, &be, &ad)] {
        assert_eq!(
            core.runtime
                .block_on(device_sync_peer_transport(endpoint, &test_fips_peer(peer)))
                .as_deref(),
            Some("udp")
        );
    }
    a.handle_action(AppAction::StartCall {
        chat_id: bo.public_key().to_hex(),
        video: true,
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |_, b| {
        b.state.call.as_ref().is_some_and(|c| c.phase == "incoming")
    });
    let id = b.state.call.as_ref().unwrap().call_id.clone();
    b.handle_action(AppAction::AnswerCall {
        call_id: id.clone(),
    });
    wait_call_pair(&mut a, &ar, &mut b, &br, |a, _| {
        a.state
            .call
            .as_ref()
            .is_some_and(|c| c.phase == "connected")
    });

    let start = std::time::Instant::now();
    let mut video_sent = 0u64;
    let mut audio_sent = 0u64;
    let mut latencies = [Vec::new(), Vec::new()];
    let mut video_times = [Vec::new(), Vec::new()];
    let mut keys = [0usize; 2];
    let mut audio_received = [0usize; 2];
    let mut audio_latencies = [Vec::new(), Vec::new()];
    const FRAMES: u64 = 180;
    while start.elapsed() < Duration::from_secs(7) {
        let elapsed = start.elapsed();
        if video_sent < FRAMES && elapsed.as_micros() >= u128::from(video_sent * 1_000_000 / 30) {
            let key = video_sent.is_multiple_of(30);
            // A large IDR burst once a second, with 30 fps and concurrent audio.
            let mut data = vec![42; if key { 262_144 } else { 6_000 }];
            data[..4].copy_from_slice(&[0, 0, 0, 1]);
            for core in [&mut a, &mut b] {
                core.handle_action(AppAction::SendCallMedia {
                    call_id: id.clone(),
                    kind: 2,
                    timestamp_us: elapsed.as_micros() as u64,
                    key_frame: key,
                    data: data.clone(),
                });
            }
            video_sent += 1;
            // Model ordinary core/UI scheduling pauses, including at an IDR burst.
            if key {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        if audio_sent < 300 && elapsed.as_micros() >= u128::from(audio_sent * 20_000) {
            for core in [&mut a, &mut b] {
                core.handle_action(AppAction::SendCallMedia {
                    call_id: id.clone(),
                    kind: 1,
                    timestamp_us: elapsed.as_micros() as u64,
                    key_frame: false,
                    data: vec![42; 160],
                });
            }
            audio_sent += 1;
        }
        pump_call_pair(&mut a, &ar, &mut b, &br);
        for (side, updates) in [&au, &bu].into_iter().enumerate() {
            for update in updates.try_iter() {
                if let AppUpdate::CallMedia {
                    kind,
                    timestamp_us,
                    key_frame,
                    ..
                } = update
                {
                    let now = start.elapsed();
                    let latency = now
                        .saturating_sub(Duration::from_micros(timestamp_us))
                        .as_secs_f64()
                        * 1000.0;
                    if kind == 2 {
                        latencies[side].push(latency);
                        video_times[side].push(now.as_secs_f64() * 1000.0);
                        keys[side] += usize::from(key_frame);
                    } else {
                        audio_received[side] += 1;
                        audio_latencies[side].push(latency);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    a.stop_device_sync_now();
    b.stop_device_sync_now();
    assert_eq!(video_sent, FRAMES);
    assert_eq!(audio_sent, 300);
    for side in 0..2 {
        latencies[side].sort_by(f64::total_cmp);
        audio_latencies[side].sort_by(f64::total_cmp);
        let percentile = |values: &[f64]| {
            values
                .get(values.len() * 95 / 100)
                .copied()
                .unwrap_or(f64::INFINITY)
        };
        let p95 = percentile(&latencies[side]);
        let audio_p95 = percentile(&audio_latencies[side]);
        // Include startup and tail freezes, so a stalled stream cannot look healthy.
        let mut times = vec![0.0];
        times.extend(&video_times[side]);
        times.push(6000.0_f64.max(*times.last().unwrap()));
        let max_gap = times.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
        println!(
            "CALL_QUALITY {}",
            serde_json::json!({
                "side": side, "sent": video_sent, "received": latencies[side].len(),
                "fps": latencies[side].len() as f64 / 6.0, "video_p95_ms": p95,
                "max_video_gap_ms": max_gap, "keyframes": keys[side],
                "audio_received": audio_received[side], "audio_p95_ms": audio_p95,
            })
        );
        assert!(
            latencies[side].len() >= 177,
            "video loss exceeds 2% on loopback"
        );
        assert_eq!(
            keys[side], 6,
            "missing IDRs can freeze dependent video for a second"
        );
        assert!(p95 < 150.0, "video latency budget exceeded: {p95} ms");
        assert!(max_gap < 250.0, "video freeze: {max_gap} ms");
        assert!(audio_received[side] >= 294, "video must not starve audio");
        assert!(
            audio_p95 < 100.0,
            "audio latency budget exceeded: {audio_p95} ms"
        );
    }
}
