use super::*;
use nostr::nips::nip19::{FromBech32, Nip19};

pub(crate) fn parse_peer_input(input: &str) -> anyhow::Result<(String, PublicKey)> {
    let normalized = normalize_peer_input_for_display(input);
    let pubkey = PublicKey::parse(&normalized)?;
    Ok((pubkey.to_hex(), pubkey))
}

pub(crate) fn normalize_peer_input_for_display(input: &str) -> String {
    let normalized = compact_identity_input(input);

    if let Some(pubkey) = extract_nip19_identity(&normalized) {
        return pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex());
    }

    match PublicKey::parse(&normalized) {
        Ok(pubkey) if normalized.starts_with("npub1") => {
            pubkey.to_bech32().unwrap_or_else(|_| normalized.clone())
        }
        Ok(pubkey) => pubkey.to_hex(),
        Err(_) => normalized,
    }
}

fn compact_identity_input(input: &str) -> String {
    let compact = input
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact
        .strip_prefix("nostr:")
        .unwrap_or(&compact)
        .to_string()
}

fn extract_nip19_identity(input: &str) -> Option<PublicKey> {
    for prefix in ["npub1", "nprofile1"] {
        let Some(start) = input.find(prefix) else {
            continue;
        };
        let token = take_bech32_token(&input[start..]);
        if let Ok(nip19) = Nip19::from_bech32(token) {
            match nip19 {
                Nip19::Pubkey(pubkey) => return Some(pubkey),
                Nip19::Profile(profile) => return Some(profile.public_key),
                _ => {}
            }
        }
    }
    None
}

fn take_bech32_token(input: &str) -> &str {
    let end = input
        .find(|ch: char| !ch.is_ascii_alphanumeric())
        .unwrap_or(input.len());
    &input[..end]
}

pub(super) fn parse_owner_input(input: &str) -> anyhow::Result<OwnerPubkey> {
    let (_, pubkey) = parse_peer_input(input)?;
    Ok(pubkey)
}

pub(super) fn parse_owner_inputs(
    inputs: &[String],
    exclude_owner: OwnerPubkey,
) -> anyhow::Result<Vec<OwnerPubkey>> {
    let mut owners = inputs
        .iter()
        .map(|input| parse_owner_input(input))
        .collect::<anyhow::Result<Vec<_>>>()?;
    owners.retain(|owner| *owner != exclude_owner);
    owners.sort_by_key(|owner| owner.to_hex());
    owners.dedup();
    Ok(owners)
}

pub(super) fn parse_device_input(input: &str) -> anyhow::Result<DevicePubkey> {
    let (_, pubkey) = parse_peer_input(input)?;
    Ok(pubkey)
}

pub(super) fn local_device_from_keys(keys: &Keys) -> DevicePubkey {
    keys.public_key()
}

pub(super) fn owner_npub_from_owner(owner_pubkey: OwnerPubkey) -> Option<String> {
    owner_pubkey.to_bech32().ok()
}

pub(super) fn device_npub(device_hex: &str) -> Option<String> {
    PublicKey::parse(device_hex).ok()?.to_bech32().ok()
}

pub(super) fn public_authorization_state(
    state: LocalAuthorizationState,
) -> DeviceAuthorizationState {
    match state {
        LocalAuthorizationState::Authorized => DeviceAuthorizationState::Authorized,
        LocalAuthorizationState::AwaitingApproval => DeviceAuthorizationState::AwaitingApproval,
        LocalAuthorizationState::Revoked => DeviceAuthorizationState::Revoked,
    }
}

pub(super) fn chat_unavailable_message(logged_in: Option<&LoggedInState>) -> &'static str {
    match logged_in.map(|logged_in| logged_in.authorization_state) {
        Some(LocalAuthorizationState::AwaitingApproval) => {
            "This device is still waiting for approval."
        }
        Some(LocalAuthorizationState::Revoked) => {
            "This device has been removed from the profile. Log out to continue."
        }
        _ => "Create or restore a profile first.",
    }
}

pub(super) fn unix_now() -> UnixSeconds {
    UnixSeconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}
