// Optional encrypted extension on the existing signed invite response. Keeping
// the original content unchanged lets older clients continue parsing it.
const HANDSHAKE_OWNER_PROOF_TAG: &str = "owner-proof";
const HANDSHAKE_OWNER_PROOF_MAX_BYTES: usize = 32 * 1024;

impl ProtocolEngine {
    /// Whether retained, verified identity-signed evidence authorizes this device.
    pub fn signed_local_device_authorization(&self) -> Option<bool> {
        self.invite_owner_app_keys_evidence
            .get(&self.local_owner)
            .map(|_| self.local_handshake_owner_proof().is_some())
    }

    fn local_handshake_owner_proof(&self) -> Option<&Event> {
        let ProtocolAppKeysEvidence::Verified(event) =
            self.invite_owner_app_keys_evidence.get(&self.local_owner)?
        else {
            return None;
        };
        let device = public_device(self.local_device).ok()?;
        self.invite_owner_exact_app_keys_membership(self.local_owner, device)
            .filter(|authorized| *authorized)?;
        Some(event)
    }

    /// Read the account-signed authorization carried by a successfully decrypted
    /// handshake. The caller supplies the owner and device authenticated by that
    /// handshake, so unrelated signed records cannot establish its sender.
    pub fn ingest_invite_response_owner_proof(
        &mut self,
        invite: &Invite,
        response: &Event,
        claimed_owner: PublicKey,
        authenticated_device: PublicKey,
    ) -> anyhow::Result<ProtocolRetryBatch> {
        let Some(proof) = handshake_owner_proof(invite, response)
            .filter(|proof| proof.pubkey == claimed_owner)
            .filter(|proof| {
                AppKeys::from_event(proof)
                    .is_ok_and(|keys| keys.get_device(&authenticated_device).is_some())
            })
        else {
            return Ok(ProtocolRetryBatch::default());
        };
        // The normal ingestion path retains newer revocations and conflicting
        // heads, persists exact signed evidence, and retries parked messages.
        self.ingest_app_keys_event(&proof)
    }
}

fn invite_response_with_owner_proof(
    response: &nostr_double_ratchet::InviteResponseEnvelope,
    proof: Option<&Event>,
) -> anyhow::Result<Event> {
    let event = invite_response_event(response)?;
    let Some(proof) = proof else {
        return Ok(event);
    };
    let json = serde_json::to_string(proof)?;
    anyhow::ensure!(
        json.len() <= HANDSHAKE_OWNER_PROOF_MAX_BYTES,
        "owner proof too large"
    );
    let keys = Keys::new(nostr::SecretKey::from_slice(&response.signer_secret_key)?);
    let encrypted = nostr::nips::nip44::encrypt(
        keys.secret_key(),
        &public_device(response.recipient)?,
        json,
        nostr::nips::nip44::Version::V2,
    )?;
    Ok(nostr::EventBuilder::new(event.kind, event.content.clone())
        .tags(event.tags.iter().cloned())
        .tag(nostr::Tag::parse([
            HANDSHAKE_OWNER_PROOF_TAG,
            encrypted.as_str(),
        ])?)
        .custom_created_at(event.created_at)
        .sign_with_keys(&keys)?)
}

fn handshake_owner_proof(invite: &Invite, response: &Event) -> Option<Event> {
    // The optional extension never replaces validation of the original handshake.
    let envelope = parse_invite_response_event(response).ok()?;
    if envelope.recipient != invite.inviter_ephemeral_public_key {
        return None;
    }
    let mut tags = response.tags.iter().filter(|tag| {
        tag.as_slice()
            .first()
            .is_some_and(|name| name == HANDSHAKE_OWNER_PROOF_TAG)
    });
    let tag = tags.next()?.as_slice();
    if tag.len() != 2 || tags.next().is_some() || tag[1].len() > HANDSHAKE_OWNER_PROOF_MAX_BYTES * 2
    {
        return None;
    }
    let secret = nostr::SecretKey::from_slice(&invite.inviter_ephemeral_private_key?).ok()?;
    let json = nostr::nips::nip44::decrypt(&secret, &response.pubkey, &tag[1]).ok()?;
    if json.len() > HANDSHAKE_OWNER_PROOF_MAX_BYTES {
        return None;
    }
    let proof: Event = serde_json::from_str(&json).ok()?;
    app_keys_event_is_acceptable_for_owner(proof.pubkey, &proof, unix_now().get()).then_some(proof)
}
