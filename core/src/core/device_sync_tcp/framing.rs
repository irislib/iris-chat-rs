use fips_core::PeerIdentity;
use std::time::{Instant as StdInstant, SystemTime, UNIX_EPOCH};

use super::FRAME_HEADER_BYTES;

pub(super) struct RecordReader {
    bytes: Vec<u8>,
    max_record_bytes: usize,
}

impl RecordReader {
    pub(super) fn new(max_record_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_record_bytes,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
        self.bytes.extend_from_slice(chunk);
        let mut records = Vec::new();
        let mut consumed = 0;
        while self.bytes.len().saturating_sub(consumed) >= FRAME_HEADER_BYTES {
            let header = self
                .bytes
                .get(consumed..consumed + FRAME_HEADER_BYTES)
                .ok_or(())?;
            let length = u32::from_be_bytes(header.try_into().map_err(|_| ())?) as usize;
            if length > self.max_record_bytes {
                return Err(());
            }
            let end = consumed + FRAME_HEADER_BYTES + length;
            if end > self.bytes.len() {
                break;
            }
            records.push(
                self.bytes
                    .get(consumed + FRAME_HEADER_BYTES..end)
                    .ok_or(())?
                    .to_vec(),
            );
            consumed = end;
        }
        self.bytes.drain(..consumed);
        Ok(records)
    }
}

pub(super) fn frame(record: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FRAME_HEADER_BYTES + record.len());
    bytes.extend_from_slice(&(record.len() as u32).to_be_bytes());
    bytes.extend_from_slice(record);
    bytes
}

pub(super) fn comparison_key(identity: &PeerIdentity) -> String {
    let value = identity.pubkey().to_string().to_lowercase();
    if value.len() == 66 && (value.starts_with("02") || value.starts_with("03")) {
        value[2..].to_owned()
    } else {
        value
    }
}

pub(super) fn elapsed_millis(started: StdInstant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(super) fn random_isn_seed() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs() ^ u64::from(now.subsec_nanos())
}
