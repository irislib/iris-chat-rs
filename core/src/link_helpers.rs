use anyhow::Context;
use iris_chat_protocol::APP_KEYS_EVENT_KIND;
use nostr::{Filter, Kind, PublicKey, SecretKey};
use nostr_double_ratchet::{DevicePubkey, Invite, UnixSeconds};
use sha2::{Digest, Sha256};

const APP_KEYS_D_TAG: &str = "double-ratchet/app-keys";
const LINK_INVITE_PURPOSE: &str = "link";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceLinkRequest {
    pub(crate) device_app_key_pubkey: PublicKey,
    pub(crate) request_secret: String,
}

pub(crate) fn encode_compact_device_link_request(
    device_pubkey: PublicKey,
    request_secret: &str,
) -> anyhow::Result<String> {
    parse_secret_bytes(request_secret)?;
    Ok(format!(
        "{}.{}",
        device_pubkey.to_hex(),
        request_secret.trim()
    ))
}

pub(crate) fn parse_compact_device_link_request(raw: &str) -> anyhow::Result<DeviceLinkRequest> {
    let trimmed = raw.trim();
    let (device_hex, secret_hex) = trimmed
        .split_once('.')
        .ok_or_else(|| anyhow::anyhow!("invalid compact link request"))?;
    if secret_hex.contains('.') {
        anyhow::bail!("invalid compact link request");
    }
    let device_app_key_pubkey =
        PublicKey::parse(device_hex).context("invalid compact link device pubkey")?;
    parse_secret_bytes(secret_hex).context("invalid compact link secret")?;
    Ok(DeviceLinkRequest {
        device_app_key_pubkey,
        request_secret: secret_hex.to_string(),
    })
}

pub(crate) fn deterministic_link_invite_for_device(
    device_pubkey: PublicKey,
    request_secret: &str,
) -> anyhow::Result<Invite> {
    let secret = parse_secret_bytes(request_secret)?;
    let inviter_device_pubkey = DevicePubkey::from_bytes(device_pubkey.to_bytes());
    let inviter_ephemeral_public_key = DevicePubkey::from_secret_bytes(secret)?;
    let shared_secret = derive_link_shared_secret(device_pubkey, &secret);
    Ok(Invite {
        inviter_device_pubkey,
        inviter_ephemeral_public_key,
        shared_secret,
        inviter_ephemeral_private_key: Some(secret),
        max_uses: Some(1),
        used_by: Vec::new(),
        used_response_contents: Vec::new(),
        created_at: UnixSeconds(0),
        inviter_owner_pubkey: None,
        purpose: Some(LINK_INVITE_PURPOSE.to_string()),
        inviter: device_pubkey,
        device_id: Some(device_pubkey.to_hex()),
        owner_public_key: None,
    })
}

pub(crate) fn deterministic_link_invite_for_device_link_request(
    request: &DeviceLinkRequest,
) -> anyhow::Result<Invite> {
    deterministic_link_invite_for_device(request.device_app_key_pubkey, &request.request_secret)
}

pub(crate) fn build_app_keys_device_authorization_filter(_device_pubkey: PublicKey) -> Filter {
    Filter::new()
        .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
        .identifier(APP_KEYS_D_TAG)
        .limit(100)
}

fn parse_secret_bytes(secret_hex: &str) -> anyhow::Result<[u8; 32]> {
    let secret = SecretKey::parse(secret_hex.trim())?;
    Ok(secret.secret_bytes())
}

fn derive_link_shared_secret(device_pubkey: PublicKey, request_secret: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"iris-chat:device-link:shared-secret:v1");
    hasher.update(device_pubkey.to_bytes());
    hasher.update(request_secret);
    hasher.finalize().into()
}
