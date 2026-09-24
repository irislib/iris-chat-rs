use super::*;

fn feedback(f: &mut Fixture, number: u32, highest: Option<u32>, frames: u32, bytes: u32) {
    let mut packet = Signal::new("feedback", CALL_ID, true, false);
    packet.feedback_seq = Some(number);
    packet.video_seq = highest;
    packet.received_frames = Some(frames);
    packet.received_bytes = Some(bytes);
    packet.interval_ms = Some(1000);
    f.core
        .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&packet).unwrap());
}

#[test]
fn stale_video_feedback_cannot_consume_a_valid_feedback_sequence() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    feedback(&mut f, 0, Some(29), 30, 100_000);
    let before = f.snapshot().target_bitrate_bps;
    feedback(&mut f, 1, Some(28), 0, 0);
    assert_eq!(f.snapshot().target_bitrate_bps, before);
    feedback(&mut f, 1, Some(59), 10, 20_000);
    assert!(
        f.snapshot().target_bitrate_bps < before,
        "invalid feedback must not suppress the next valid report"
    );
}

#[test]
fn reporting_boundaries_do_not_look_like_persistent_loss() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    for seq in 0..20 {
        feedback(
            &mut f,
            seq,
            Some(29 + seq * 30),
            if seq == 0 { 29 } else { 30 },
            100_000,
        );
    }
    assert_eq!(f.snapshot().target_bitrate_bps, 2_000_000);
}

#[test]
fn malformed_feedback_does_not_consume_a_valid_sequence_or_change_the_rate() {
    for (frames, bytes, interval) in [
        (301, 10_000, 1000),
        (20, 10_000_001, 1000),
        (20, 10_000, 199),
        (20, 10_000, 5001),
    ] {
        let mut f = Fixture::new();
        f.connected_incoming(true);
        feedback(&mut f, 0, Some(29), 30, 100_000);
        let before = f.snapshot().target_bitrate_bps;
        let mut packet = Signal::new("feedback", CALL_ID, true, false);
        packet.feedback_seq = Some(1);
        packet.video_seq = Some(59);
        packet.received_frames = Some(frames);
        packet.received_bytes = Some(bytes);
        packet.interval_ms = Some(interval);
        f.core
            .handle_call_packet(&f.devices[0], PORT, &serde_json::to_vec(&packet).unwrap());
        assert_eq!(f.snapshot().target_bitrate_bps, before);
        feedback(&mut f, 1, Some(59), 20, 50_000);
        assert_eq!(f.snapshot().target_bitrate_bps, 340_000);
    }
}

#[test]
fn bitrate_handles_wraparound_caps_feedback_outage_and_camera_pause() {
    let mut f = Fixture::new();
    f.connected_incoming(true);
    f.core.set_call_quality("custom".into(), 350_000);
    let active = f.core.calls.active.as_mut().unwrap();
    active.feedback_video_seq = Some(u32::MAX - 14);
    active.last_feedback_seq = Some(u32::MAX);
    feedback(&mut f, 0, Some(15), 30, 35_000);
    assert_eq!(f.snapshot().target_bitrate_bps, 350_000);
    for _ in 0..20 {
        let active = f.core.calls.active.as_mut().unwrap();
        active.video_seq = 60;
        active.last_feedback_at = Clock::now() - Duration::from_secs(4);
        f.core.check_call_feedback();
    }
    assert_eq!(f.snapshot().target_bitrate_bps, 100_000);
    f.core.set_call_video(false);
    feedback(&mut f, 1, Some(45), 30, 35_000);
    assert_eq!(
        f.snapshot().target_bitrate_bps,
        100_000,
        "a paused camera must not probe bandwidth"
    );
    f.core.set_call_video(true);
    for seq in 2..32 {
        feedback(&mut f, seq, Some(45 + (seq - 1) * 30), 30, 35_000);
    }
    assert_eq!(
        f.snapshot().target_bitrate_bps,
        350_000,
        "feedback recovery must restore the selected ceiling"
    );
}

/// Virtual-time packet bottleneck, including audio and transport overhead. The
/// production packetizer, reassembler and authenticated feedback handler run in
/// the loop; synthetic encoded sizes follow the actual controller's target.
#[test]
fn bitrate_tracks_constrained_links_and_recovers_without_persistent_frame_loss() {
    for limited in [256_000u32, 400_000, 800_000] {
        let mut f = Fixture::new();
        f.connected_incoming(true);
        let mut receiver = Assembler::default();
        let began = Clock::now();
        let mut queue = VecDeque::<(f64, Vec<u8>)>::new();
        let mut tail = 0f64;
        let mut frame = 0u32;
        let mut delivered = 0u32;
        let mut last_arrival = 30_000;
        let mut max_gap = 0;
        let mut targets = Vec::new();
        for ms in (0..60_000u64).step_by(5) {
            let capacity = if !(5000..40_000).contains(&ms) {
                4_000_000
            } else {
                limited
            };
            let now = began + Duration::from_millis(ms);
            let mut transmit = |packet: Vec<u8>| {
                let finish =
                    tail.max(ms as f64) + (packet.len() + 80) as f64 * 8000.0 / capacity as f64;
                if finish - ms as f64 <= 120.0 {
                    tail = finish;
                    queue.push_back((finish + 20.0, packet));
                }
            };
            if ms.is_multiple_of(20) {
                transmit(
                    wire::encode(CALL_ID, 1, (ms / 20) as u32, ms * 1000, false, &[42; 80])
                        .remove(0),
                );
            }
            if ms * 30 >= u64::from(frame) * 1000 {
                let key = frame.is_multiple_of(30);
                let size =
                    (f.snapshot().target_bitrate_bps as usize / 8 / 33) * if key { 4 } else { 1 };
                let mut data = vec![42; size.max(5)];
                data[..4].copy_from_slice(&[0, 0, 0, 1]);
                f.core.send_call_media(CALL_ID, 2, ms * 1000, key, data);
                for packet in &f
                    .core
                    .calls
                    .active
                    .as_ref()
                    .unwrap()
                    .sent_video
                    .back()
                    .unwrap()
                    .2
                {
                    transmit(packet.clone());
                }
                frame += 1;
            }
            while queue.front().is_some_and(|(at, _)| *at <= ms as f64) {
                let (_, packet) = queue.pop_front().unwrap();
                if let Some(frame) = receiver.receive(CALL_ID, &packet, now) {
                    if frame.kind == 2 && (30_000..40_000).contains(&ms) {
                        delivered += 1;
                        max_gap = max_gap.max(ms - last_arrival);
                        last_arrival = ms;
                    }
                }
            }
            if ms > 0 && ms.is_multiple_of(1000) {
                let frames = std::mem::take(&mut receiver.received_frames);
                let bytes = std::mem::take(&mut receiver.received_bytes);
                feedback(
                    &mut f,
                    (ms / 1000) as u32,
                    receiver.highest_video,
                    frames,
                    bytes,
                );
                targets.push(f.snapshot().target_bitrate_bps);
            }
        }
        println!("CALL_ADAPTATION bandwidth={limited} delivered={delivered}/300 max_gap_ms={max_gap} targets={targets:?}");
        assert!(
            delivered >= 285,
            "controller must deliver at least 95% of frames at {limited} bps"
        );
        assert!(
            max_gap.max(40_000 - last_arrival) < 250,
            "sustained frame delivery stalled"
        );
        assert!(
            targets[15..40].iter().any(|target| *target < limited),
            "failed to converge below link capacity"
        );
        assert!(
            f.snapshot().target_bitrate_bps > 500_000,
            "failed to recover after bandwidth returns"
        );
    }
}
