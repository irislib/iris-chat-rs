use nostr_sdk::{Keys, PublicKey};
use qrcode::{EcLevel, QrCode};

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct QrCodeMatrix {
    /// Square module count (one side of the matrix).
    pub size: u32,
    /// "1" = dark module, "0" = light module. Length == size * size.
    /// We use a string instead of Vec<bool> to keep the FFI surface cheap.
    pub modules: String,
}

/// Render `text` to a QR-code module matrix. Returns a square matrix encoded
/// as `1`/`0` characters in row-major order. Returns `None` for inputs that
/// don't fit at the medium error-correction level.
#[uniffi::export]
pub fn encode_text_qr(text: String) -> Option<QrCodeMatrix> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let code = QrCode::with_error_correction_level(trimmed.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    let cells = code.to_colors();
    let mut modules = String::with_capacity(width * width);
    for color in cells {
        modules.push(if color == qrcode::Color::Dark {
            '1'
        } else {
            '0'
        });
    }
    Some(QrCodeMatrix {
        size: width as u32,
        modules,
    })
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DeviceApprovalQrPayload {
    pub owner_input: String,
    pub device_input: String,
}

#[uniffi::export]
pub fn encode_device_approval_qr(_owner_input: String, _device_input: String) -> String {
    String::new()
}

#[uniffi::export]
pub fn decode_device_approval_qr(raw: String) -> Option<DeviceApprovalQrPayload> {
    parse_compact_nostr_identity_device_approval_request(&raw).map(|request| {
        DeviceApprovalQrPayload {
            owner_input: String::new(),
            device_input: request.device_app_key_pubkey,
        }
    })
}

pub(crate) fn parse_compact_nostr_identity_device_approval_request(
    raw: &str,
) -> Option<nostr_identity::NostrIdentityDeviceApprovalRequest> {
    let payload = raw.trim();
    let mut parts = payload.split('.');
    let device_app_key_pubkey = parts.next()?.trim().to_ascii_lowercase();
    let request_secret = parts.next()?.trim().to_ascii_lowercase();
    if parts.next().is_some() {
        return None;
    }
    if !is_hex_32_bytes(&device_app_key_pubkey) || !is_hex_32_bytes(&request_secret) {
        return None;
    }
    PublicKey::parse(&device_app_key_pubkey).ok()?;
    let request_keys = Keys::parse(&request_secret).ok()?;

    Some(nostr_identity::NostrIdentityDeviceApprovalRequest {
        request_pubkey: request_keys.public_key().to_hex(),
        device_app_key_pubkey,
        request_secret,
        device_app_key_proof: String::new(),
        requested_at: 0,
        request_type: None,
        resources: Vec::new(),
        expires_at: None,
        profile_id: None,
        admin_app_key_pubkey: None,
        label: None,
    })
}

fn is_hex_32_bytes(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{decode_device_approval_qr, encode_device_approval_qr, DeviceApprovalQrPayload};
    use nostr_sdk::Keys;

    #[test]
    fn removed_device_approval_qr_encoder_returns_empty() {
        let encoded = encode_device_approval_qr("npub-owner".into(), "npub-device".into());
        assert_eq!(encoded, "");
    }

    #[test]
    fn compact_device_approval_qr_rejects_wrong_inputs() {
        assert!(decode_device_approval_qr("".into()).is_none());
        assert!(decode_device_approval_qr("npub1plainvalue".into()).is_none());
        assert!(decode_device_approval_qr("https://example.com".into()).is_none());
        assert!(decode_device_approval_qr("nostr-identity://device-approval/abc".into()).is_none());
        assert!(decode_device_approval_qr(format!(
            "nostr-identity://device-approval/{}.{}",
            "1".repeat(64),
            "1".repeat(64)
        ))
        .is_none());
    }

    #[test]
    fn compact_nostr_identity_device_approval_qr_decodes_to_device() {
        let device = Keys::generate();
        let request = Keys::generate();
        let encoded = format!(
            "{}.{}",
            device.public_key().to_hex(),
            request.secret_key().to_secret_hex()
        );

        let decoded = decode_device_approval_qr(encoded.clone()).expect("decode compact request");
        assert_eq!(
            decoded,
            DeviceApprovalQrPayload {
                owner_input: String::new(),
                device_input: device.public_key().to_hex(),
            }
        );

        let prefixed = format!("nostr-identity://device-approval/{encoded}");
        assert!(decode_device_approval_qr(prefixed).is_none());
    }
}
