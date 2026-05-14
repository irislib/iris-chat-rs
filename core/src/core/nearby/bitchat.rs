#![allow(dead_code)]

pub(crate) const PEER_ID_SIZE: usize = 8;
pub(crate) const SIGNATURE_SIZE: usize = 64;

pub(crate) const BROADCAST_RECIPIENT: [u8; PEER_ID_SIZE] = [0xff; PEER_ID_SIZE];
pub(crate) const NOISE_PAYLOAD_NDR_EVENT: u8 = 0x12;

const VERSION_1: u8 = 1;
const VERSION_2: u8 = 2;
const HEADER_SIZE_V1: usize = 14;
const HEADER_SIZE_V2: usize = 16;
const FLAG_HAS_RECIPIENT: u8 = 0x01;
const FLAG_HAS_SIGNATURE: u8 = 0x02;
const FLAG_IS_COMPRESSED: u8 = 0x04;
const FLAG_HAS_ROUTE: u8 = 0x08;
const FLAG_IS_RSR: u8 = 0x10;
const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const PADDING_BLOCK_SIZES: [usize; 4] = [256, 512, 1024, 2048];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MessageType {
    Announce = 0x01,
    Message = 0x02,
    Leave = 0x03,
    NoiseHandshake = 0x10,
    NoiseEncrypted = 0x11,
    Fragment = 0x20,
    RequestSync = 0x21,
    FileTransfer = 0x22,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BitchatPacket {
    pub(crate) version: u8,
    pub(crate) packet_type: u8,
    pub(crate) ttl: u8,
    pub(crate) timestamp_ms: u64,
    pub(crate) sender_id: [u8; PEER_ID_SIZE],
    pub(crate) recipient_id: Option<[u8; PEER_ID_SIZE]>,
    pub(crate) route: Vec<[u8; PEER_ID_SIZE]>,
    pub(crate) is_rsr: bool,
    pub(crate) payload: Vec<u8>,
    pub(crate) signature: Option<[u8; SIGNATURE_SIZE]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnnouncementPacket {
    pub(crate) nickname: String,
    pub(crate) noise_public_key: [u8; 32],
    pub(crate) signing_public_key: [u8; 32],
    pub(crate) direct_neighbors: Vec<[u8; PEER_ID_SIZE]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BitchatCodecError {
    FrameTooShort,
    InvalidLength,
    InvalidTlv,
    LengthOverflow,
    TooManyRouteHops,
    UnsupportedCompression,
    UnsupportedVersion(u8),
}

impl BitchatPacket {
    pub(crate) fn encode(&self, padding: bool) -> Result<Vec<u8>, BitchatCodecError> {
        if self.version != VERSION_1 && self.version != VERSION_2 {
            return Err(BitchatCodecError::UnsupportedVersion(self.version));
        }
        if self.payload.len() > MAX_PAYLOAD_BYTES {
            return Err(BitchatCodecError::InvalidLength);
        }
        if self.route.len() > u8::MAX as usize {
            return Err(BitchatCodecError::TooManyRouteHops);
        }
        if self.version == VERSION_1 && self.payload.len() > u16::MAX as usize {
            return Err(BitchatCodecError::LengthOverflow);
        }
        if self.version == VERSION_2 && self.payload.len() > u32::MAX as usize {
            return Err(BitchatCodecError::LengthOverflow);
        }

        let has_route = self.version == VERSION_2 && !self.route.is_empty();
        let mut flags = 0u8;
        if self.recipient_id.is_some() {
            flags |= FLAG_HAS_RECIPIENT;
        }
        if self.signature.is_some() {
            flags |= FLAG_HAS_SIGNATURE;
        }
        if has_route {
            flags |= FLAG_HAS_ROUTE;
        }
        if self.is_rsr {
            flags |= FLAG_IS_RSR;
        }

        let header_size = header_size(self.version)?;
        let mut data = Vec::with_capacity(
            header_size
                + PEER_ID_SIZE
                + self
                    .recipient_id
                    .as_ref()
                    .map(|_| PEER_ID_SIZE)
                    .unwrap_or_default()
                + if has_route {
                    1 + self.route.len() * PEER_ID_SIZE
                } else {
                    0
                }
                + self.payload.len()
                + self
                    .signature
                    .as_ref()
                    .map(|_| SIGNATURE_SIZE)
                    .unwrap_or_default(),
        );

        data.push(self.version);
        data.push(self.packet_type);
        data.push(self.ttl);
        data.extend_from_slice(&self.timestamp_ms.to_be_bytes());
        data.push(flags);
        match self.version {
            VERSION_1 => data.extend_from_slice(&(self.payload.len() as u16).to_be_bytes()),
            VERSION_2 => data.extend_from_slice(&(self.payload.len() as u32).to_be_bytes()),
            other => return Err(BitchatCodecError::UnsupportedVersion(other)),
        }
        data.extend_from_slice(&self.sender_id);
        if let Some(recipient_id) = self.recipient_id {
            data.extend_from_slice(&recipient_id);
        }
        if has_route {
            data.push(self.route.len() as u8);
            for hop in &self.route {
                data.extend_from_slice(hop);
            }
        }
        data.extend_from_slice(&self.payload);
        if let Some(signature) = self.signature {
            data.extend_from_slice(&signature);
        }

        if padding {
            Ok(pad_to_optimal_size(data))
        } else {
            Ok(data)
        }
    }

    pub(crate) fn decode(data: &[u8]) -> Result<Self, BitchatCodecError> {
        match decode_core(data) {
            Ok(packet) => Ok(packet),
            Err(first_error) => {
                let unpadded = unpad(data);
                if unpadded.len() == data.len() {
                    Err(first_error)
                } else {
                    decode_core(&unpadded)
                }
            }
        }
    }

    pub(crate) fn signing_bytes(&self) -> Result<Vec<u8>, BitchatCodecError> {
        let mut unsigned = self.clone();
        unsigned.ttl = 0;
        unsigned.is_rsr = false;
        unsigned.signature = None;
        unsigned.encode(true)
    }
}

impl AnnouncementPacket {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, BitchatCodecError> {
        let nickname = self.nickname.as_bytes();
        if nickname.len() > u8::MAX as usize {
            return Err(BitchatCodecError::InvalidLength);
        }

        let neighbor_count = self.direct_neighbors.len().min(10);
        let neighbor_bytes = neighbor_count * PEER_ID_SIZE;
        if neighbor_bytes > u8::MAX as usize {
            return Err(BitchatCodecError::InvalidLength);
        }

        let mut data = Vec::with_capacity(
            2 + nickname.len()
                + 2
                + self.noise_public_key.len()
                + 2
                + self.signing_public_key.len()
                + if neighbor_count > 0 {
                    2 + neighbor_bytes
                } else {
                    0
                },
        );

        push_tlv(&mut data, 0x01, nickname)?;
        push_tlv(&mut data, 0x02, &self.noise_public_key)?;
        push_tlv(&mut data, 0x03, &self.signing_public_key)?;
        if neighbor_count > 0 {
            data.push(0x04);
            data.push(neighbor_bytes as u8);
            for neighbor in self.direct_neighbors.iter().take(neighbor_count) {
                data.extend_from_slice(neighbor);
            }
        }
        Ok(data)
    }

    pub(crate) fn decode(data: &[u8]) -> Result<Self, BitchatCodecError> {
        let mut offset = 0usize;
        let mut nickname = None;
        let mut noise_public_key = None;
        let mut signing_public_key = None;
        let mut direct_neighbors = Vec::new();

        while offset < data.len() {
            if data.len().saturating_sub(offset) < 2 {
                return Err(BitchatCodecError::InvalidTlv);
            }
            let Some(&tlv_type) = data.get(offset) else {
                return Err(BitchatCodecError::InvalidTlv);
            };
            offset += 1;
            let Some(&length) = data.get(offset) else {
                return Err(BitchatCodecError::InvalidTlv);
            };
            let length = length as usize;
            offset += 1;
            let Some(end) = offset.checked_add(length) else {
                return Err(BitchatCodecError::InvalidTlv);
            };
            let Some(value) = data.get(offset..end) else {
                return Err(BitchatCodecError::InvalidTlv);
            };
            offset = end;

            match tlv_type {
                0x01 => {
                    nickname = Some(
                        std::str::from_utf8(value)
                            .map_err(|_| BitchatCodecError::InvalidTlv)?
                            .to_string(),
                    );
                }
                0x02 if value.len() == 32 => {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(value);
                    noise_public_key = Some(key);
                }
                0x03 if value.len() == 32 => {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(value);
                    signing_public_key = Some(key);
                }
                0x04 if !value.is_empty() && value.len().is_multiple_of(PEER_ID_SIZE) => {
                    direct_neighbors.clear();
                    for chunk in value.chunks_exact(PEER_ID_SIZE) {
                        let mut peer_id = [0u8; PEER_ID_SIZE];
                        peer_id.copy_from_slice(chunk);
                        direct_neighbors.push(peer_id);
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            nickname: nickname.ok_or(BitchatCodecError::InvalidTlv)?,
            noise_public_key: noise_public_key.ok_or(BitchatCodecError::InvalidTlv)?,
            signing_public_key: signing_public_key.ok_or(BitchatCodecError::InvalidTlv)?,
            direct_neighbors,
        })
    }
}

fn decode_core(data: &[u8]) -> Result<BitchatPacket, BitchatCodecError> {
    if data.len() < HEADER_SIZE_V1 + PEER_ID_SIZE {
        return Err(BitchatCodecError::FrameTooShort);
    }

    let Some(&version) = data.first() else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    let fixed_header_size = header_size(version)?;
    if data.len() < fixed_header_size + PEER_ID_SIZE {
        return Err(BitchatCodecError::FrameTooShort);
    }

    let Some(&packet_type) = data.get(1) else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    let Some(&ttl) = data.get(2) else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    let timestamp_ms = u64::from_be_bytes(
        data.get(3..11)
            .ok_or(BitchatCodecError::FrameTooShort)?
            .try_into()
            .map_err(|_| BitchatCodecError::FrameTooShort)?,
    );
    let Some(&flags) = data.get(11) else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    let has_recipient = (flags & FLAG_HAS_RECIPIENT) != 0;
    let has_signature = (flags & FLAG_HAS_SIGNATURE) != 0;
    let is_compressed = (flags & FLAG_IS_COMPRESSED) != 0;
    let has_route = version == VERSION_2 && (flags & FLAG_HAS_ROUTE) != 0;
    let is_rsr = (flags & FLAG_IS_RSR) != 0;

    let (payload_len, mut offset) = match version {
        VERSION_1 => {
            let bytes: [u8; 2] = data
                .get(12..14)
                .ok_or(BitchatCodecError::FrameTooShort)?
                .try_into()
                .map_err(|_| BitchatCodecError::FrameTooShort)?;
            (u16::from_be_bytes(bytes) as usize, HEADER_SIZE_V1)
        }
        VERSION_2 => {
            let bytes: [u8; 4] = data
                .get(12..16)
                .ok_or(BitchatCodecError::FrameTooShort)?
                .try_into()
                .map_err(|_| BitchatCodecError::FrameTooShort)?;
            (u32::from_be_bytes(bytes) as usize, HEADER_SIZE_V2)
        }
        other => return Err(BitchatCodecError::UnsupportedVersion(other)),
    };
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(BitchatCodecError::InvalidLength);
    }

    let sender_id = read_peer_id(data, &mut offset)?;
    let recipient_id = if has_recipient {
        Some(read_peer_id(data, &mut offset)?)
    } else {
        None
    };

    let route = if has_route {
        if data.len().saturating_sub(offset) < 1 {
            return Err(BitchatCodecError::FrameTooShort);
        }
        let Some(&route_count) = data.get(offset) else {
            return Err(BitchatCodecError::FrameTooShort);
        };
        let route_count = route_count as usize;
        offset += 1;
        let mut hops = Vec::with_capacity(route_count);
        for _ in 0..route_count {
            hops.push(read_peer_id(data, &mut offset)?);
        }
        hops
    } else {
        Vec::new()
    };

    if is_compressed {
        return Err(BitchatCodecError::UnsupportedCompression);
    }
    if data.len().saturating_sub(offset) < payload_len {
        return Err(BitchatCodecError::FrameTooShort);
    }
    let Some(payload_slice) = data.get(offset..offset + payload_len) else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    let payload = payload_slice.to_vec();
    offset += payload_len;

    let signature = if has_signature {
        if data.len().saturating_sub(offset) < SIGNATURE_SIZE {
            return Err(BitchatCodecError::FrameTooShort);
        }
        let mut signature = [0u8; SIGNATURE_SIZE];
        let Some(signature_slice) = data.get(offset..offset + SIGNATURE_SIZE) else {
            return Err(BitchatCodecError::FrameTooShort);
        };
        signature.copy_from_slice(signature_slice);
        Some(signature)
    } else {
        None
    };

    Ok(BitchatPacket {
        version,
        packet_type,
        ttl,
        timestamp_ms,
        sender_id,
        recipient_id,
        route,
        is_rsr,
        payload,
        signature,
    })
}

fn header_size(version: u8) -> Result<usize, BitchatCodecError> {
    match version {
        VERSION_1 => Ok(HEADER_SIZE_V1),
        VERSION_2 => Ok(HEADER_SIZE_V2),
        other => Err(BitchatCodecError::UnsupportedVersion(other)),
    }
}

fn read_peer_id(data: &[u8], offset: &mut usize) -> Result<[u8; PEER_ID_SIZE], BitchatCodecError> {
    if data.len().saturating_sub(*offset) < PEER_ID_SIZE {
        return Err(BitchatCodecError::FrameTooShort);
    }
    let mut peer_id = [0u8; PEER_ID_SIZE];
    let Some(peer_id_slice) = data.get(*offset..*offset + PEER_ID_SIZE) else {
        return Err(BitchatCodecError::FrameTooShort);
    };
    peer_id.copy_from_slice(peer_id_slice);
    *offset += PEER_ID_SIZE;
    Ok(peer_id)
}

fn push_tlv(data: &mut Vec<u8>, tlv_type: u8, value: &[u8]) -> Result<(), BitchatCodecError> {
    if value.len() > u8::MAX as usize {
        return Err(BitchatCodecError::InvalidLength);
    }
    data.push(tlv_type);
    data.push(value.len() as u8);
    data.extend_from_slice(value);
    Ok(())
}

fn pad_to_optimal_size(mut data: Vec<u8>) -> Vec<u8> {
    let Some(target_size) = optimal_padding_size(data.len()) else {
        return data;
    };
    let padding_needed = target_size - data.len();
    if padding_needed == 0 || padding_needed > u8::MAX as usize {
        return data;
    }
    data.extend(std::iter::repeat_n(padding_needed as u8, padding_needed));
    data
}

fn optimal_padding_size(data_len: usize) -> Option<usize> {
    let total_size = data_len.saturating_add(16);
    PADDING_BLOCK_SIZES
        .iter()
        .copied()
        .find(|block_size| total_size <= *block_size)
        .filter(|block_size| *block_size > data_len)
}

fn unpad(data: &[u8]) -> Vec<u8> {
    let Some(&last) = data.last() else {
        return data.to_vec();
    };
    let padding_len = last as usize;
    if padding_len == 0 || padding_len > data.len() {
        return data.to_vec();
    }
    let padding_start = data.len() - padding_len;
    let Some(padding) = data.get(padding_start..) else {
        return data.to_vec();
    };
    if padding.iter().all(|byte| *byte == last) {
        data.get(..padding_start)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| data.to_vec())
    } else {
        data.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_packet() -> BitchatPacket {
        BitchatPacket {
            version: 1,
            packet_type: MessageType::Announce as u8,
            ttl: 7,
            timestamp_ms: 0x0102_0304_0506_0708,
            sender_id: [1, 2, 3, 4, 5, 6, 7, 8],
            recipient_id: None,
            route: Vec::new(),
            is_rsr: false,
            payload: vec![9, 10, 11],
            signature: None,
        }
    }

    #[test]
    fn v1_packet_encoding_matches_current_bitchat_layout() {
        let encoded = sample_packet().encode(false).expect("encode");

        assert_eq!(
            encoded,
            vec![
                0x01, 0x01, 0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x00, 0x00, 0x03,
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
            ]
        );
    }

    #[test]
    fn padded_v1_packet_decodes() {
        let packet = sample_packet();
        let encoded = packet.encode(true).expect("encode");

        assert_eq!(encoded.len(), 256);
        assert_eq!(BitchatPacket::decode(&encoded), Ok(packet));
    }

    #[test]
    fn v2_route_round_trips() {
        let mut packet = sample_packet();
        packet.version = 2;
        packet.packet_type = MessageType::NoiseEncrypted as u8;
        packet.recipient_id = Some([8, 7, 6, 5, 4, 3, 2, 1]);
        packet.route = vec![[0xaa; PEER_ID_SIZE], [0xbb; PEER_ID_SIZE]];

        let encoded = packet.encode(false).expect("encode");
        assert_eq!(BitchatPacket::decode(&encoded), Ok(packet));
    }

    #[test]
    fn signing_bytes_clear_mutable_fields() {
        let mut packet = sample_packet();
        packet.ttl = 5;
        packet.is_rsr = true;
        packet.signature = Some([0x77; SIGNATURE_SIZE]);

        let signed = BitchatPacket::decode(&packet.signing_bytes().expect("signing bytes"))
            .expect("decode signing bytes");

        assert_eq!(packet.signing_bytes().expect("signing bytes").len(), 256);
        assert_eq!(signed.ttl, 0);
        assert!(!signed.is_rsr);
        assert!(signed.signature.is_none());
        assert_eq!(signed.payload, packet.payload);
    }

    #[test]
    fn announcement_tlv_round_trips_and_skips_unknown_fields() {
        let packet = AnnouncementPacket {
            nickname: "mac".to_string(),
            noise_public_key: [0x11; 32],
            signing_public_key: [0x22; 32],
            direct_neighbors: vec![[0x33; PEER_ID_SIZE]],
        };
        let mut encoded = packet.encode().expect("encode");
        encoded.extend_from_slice(&[0xf0, 0x03, 0xaa, 0xbb, 0xcc]);

        assert_eq!(AnnouncementPacket::decode(&encoded), Ok(packet));
    }
}
