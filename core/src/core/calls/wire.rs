//! Version-one, independently decodable real-time frames. Media never enters
//! the message store, TCP retransmission queue, or attachment caches.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(super) const PORT: u16 = 39511;
pub(super) const CHUNK: usize = 1100;
const HEADER: usize = 29;
const MAX_FRAME: usize = 65_536;
const MAX_PARTS: usize = MAX_FRAME.div_ceil(CHUNK);
const FRAME_TTL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Signal {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub call_id: String,
    #[serde(default)]
    pub video: Option<bool>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl Signal {
    pub fn new(kind: &str, call_id: &str, video: bool, muted: bool) -> Self {
        Self {
            v: 1,
            kind: kind.into(),
            call_id: call_id.into(),
            video: Some(video),
            muted: Some(muted),
            codec: Some("pcm16-jpeg-v1".into()),
            reason: None,
        }
    }
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() > 1024 {
            return None;
        }
        let value: Self = serde_json::from_slice(data).ok()?;
        if value.v != 1
            || id_bytes(&value.call_id).is_none()
            || value
                .codec
                .as_deref()
                .is_some_and(|codec| codec != "pcm16-jpeg-v1")
            || !matches!(
                value.kind.as_str(),
                "offer" | "answer" | "reject" | "end" | "ping" | "pong" | "media_state"
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
    let mut bytes = [0u8; 16];
    for (slot, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(bytes)
}
pub(super) fn valid_frame(kind: u8, data: &[u8]) -> bool {
    match kind {
        1 => data.len() == 640,
        2 => {
            (4..=MAX_FRAME).contains(&data.len())
                && data.starts_with(&[0xff, 0xd8])
                && data.ends_with(&[0xff, 0xd9])
        }
        _ => false,
    }
}
pub(super) fn encode(call_id: &str, kind: u8, seq: u32, data: &[u8]) -> Vec<Vec<u8>> {
    let Some(id) = id_bytes(call_id) else {
        return Vec::new();
    };
    if !valid_frame(kind, data) {
        return Vec::new();
    }
    let count = data.len().div_ceil(CHUNK) as u16;
    data.chunks(CHUNK)
        .enumerate()
        .map(|(index, part)| {
            let mut packet = Vec::with_capacity(HEADER + part.len());
            packet.extend_from_slice(b"IC01");
            packet.extend_from_slice(&id);
            packet.push(kind);
            packet.extend_from_slice(&seq.to_be_bytes());
            packet.extend_from_slice(&(index as u16).to_be_bytes());
            packet.extend_from_slice(&count.to_be_bytes());
            packet.extend_from_slice(part);
            packet
        })
        .collect()
}
struct PendingFrame {
    created: Instant,
    parts: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
pub(super) struct Assembler {
    pending: BTreeMap<(u8, u32), PendingFrame>,
    delivered: BTreeMap<u8, u32>,
}
impl Assembler {
    pub fn receive(&mut self, call_id: &str, packet: &[u8], now: Instant) -> Option<(u8, Vec<u8>)> {
        self.pending
            .retain(|_, f| now.saturating_duration_since(f.created) <= FRAME_TTL);
        if packet.len() <= HEADER
            || packet.len() > HEADER + CHUNK
            || packet.get(..4)? != b"IC01"
            || packet.get(4..20)? != id_bytes(call_id)?
        {
            return None;
        }
        let kind = *packet.get(20)?;
        if !matches!(kind, 1 | 2) {
            return None;
        }
        let seq = u32::from_be_bytes(packet.get(21..25)?.try_into().ok()?);
        let index = u16::from_be_bytes(packet.get(25..27)?.try_into().ok()?) as usize;
        let count = u16::from_be_bytes(packet.get(27..29)?.try_into().ok()?) as usize;
        if count == 0
            || count > MAX_PARTS
            || index >= count
            || (kind == 1 && count != 1)
            || (index + 1 < count && packet.len() != HEADER + CHUNK)
            || self.delivered.get(&kind).is_some_and(|last| {
                seq.wrapping_sub(*last) == 0 || seq.wrapping_sub(*last) >= (1 << 31)
            })
        {
            return None;
        }
        if !self.pending.contains_key(&(kind, seq)) && self.pending.len() >= 4 {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(_, v)| v.created)
                .map(|(k, _)| *k)?;
            self.pending.remove(&oldest);
        }
        let entry = self
            .pending
            .entry((kind, seq))
            .or_insert_with(|| PendingFrame {
                created: now,
                parts: vec![None; count],
            });
        if entry.parts.len() != count {
            return None;
        }
        *entry.parts.get_mut(index)? = Some(packet.get(HEADER..)?.to_vec());
        if entry.parts.iter().any(Option::is_none) {
            return None;
        }
        let entry = self.pending.remove(&(kind, seq))?;
        let frame: Vec<u8> = entry.parts.into_iter().flatten().flatten().collect();
        if !valid_frame(kind, &frame) {
            return None;
        }
        self.delivered.insert(kind, seq);
        self.pending.retain(|(k, s), _| {
            *k != kind || (s.wrapping_sub(seq) > 0 && s.wrapping_sub(seq) < (1 << 31))
        });
        Some((kind, frame))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "00112233445566778899aabbccddeeff";
    #[test]
    fn call_frames_reassemble_out_of_order_but_drop_duplicates_and_stale_frames() {
        let mut image = vec![42; 8000];
        image[..2].copy_from_slice(&[255, 216]);
        image[7998..].copy_from_slice(&[255, 217]);
        let packets = encode(ID, 2, 7, &image);
        let now = Instant::now();
        let mut a = Assembler::default();
        let mut result = None;
        for p in packets.iter().rev() {
            result = a.receive(ID, p, now);
        }
        assert_eq!(result, Some((2, image)));
        for p in &packets {
            assert!(a.receive(ID, p, now).is_none());
        }
        let pcm = vec![0; 640];
        let p = encode(ID, 1, 1, &pcm).remove(0);
        assert_eq!(a.receive(ID, &p, now), Some((1, pcm)));
        assert!(a
            .receive("ffeeddccbbaa99887766554433221100", &p, now)
            .is_none());
    }
    #[test]
    fn call_frames_bound_memory_and_expire_partial_video() {
        let mut image = vec![0; 2000];
        image[..2].copy_from_slice(&[255, 216]);
        image[1998..].copy_from_slice(&[255, 217]);
        let mut a = Assembler::default();
        let now = Instant::now();
        for seq in 0..100 {
            assert!(a.receive(ID, &encode(ID, 2, seq, &image)[0], now).is_none());
        }
        assert_eq!(a.pending.len(), 4);
        assert!(a
            .receive(
                ID,
                &encode(ID, 2, 99, &image)[1],
                now + Duration::from_secs(1)
            )
            .is_none());
        assert_eq!(a.pending.len(), 1);
    }
    #[test]
    fn call_wire_rejects_bad_ids_versions_codecs_and_oversized_media() {
        assert!(Signal::decode(
            br#"{"v":2,"type":"offer","call_id":"00112233445566778899aabbccddeeff"}"#
        )
        .is_none());
        assert!(encode(ID, 1, 1, &[0; 641]).is_empty());
        assert!(encode(ID, 2, 1, &vec![0; 100000]).is_empty());
        assert!(id_bytes("00112233445566778899AABBCCDDEEFF").is_none());
    }
}
