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
    pub device_label: Option<String>,
    pub client_label: Option<String>,
}

#[uniffi::export]
pub fn encode_device_approval_qr(_owner_input: String, _device_input: String) -> String {
    String::new()
}

#[uniffi::export]
pub fn decode_device_approval_qr(raw: String) -> Option<DeviceApprovalQrPayload> {
    nostr_identity::parse_nostr_identity_device_approval_request(&raw, &[])
        .ok()
        .flatten()
        .map(|request| DeviceApprovalQrPayload {
            owner_input: String::new(),
            device_input: request.device_app_key_pubkey,
            device_label: request.label,
            client_label: None,
        })
}

#[cfg(test)]
mod tests {
    use super::{decode_device_approval_qr, encode_device_approval_qr, DeviceApprovalQrPayload};
    use nostr_identity::{
        create_nostr_identity_device_approval_request,
        encode_nostr_identity_device_approval_request,
        CreateNostrIdentityDeviceApprovalRequestOptions,
    };
    use nostr_sdk::Keys;

    #[test]
    fn removed_device_approval_qr_encoder_returns_empty() {
        let encoded = encode_device_approval_qr("npub-owner".into(), "npub-device".into());
        assert_eq!(encoded, "");
    }

    #[test]
    fn device_approval_qr_rejects_wrong_inputs() {
        assert!(decode_device_approval_qr("".into()).is_none());
        assert!(decode_device_approval_qr("npub1plainvalue".into()).is_none());
        assert!(decode_device_approval_qr("https://example.com".into()).is_none());
        assert!(decode_device_approval_qr("not-a-device-approval-request".into()).is_none());
        assert!(decode_device_approval_qr(format!(
            "{}.{}.not-base64!*",
            "1".repeat(64),
            "1".repeat(64)
        ))
        .is_none());
    }

    #[test]
    fn device_link_qr_decodes_full_approval_request_to_device() {
        let device = Keys::generate();
        let request = Keys::generate();
        let local = create_nostr_identity_device_approval_request(
            &device,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: Some(request),
                request_secret: Some(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                ),
                requested_at: 41,
                request_type: Some("device_link".to_string()),
                resources: Vec::new(),
                expires_at: None,
                profile_id: None,
                admin_app_key_pubkey: None,
                label: Some("Safari on macOS".to_string()),
            },
        )
        .expect("approval request");
        let encoded =
            encode_nostr_identity_device_approval_request(&local.request, None).expect("encode");

        let decoded = decode_device_approval_qr(encoded.clone()).expect("decode approval request");
        assert_eq!(
            decoded,
            DeviceApprovalQrPayload {
                owner_input: String::new(),
                device_input: device.public_key().to_hex(),
                device_label: Some("Safari on macOS".to_string()),
                client_label: None,
            }
        );

        let prefixed = format!("prefix:{encoded}");
        assert!(decode_device_approval_qr(prefixed).is_none());
    }
}
