//! Outgoing progress sounds. This output-only worker never opens capture devices
//! or blocks the UI while the operating system opens an audio stream.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[derive(uniffi::Object)]
pub struct DesktopCallTone {
    state: Arc<AtomicU8>,
}
#[uniffi::export]
impl DesktopCallTone {
    #[uniffi::constructor]
    pub fn new(ringing: bool) -> Arc<Self> {
        let state = Arc::new(AtomicU8::new(if ringing { 2 } else { 1 }));
        #[cfg(feature = "desktop-media")]
        {
            let state = state.clone();
            std::thread::spawn(move || play(state));
        }
        Arc::new(Self { state })
    }
    pub fn set_ringing(&self, ringing: bool) {
        // Stop is terminal: a stale UI snapshot cannot revive a retired worker.
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                (old != 0).then_some(if ringing { 2 } else { 1 })
            });
    }
    pub fn stop(&self) {
        self.state.store(0, Ordering::Release);
    }
}
impl Drop for DesktopCallTone {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(feature = "desktop-media")]
fn play(state: Arc<AtomicU8>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let Some(device) = cpal::default_host().default_output_device() else {
        return;
    };
    let Ok(config) = device.default_output_config() else {
        return;
    };
    if state.load(Ordering::Acquire) == 0 {
        return;
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config.into(), state.clone()),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config.into(), state.clone()),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config.into(), state.clone()),
        _ => return,
    };
    let Ok(stream) = stream else { return };
    if stream.play().is_err() {
        return;
    }
    while state.load(Ordering::Acquire) != 0 {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(feature = "desktop-media")]
fn stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<AtomicU8>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;
    let mut position = 0.0;
    let mut last = 0;
    let failed = state.clone();
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let tone = state.load(Ordering::Acquire);
            if tone != last {
                position = 0.0;
                last = tone;
            }
            for frame in output.chunks_mut(channels) {
                let value = sample(tone, position as usize);
                position += 16_000.0 / rate;
                frame.fill(T::from_sample(value));
            }
        },
        move |_| {
            failed.store(0, Ordering::Release);
        },
        None,
    )
}

#[cfg(any(feature = "desktop-media", test))]
fn sample(tone: u8, position: usize) -> f32 {
    let wav: &[u8] = match tone {
        1 => include_bytes!("../../assets/call-audio/call-connecting.wav"),
        2 => include_bytes!("../../assets/call-audio/call-ringing.wav"),
        _ => return 0.0,
    };
    // Canonical mono 16-bit PCM files from scripts/generate-call-tones.py.
    let index = 44 + (position % ((wav.len() - 44) / 2)) * 2;
    f32::from(i16::from_le_bytes([wav[index], wav[index + 1]])) / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outgoing_tones_have_distinct_cadence_and_stop_is_terminal() {
        assert!((0..1_920).any(|i| sample(1, i).abs() > 0.1));
        assert!((8_000..32_000).all(|i| sample(1, i) == 0.0));
        assert!((16_000..32_000).any(|i| sample(2, i).abs() > 0.1));
        assert!((32_000..96_000).all(|i| sample(2, i) == 0.0));
        assert_eq!(sample(2, 123), sample(2, 96_123));
        let tone = DesktopCallTone {
            state: Arc::new(AtomicU8::new(1)),
        };
        tone.set_ringing(true);
        assert_eq!(tone.state.load(Ordering::Acquire), 2);
        tone.stop();
        tone.set_ringing(false);
        assert_eq!(tone.state.load(Ordering::Acquire), 0);
        assert_eq!(sample(0, 123), 0.0);
    }
}
