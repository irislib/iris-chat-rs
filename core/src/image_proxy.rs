use crate::state::PreferencesSnapshot;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use url::Url;

pub const DEFAULT_IMAGE_PROXY_URL: &str = "https://imgproxy.iris.to";
pub const DEFAULT_IMAGE_PROXY_KEY_HEX: &str =
    "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996";
pub const DEFAULT_IMAGE_PROXY_SALT_HEX: &str =
    "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c";

type HmacSha256 = Hmac<Sha256>;

pub fn proxied_image_url(
    original_src: &str,
    preferences: &PreferencesSnapshot,
    width: Option<u32>,
    height: Option<u32>,
    square: bool,
) -> String {
    let input = original_src.trim();
    if input.is_empty() || !preferences.image_proxy_enabled {
        return original_src.to_string();
    }
    if input.starts_with("data:") || input.starts_with("blob:") {
        return original_src.to_string();
    }

    let Ok(source_url) = Url::parse(input) else {
        return original_src.to_string();
    };
    if !is_http_url(&source_url) {
        return original_src.to_string();
    }

    let proxy_base = resolved_proxy_url(preferences);
    let Ok(proxy_url) = Url::parse(&proxy_base) else {
        return original_src.to_string();
    };
    if !is_http_url(&proxy_url) {
        return original_src.to_string();
    }
    if input.starts_with(&proxy_base) {
        return original_src.to_string();
    }

    let mut options = Vec::new();
    if let (Some(resize_width), Some(resize_height)) = (
        normalized_dimension(width, height),
        normalized_dimension(height, width),
    ) {
        let mode = if square { "fill" } else { "fit" };
        options.push(format!("rs:{mode}:{resize_width}:{resize_height}"));
    }
    options.push("dpr:2".to_string());

    let encoded_source = URL_SAFE_NO_PAD.encode(input.as_bytes());
    let path = format!("/{}/{}", options.join("/"), encoded_source);
    let Some(signature) = sign_path(&path, preferences) else {
        return original_src.to_string();
    };

    format!("{}/{}{}", proxy_base.trim_end_matches('/'), signature, path)
}

fn resolved_proxy_url(preferences: &PreferencesSnapshot) -> String {
    let trimmed = preferences.image_proxy_url.trim();
    if trimmed.is_empty() {
        DEFAULT_IMAGE_PROXY_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn sign_path(path: &str, preferences: &PreferencesSnapshot) -> Option<String> {
    let key = decode_hex(resolved_hex(
        &preferences.image_proxy_key_hex,
        DEFAULT_IMAGE_PROXY_KEY_HEX,
    ))?;
    let salt = decode_hex(resolved_hex(
        &preferences.image_proxy_salt_hex,
        DEFAULT_IMAGE_PROXY_SALT_HEX,
    ))?;
    let mut mac = HmacSha256::new_from_slice(&key).ok()?;
    mac.update(&salt);
    mac.update(path.as_bytes());
    Some(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn resolved_hex<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(normalized.len() / 2);
    for pair in normalized.as_bytes().chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn is_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}

fn normalized_dimension(value: Option<u32>, fallback: Option<u32>) -> Option<u32> {
    match value.or(fallback) {
        Some(candidate) if candidate > 0 => Some(candidate),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preferences() -> PreferencesSnapshot {
        PreferencesSnapshot::default()
    }

    #[test]
    fn disabled_proxy_returns_original_url() {
        let mut preferences = preferences();
        preferences.image_proxy_enabled = false;
        let input = "https://example.com/avatar.jpg";

        assert_eq!(
            proxied_image_url(input, &preferences, Some(64), Some(64), false),
            input
        );
    }

    #[test]
    fn ignores_non_http_data_blob_and_existing_proxy_urls() {
        let preferences = preferences();

        assert_eq!(
            proxied_image_url("data:image/png;base64,abc", &preferences, None, None, false),
            "data:image/png;base64,abc"
        );
        assert_eq!(
            proxied_image_url(
                "blob:https://example.com/123",
                &preferences,
                None,
                None,
                false
            ),
            "blob:https://example.com/123"
        );
        assert_eq!(
            proxied_image_url("file:///tmp/avatar.jpg", &preferences, None, None, false),
            "file:///tmp/avatar.jpg"
        );
        assert_eq!(
            proxied_image_url(
                "https://imgproxy.iris.to/signature/dpr:2/source",
                &preferences,
                None,
                None,
                false,
            ),
            "https://imgproxy.iris.to/signature/dpr:2/source"
        );
    }

    #[test]
    fn generates_deterministic_signed_proxy_url() {
        let preferences = preferences();
        let input = "https://example.com/avatar.jpg";

        let proxied = proxied_image_url(input, &preferences, Some(64), Some(64), true);

        assert!(proxied.starts_with("https://imgproxy.iris.to/"));
        assert!(proxied.contains("/rs:fill:64:64/"));
        assert!(proxied.contains("/dpr:2/"));
        assert!(proxied.contains("aHR0cHM6Ly9leGFtcGxlLmNvbS9hdmF0YXIuanBn"));
        assert_eq!(
            proxied_image_url(input, &preferences, Some(64), Some(64), true),
            proxied
        );
    }

    #[test]
    fn invalid_key_or_salt_returns_original_url() {
        let mut preferences = preferences();
        preferences.image_proxy_key_hex = "not-hex".to_string();
        preferences.image_proxy_salt_hex = "also-not-hex".to_string();
        let input = "https://example.com/avatar.jpg";

        assert_eq!(
            proxied_image_url(input, &preferences, None, None, false),
            input
        );
    }
}
