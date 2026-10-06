//! Desktop capture/playout. The engine owns no sockets: compressed media is
//! handed back to the existing authenticated FIPS call actions.
mod tones;
use std::sync::Arc;
#[cfg(not(feature = "desktop-media"))]
use std::sync::Mutex;
pub use tones::DesktopCallTone;
#[cfg(feature = "desktop-media")]
mod audio;
#[cfg(feature = "desktop-media")]
mod devices;
#[cfg(feature = "desktop-media")]
mod engine;
#[cfg(feature = "desktop-media")]
mod video;
#[cfg(feature = "desktop-media")]
mod video_rgb;

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DesktopAudioDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct DesktopAudioDevices {
    pub microphones: Vec<DesktopAudioDevice>,
    pub speakers: Vec<DesktopAudioDevice>,
    pub microphone: String,
    pub speaker: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum DesktopCallEvent {
    Encoded {
        kind: u8,
        timestamp_us: u64,
        key_frame: bool,
        data: Vec<u8>,
    },
    Video {
        local: bool,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Ready,
    RequestKeyFrame,
    Error {
        message: String,
    },
}

#[derive(uniffi::Object)]
pub struct DesktopCallMedia {
    #[cfg(feature = "desktop-media")]
    engine: engine::Engine,
    #[cfg(not(feature = "desktop-media"))]
    error: Mutex<bool>,
}
#[uniffi::export]
impl DesktopCallMedia {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            #[cfg(feature = "desktop-media")]
            engine: engine::Engine::new(),
            #[cfg(not(feature = "desktop-media"))]
            error: Mutex::new(true),
        })
    }
    pub fn configure(&self, muted: bool, video: bool, bitrate: u32, key_generation: u32) {
        #[cfg(feature = "desktop-media")]
        self.engine.configure(muted, video, bitrate, key_generation);
        #[cfg(not(feature = "desktop-media"))]
        let _ = (muted, video, bitrate, key_generation);
    }
    /// Select a platform-owned screen capture source instead of the camera.
    /// The platform must obtain explicit user consent before enabling this.
    pub fn set_external_video(&self, enabled: bool) {
        #[cfg(feature = "desktop-media")]
        self.engine.set_external_video(enabled);
        #[cfg(not(feature = "desktop-media"))]
        let _ = enabled;
    }
    /// Supplies RGBA pixels to the existing bounded H.264 video worker.
    pub fn submit_video_frame(&self, width: u32, height: u32, rgba: Vec<u8>) {
        #[cfg(feature = "desktop-media")]
        self.engine.submit_video_frame(width, height, rgba);
        #[cfg(not(feature = "desktop-media"))]
        let _ = (width, height, rgba);
    }
    /// Device discovery and switching run on the audio worker, never on the UI thread.
    pub fn audio_devices(&self) -> DesktopAudioDevices {
        #[cfg(feature = "desktop-media")]
        {
            self.engine.audio_devices()
        }
        #[cfg(not(feature = "desktop-media"))]
        {
            DesktopAudioDevices::default()
        }
    }
    pub fn select_audio_devices(&self, microphone: String, speaker: String) {
        #[cfg(feature = "desktop-media")]
        self.engine.select_audio_devices(microphone, speaker);
        #[cfg(not(feature = "desktop-media"))]
        let _ = (microphone, speaker);
    }
    pub fn receive(
        &self,
        kind: u8,
        sequence: u32,
        timestamp_us: u64,
        key_frame: bool,
        data: Vec<u8>,
    ) {
        #[cfg(feature = "desktop-media")]
        self.engine
            .receive(kind, sequence, timestamp_us, key_frame, data);
        #[cfg(not(feature = "desktop-media"))]
        let _ = (kind, sequence, timestamp_us, key_frame, data);
    }
    /// Bounded output; a slow UI cannot accumulate old capture or video frames.
    pub fn poll(&self) -> Vec<DesktopCallEvent> {
        #[cfg(feature = "desktop-media")]
        {
            self.engine.poll()
        }
        #[cfg(not(feature = "desktop-media"))]
        {
            if let Ok(mut error) = self.error.lock() {
                if std::mem::take(&mut *error) {
                    return vec![DesktopCallEvent::Error {
                        message: "Calls are unavailable in this build.".into(),
                    }];
                }
            }
            Vec::new()
        }
    }
    pub fn stop(&self) {
        #[cfg(feature = "desktop-media")]
        self.engine.stop();
    }
}
impl Drop for DesktopCallMedia {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Device-free interoperability fixture. It decodes the peer's real H.264 and
/// re-encodes it with the exact desktop codecs and current core bitrate target.
#[cfg(all(feature = "desktop-media", feature = "stack-fixture"))]
pub struct DesktopVideoTranscoder {
    decoder: video::VideoDecoder,
    encoder: video::VideoEncoder,
    pacer: video::FramePacer,
    key_generation: u32,
}
#[cfg(all(feature = "desktop-media", feature = "stack-fixture"))]
impl DesktopVideoTranscoder {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: video::VideoDecoder::new().map_err(|e| e.to_string())?,
            encoder: video::VideoEncoder::new(1_200_000).map_err(|e| e.to_string())?,
            pacer: video::FramePacer::default(),
            key_generation: 0,
        })
    }
    pub fn receive(
        &mut self,
        seq: u32,
        key: bool,
        bytes: Vec<u8>,
        target: u32,
        generation: u32,
    ) -> Vec<DesktopCallEvent> {
        let now = std::time::Instant::now();
        self.decoder.receive(seq, key, bytes, now);
        let mut output = Vec::new();
        for event in self.decoder.drain(now) {
            match event {
                DesktopCallEvent::Video {
                    width,
                    height,
                    rgba,
                    ..
                } => {
                    let (w, h, fps) = video::settings(target, width, height);
                    if !self.pacer.accept(now, fps) {
                        continue;
                    }
                    let Some(image) = image::RgbaImage::from_raw(width, height, rgba) else {
                        continue;
                    };
                    let scaled = image::DynamicImage::ImageRgba8(image)
                        .resize_exact(w, h, image::imageops::FilterType::Triangle)
                        .into_rgb8();
                    if let Ok(Some((data, key_frame))) = self.encoder.encode(
                        scaled.as_raw(),
                        w,
                        h,
                        target,
                        self.key_generation != generation,
                    ) {
                        self.key_generation = generation;
                        output.push(DesktopCallEvent::Encoded {
                            kind: 2,
                            timestamp_us: 0,
                            key_frame,
                            data,
                        });
                    }
                }
                DesktopCallEvent::RequestKeyFrame => output.push(event),
                _ => {}
            }
        }
        output
    }
}
