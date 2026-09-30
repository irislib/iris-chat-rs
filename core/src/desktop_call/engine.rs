use super::audio::{Resampler, VoiceProcessing};
use super::{
    video::{self, VideoDecoder, VideoEncoder},
    DesktopAudioDevices, DesktopCallEvent,
};
use crate::CallAudioCodec;
use cpal::traits::{DeviceTrait, StreamTrait};
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
    external_video: bool,
    bitrate: u32,
    key: u32,
    generation: u64,
}
#[derive(Clone, Default, PartialEq, Eq)]
struct AudioSelection {
    microphone: String,
    speaker: String,
    revision: u64,
}
struct Shared {
    stopped: AtomicBool,
    settings: Mutex<Settings>,
    output: Mutex<VecDeque<(u64, DesktopCallEvent)>>,
    force_key: AtomicBool,
    external_frame: Mutex<Option<image::RgbImage>>,
    audio_selection: Mutex<AudioSelection>,
    audio_devices: Mutex<DesktopAudioDevices>,
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
                external_video: false,
                bitrate: 1_200_000,
                key: 0,
                generation: 0,
            }),
            output: Mutex::new(VecDeque::new()),
            force_key: AtomicBool::new(true),
            external_frame: Mutex::new(None),
            audio_selection: Mutex::new(AudioSelection::default()),
            audio_devices: Mutex::new(DesktopAudioDevices::default()),
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
    pub(super) fn set_external_video(&self, enabled: bool) {
        if let Ok(mut settings) = self.shared.settings.lock() {
            if settings.external_video != enabled {
                settings.external_video = enabled;
                settings.generation = settings.generation.wrapping_add(1);
                self.shared.force_key.store(true, Ordering::Release);
                if let Ok(mut frame) = self.shared.external_frame.lock() {
                    *frame = None;
                }
            }
        }
    }
    pub(super) fn submit_video_frame(&self, width: u32, height: u32, rgba: Vec<u8>) {
        // Capture is untrusted in dimensions and bounded to one newest frame.
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || u64::from(width) * u64::from(height) > 16_777_216
            || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return;
        }
        let Some(initial) = self.shared.settings() else {
            return;
        };
        if !initial.external_video || self.shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
            return;
        };
        let rgb = image::DynamicImage::ImageRgba8(image).into_rgb8();
        let Ok(settings) = self.shared.settings.lock() else {
            return;
        };
        if settings.generation != initial.generation
            || !settings.external_video
            || self.shared.stopped.load(Ordering::Acquire)
        {
            return;
        }
        if let Ok(mut frame) = self.shared.external_frame.lock() {
            *frame = Some(rgb);
        }
    }
    pub(super) fn audio_devices(&self) -> DesktopAudioDevices {
        self.shared
            .audio_devices
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
    pub(super) fn select_audio_devices(&self, microphone: String, speaker: String) {
        let Ok(devices) = self.shared.audio_devices.lock() else {
            return;
        };
        if !devices.microphones.iter().any(|d| d.id == microphone)
            || !devices.speakers.iter().any(|d| d.id == speaker)
        {
            return;
        }
        let Ok(mut selection) = self.shared.audio_selection.lock() else {
            return;
        };
        if selection.microphone == microphone
            && selection.speaker == speaker
            && devices.error.is_none()
        {
            return;
        }
        selection.microphone = microphone;
        selection.speaker = speaker;
        selection.revision = selection.revision.wrapping_add(1);
        if let Ok(mut settings) = self.shared.settings.lock() {
            // Drop capture queued before a user switches microphones. Video shares
            // this generation, so replace any discarded references with a keyframe.
            settings.generation = settings.generation.wrapping_add(1);
            self.shared.force_key.store(true, Ordering::Release);
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
        if let Ok(mut frame) = self.shared.external_frame.lock() {
            *frame = None;
        }
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
    let mut capture_generation = u64::MAX;
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
        let rgb = if s.external_video {
            if let Some(mut old) = camera.take() {
                let _ = old.stop_stream();
            }
            let frame = shared
                .external_frame
                .lock()
                .ok()
                .and_then(|mut frame| frame.take());
            let Some(frame) = frame else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            frame
        } else {
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
            match cam
                .frame()
                .and_then(|frame| frame.decode_image::<RgbFormat>())
            {
                Ok(frame) => frame,
                Err(_) => {
                    shared.fail("Couldn’t read the camera.");
                    return;
                }
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
            || s.generation != capture_generation
            || last_key.elapsed() >= Duration::from_secs(1);
        generation = s.key;
        // An old iteration may consume force_key after a microphone switch;
        // the first retained frame of each capture generation must still be a key.
        capture_generation = s.generation;
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
    failed: Arc<AtomicBool>,
    audio_revision: u64,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let config_rate = config.sample_rate.0;
    let mut resampler = Resampler::new(config_rate, 48000);
    let mut generation = 0;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let Some(s) = shared.settings() else { return };
                if !shared
                    .audio_selection
                    .lock()
                    .is_ok_and(|selection| selection.revision == audio_revision)
                {
                    return;
                }
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
            move |_| {
                failed.store(true, Ordering::Release);
            },
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
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
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
            move |_| {
                failed.store(true, Ordering::Release);
            },
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
struct AudioIo {
    _input: cpal::Stream,
    _output: cpal::Stream,
    failed: Arc<AtomicBool>,
    captured: flume::Receiver<(u64, Vec<f32>)>,
    rendered: flume::Receiver<Vec<f32>>,
    pcm: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    voice: VoiceProcessing,
    resampler: Resampler,
}
fn open_audio(
    shared: &Arc<Shared>,
    input: &cpal::Device,
    output: &cpal::Device,
    audio_revision: u64,
) -> Result<AudioIo, String> {
    let failed = Arc::new(AtomicBool::new(false));
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
        cpal::SampleFormat::F32 => capture::<f32>(
            input,
            &input_config.into(),
            tx,
            shared.clone(),
            failed.clone(),
            audio_revision,
        ),
        cpal::SampleFormat::I16 => capture::<i16>(
            input,
            &input_config.into(),
            tx,
            shared.clone(),
            failed.clone(),
            audio_revision,
        ),
        cpal::SampleFormat::U16 => capture::<u16>(
            input,
            &input_config.into(),
            tx,
            shared.clone(),
            failed.clone(),
            audio_revision,
        ),
        _ => Err("Microphone format is unavailable.".into()),
    }?;
    let rate = output_config.sample_rate().0;
    let output_stream = match output_config.sample_format() {
        cpal::SampleFormat::F32 => playback::<f32>(
            output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
            failed.clone(),
        ),
        cpal::SampleFormat::I16 => playback::<i16>(
            output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
            failed.clone(),
        ),
        cpal::SampleFormat::U16 => playback::<u16>(
            output,
            &output_config.into(),
            pcm.clone(),
            render_tx,
            shared.clone(),
            failed.clone(),
        ),
        _ => Err("Speaker format is unavailable.".into()),
    }?;
    input_stream
        .play()
        .map_err(|_| "Couldn’t start the microphone.")?;
    output_stream
        .play()
        .map_err(|_| "Couldn’t start the speakers.")?;
    Ok(AudioIo {
        _input: input_stream,
        _output: output_stream,
        failed,
        captured: rx,
        rendered: render_rx,
        pcm,
        rate,
        voice: VoiceProcessing::new(),
        resampler: Resampler::new(48000, rate),
    })
}
fn run_audio(shared: &Arc<Shared>, incoming: flume::Receiver<Incoming>) -> Result<(), String> {
    // Device enumeration may query slow drivers. Keep it off the codec/playout loop.
    let (device_tx, device_rx) = flume::bounded(1);
    let device_state = Arc::downgrade(shared);
    thread::spawn(move || {
        let host = cpal::default_host();
        loop {
            if device_state
                .upgrade()
                .is_none_or(|s| s.stopped.load(Ordering::Acquire))
            {
                return;
            }
            let _ = device_tx.try_send((
                super::devices::Devices::scan(&host, true),
                super::devices::Devices::scan(&host, false),
            ));
            for _ in 0..30 {
                if device_tx.is_disconnected() {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    });
    let mut inventory = None;
    let codec = CallAudioCodec::new().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut io: Option<AudioIo> = None;
    let mut capture_generation = 0;
    let mut captured = VecDeque::new();
    let mut ready = false;
    let mut next_retry = Instant::now();
    let mut applied_revision = u64::MAX;
    let mut fingerprint = (String::new(), String::new());
    let mut active_devices = (String::new(), String::new());
    let mut device_error = None;
    while !shared.stopped.load(Ordering::Acquire) {
        let selection = shared
            .audio_selection
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let device_failed = io
            .as_ref()
            .is_some_and(|io| io.failed.load(Ordering::Acquire));
        let refreshed = match device_rx.try_recv() {
            Ok(devices) => {
                inventory = Some(devices);
                true
            }
            Err(_) => false,
        };
        if let Some((microphones, speakers)) = inventory.as_ref().filter(|_| {
            refreshed
                || selection.revision != applied_revision
                || ((device_failed || io.is_none()) && Instant::now() >= next_retry)
        }) {
            let next_fingerprint = (
                microphones.fingerprint(&selection.microphone),
                speakers.fingerprint(&selection.speaker),
            );
            let (mut microphone, mut input) = microphones.selected(&selection.microphone);
            let (mut speaker, mut output) = speakers.selected(&selection.speaker);
            if io.is_none()
                || device_failed
                || selection.revision != applied_revision
                || fingerprint != next_fingerprint
            {
                // Release old handles before opening replacements (ALSA may be exclusive).
                io = None;
                captured.clear();
                device_error = None;
                let mut opened = input
                    .zip(output)
                    .ok_or_else(|| "Connect a microphone and speakers.".to_string())
                    .and_then(|(input, output)| {
                        open_audio(shared, input, output, selection.revision)
                    });
                if opened.is_err() && (!microphone.is_empty() || !speaker.is_empty()) {
                    (microphone, input) = microphones.selected("");
                    (speaker, output) = speakers.selected("");
                    opened = input
                        .zip(output)
                        .ok_or_else(|| "Connect a microphone and speakers.".to_string())
                        .and_then(|(input, output)| {
                            open_audio(shared, input, output, selection.revision)
                        });
                    if opened.is_ok() {
                        device_error = Some("Audio switched to system default.".into());
                    }
                }
                match opened {
                    Ok(opened) => {
                        io = Some(opened);
                        active_devices = (microphone, speaker);
                    }
                    Err(message) => device_error = Some(message),
                }
            }
            if let Ok(mut state) = shared.audio_devices.lock() {
                *state = DesktopAudioDevices {
                    microphones: microphones.options(),
                    speakers: speakers.options(),
                    microphone: active_devices.0.clone(),
                    speaker: active_devices.1.clone(),
                    error: device_error.clone(),
                };
            }
            fingerprint = next_fingerprint;
            applied_revision = selection.revision;
            next_retry = Instant::now() + Duration::from_secs(3);
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
        let Some(io) = io.as_mut() else {
            // Preserve the call and drain compressed media while a headset is unplugged.
            let _ = codec.playout();
            thread::sleep(Duration::from_millis(20));
            continue;
        };
        let Some(s) = shared.settings() else { break };
        if !shared
            .audio_selection
            .lock()
            .is_ok_and(|selection| selection.revision == applied_revision)
        {
            continue;
        }
        if capture_generation != s.generation {
            captured.clear();
            capture_generation = s.generation;
        }
        for (generation, samples) in io.captured.try_iter().take(8) {
            if generation == s.generation && !s.muted {
                captured.extend(samples);
            }
        }
        for played in io.rendered.try_iter().take(16) {
            io.voice.render(&played)?;
        }
        while captured.len() >= 960 {
            let samples = io
                .voice
                .capture(&captured.drain(..960).collect::<Vec<_>>())?;
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
        // Device consumption drives playout: scheduler jitter cannot accumulate delay.
        if io
            .pcm
            .lock()
            .map(|p| p.len() < io.rate as usize / 50)
            .unwrap_or(false)
        {
            let data: Vec<f32> = codec
                .playout()
                .into_iter()
                .map(|v| v as f32 / 32768.0)
                .collect();
            if let Ok(mut buffer) = io.pcm.lock() {
                if buffer.len() > io.rate as usize / 10 {
                    buffer.clear();
                }
                buffer.extend(io.resampler.process(&data));
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
