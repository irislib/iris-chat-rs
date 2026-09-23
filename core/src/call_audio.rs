//! Shared, bounded 48 kHz Opus voice codec and playout buffer. Every platform
//! supplies 20 ms mono PCM after its platform echo/noise processing.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
const SAMPLES: usize = 960;
const MAX_PACKET: usize = 1275;
#[derive(Debug, uniffi::Error)]
pub enum CallAudioError {
    CodecUnavailable,
    InvalidSamples,
}
impl std::fmt::Display for CallAudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CodecUnavailable => "Call audio is unavailable",
            Self::InvalidSamples => "Expected 20 ms of mono call audio",
        })
    }
}
impl std::error::Error for CallAudioError {}
struct Playout {
    decoder: opus::Decoder,
    packets: BTreeMap<u32, Vec<u8>>,
    next: Option<u32>,
    priming: u8,
    lost: u8,
}
#[derive(uniffi::Object)]
pub struct CallAudioCodec {
    encoder: Mutex<opus::Encoder>,
    playout: Mutex<Playout>,
}
#[uniffi::export]
impl CallAudioCodec {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, CallAudioError> {
        let build = || -> opus::Result<Self> {
            let mut encoder =
                opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip)?;
            encoder.set_bitrate(opus::Bitrate::Bits(32000))?;
            encoder.set_vbr(false)?;
            encoder.set_inband_fec(true)?;
            encoder.set_packet_loss_perc(10)?;
            encoder.set_dtx(true)?;
            encoder.set_complexity(5)?;
            Ok(Self {
                encoder: Mutex::new(encoder),
                playout: Mutex::new(Playout {
                    decoder: opus::Decoder::new(48000, opus::Channels::Mono)?,
                    packets: BTreeMap::new(),
                    next: None,
                    priming: 3,
                    lost: 0,
                }),
            })
        };
        build()
            .map(Arc::new)
            .map_err(|_| CallAudioError::CodecUnavailable)
    }
    pub fn encode(&self, samples: Vec<i16>) -> Result<Vec<u8>, CallAudioError> {
        if samples.len() != SAMPLES {
            return Err(CallAudioError::InvalidSamples);
        }
        self.encoder
            .lock()
            .map_err(|_| CallAudioError::CodecUnavailable)?
            .encode_vec(&samples, MAX_PACKET)
            .map_err(|_| CallAudioError::CodecUnavailable)
    }
    pub fn queue(&self, sequence: u32, data: Vec<u8>) {
        if data.is_empty() || data.len() > MAX_PACKET {
            return;
        }
        let Ok(mut state) = self.playout.lock() else {
            return;
        };
        if state.decoder.get_nb_samples(&data).ok() != Some(SAMPLES) {
            return;
        }
        if let Some(next) = state.next {
            let delta = sequence.wrapping_sub(next);
            if delta >= 1 << 31 {
                if state.priming > 0 && next.wrapping_sub(sequence) <= 3 {
                    state.next = Some(sequence);
                } else {
                    return;
                }
            } else if delta > 12 {
                // Rebase after a pause instead of replaying an old audio backlog.
                state.packets.clear();
                state.next = Some(sequence);
                state.priming = 3;
                state.lost = 0;
                let _ = state.decoder.reset_state();
            }
        } else {
            state.next = Some(sequence);
        }
        if state.packets.len() < 12 {
            state.packets.entry(sequence).or_insert(data);
        }
    }
    /// Call once per 20 ms of device output. Missing packets use in-band FEC
    /// from the next packet, then Opus PLC, followed by silence after 120 ms.
    pub fn playout(&self) -> Vec<i16> {
        let mut output = vec![0; SAMPLES];
        let Ok(mut state) = self.playout.lock() else {
            return output;
        };
        let Some(next) = state.next else {
            return output;
        };
        if state.priming > 0 {
            state.priming -= 1;
            return output;
        }
        let packet = state.packets.remove(&next);
        let decoded = if let Some(packet) = packet {
            state.lost = 0;
            state.decoder.decode(&packet, &mut output, false)
        } else {
            state.lost = state.lost.saturating_add(1);
            let fec = state.packets.get(&next.wrapping_add(1)).cloned();
            if let Some(packet) = fec {
                state.decoder.decode(&packet, &mut output, true)
            } else if state.lost <= 6 {
                state.decoder.decode(&[], &mut output, false)
            } else {
                Ok(SAMPLES)
            }
        };
        if decoded.ok() != Some(SAMPLES) {
            output.fill(0);
        }
        if state.lost > 6 && state.packets.is_empty() {
            state.next = None;
            state.priming = 3;
            state.lost = 0;
            let _ = state.decoder.reset_state();
        } else {
            state.next = Some(next.wrapping_add(1));
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opus_compresses_voice_and_jitter_playout_recovers_reordering_and_loss() {
        let codec = CallAudioCodec::new().unwrap();
        let samples: Vec<_> = (0..960)
            .map(|i| ((i as f32 * 0.05).sin() * 10000.0) as i16)
            .collect();
        let packets: Vec<_> = (0..8)
            .map(|_| codec.encode(samples.clone()).unwrap())
            .collect();
        assert!(packets.iter().all(|p| p.len() <= 100));
        for i in [0, 2, 1, 4, 5, 6, 7] {
            codec.queue(i as u32, packets[i].clone());
        }
        for _ in 0..3 {
            assert!(codec.playout().iter().all(|v| *v == 0));
        }
        for _ in 0..8 {
            let decoded = codec.playout();
            assert_eq!(decoded.len(), 960);
            assert!(decoded.iter().any(|v| v.abs() > 20));
        }
        for _ in 0..7 {
            codec.playout();
        }
        assert!(codec.playout().iter().all(|v| *v == 0));
    }
    #[test]
    fn audio_resumes_after_mute_without_waiting_for_old_sequence_numbers() {
        let codec = CallAudioCodec::new().unwrap();
        let data = codec
            .encode(
                (0..960)
                    .map(|i| ((i as f32 * 0.05).sin() * 10000.0) as i16)
                    .collect(),
            )
            .unwrap();
        codec.queue(0, data.clone());
        for _ in 0..100 {
            codec.playout();
        }
        codec.queue(1, data);
        for _ in 0..3 {
            codec.playout();
        }
        assert!(codec.playout().iter().any(|v| v.abs() > 20));
    }

    #[test]
    fn malformed_input_and_far_future_packets_cannot_grow_audio_queues() {
        let codec = CallAudioCodec::new().unwrap();
        assert!(codec.encode(vec![0; 640]).is_err());
        codec.queue(0, vec![0; 2000]);
        assert_eq!(codec.playout.lock().unwrap().packets.len(), 0);
        let data = codec.encode(vec![0; 960]).unwrap();
        for i in 0..10000 {
            codec.queue(i, data.clone());
        }
        assert!(codec.playout.lock().unwrap().packets.len() <= 12);
    }
}
