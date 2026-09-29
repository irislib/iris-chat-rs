//! Virtual time drives the real shared Opus encoder, network queue and device
//! playout cadence. These are continuity/recovery tests, not perceptual MOS or
//! substitutes for sustained hardware/network/echo-cancellation tests.
use super::*;

const FRAME_US: u64 = 20_000;
const TEN_MINUTES: u64 = 600_000_000;

#[derive(Clone, Copy, Debug)]
enum Network {
    Clean,
    Unstable,
}

impl Network {
    fn delivery(self, sequence: u32, sent: u64) -> Option<u64> {
        match self {
            Self::Clean => Some(sent + 40_000),
            Self::Unstable => {
                // About 3% packet loss in short clusters, 100 ms burst every 30 s,
                // and a 600 ms outage every two minutes, then a clean tail.
                let phase = sent % 120_000_000;
                if sent < TEN_MINUTES - 10_000_000
                    && (sequence % 97 < 3
                        || (10_000_000..10_100_000).contains(&(sent % 30_000_000))
                        || (60_000_000..60_600_000).contains(&phase))
                {
                    return None;
                }
                // Bounded jitter also reorders packets; delay changes model a
                // route change. The final 10 s establish independent recovery.
                let jitter = if sent < TEN_MINUTES - 10_000_000 {
                    (u64::from(sequence) * 17 % 5) * 10_000
                } else {
                    0
                };
                let extra = if (30_000_000..40_000_000).contains(&phase) {
                    100_000
                } else {
                    0
                };
                Some(sent + 40_000 + jitter + extra)
            }
        }
    }
}

#[derive(Debug, Default)]
struct Metrics {
    sent: usize,
    dropped: usize,
    dtx: usize,
    late: usize,
    decoded: usize,
    concealed: usize,
    priming: usize,
    silent: usize,
    max_silence: usize,
    max_recovery_silence: usize,
    max_queue: usize,
    clean_tail_concealed: usize,
    clean_tail_silent: usize,
}

fn sustained(network: Network, sender_clock_ppm: i64) -> Metrics {
    let sender = CallAudioCodec::new().unwrap();
    let receiver = CallAudioCodec::new().unwrap();
    let mut transit = BTreeMap::<(u64, u32), Vec<u8>>::new();
    let mut sequence = 0u32;
    let mut metrics = Metrics::default();
    let mut silence_run = 0;
    let mut recovery_run = 0;
    let mut last_received_at = None;
    for now in (0..TEN_MINUTES).step_by(FRAME_US as usize) {
        // A positive ppm means the sender's hardware clock is faster. No sleeps
        // or wall-clock assertions: replaying a scenario produces the same stream.
        loop {
            let sent = (u64::from(sequence) * FRAME_US * 1_000_000)
                / (1_000_000i64 + sender_clock_ppm) as u64;
            if sent > now {
                break;
            }
            let samples = (0..SAMPLES)
                .map(|i| {
                    let t = (u64::from(sequence) * SAMPLES as u64 + i as u64) as f64 / 48_000.0;
                    // A changing voiced signal avoids classifying a steady sine
                    // as background noise (DTX), while keeping reference energy
                    // nonzero so playback gaps are measurable without a MOS model.
                    let pitch = 160.0 * t + 4.0 * (t * 5.0).sin();
                    let phase = pitch * 2.0 * std::f64::consts::PI;
                    let envelope = 0.5 + 0.5 * (t * 11.0).sin().powi(2);
                    ((phase.sin() * 6500.0
                        + (phase * 2.0).sin() * 2200.0
                        + (phase * 3.0).sin() * 1100.0)
                        * envelope) as i16
                })
                .collect();
            let encoded = sender.encode(samples).unwrap();
            metrics.sent += 1;
            metrics.dtx += usize::from(encoded.len() < 8);
            if let Some(arrives) = network.delivery(sequence, sent) {
                transit.insert((arrives, sequence), encoded.clone());
                // Duplicate packets exercise idempotence without doubling media.
                if matches!(network, Network::Unstable) && sequence % 53 == 0 {
                    transit.insert((arrives + 10_000, sequence), encoded);
                }
            } else {
                metrics.dropped += 1;
            }
            sequence += 1;
        }
        while transit
            .first_key_value()
            .is_some_and(|((at, _), _)| *at <= now)
        {
            let ((_, seq), data) = transit.pop_first().unwrap();
            {
                let state = receiver.playout.lock().unwrap();
                if state
                    .next
                    .is_some_and(|next| seq.wrapping_sub(next) >= 1 << 31)
                    && state.priming == 0
                {
                    metrics.late += 1;
                }
            }
            receiver.queue(seq, data);
            last_received_at = Some(now);
        }
        let in_tail = now >= TEN_MINUTES - 8_000_000;
        {
            let state = receiver.playout.lock().unwrap();
            metrics.max_queue = metrics.max_queue.max(state.packets.len());
            if state.next.is_some() && state.priming > 0 {
                metrics.priming += 1;
            } else if let Some(next) = state.next {
                if state.packets.contains_key(&next) {
                    metrics.decoded += 1;
                } else {
                    metrics.concealed += 1;
                    metrics.clean_tail_concealed += usize::from(in_tail);
                }
            }
        }
        let output = receiver.playout();
        assert_eq!(output.len(), SAMPLES);
        // Exclude startup: the first packet plus jitter priming is expected.
        if now < 1_000_000 {
            continue;
        }
        let silent = output.iter().all(|sample| sample.unsigned_abs() <= 20);
        if silent {
            silence_run += 1;
            metrics.silent += 1;
            metrics.clean_tail_silent += usize::from(in_tail);
            // Once a new packet arrives during silence, count all subsequent
            // silent output slots until sound resumes, including jitter between
            // later arrivals rather than resetting the recovery timer each gap.
            if recovery_run > 0 || last_received_at == Some(now) {
                recovery_run += 1;
            }
        } else {
            silence_run = 0;
            recovery_run = 0;
        }
        metrics.max_silence = metrics.max_silence.max(silence_run);
        metrics.max_recovery_silence = metrics.max_recovery_silence.max(recovery_run);
    }
    println!("ten-minute {network:?}, sender clock {sender_clock_ppm:+} ppm: {metrics:?}");
    metrics
}

#[test]
fn ten_minutes_of_clean_audio_have_no_playout_gaps() {
    let metrics = sustained(Network::Clean, 0);
    assert_eq!(
        metrics.dtx, 0,
        "the voiced reference must keep encoding audio"
    );
    assert_eq!(metrics.silent, 0);
    assert_eq!(metrics.concealed, 0);
    assert_eq!(metrics.late, 0);
    assert!(metrics.max_queue <= 12);
}

#[test]
fn ten_minutes_of_bursts_reordering_and_outages_recover_without_a_stuck_playout() {
    let metrics = sustained(Network::Unstable, 0);
    assert!(metrics.dropped > 1000, "the scenario must exercise loss");
    assert!(metrics.concealed > 0 && metrics.late > 0);
    assert!(
        metrics.max_silence <= 40,
        "600 ms outage must recover within 800 ms: {metrics:?}"
    );
    assert!(
        metrics.max_recovery_silence <= 10,
        "arriving packets must resume playback within 200 ms: {metrics:?}"
    );
    assert_eq!(metrics.clean_tail_silent, 0);
    assert_eq!(metrics.clean_tail_concealed, 0);
    assert!(metrics.max_queue <= 12);
}

#[test]
fn ten_minutes_of_device_clock_drift_keep_playout_bounded() {
    for ppm in [-100, 100] {
        let metrics = sustained(Network::Clean, ppm);
        assert!(metrics.max_queue <= 12);
        assert!(
            metrics.max_silence <= 10,
            "small clock drift must not stall playback: {metrics:?}"
        );
    }
}
