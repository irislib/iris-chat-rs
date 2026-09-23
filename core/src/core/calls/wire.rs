//! Version-three compressed codec frames on authenticated FIPS datagrams.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
pub(super) const PORT: u16 = 39511;
pub(super) const CHUNK: usize = 1100;
const HEADER: usize = 38;
const MAX_VIDEO: usize = 262_144;
const FRAME_TTL: Duration = Duration::from_millis(250);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Signal {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_seq: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_seq: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_seq: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Vec<u16>>,
}
impl Signal {
    pub fn new(kind: &str, call_id: &str, video: bool, muted: bool) -> Self {
        Self {
            v: 3,
            kind: kind.into(),
            reason: None,
            call_id: call_id.into(),
            video: Some(video),
            muted: Some(muted),
            codec: Some("opus-h264-v3".into()),
            feedback_seq: None,
            video_seq: None,
            received_frames: None,
            received_bytes: None,
            interval_ms: None,
            frame_seq: None,
            missing: None,
        }
    }
    pub fn answered_elsewhere(call_id: &str, video: bool) -> Self {
        let mut signal = Self::new("end", call_id, video, false);
        signal.reason = Some("answered_elsewhere".into());
        signal
    }
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() > 2048 {
            return None;
        }
        let value: Self = serde_json::from_slice(data).ok()?;
        if value.v != 3
            || id_bytes(&value.call_id).is_none()
            || value.codec.as_deref().is_some_and(|c| c != "opus-h264-v3")
            || !matches!(
                value.kind.as_str(),
                "offer"
                    | "answer"
                    | "reject"
                    | "end"
                    | "ping"
                    | "pong"
                    | "media_state"
                    | "feedback"
                    | "keyframe"
                    | "nack"
            )
        {
            return None;
        }
        Some(value)
    }
}
pub(super) fn id_bytes(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut id = [0; 16];
    for (slot, pair) in id.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(id)
}
pub(super) fn valid_frame(kind: u8, data: &[u8]) -> bool {
    match kind {
        1 => (1..=1275).contains(&data.len()),
        2 => {
            (5..=MAX_VIDEO).contains(&data.len())
                && (data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]))
        }
        _ => false,
    }
}
pub(super) fn encode(
    id: &str,
    kind: u8,
    seq: u32,
    timestamp_us: u64,
    key: bool,
    data: &[u8],
) -> Vec<Vec<u8>> {
    let Some(id) = id_bytes(id) else {
        return vec![];
    };
    if !valid_frame(kind, data) {
        return vec![];
    }
    let count = data.len().div_ceil(CHUNK) as u16;
    data.chunks(CHUNK)
        .enumerate()
        .map(|(index, part)| {
            let mut p = Vec::with_capacity(HEADER + part.len());
            p.extend_from_slice(b"IC03");
            p.extend_from_slice(&id);
            p.push(kind);
            p.push(u8::from(key));
            p.extend_from_slice(&seq.to_be_bytes());
            p.extend_from_slice(&timestamp_us.to_be_bytes());
            p.extend_from_slice(&(index as u16).to_be_bytes());
            p.extend_from_slice(&count.to_be_bytes());
            p.extend_from_slice(part);
            p
        })
        .collect()
}
#[derive(Debug, PartialEq)]
pub(super) struct Frame {
    pub kind: u8,
    pub seq: u32,
    pub timestamp_us: u64,
    pub key: bool,
    pub data: Vec<u8>,
}
struct Partial {
    created: Instant,
    nack_at: Option<Instant>,
    attempts: u8,
    timestamp_us: u64,
    key: bool,
    parts: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
pub(super) struct Assembler {
    pending: BTreeMap<(u8, u32), Partial>,
    delivered: BTreeMap<(u8, u32), Instant>,
    pub highest_video: Option<u32>,
    pub received_frames: u32,
    pub received_bytes: u32,
}
impl Assembler {
    pub fn nacks(&mut self, now: Instant) -> Vec<(u32, Vec<u16>)> {
        self.pending
            .retain(|_, f| now.saturating_duration_since(f.created) <= FRAME_TTL);
        self.pending
            .iter_mut()
            .filter_map(|((kind, seq), frame)| {
                if *kind != 2
                    || frame.attempts >= 2
                    || now.saturating_duration_since(frame.created) < Duration::from_millis(40)
                    || frame.nack_at.is_some_and(|at| {
                        now.saturating_duration_since(at) < Duration::from_millis(50)
                    })
                {
                    return None;
                }
                let missing = frame
                    .parts
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| p.is_none().then_some(i as u16))
                    .take(64)
                    .collect();
                frame.nack_at = Some(now);
                frame.attempts += 1;
                Some((*seq, missing))
            })
            .take(2)
            .collect()
    }

    pub fn receive(&mut self, id: &str, p: &[u8], now: Instant) -> Option<Frame> {
        self.pending
            .retain(|_, f| now.saturating_duration_since(f.created) <= FRAME_TTL);
        self.delivered
            .retain(|_, at| now.saturating_duration_since(*at) <= Duration::from_secs(2));
        if p.len() <= HEADER
            || p.len() > HEADER + CHUNK
            || p.get(..4)? != b"IC03"
            || p.get(4..20)? != id_bytes(id)?
        {
            return None;
        }
        let kind = *p.get(20)?;
        let key = *p.get(21)?;
        if !matches!(kind, 1 | 2) || key > 1 {
            return None;
        }
        let seq = u32::from_be_bytes(p.get(22..26)?.try_into().ok()?);
        let timestamp_us = u64::from_be_bytes(p.get(26..34)?.try_into().ok()?);
        let index = u16::from_be_bytes(p.get(34..36)?.try_into().ok()?) as usize;
        let count = u16::from_be_bytes(p.get(36..38)?.try_into().ok()?) as usize;
        let limit = if kind == 1 { 1275usize } else { MAX_VIDEO };
        if count == 0
            || count > limit.div_ceil(CHUNK)
            || index >= count
            || (index + 1 < count && p.len() != HEADER + CHUNK)
            || self.delivered.contains_key(&(kind, seq))
        {
            return None;
        }
        if kind == 2
            && self
                .highest_video
                .is_none_or(|last| seq.wrapping_sub(last) > 0 && seq.wrapping_sub(last) < 1 << 31)
        {
            self.highest_video = Some(seq);
        }
        if !self.pending.contains_key(&(kind, seq)) && self.pending.len() >= 16 {
            if let Some(old) = self
                .pending
                .iter()
                .min_by_key(|(_, f)| f.created)
                .map(|(key, _)| *key)
            {
                self.pending.remove(&old);
            }
        }
        let entry = self.pending.entry((kind, seq)).or_insert_with(|| Partial {
            created: now,
            nack_at: None,
            attempts: 0,
            timestamp_us,
            key: key == 1,
            parts: vec![None; count],
        });
        if entry.parts.len() != count
            || entry.timestamp_us != timestamp_us
            || entry.key != (key == 1)
        {
            return None;
        }
        *entry.parts.get_mut(index)? = Some(p.get(HEADER..)?.to_vec());
        if entry.parts.iter().any(Option::is_none) {
            return None;
        }
        let frame = self.pending.remove(&(kind, seq))?;
        let data: Vec<_> = frame.parts.into_iter().flatten().flatten().collect();
        if !valid_frame(kind, &data) {
            return None;
        }
        if self.delivered.len() >= 128 {
            if let Some(old) = self
                .delivered
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(key, _)| *key)
            {
                self.delivered.remove(&old);
            }
        }
        self.delivered.insert((kind, seq), now);
        if kind == 2 {
            self.received_frames = self.received_frames.saturating_add(1);
            self.received_bytes = self.received_bytes.saturating_add(data.len() as u32);
        }
        Some(Frame {
            kind,
            seq,
            timestamp_us,
            key: key == 1,
            data,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "00112233445566778899aabbccddeeff";
    #[test]
    fn compressed_frames_reorder_without_duplicate_delivery() {
        let mut a = Assembler::default();
        let now = Instant::now();
        for seq in [3, 1, 2] {
            let packets = encode(ID, 1, seq, 123, true, &[42; 80]);
            assert_eq!(a.receive(ID, &packets[0], now).unwrap().seq, seq);
            assert!(a.receive(ID, &packets[0], now).is_none());
        }
    }
    #[test]
    fn missing_video_fragment_is_requested_and_recovered_before_deadline() {
        let mut video = vec![42; 2500];
        video[..4].copy_from_slice(&[0, 0, 0, 1]);
        let packets = encode(ID, 2, 0, 100, true, &video);
        let mut a = Assembler::default();
        let now = Instant::now();
        a.receive(ID, &packets[0], now);
        a.receive(ID, &packets[2], now);
        assert!(a.nacks(now + Duration::from_millis(30)).is_empty());
        assert_eq!(a.nacks(now + Duration::from_millis(50)), vec![(0, vec![1])]);
        assert!(a.nacks(now + Duration::from_millis(70)).is_empty());
        let recovered = a
            .receive(ID, &packets[1], now + Duration::from_millis(80))
            .unwrap();
        assert_eq!(recovered.data, video);
        assert!(a.nacks(now + Duration::from_millis(110)).is_empty());
        let packets = encode(ID, 2, 1, 200, true, &video);
        a.receive(ID, &packets[0], now);
        assert!(a.nacks(now + Duration::from_millis(300)).is_empty());
    }

    #[test]
    fn large_h264_keyframes_fragment_and_bound_memory() {
        let mut frame = vec![42; 150_000];
        frame[..4].copy_from_slice(&[0, 0, 0, 1]);
        let packets = encode(ID, 2, 0, 123, true, &frame);
        let mut a = Assembler::default();
        let mut result = None;
        for p in packets.iter().rev() {
            result = a.receive(ID, p, Instant::now());
        }
        assert_eq!(result.unwrap().data, frame);
        assert_eq!(a.received_frames, 1);
        for seq in 1..100 {
            a.receive(
                ID,
                &encode(ID, 2, seq, 123, true, &frame)[0],
                Instant::now(),
            );
        }
        assert_eq!(a.pending.len(), 16);
        assert_eq!(a.highest_video, Some(99));
        assert!(encode(ID, 2, 0, 0, false, &vec![0; 300000]).is_empty());
        assert!(encode(ID, 1, 0, 0, true, &[0; 1276]).is_empty());
    }
}
