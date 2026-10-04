use super::CHUNK;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Control {
    Accept { id: String, token: String },
    Decline { id: String, token: String },
    Done,
    Received,
    Cancel,
}

pub(super) fn validate_claim(id: &str, token: &str) -> Result<(), String> {
    if !crate::direct_files::valid_transfer_id(id) {
        return Err("Invalid file offer".into());
    }
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid file offer".into());
    }
    Ok(())
}
fn frame(kind: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(data.len() + 5);
    bytes.extend_from_slice(&((data.len() + 1) as u32).to_be_bytes());
    bytes.push(kind);
    bytes.extend_from_slice(data);
    bytes
}
pub(super) fn control(value: Control) -> Result<Vec<u8>, String> {
    let data = serde_json::to_vec(&value).map_err(|_| "Could not encode file request")?;
    Ok(frame(0, &data))
}
pub(super) fn data(bytes: Vec<u8>) -> Vec<u8> {
    frame(1, &bytes)
}
pub(super) fn parse_control(packet: &[u8]) -> Result<Control, String> {
    if packet.first() != Some(&0) || packet.len() > 1024 {
        return Err("Invalid file transfer frame".into());
    }
    let body = packet.get(1..).ok_or("Invalid file transfer frame")?;
    serde_json::from_slice(body).map_err(|_| "Invalid file transfer frame".into())
}

#[derive(Default)]
pub(super) struct Reader {
    bytes: Vec<u8>,
}
impl Reader {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.bytes.extend_from_slice(bytes);
        let mut consumed = 0;
        let mut packets = Vec::new();
        while self.bytes.len() - consumed >= 4 {
            let header = self
                .bytes
                .get(consumed..consumed + 4)
                .ok_or("Invalid file frame header")?;
            let length =
                u32::from_be_bytes(header.try_into().map_err(|_| "Invalid file frame header")?)
                    as usize;
            if length == 0 || length > CHUNK + 1 {
                return Err("Invalid file transfer frame size".into());
            }
            let end = consumed + 4 + length;
            if end > self.bytes.len() {
                break;
            }
            packets.push(
                self.bytes
                    .get(consumed + 4..end)
                    .ok_or("Invalid file frame data")?
                    .to_vec(),
            );
            consumed = end;
        }
        self.bytes.drain(..consumed);
        Ok(packets)
    }
}
