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

const DEVICE_APPROVAL_QR_SCHEME: &str = "ndrdemo";
const DEVICE_APPROVAL_QR_HOST: &str = "device-link";

#[uniffi::export]
pub fn encode_device_approval_qr(owner_input: String, device_input: String) -> String {
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
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = Url::parse(trimmed).ok()?;
    if !parsed
        .scheme()
        .eq_ignore_ascii_case(DEVICE_APPROVAL_QR_SCHEME)
    {
        return None;
    }
    if !parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(DEVICE_APPROVAL_QR_HOST))
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

    #[test]
    fn device_approval_qr_round_trip() {
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
    fn device_approval_qr_rejects_wrong_inputs() {
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
}
