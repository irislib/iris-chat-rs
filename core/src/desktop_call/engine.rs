use super::audio::{Resampler, VoiceProcessing};
use super::{
    video::{self, VideoDecoder, VideoEncoder},
    DesktopCallEvent,
};
use crate::CallAudioCodec;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nokhwa::{
    pixel_format::{FormatDecoder, RgbFormat},
    utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType},
    Camera,
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Settings {
    muted: bool,
    video: bool,
    bitrate: u32,
    key: u32,
    generation: u64,
}
struct Shared {
    stopped: AtomicBool,
    settings: Mutex<Settings>,
    output: Mutex<VecDeque<(u64, DesktopCallEvent)>>,
    force_key: AtomicBool,
}
impl Shared {
    fn settings(&self) -> Option<Settings> {
        self.settings.lock().ok().map(|s| *s)
    }
    fn emit(&self, generation: u64, event: DesktopCallEvent) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut out) = self.output.lock() else {
            return;
        };
        if let DesktopCallEvent::Video { local, .. } = &event {
            out.retain(|(_, e)| !matches!(e,DesktopCallEvent::Video{local:l,..} if l==local));
        }
        if out.len() >= 24 {
            self.force_key.store(true, Ordering::Release);
            if matches!(event, DesktopCallEvent::Error { .. }) {
                out.pop_front();
            } else {
                return;
            }
        }
        out.push_back((generation, event));
    }
    fn fail(&self, message: &str) {
        self.emit(
            0,
            DesktopCallEvent::Error {
                message: message.into(),
            },
        );
    }
}
struct Incoming {
    kind: u8,
    seq: u32,
    key: bool,
    data: Vec<u8>,
}
pub(super) struct Engine {
    shared: Arc<Shared>,
    incoming: flume::Sender<Incoming>,
    video: flume::Sender<Incoming>,
}
impl Engine {
    pub(super) fn new() -> Self {
        Self::create(true)
    }
    fn create(devices: bool) -> Self {
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            settings: Mutex::new(Settings {
                muted: true,
                video: false,
                bitrate: 1_200_000,
                key: 0,
                generation: 0,
            }),
            output: Mutex::new(VecDeque::new()),
            force_key: AtomicBool::new(true),
        });
        let (incoming, rx) = flume::bounded(16);
        let (video, video_rx) = flume::bounded(8);
        if devices {
            let state = shared.clone();
            thread::spawn(move || {
                if let Err(error) = run_audio(&state, rx) {
                    state.fail(&error);
                }
            });
            let state = shared.clone();
            thread::spawn(move || run_camera(state));
            let state = shared.clone();
            thread::spawn(move || {
                if let Err(error) = run_video(&state, video_rx) {
                    state.fail(&error);
                }
            });
        }
        Self {
            shared,
            incoming,
            video,
        }
    }
    pub(super) fn configure(&self, muted: bool, video: bool, bitrate: u32, key: u32) {
        if let Ok(mut s) = self.shared.settings.lock() {
            if s.muted != muted || s.video != video {
                s.generation = s.generation.wrapping_add(1);
                self.shared.force_key.store(true, Ordering::Release);
            }
            s.muted = muted;
            s.video = video;
            s.bitrate = bitrate.clamp(100_000, 10_000_000);
            s.key = key;
        }
    }
    pub(super) fn receive(&self, kind: u8, seq: u32, _timestamp: u64, key: bool, data: Vec<u8>) {
        if (kind == 1 && data.len() <= 1275) || (kind == 2 && data.len() <= 262144) {
            let queue = if kind == 1 {
                &self.incoming
            } else {
                &self.video
            };
            if queue
                .try_send(Incoming {
                    kind,
                    seq,
                    key,
                    data,
                })
                .is_err()
                && kind == 2
            {
                self.shared.emit(0, DesktopCallEvent::RequestKeyFrame);
            }
        }
    }
    pub(super) fn poll(&self) -> Vec<DesktopCallEvent> {
        let Some(settings) = self.shared.settings() else {
            return Vec::new();
        };
        let Ok(mut out) = self.shared.output.lock() else {
            return Vec::new();
        };
        out.drain(..)
            .filter_map(|(generation, event)| {
                let allowed = match &event {
                    DesktopCallEvent::Encoded { kind, .. } => {
                        generation == settings.generation
                            && if *kind == 1 {
                                !settings.muted
                            } else {
                                settings.video
                            }
                    }
                    DesktopCallEvent::Video { local: true, .. } => {
                        generation == settings.generation && settings.video
                    }
                    _ => true,
                };
                (allowed && !self.shared.stopped.load(Ordering::Acquire)).then_some(event)
            })
            .collect()
    }
    pub(super) fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        if let Ok(mut out) = self.shared.output.lock() {
            out.clear();
        }
    }
}
fn camera_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    // Select an advertised format. Nokhwa's Closest request requires both the
    // requested pixel format and exact resolution when choosing frame rates.
    formats
        .iter()
        .copied()
        .filter(|f| {
            RgbFormat::FORMATS.contains(&f.format())
                && f.width() > 0
                && f.height() > 0
                && f.frame_rate() > 0
        })
        .min_by_key(|f| {
            (
                f.frame_rate() < 20,
                u64::from(f.width()) * u64::from(f.height()) > 1280 * 720,
                u64::from(f.width().abs_diff(1280)) + u64::from(f.height().abs_diff(720)),
                f.frame_rate().abs_diff(30),
                f.format() != FrameFormat::MJPEG,
            )
        })
}
fn run_camera(shared: Arc<Shared>) {
    let mut camera: Option<Camera> = None;
    let mut encoder: Option<VideoEncoder> = None;
    let began = Instant::now();
    let mut pacer = video::FramePacer::default();
    let mut last_key = Instant::now() - Duration::from_secs(1);
    let mut generation = 0;
    while !shared.stopped.load(Ordering::Acquire) {
        let Some(s) = shared.settings() else { break };
        if !s.video {
            if let Some(mut camera) = camera.take() {
                let _ = camera.stop_stream();
            }
            encoder = None;
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        if camera.is_none() {
            let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
            match Camera::new(CameraIndex::Index(0), format).and_then(|mut c| {
                if let Ok(formats) = c.compatible_camera_formats() {
                    if let Some(format) = camera_format(&formats) {
                        c.set_camera_requset(RequestedFormat::new::<RgbFormat>(
                            RequestedFormatType::Exact(format),
                        ))?;
                    }
                }
                c.open_stream()?;
                Ok(c)
            }) {
                Ok(c) => camera = Some(c),
                Err(_) => {
                    shared.fail("Couldn’t open the camera.");
                    return;
                }
            }
        }
        let Some(cam) = camera.as_mut() else { continue };
        let rgb = match cam
            .frame()
            .and_then(|frame| frame.decode_image::<RgbFormat>())
        {
            Ok(frame) => frame,
            Err(_) => {
                shared.fail("Couldn’t read the camera.");
                return;
            }
        };
        let (w, h, fps) = video::settings(s.bitrate, rgb.width(), rgb.height());
        if !pacer.accept(Instant::now(), fps) {
            continue;
        }
        let captured = began.elapsed().as_micros() as u64;
        let scaled = image::imageops::resize(&rgb, w, h, image::imageops::FilterType::Triangle);
        let key = shared.force_key.swap(false, Ordering::AcqRel)
            || s.key != generation
            || last_key.elapsed() >= Duration::from_secs(1);
        generation = s.key;
        if key {
            last_key = Instant::now();
        }
        if encoder.is_none() {
            encoder = VideoEncoder::new(s.bitrate).ok();
        }
        let Some(enc) = encoder.as_mut() else {
            shared.fail("Couldn’t start call video.");
            return;
        };
        match enc.encode(scaled.as_raw(), w, h, s.bitrate, key) {
            Ok(Some((data, key_frame))) if data.len() <= 262144 => shared.emit(
                s.generation,
                DesktopCallEvent::Encoded {
                    kind: 2,
                    timestamp_us: captured,
                    key_frame,
                    data,
                },
            ),
            Ok(None) => {}
            _ => shared.force_key.store(true, Ordering::Release),
        }
        let rgba = scaled
            .pixels()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        shared.emit(
            s.generation,
            DesktopCallEvent::Video {
                local: true,
                width: w,
                height: h,
                rgba,
            },
        );
    }
    if let Some(mut camera) = camera {
        let _ = camera.stop_stream();
    }
}

fn capture<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: flume::Sender<(u64, Vec<f32>)>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let config_rate = config.sample_rate.0;
    let mut resampler = Resampler::new(config_rate, 48000);
    let mut generation = 0;
    let errors = shared.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let Some(s) = shared.settings() else { return };
                if generation != s.generation {
                    resampler = Resampler::new(config_rate, 48000);
                    generation = s.generation;
                }
                if s.muted || shared.stopped.load(Ordering::Acquire) {
                    return;
                }
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| {
                        frame
                            .iter()
                            .map(|v| cpal::Sample::to_sample::<f32>(*v))
                            .sum::<f32>()
                            / channels as f32
                    })
                    .collect();
                let _ = tx.try_send((s.generation, resampler.process(&mono)));
            },
            move |_| errors.fail("Microphone disconnected."),
            None,
        )
        .map_err(|_| "Couldn’t open the microphone.".into())
}
fn playback<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    pcm: Arc<Mutex<VecDeque<f32>>>,
    rendered: flume::Sender<Vec<f32>>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let errors = shared.clone();
    let mut resampler = Resampler::new(config.sample_rate.0, 48000);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let mut reference = Vec::with_capacity(data.len() / channels);
                if let Ok(mut samples) = pcm.try_lock() {
                    for frame in data.chunks_mut(channels) {
                        let v = if shared.stopped.load(Ordering::Acquire) {
                            0.0
                        } else {
                            samples.pop_front().unwrap_or(0.0)
                        };
                        reference.push(v);
                        for channel in frame {
                            *channel = T::from_sample(v);
                        }
                    }
                } else {
                    reference.resize(data.len() / channels, 0.0);
                    for value in data {
                        *value = T::from_sample(0.0);
                    }
                }
                let _ = rendered.try_send(resampler.process(&reference));
            },
            move |_| errors.fail("Speakers disconnected."),
            None,
        )
        .map_err(|_| "Couldn’t open the speakers.".into())
}
fn run_video(shared: &Arc<Shared>, incoming: flume::Receiver<Incoming>) -> Result<(), String> {
    let mut decoder = VideoDecoder::new().map_err(|e| e.to_string())?;
    let mut ready = false;
    while !shared.stopped.load(Ordering::Acquire) {
        if let Ok(frame) = incoming.recv_timeout(Duration::from_millis(10)) {
            decoder.receive(frame.seq, frame.key, frame.data, Instant::now());
        }
        for event in decoder.drain(Instant::now()) {
            if !ready && matches!(event, DesktopCallEvent::Video { .. }) {
                ready = true;
                shared.emit(0, DesktopCallEvent::Ready);
            }
            shared.emit(0, event);
        }
    }
    Ok(())
}
fn run_audio(shared: &Arc<Shared>, incoming: flume::Receiver<Incoming>) -> Result<(), String> {
    let host = cpal::default_host();
    let input = host.default_input_device().ok_or("No microphone found.")?;
    let output = host.default_output_device().ok_or("No speakers found.")?;
    let input_config = input
        .default_input_config()
        .map_err(|_| "Couldn’t open the microphone.")?;
    let output_config = output
        .default_output_config()
        .map_err(|_| "Couldn’t open the speakers.")?;
    let (tx, rx) = flume::bounded(8);
    let pcm = Arc::new(Mutex::new(VecDeque::new()));
    let (render_tx, render_rx) = flume::bounded(16);
    let input_stream = match input_config.sample_format() {
        cpal::SampleFormat::F32 => capture::<f32>(&input, &input_config.into(), tx, shared.clone()),
        cpal::SampleFormat::I16 => capture::<i16>(&input, &input_config.into(), tx, shared.clone()),
        cpal::SampleFormat::U16 => capture::<u16>(&input, &input_config.into(), tx, shared.clone()),
        _ => Err("Microphone format is unavailable.".into()),
    }?;
    let rate = output_config.sample_rate().0;
    let output_stream = match output_config.sample_format() {
        cpal::SampleFormat::F32 => playback::<f32>(
            &output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
        ),
        cpal::SampleFormat::I16 => playback::<i16>(
            &output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
        ),
        cpal::SampleFormat::U16 => playback::<u16>(
            &output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
        ),
        _ => Err("Speaker format is unavailable.".into()),
    }?;
    input_stream
        .play()
        .map_err(|_| "Couldn’t start the microphone.")?;
    output_stream
        .play()
        .map_err(|_| "Couldn’t start the speakers.")?;
    let codec = CallAudioCodec::new().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut voice = VoiceProcessing::new();
    let mut output_resampler = Resampler::new(48000, rate);
    let mut capture_generation = 0;
    let mut captured = VecDeque::new();
    let mut ready = false;
    while !shared.stopped.load(Ordering::Acquire) {
        let Some(s) = shared.settings() else { break };
        if capture_generation != s.generation {
            captured.clear();
            capture_generation = s.generation;
        }
        for (generation, samples) in rx.try_iter().take(8) {
            if generation == s.generation && !s.muted {
                captured.extend(samples);
            }
        }
        for played in render_rx.try_iter().take(16) {
            voice.render(&played)?;
        }
        while captured.len() >= 960 {
            let samples = voice.capture(&captured.drain(..960).collect::<Vec<_>>())?;
            let data = codec.encode(samples).map_err(|e| e.to_string())?;
            shared.emit(
                s.generation,
                DesktopCallEvent::Encoded {
                    kind: 1,
                    timestamp_us: start.elapsed().as_micros() as u64,
                    key_frame: false,
                    data,
                },
            );
        }
        for frame in incoming.try_iter().take(32) {
            if frame.kind == 1 {
                if !ready && opus::packet::get_nb_samples(&frame.data, 48000).ok() == Some(960) {
                    ready = true;
                    shared.emit(0, DesktopCallEvent::Ready);
                }
                codec.queue(frame.seq, frame.data);
            }
        }
        // Device consumption drives playout: scheduler jitter cannot accumulate delay.
        if pcm
            .lock()
            .map(|p| p.len() < rate as usize / 50)
            .unwrap_or(false)
        {
            let data: Vec<f32> = codec
                .playout()
                .into_iter()
                .map(|v| v as f32 / 32768.0)
                .collect();
            if let Ok(mut buffer) = pcm.lock() {
                if buffer.len() > rate as usize / 10 {
                    buffer.clear();
                }
                buffer.extend(output_resampler.process(&data));
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cameras_without_mjpeg_or_720p_use_an_advertised_decodable_mode() {
        use nokhwa::utils::Resolution;
        let vga = CameraFormat::new(Resolution::new(640, 480), FrameFormat::YUYV, 30);
        assert_eq!(camera_format(&[vga]), Some(vga));
        let hd = CameraFormat::new(Resolution::new(1280, 720), FrameFormat::MJPEG, 30);
        let huge = CameraFormat::new(Resolution::new(3840, 2160), FrameFormat::MJPEG, 30);
        let slow = CameraFormat::new(Resolution::new(1280, 720), FrameFormat::MJPEG, 5);
        assert_eq!(camera_format(&[huge, slow, vga, hd]), Some(hd));
        assert_eq!(camera_format(&[slow, vga]), Some(vga));
        assert_eq!(camera_format(&[]), None);
    }
    #[test]
    fn mute_camera_off_and_stop_discard_queued_capture() {
        let e = Engine::create(false);
        e.configure(false, true, 150_000, 1);
        let generation = e.shared.settings().unwrap().generation;
        for kind in [1, 2] {
            e.shared.emit(
                generation,
                DesktopCallEvent::Encoded {
                    kind,
                    timestamp_us: 0,
                    key_frame: true,
                    data: vec![1],
                },
            );
        }
        e.configure(true, false, 150_000, 1);
        e.configure(false, true, 150_000, 1);
        assert!(e.poll().is_empty(), "pre-mute media escaped after unmute");
        let generation = e.shared.settings().unwrap().generation;
        e.shared.emit(
            generation,
            DesktopCallEvent::Encoded {
                kind: 1,
                timestamp_us: 0,
                key_frame: false,
                data: vec![1],
            },
        );
        assert_eq!(e.poll().len(), 1);
        for _ in 0..100 {
            e.shared.emit(generation, DesktopCallEvent::RequestKeyFrame);
        }
        assert!(e.shared.output.lock().unwrap().len() <= 24);
        e.shared.fail("Microphone disconnected.");
        assert!(
            e.poll()
                .iter()
                .any(|event| matches!(event, DesktopCallEvent::Error { .. })),
            "a full media queue must not hide device failure"
        );
        e.stop();
        e.shared.emit(0, DesktopCallEvent::Ready);
        assert!(e.poll().is_empty());
    }
}
