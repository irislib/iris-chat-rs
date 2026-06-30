use super::*;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(super) fn build_nostr_identity_add_app_key_event(
    owner_keys: &Keys,
    owner_pubkey: PublicKey,
    device_pubkey: PublicKey,
    parents: Vec<String>,
    key_added_at_secs: u64,
    event_created_at_secs: u64,
    can_admin: bool,
) -> anyhow::Result<Event> {
    if owner_keys.public_key() != owner_pubkey {
        anyhow::bail!("NostrIdentity roster op signer must be the owner key");
    }

    let profile_id = nostr_identity_profile_id_for_owner(owner_pubkey);
    let key_added_at = i64::try_from(key_added_at_secs)
        .map_err(|_| anyhow::anyhow!("NostrIdentity key added_at overflows i64"))?;
    let event_created_at = i64::try_from(event_created_at_secs)
        .map_err(|_| anyhow::anyhow!("NostrIdentity event created_at overflows i64"))?;
    let capabilities = if can_admin {
        nostr_identity::NostrIdentityCapabilities::app_admin()
    } else {
        nostr_identity::NostrIdentityCapabilities::app_writer()
    };
    let facet = nostr_identity::NostrIdentityFacet::app_key(
        device_pubkey.to_hex(),
        key_added_at,
        None,
        capabilities,
    );

    nostr_identity::build_nostr_identity_roster_op_event(
        owner_keys,
        profile_id,
        parents,
        None,
        nostr_identity::NostrIdentityRosterOp::AddFacet { facet },
        event_created_at,
    )
    .map_err(|error| anyhow::anyhow!(error))
}

pub(super) fn build_nostr_identity_owner_admin_event(
    owner_keys: &Keys,
    owner_pubkey: PublicKey,
    parents: Vec<String>,
    added_at_secs: u64,
    event_created_at_secs: u64,
) -> anyhow::Result<Event> {
    if owner_keys.public_key() != owner_pubkey {
        anyhow::bail!("NostrIdentity roster op signer must be the owner key");
    }

    let profile_id = nostr_identity_profile_id_for_owner(owner_pubkey);
    let added_at = i64::try_from(added_at_secs)
        .map_err(|_| anyhow::anyhow!("NostrIdentity owner added_at overflows i64"))?;
    let event_created_at = i64::try_from(event_created_at_secs)
        .map_err(|_| anyhow::anyhow!("NostrIdentity event created_at overflows i64"))?;
    let facet = nostr_identity::NostrIdentityFacet::app_key(
        owner_pubkey.to_hex(),
        added_at,
        None,
        nostr_identity::NostrIdentityCapabilities::app_admin(),
    );

    nostr_identity::build_nostr_identity_roster_op_event(
        owner_keys,
        profile_id,
        parents,
        None,
        nostr_identity::NostrIdentityRosterOp::AddFacet { facet },
        event_created_at,
    )
    .map_err(|error| anyhow::anyhow!(error))
}

pub(super) fn nostr_identity_profile_id_for_owner(
    owner_pubkey: PublicKey,
) -> nostr_identity::NostrIdentityId {
    let mut hasher = Sha256::new();
    hasher.update(b"iris-chat:nostr-identity-profile:");
    hasher.update(owner_pubkey.to_hex().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    for (target, source) in bytes.iter_mut().zip(digest.iter()) {
        *target = *source;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    nostr_identity::NostrIdentityId::from_uuid(Uuid::from_bytes(bytes))
}
