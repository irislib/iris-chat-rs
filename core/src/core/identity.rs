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
    Ok(OwnerPubkey::from_bytes(pubkey.to_bytes()))
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
    owners.sort_by_key(|owner| owner.to_string());
    owners.dedup();
    Ok(owners)
}

pub(super) fn owner_pubkeys_from_hexes(hexes: &[String]) -> anyhow::Result<Vec<OwnerPubkey>> {
    hexes
        .iter()
        .map(|hex| parse_owner_input(hex))
        .collect::<anyhow::Result<Vec<_>>>()
}

pub(super) fn sorted_owner_hexes(owners: &[OwnerPubkey]) -> Vec<String> {
    let mut hexes = owners.iter().map(ToString::to_string).collect::<Vec<_>>();
    hexes.sort();
    hexes.dedup();
    hexes
}

pub(super) fn parse_device_input(input: &str) -> anyhow::Result<DevicePubkey> {
    let (_, pubkey) = parse_peer_input(input)?;
    Ok(DevicePubkey::from_bytes(pubkey.to_bytes()))
}

#[cfg(test)]
pub(super) fn local_owner_from_keys(keys: &Keys) -> OwnerPubkey {
    OwnerPubkey::from_bytes(keys.public_key().to_bytes())
}

pub(super) fn local_device_from_keys(keys: &Keys) -> DevicePubkey {
    DevicePubkey::from_bytes(keys.public_key().to_bytes())
}

pub(super) fn owner_npub_from_owner(owner_pubkey: OwnerPubkey) -> Option<String> {
    PublicKey::parse(owner_pubkey.to_string())
        .ok()?
        .to_bech32()
        .ok()
}

pub(super) fn device_npub(device_hex: &str) -> Option<String> {
    PublicKey::parse(device_hex).ok()?.to_bech32().ok()
}

pub(super) fn local_roster_from_session_manager(
    session_manager: &SessionManager,
) -> Option<DeviceRoster> {
    let snapshot = session_manager.snapshot();
    let owner = snapshot.local_owner_pubkey;
    snapshot
        .users
        .into_iter()
        .find(|user| user.owner_pubkey == owner)
        .and_then(|user| user.roster)
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

pub(super) fn derive_local_authorization_state(
    has_owner_signing_authority: bool,
    owner_pubkey: OwnerPubkey,
    local_device_pubkey: DevicePubkey,
    session_manager: &SessionManager,
    previous_state: Option<LocalAuthorizationState>,
) -> LocalAuthorizationState {
    let local_roster = session_manager
        .snapshot()
        .users
        .into_iter()
        .find(|user| user.owner_pubkey == owner_pubkey)
        .and_then(|user| user.roster);
    match local_roster {
        Some(roster) => {
            if roster.get_device(&local_device_pubkey).is_some() {
                LocalAuthorizationState::Authorized
            } else if has_owner_signing_authority {
                LocalAuthorizationState::Authorized
            } else if matches!(
                previous_state,
                Some(LocalAuthorizationState::Authorized) | Some(LocalAuthorizationState::Revoked)
            ) {
                LocalAuthorizationState::Revoked
            } else {
                LocalAuthorizationState::AwaitingApproval
            }
        }
        None if has_owner_signing_authority => LocalAuthorizationState::Authorized,
        None => LocalAuthorizationState::AwaitingApproval,
    }
}

pub(super) fn chat_unavailable_message(logged_in: Option<&LoggedInState>) -> &'static str {
    match logged_in.map(|logged_in| logged_in.authorization_state) {
        Some(LocalAuthorizationState::AwaitingApproval) => {
            "This device is still waiting for approval."
        }
        Some(LocalAuthorizationState::Revoked) => {
            "This device has been removed from the roster. Log out to continue."
        }
        _ => "Create or restore an account first.",
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
