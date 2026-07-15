use super::*;

pub(super) fn pending_invite_response_owner(
    data_dir: &str,
    owner_pubkey: PublicKey,
    device_keys: &Keys,
    event: &nostr::Event,
) -> Option<String> {
    let shared_conn = super::super::storage::open_database(Path::new(data_dir)).ok()?;
    let storage = Arc::new(super::super::storage::SqliteStorageAdapter::new(
        shared_conn,
        owner_pubkey.to_hex(),
        device_keys.public_key().to_hex(),
    ));
    let device_secret = device_keys.secret_key().to_secret_bytes();
    let mut invites = Vec::new();

    let device_id = device_keys.public_key().to_hex();
    if let Some(serialized) = storage
        .get(&format!("device-invite/{device_id}"))
        .ok()
        .flatten()
    {
        if let Ok(mut invite) = Invite::deserialize(&serialized) {
            invite.owner_public_key = Some(owner_pubkey);
            invites.push(invite);
        }
    }
    if let Ok(private_invites) = super::super::invites::load_private_chat_invites(storage.as_ref())
    {
        invites.extend(private_invites.into_values());
    }

    invites.into_iter().find_map(|invite| {
        let response =
            nostr_double_ratchet::process_invite_response_event(&invite, event, device_secret)
                .ok()
                .flatten()?;
        let authenticated_device = response.invitee_identity;
        match (response.owner_public_key, response.invitee_owner_pubkey) {
            (None, None) => Some(authenticated_device.to_hex()),
            (Some(owner), Some(owner_claim))
                if owner.to_bytes() == owner_claim.to_bytes()
                    && iris_chat_protocol::ProtocolEngine::persisted_invite_owner_device_is_authorized(
                        storage.clone(),
                        owner_pubkey,
                        device_keys,
                        owner,
                        authenticated_device,
                    )
                    .unwrap_or(false) =>
            {
                Some(owner.to_hex())
            }
            _ => None,
        }
    })
}
