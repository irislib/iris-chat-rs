use qrcode::{EcLevel, QrCode};
use url::Url;

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

const LEGACY_DEVICE_APPROVAL_QR_SCHEME: &str = "ndrdemo";
const LEGACY_DEVICE_APPROVAL_QR_HOST: &str = "device-link";

#[uniffi::export]
pub fn encode_device_approval_qr(owner_input: String, device_input: String) -> String {
    encode_legacy_device_approval_qr(owner_input, device_input)
}

fn encode_legacy_device_approval_qr(owner_input: String, device_input: String) -> String {
    let owner = owner_input.trim();
    let device = device_input.trim();
    if owner.is_empty() || device.is_empty() {
        return String::new();
    }

    let Ok(mut url) = Url::parse("ndrdemo://device-link") else {
        return String::new();
    };
    url.query_pairs_mut()
        .append_pair("owner", owner)
        .append_pair("device", device);
    url.to_string()
}

#[uniffi::export]
pub fn decode_device_approval_qr(raw: String) -> Option<DeviceApprovalQrPayload> {
    decode_nostr_identity_device_approval_qr(&raw)
        .or_else(|| decode_legacy_device_approval_qr(&raw))
}

fn decode_nostr_identity_device_approval_qr(raw: &str) -> Option<DeviceApprovalQrPayload> {
    let request = nostr_identity::parse_nostr_identity_device_approval_request(raw.trim(), &[])
        .ok()
        .flatten()?;
    Some(DeviceApprovalQrPayload {
        owner_input: request.admin_app_key_pubkey?,
        device_input: request.device_app_key_pubkey,
    })
}

fn decode_legacy_device_approval_qr(raw: &str) -> Option<DeviceApprovalQrPayload> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = Url::parse(trimmed).ok()?;
    if !parsed
        .scheme()
        .eq_ignore_ascii_case(LEGACY_DEVICE_APPROVAL_QR_SCHEME)
    {
        return None;
    }
    if !parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(LEGACY_DEVICE_APPROVAL_QR_HOST))
    {
        return None;
    }

    let mut owner_input = None;
    let mut device_input = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "owner" => {
                let trimmed_value = value.trim();
                if !trimmed_value.is_empty() {
                    owner_input = Some(trimmed_value.to_string());
                }
            }
            "device" => {
                let trimmed_value = value.trim();
                if !trimmed_value.is_empty() {
                    device_input = Some(trimmed_value.to_string());
                }
            }
            _ => {}
        }
    }

    Some(DeviceApprovalQrPayload {
        owner_input: owner_input?,
        device_input: device_input?,
    })
}

#[cfg(test)]
mod tests {
    use super::{decode_device_approval_qr, encode_device_approval_qr, DeviceApprovalQrPayload};
    use nostr_identity::{
        create_nostr_identity_device_approval_request,
        encode_nostr_identity_device_approval_request,
        CreateNostrIdentityDeviceApprovalRequestOptions, NostrIdentityId,
    };
    use nostr_sdk::Keys;

    #[test]
    fn legacy_device_approval_qr_round_trip() {
        let encoded = encode_device_approval_qr("npub-owner".into(), "npub-device".into());
        let decoded = decode_device_approval_qr(encoded).expect("decode");
        assert_eq!(
            decoded,
            DeviceApprovalQrPayload {
                owner_input: "npub-owner".into(),
                device_input: "npub-device".into(),
            }
        );
    }

    #[test]
    fn legacy_device_approval_qr_rejects_wrong_inputs() {
        assert!(decode_device_approval_qr("".into()).is_none());
        assert!(decode_device_approval_qr("npub1plainvalue".into()).is_none());
        assert!(decode_device_approval_qr("https://example.com".into()).is_none());
        assert!(
            decode_device_approval_qr("ndrdemo://device-link?owner=npub1owneronly".into())
                .is_none()
        );
        assert!(
            decode_device_approval_qr("ndrdemo://device-link?device=npub1deviceonly".into())
                .is_none()
        );
    }

    #[test]
    fn shared_nostr_identity_device_approval_qr_decodes_to_owner_and_device() {
        let admin = Keys::generate();
        let device = Keys::generate();
        let request = create_nostr_identity_device_approval_request(
            &device,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: None,
                requested_at: 123,
                request_type: Some("device_link".to_string()),
                resources: Vec::new(),
                expires_at: None,
                profile_id: Some(NostrIdentityId::new_v4()),
                admin_app_key_pubkey: Some(admin.public_key().to_hex()),
                label: Some("Phone".to_string()),
            },
        )
        .expect("approval request");
        let encoded =
            encode_nostr_identity_device_approval_request(&request.request, None).expect("encode");

        let decoded = decode_device_approval_qr(encoded).expect("decode shared request");
        assert_eq!(decoded.owner_input, admin.public_key().to_hex());
        assert_eq!(decoded.device_input, device.public_key().to_hex());
    }
}
