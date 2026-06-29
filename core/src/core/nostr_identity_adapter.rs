use super::*;
use sha2::{Digest, Sha256};

const NOSTR_IDENTITY_ROSTER_TYPE: &str = "nostr_identity_roster_op";
const NOSTR_IDENTITY_ROSTER_SCHEMA: &str = "1";
const NOSTR_IDENTITY_KEY_PURPOSE_APP: &str = "app";
const NOSTR_IDENTITY_CAPABILITY_ADMIN: &str = "admin";
const NOSTR_IDENTITY_CAPABILITY_WRITE: &str = "write";

pub(super) fn build_nostr_identity_add_app_key_event(
    owner_keys: &Keys,
    owner_pubkey: PublicKey,
    device_pubkey: PublicKey,
    key_added_at_secs: u64,
    event_created_at_secs: u64,
    can_admin: bool,
) -> anyhow::Result<Event> {
    if owner_keys.public_key() != owner_pubkey {
        anyhow::bail!("NostrIdentity roster op signer must be the owner key");
    }

    let profile_id = nostr_identity_profile_id_for_owner(owner_pubkey);
    let owner_hex = owner_pubkey.to_hex();
    let device_hex = device_pubkey.to_hex();
    let event_created_at = event_created_at_secs.to_string();
    let key_added_at = key_added_at_secs.to_string();
    let client_nonce = format!("iris-chat-add-app-key-{device_hex}-{event_created_at}");

    let mut tags = vec![
        tag(["i", profile_id.as_str(), "subject"])?,
        tag(["type", NOSTR_IDENTITY_ROSTER_TYPE])?,
        tag(["schema", NOSTR_IDENTITY_ROSTER_SCHEMA])?,
        tag(["actor_pubkey", owner_hex.as_str()])?,
        tag(["client_nonce", client_nonce.as_str()])?,
        tag(["created_at", event_created_at.as_str()])?,
        tag(["op", "add_key"])?,
        tag(["key_pubkey", device_hex.as_str()])?,
        tag(["key_purpose", NOSTR_IDENTITY_KEY_PURPOSE_APP])?,
        tag(["key_capability", NOSTR_IDENTITY_CAPABILITY_WRITE])?,
    ];
    if can_admin {
        tags.push(tag(["key_capability", NOSTR_IDENTITY_CAPABILITY_ADMIN])?);
    }
    tags.push(tag(["key_added_at", key_added_at.as_str()])?);

    EventBuilder::new(Kind::from(NOSTR_IDENTITY_ROSTER_OP_KIND as u16), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(event_created_at_secs))
        .sign_with_keys(owner_keys)
        .map_err(|error| anyhow::anyhow!(error))
}

fn tag<const N: usize>(values: [&str; N]) -> anyhow::Result<nostr::Tag> {
    nostr::Tag::parse(values).map_err(|error| anyhow::anyhow!(error))
}

fn nostr_identity_profile_id_for_owner(owner_pubkey: PublicKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"iris-chat:nostr-identity-profile:");
    hasher.update(owner_pubkey.to_hex().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // TODO: switch to the shared nostr-identity crate's NostrIdentityId
    // once iris-chat-rs can depend on it without a local cross-repo path.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}
