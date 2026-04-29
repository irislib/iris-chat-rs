use std::io::{Read, Write};

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::Value;

const MAGIC: &[u8; 4] = b"IRIS";
const COMPRESSED_FLAG: u8 = 0x01;
const HEADER_SIZE: usize = 13;
const COMPRESSION_THRESHOLD: usize = 100;
const MAX_FRAME_BYTES: usize = 256 * 1024;

pub(crate) fn encode_frame_json(envelope_json: &str) -> Option<Vec<u8>> {
    let envelope: Value = serde_json::from_str(envelope_json).ok()?;
    if !envelope.is_object() {
        return None;
    }
    let payload = serde_json::to_vec(&envelope).ok()?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return None;
    }

    let compressed = compress_if_beneficial(&payload);
    let body = compressed.as_deref().unwrap_or(&payload);
    if body.len() > MAX_FRAME_BYTES {
        return None;
    }

    let mut frame = Vec::with_capacity(HEADER_SIZE + body.len());
    frame.extend_from_slice(MAGIC);
    frame.push(if compressed.is_some() {
        COMPRESSED_FLAG
    } else {
        0
    });
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Some(frame)
}

pub(crate) fn decode_frame_json(frame: &[u8]) -> Option<String> {
    if frame.len() < HEADER_SIZE || &frame[..4] != MAGIC {
        return None;
    }
    let flags = frame[4];
    if flags & !COMPRESSED_FLAG != 0 {
        return None;
    }

    let body_len = u32::from_be_bytes(frame[5..9].try_into().ok()?) as usize;
    let original_len = u32::from_be_bytes(frame[9..13].try_into().ok()?) as usize;
    if body_len == 0
        || original_len == 0
        || body_len > MAX_FRAME_BYTES
        || original_len > MAX_FRAME_BYTES
        || frame.len() != HEADER_SIZE + body_len
    {
        return None;
    }

    let body = &frame[HEADER_SIZE..];
    let payload = if flags & COMPRESSED_FLAG != 0 {
        decompress(body, original_len)?
    } else {
        if body_len != original_len {
            return None;
        }
        body.to_vec()
    };

    let envelope: Value = serde_json::from_slice(&payload).ok()?;
    if !envelope.is_object() {
        return None;
    }
    serde_json::to_string(&envelope).ok()
}

pub(crate) fn frame_body_len_from_header(header: &[u8]) -> Option<usize> {
    if header.len() < HEADER_SIZE || &header[..4] != MAGIC {
        return None;
    }
    let body_len = u32::from_be_bytes(header[5..9].try_into().ok()?) as usize;
    if body_len == 0 || body_len > MAX_FRAME_BYTES {
        return None;
    }
    Some(body_len)
}

fn compress_if_beneficial(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < COMPRESSION_THRESHOLD {
        return None;
    }
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).ok()?;
    let compressed = encoder.finish().ok()?;
    if compressed.is_empty() || compressed.len() >= data.len() {
        return None;
    }
    Some(compressed)
}

fn decompress(data: &[u8], original_len: usize) -> Option<Vec<u8>> {
    let mut decoder = DeflateDecoder::new(data);
    let mut output = Vec::with_capacity(original_len);
    decoder.read_to_end(&mut output).ok()?;
    if output.len() != original_len || output.len() > MAX_FRAME_BYTES {
        return None;
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_small_uncompressed_frame() {
        let frame = encode_frame_json(r#"{"v":1,"type":"hello","peer_id":"abc"}"#).unwrap();
        assert_eq!(&frame[..4], MAGIC);
        assert_eq!(frame[4], 0);
        let decoded = decode_frame_json(&frame).unwrap();
        let value: Value = serde_json::from_str(&decoded).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["peer_id"], "abc");
    }

    #[test]
    fn encodes_and_decodes_compressed_raw_deflate_frame() {
        let event_json = "x".repeat(2048);
        let envelope = serde_json::json!({
            "v": 1,
            "type": "event",
            "peer_id": "peer",
            "event_json": event_json,
        });
        let frame = encode_frame_json(&envelope.to_string()).unwrap();
        assert_eq!(frame[4], COMPRESSED_FLAG);
        let decoded = decode_frame_json(&frame).unwrap();
        let value: Value = serde_json::from_str(&decoded).unwrap();
        assert_eq!(value["type"], "event");
        assert_eq!(value["event_json"].as_str().unwrap().len(), 2048);
    }

    #[test]
    fn rejects_zlib_wrapped_payload_when_marked_compressed() {
        let payload = br#"{"v":1,"type":"hello","peer_id":"abc"}"#;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let body = encoder.finish().unwrap();

        let mut frame = Vec::new();
        frame.extend_from_slice(MAGIC);
        frame.push(COMPRESSED_FLAG);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);

        assert!(decode_frame_json(&frame).is_none());
    }
}
