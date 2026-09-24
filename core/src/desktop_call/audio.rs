use sonora::{
    config::{EchoCanceller, HighPassFilter, NoiseSuppression},
    AudioProcessing, Config, StreamConfig,
};
use std::collections::VecDeque;

/// Carries interpolation phase across device callbacks (including 44.1 kHz).
/// Without this, rounding each callback loses samples continuously.
pub(super) struct Resampler {
    from: u32,
    to: u32,
    phase: u64,
    samples: VecDeque<f32>,
}
impl Resampler {
    pub(super) fn new(from: u32, to: u32) -> Self {
        Self {
            from,
            to,
            phase: 0,
            samples: VecDeque::new(),
        }
    }
    pub(super) fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.from == self.to {
            return input.to_vec();
        }
        self.samples.extend(input);
        let mut out = Vec::new();
        while self.phase / (self.to as u64) + 1 < self.samples.len() as u64 {
            let index = (self.phase / self.to as u64) as usize;
            let t = (self.phase % self.to as u64) as f32 / self.to as f32;
            let Some((&before, &after)) = self.samples.get(index).zip(self.samples.get(index + 1))
            else {
                break;
            };
            out.push(before * (1.0 - t) + after * t);
            self.phase += self.from as u64;
        }
        let consumed = ((self.phase / self.to as u64) as usize).min(self.samples.len());
        self.samples.drain(..consumed);
        self.phase -= consumed as u64 * self.to as u64;
        out
    }
}

/// WebRTC AEC3 and noise filtering only; this library owns no transport.
pub(super) struct VoiceProcessing {
    processor: AudioProcessing,
    render: VecDeque<f32>,
}
impl VoiceProcessing {
    pub(super) fn new() -> Self {
        let config = Config {
            echo_canceller: Some(EchoCanceller::default()),
            high_pass_filter: Some(HighPassFilter::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            ..Default::default()
        };
        let stream = StreamConfig::new(48000, 1);
        Self {
            processor: AudioProcessing::builder()
                .config(config)
                .capture_config(stream)
                .render_config(stream)
                .build(),
            render: VecDeque::new(),
        }
    }
    pub(super) fn render(&mut self, samples: &[f32]) -> Result<(), String> {
        self.render.extend(samples);
        while self.render.len() >= 480 {
            let input: Vec<_> = self.render.drain(..480).collect();
            let mut out = [0.0; 480];
            self.processor
                .process_render_f32(&[&input], &mut [&mut out])
                .map_err(|_| "Couldn’t process speaker audio.")?;
        }
        Ok(())
    }
    pub(super) fn capture(&mut self, samples: &[f32]) -> Result<Vec<i16>, String> {
        let mut out = Vec::with_capacity(samples.len());
        for input in samples.as_chunks::<480>().0 {
            let mut processed = [0.0; 480];
            self.processor
                .process_capture_f32(&[input], &mut [&mut processed])
                .map_err(|_| "Couldn’t process microphone audio.")?;
            out.extend(processed.map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16));
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_callback_sizes_do_not_change_resampling_or_accumulate_drift() {
        for (from, to) in [(44100, 48000), (48000, 44100), (96000, 48000)] {
            let input: Vec<_> = (0..from * 3).map(|i| (i as f32 * 0.014).sin()).collect();
            let expected = Resampler::new(from, to).process(&input);
            let mut r = Resampler::new(from, to);
            let actual: Vec<_> = input
                .chunks(257)
                .flat_map(|chunk| r.process(chunk))
                .collect();
            assert_eq!(actual, expected);
            assert!(actual.len().abs_diff(to as usize * 3) <= 2);
            assert!(r.samples.len() <= 1);
        }
    }
    #[test]
    fn delayed_speaker_echo_is_removed_and_near_voice_survives() {
        let mut dsp = VoiceProcessing::new();
        let mut delay = VecDeque::from(vec![0.0; 480 * 5]);
        let mut seed = 1u32;
        let mut before = 0.0f64;
        let mut after = 0.0f64;
        for frame in 0..1600 {
            let speaker: Vec<_> = (0..480)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    (seed as i32 as f32 / i32::MAX as f32) * 0.2
                })
                .collect();
            delay.extend(speaker.iter().map(|v| v * 0.65));
            let mut mic: Vec<_> = delay.drain(..480).collect();
            if frame >= 1400 {
                for (i, v) in mic.iter_mut().enumerate() {
                    *v += ((frame * 480 + i) as f32 * 0.058).sin() * 0.15;
                }
            }
            dsp.render(&speaker).unwrap();
            let result = dsp.capture(&mic).unwrap();
            if (1000..1400).contains(&frame) {
                before += mic.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
                after += result
                    .iter()
                    .map(|v| (*v as f64 / 32768.0).powi(2))
                    .sum::<f64>();
            }
            if frame == 1599 {
                assert!(
                    result.iter().any(|v| v.abs() > 500),
                    "near-end speech was suppressed"
                );
            }
        }
        let attenuation = 10.0 * (before / after.max(1e-15)).log10();
        println!("DESKTOP_AUDIO echo_attenuation_db={attenuation:.1}");
        assert!(
            attenuation > 15.0,
            "insufficient echo removal: {attenuation:.1} dB"
        );
    }
}
