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

#[uniffi::export]
pub fn is_device_approval_bootstrap(raw: String) -> bool {
    nostr_identity::parse_nostr_identity_device_approval_bootstrap(&raw, &[])
        .ok()
        .flatten()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::is_device_approval_bootstrap;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use nostr_identity::{
        create_nostr_identity_device_approval_request,
        encode_nostr_identity_device_approval_bootstrap, nostr_identity_device_approval_bootstrap,
        CreateNostrIdentityDeviceApprovalRequestOptions,
    };
    use nostr_sdk::Keys;

    #[test]
    fn device_approval_bootstrap_rejects_non_bootstrap_inputs() {
        assert!(!is_device_approval_bootstrap("".into()));
        assert!(!is_device_approval_bootstrap("npub1plainvalue".into()));
        assert!(!is_device_approval_bootstrap("https://example.com".into()));
        assert!(!is_device_approval_bootstrap(
            "nostr-identity://device-approval/?app_key=npub1legacy".into()
        ));
        assert!(!is_device_approval_bootstrap(format!(
            "{}.{}.not-base64!*",
            "1".repeat(64),
            "1".repeat(64)
        )));
    }

    #[test]
    fn device_link_qr_accepts_only_shared_bootstrap() {
        let device = Keys::generate();
        let request = Keys::generate();
        let local = create_nostr_identity_device_approval_request(
            &device,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: Some(request),
                request_secret: Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE".to_string()),
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
        let bootstrap =
            nostr_identity_device_approval_bootstrap(&local.request).expect("bootstrap");
        let encoded = encode_nostr_identity_device_approval_bootstrap(&bootstrap, None)
            .expect("encode bootstrap");
        let legacy_full_request = format!(
            "nostr-identity://device-approval/{}",
            URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&local.request).expect("legacy full request JSON"))
        );

        assert!(is_device_approval_bootstrap(encoded.clone()));
        assert!(!is_device_approval_bootstrap(legacy_full_request));
        assert_eq!(
            serde_json::to_value(bootstrap)
                .expect("bootstrap JSON")
                .as_object()
                .expect("bootstrap object")
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            ["deviceAppKeyNpub", "requestNpub", "requestSecret"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        let prefixed = format!("prefix:{encoded}");
        assert!(!is_device_approval_bootstrap(prefixed));
    }
}
