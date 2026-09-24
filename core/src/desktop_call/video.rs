use super::DesktopCallEvent;
use openh264::{
    decoder::Decoder,
    encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Profile,
        RateControlMode, UsageType,
    },
    formats::{RgbSliceU8, YUVBuffer, YUVSource},
    OpenH264API,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(super) fn settings(bitrate: u32, width: u32, height: u32) -> (u32, u32, u32) {
    let (edge, fps) = match bitrate {
        0..200_000 => (320, 10),
        200_000..500_000 => (640, 15),
        500_000..1_000_000 => (960, 30),
        _ => (1280, 30),
    };
    let scale = (edge as f64 / width.max(height).max(1) as f64).min(1.0);
    (
        ((width as f64 * scale) as u32 / 2 * 2).max(2),
        ((height as f64 * scale) as u32 / 2 * 2).max(2),
        fps,
    )
}
#[derive(Default)]
pub(super) struct FramePacer {
    next: Option<Instant>,
    fps: u32,
}
impl FramePacer {
    pub(super) fn accept(&mut self, now: Instant, fps: u32) -> bool {
        if self.fps != fps {
            self.next = None;
            self.fps = fps;
        }
        if self.next.is_some_and(|next| now < next) {
            return false;
        }
        let interval = Duration::from_nanos(1_000_000_000 / fps.max(1) as u64);
        self.next = Some(match self.next {
            Some(next) if now.saturating_duration_since(next) < interval => next + interval,
            _ => now + interval,
        });
        true
    }
}
pub(super) struct VideoEncoder {
    encoder: Encoder,
    bitrate: u32,
    dimensions: Option<(u32, u32)>,
}
impl VideoEncoder {
    pub(super) fn new(bitrate: u32) -> Result<Self, openh264::Error> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(FrameRate::from_hz(settings(bitrate, 1280, 720).2 as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .profile(Profile::Baseline)
            .usage_type(UsageType::CameraVideoRealTime)
            .intra_frame_period(IntraFramePeriod::from_num_frames(30))
            .scene_change_detect(false)
            .num_threads(1);
        Ok(Self {
            encoder: Encoder::with_api_config(OpenH264API::from_source(), config)?,
            bitrate,
            dimensions: None,
        })
    }
    pub(super) fn encode(
        &mut self,
        rgb: &[u8],
        width: u32,
        height: u32,
        bitrate: u32,
        key: bool,
    ) -> Result<Option<(Vec<u8>, bool)>, openh264::Error> {
        if self.dimensions.is_some_and(|size| size != (width, height)) {
            *self = Self::new(bitrate)?;
        }
        if self.bitrate != bitrate {
            if settings(self.bitrate, width, height).2 != settings(bitrate, width, height).2 {
                *self = Self::new(bitrate)?;
            } else {
                // OpenH264's per-layer maximum starts equal to the target.
                // Raise the maximum first when probing up; lower the target
                // first when backing off, so both updates remain valid.
                let target = (
                    openh264_sys2::ENCODER_OPTION_BITRATE,
                    openh264_sys2::SPATIAL_LAYER_ALL,
                );
                let maximum = (
                    openh264_sys2::ENCODER_OPTION_MAX_BITRATE,
                    openh264_sys2::SPATIAL_LAYER_0,
                );
                let options = if bitrate > self.bitrate {
                    [maximum, target]
                } else {
                    [target, maximum]
                };
                let mut result = 0;
                for (option, layer) in options {
                    let mut value = openh264_sys2::SBitrateInfo {
                        iLayer: layer,
                        iBitrate: bitrate as i32,
                    };
                    // SAFETY: SetOption consumes this stack value synchronously.
                    // Rate changes preserve the wrapper's dimensions/buffers.
                    result = unsafe {
                        self.encoder
                            .raw_api()
                            .set_option(option, std::ptr::from_mut(&mut value).cast())
                    };
                    if result != 0 {
                        break;
                    }
                }
                if result != 0 {
                    *self = Self::new(bitrate)?;
                }
                self.bitrate = bitrate;
            }
        }
        if key {
            self.encoder.force_intra_frame();
        }
        let yuv =
            YUVBuffer::from_rgb8_source(RgbSliceU8::new(rgb, (width as usize, height as usize)));
        self.dimensions = Some((width, height));
        let encoded = self.encoder.encode(&yuv)?;
        let key = matches!(encoded.frame_type(), FrameType::IDR | FrameType::I);
        let bytes = encoded.to_vec();
        Ok((!bytes.is_empty()).then_some((bytes, key)))
    }
}
struct Frame {
    at: Instant,
    key: bool,
    bytes: Vec<u8>,
}
pub(super) struct VideoDecoder {
    decoder: Decoder,
    pending: HashMap<u32, Frame>,
    next: Option<u32>,
    needs_key: bool,
}
impl VideoDecoder {
    pub(super) fn new() -> Result<Self, openh264::Error> {
        Ok(Self {
            decoder: Decoder::new()?,
            pending: HashMap::new(),
            next: None,
            needs_key: true,
        })
    }
    pub(super) fn receive(&mut self, seq: u32, key: bool, bytes: Vec<u8>, now: Instant) {
        if bytes.is_empty()
            || bytes.len() > 262144
            || self
                .next
                .is_some_and(|next| seq.wrapping_sub(next) >= 1 << 31)
        {
            return;
        }
        if self.next.is_none() && key {
            self.pending
                .retain(|number, _| number.wrapping_sub(seq) < 1 << 31);
            self.next = Some(seq);
        }
        if self.pending.len() >= 8 {
            return;
        }
        self.pending.entry(seq).or_insert(Frame {
            at: now,
            key,
            bytes,
        });
    }
    pub(super) fn drain(&mut self, now: Instant) -> Vec<DesktopCallEvent> {
        let mut out = Vec::new();
        if !self
            .next
            .is_some_and(|next| self.pending.contains_key(&next))
            && (self.pending.len() >= 8
                || self
                    .pending
                    .values()
                    .any(|f| now.saturating_duration_since(f.at) >= Duration::from_millis(200)))
        {
            self.needs_key = true;
            out.push(DesktopCallEvent::RequestKeyFrame);
            let base = self
                .next
                .unwrap_or_else(|| self.pending.keys().copied().min().unwrap_or(0));
            self.next = self
                .pending
                .iter()
                .filter(|(_, f)| f.key)
                .min_by_key(|(seq, _)| seq.wrapping_sub(base))
                .map(|(seq, _)| *seq);
            if let Some(next) = self.next {
                self.pending
                    .retain(|seq, _| seq.wrapping_sub(next) < 1 << 31);
            } else {
                self.pending.clear();
            }
        }
        while let Some(frame) = self.next.and_then(|seq| self.pending.remove(&seq)) {
            self.next = self.next.map(|seq| seq.wrapping_add(1));
            if self.needs_key && !frame.key {
                continue;
            }
            match self.decoder.decode(&frame.bytes) {
                Ok(Some(decoded)) => {
                    let (w, h) = decoded.dimensions();
                    if w * h > 1920 * 1080 {
                        self.needs_key = true;
                        continue;
                    }
                    let mut rgba = vec![0; w * h * 4];
                    decoded.write_rgba8(&mut rgba);
                    self.needs_key = false;
                    out.push(DesktopCallEvent::Video {
                        local: false,
                        width: w as u32,
                        height: h as u32,
                        rgba,
                    });
                }
                Ok(None) => {}
                Err(_) => {
                    self.needs_key = true;
                    out.push(DesktopCallEvent::RequestKeyFrame);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rgb(w: u32, h: u32, index: u32) -> Vec<u8> {
        (0..w * h * 3)
            .map(|i| (((i / 3 / w + index * 5) % 80) * 3 + (i / 3 % w + index * 8) % 120) as u8)
            .collect()
    }
    #[test]
    fn fractional_camera_rates_do_not_reduce_target_frame_rate() {
        let now = Instant::now();
        for source in [24u64, 25, 30, 60] {
            let mut pacer = FramePacer::default();
            let count = (0..source * 10)
                .filter(|i| {
                    pacer.accept(now + Duration::from_nanos(i * 1_000_000_000 / source), 10)
                })
                .count();
            assert!(
                (99..=100).contains(&count),
                "camera {source}: {count} frames"
            );
        }
    }
    #[test]
    fn live_rate_reduction_and_repaired_reference_decode() {
        let mut encoder = VideoEncoder::new(1_200_000).unwrap();
        let mut decoder = VideoDecoder::new().unwrap();
        let mut seq = 0;
        let mut now = Instant::now();
        for target in [1_200_000, 150_000, 1_200_000] {
            let (w, h, fps) = settings(target, 1280, 720);
            let mut bytes = 0;
            let mut decoded = 0;
            let mut held = None;
            let duration = 8;
            for index in 0..fps * duration {
                now += Duration::from_micros(1_000_000 / fps as u64);
                if let Some((data, key)) = encoder
                    .encode(&rgb(w, h, index), w, h, target, index % fps == 0)
                    .unwrap()
                {
                    bytes += data.len();
                    if index == 0 {
                        held = Some((seq, key, data));
                    } else {
                        decoder.receive(seq, key, data, now);
                    }
                    seq = seq.wrapping_add(1);
                }
                if index == 1 {
                    if let Some((seq, key, data)) = held.take() {
                        decoder.receive(seq, key, data, now);
                    }
                }
                decoded+=decoder.drain(now).iter().filter(|e|matches!(e,DesktopCallEvent::Video{width,height,..} if *width==w && *height==h)).count();
            }
            let actual = bytes * 8 / duration as usize;
            println!("DESKTOP_CODEC target={target} actual_bps={actual} decoded_fps={} dimensions={w}x{h}",decoded as f64/duration as f64);
            assert!(
                actual < target as usize * 135 / 100,
                "bitrate ceiling {actual} > {target}"
            );
            assert!(
                decoded >= (fps * duration * 7 / 10) as usize,
                "insufficient decoded frames: {decoded}"
            );
        }
    }
    #[test]
    fn live_bitrate_updates_keep_references_and_tier_changes_start_with_idr() {
        let mut encoder = VideoEncoder::new(600_000).unwrap();
        let mut keys = 0;
        for i in 0..12 {
            let target = if i % 2 == 0 {
                600_000 + i * 1000
            } else {
                550_000 + i * 1000
            };
            if let Some((_, key)) = encoder
                .encode(&rgb(960, 540, i), 960, 540, target, false)
                .unwrap()
            {
                keys += usize::from(key);
            }
        }
        assert_eq!(
            keys, 1,
            "bitrate probes must not repeatedly reset the encoder"
        );
        let (_, key) = encoder
            .encode(&rgb(1280, 720, 12), 1280, 720, 1_200_000, false)
            .unwrap()
            .unwrap();
        assert!(
            key,
            "new resolution must begin with independently decodable video"
        );
    }
    #[test]
    fn joining_at_a_later_key_does_not_leave_stale_repair_debt() {
        let mut encoder = VideoEncoder::new(150_000).unwrap();
        let mut decoder = VideoDecoder::new().unwrap();
        let now = Instant::now();
        for i in 0..4 {
            decoder.receive(i, false, vec![1], now);
        }
        let (key, _) = encoder
            .encode(&rgb(320, 180, 0), 320, 180, 150_000, true)
            .unwrap()
            .unwrap();
        decoder.receive(4, true, key, now);
        assert!(decoder
            .drain(now)
            .iter()
            .any(|e| matches!(e, DesktopCallEvent::Video { .. })));
        assert!(
            decoder.drain(now + Duration::from_millis(250)).is_empty(),
            "obsolete pre-key frames triggered a spurious recovery"
        );
    }
    #[test]
    fn unrepairable_loss_is_bounded_and_recovers_at_keyframe() {
        let mut encoder = VideoEncoder::new(150_000).unwrap();
        let mut decoder = VideoDecoder::new().unwrap();
        let now = Instant::now();
        let (key, _) = encoder
            .encode(&rgb(320, 180, 0), 320, 180, 150_000, true)
            .unwrap()
            .unwrap();
        decoder.receive(u32::MAX, true, key, now);
        assert!(decoder
            .drain(now)
            .iter()
            .any(|e| matches!(e, DesktopCallEvent::Video { .. })));
        for seq in 1..100 {
            decoder.receive(seq, false, vec![1; 100], now);
        }
        assert!(decoder.pending.len() <= 8);
        assert!(decoder
            .drain(now + Duration::from_millis(201))
            .iter()
            .any(|e| matches!(e, DesktopCallEvent::RequestKeyFrame)));
        let (key, _) = encoder
            .encode(&rgb(320, 180, 100), 320, 180, 150_000, true)
            .unwrap()
            .unwrap();
        decoder.receive(100, true, key, now + Duration::from_millis(220));
        assert!(decoder
            .drain(now + Duration::from_millis(220))
            .iter()
            .any(|e| matches!(e, DesktopCallEvent::Video { .. })));
    }
}
