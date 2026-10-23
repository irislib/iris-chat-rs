// Runtime-only negative results. Neither fingerprints nor failure entries are
// serialized, checkpointed, logged, or sent to another participant.
const FAILED_INBOUND_RECEIVE_CACHE_LIMIT: usize = 8_192;
type InboundReceiveFingerprints =
    BTreeMap<NdrOwnerPubkey, nostr_double_ratchet::ReceiveInputFingerprint>;

#[derive(Clone, Copy)]
enum FailedInboundReceiveKind {
    InvalidHmac,
    TooManySkippedMessages,
}

impl FailedInboundReceiveKind {
    fn from_error(error: &NdrError) -> Option<Self> {
        match error {
            NdrError::Nip44(nostr::nips::nip44::Error::V2(
                nostr::nips::nip44::v2::ErrorV2::InvalidHmac,
            )) => Some(Self::InvalidHmac),
            NdrError::Domain(DomainError::TooManySkippedMessages) => {
                Some(Self::TooManySkippedMessages)
            }
            _ => None,
        }
    }

    fn into_error(self) -> NdrError {
        match self {
            Self::InvalidHmac => NdrError::Nip44(nostr::nips::nip44::Error::V2(
                nostr::nips::nip44::v2::ErrorV2::InvalidHmac,
            )),
            Self::TooManySkippedMessages => NdrError::Domain(DomainError::TooManySkippedMessages),
        }
    }
}

struct FailedInboundReceive {
    owner: NdrOwnerPubkey,
    state: nostr_double_ratchet::ReceiveInputFingerprint,
    envelope: [u8; 32],
    failure: FailedInboundReceiveKind,
}

fn inbound_receive_envelope_fingerprint(event: &Event, envelope: &MessageEnvelope) -> [u8; 32] {
    use nostr::hashes::{sha256, Hash, HashEngine};
    let mut hash = sha256::Hash::engine();
    hash.input(b"iris-pending-receive-input-v1");
    for bytes in [
        event.id.as_bytes().as_slice(),
        event.pubkey.as_bytes().as_slice(),
        &event.created_at.as_secs().to_be_bytes(),
        &[envelope.metadata_privacy_version],
        &envelope.sender.to_bytes(),
        &[u8::from(envelope.recipient.is_some())],
        &envelope
            .recipient
            .map_or([0; 32], |recipient| recipient.to_bytes()),
        &envelope.signer_secret_key,
        &envelope.created_at.get().to_be_bytes(),
        envelope.encrypted_header.as_bytes(),
        envelope.ciphertext.as_bytes(),
    ] {
        hash.input(&(bytes.len() as u64).to_be_bytes());
        hash.input(bytes);
    }
    sha256::Hash::from_engine(hash).to_byte_array()
}

impl ProtocolEngine {
    fn prune_failed_inbound_receives(&mut self) {
        if self.failed_inbound_receives.is_empty() {
            return;
        }
        let retained = self
            .pending_inbound
            .iter()
            .map(|pending| pending.event.id)
            .collect::<HashSet<_>>();
        self.failed_inbound_receives
            .retain(|id, _| retained.contains(id));
    }

    fn retry_pending_inbound_direct_events(
        &mut self,
        now: NdrUnixSeconds,
    ) -> anyhow::Result<Vec<ProtocolDecryptedMessage>> {
        if !self.has_due_pending_inbound_retry(now) {
            // An empty queue must release its memoized failures too.
            if self.pending_inbound.is_empty() {
                self.failed_inbound_receives.clear();
            }
            return Ok(Vec::new());
        }
        self.prune_failed_inbound_receives();
        // Never reuse a fingerprint across passes: sends, handshakes, imports,
        // owner-proof changes and rollbacks may all have changed live inputs.
        let mut fingerprints = InboundReceiveFingerprints::new();
        let result = self.retry_pending_inbound_direct_events_inner(now, &mut fingerprints);
        // Also run after an error: an earlier delivery may already have removed
        // its pending event before a later storage operation failed.
        self.prune_failed_inbound_receives();
        result
    }

    fn decrypt_pending_direct_message_with_cache(
        &mut self,
        pending: &ProtocolPendingInbound,
        fingerprints: &mut InboundReceiveFingerprints,
    ) -> anyhow::Result<DirectReceive> {
        let parsed;
        let envelope = match pending.envelope.as_ref() {
            Some(envelope) => envelope,
            None => {
                parsed = parse_message_event(&pending.event)?;
                &parsed
            }
        };
        self.decrypt_direct_message_envelope_with_cache(
            &pending.event,
            envelope,
            true,
            Some(fingerprints),
        )
    }

    fn receive_direct_message_with_cache(
        &mut self,
        event: &Event,
        envelope: &MessageEnvelope,
        owner: NdrOwnerPubkey,
        mut fingerprints: Option<&mut InboundReceiveFingerprints>,
    ) -> nostr_double_ratchet::Result<Option<nostr_double_ratchet::ReceivedMessage>> {
        let inputs = fingerprints.as_deref_mut().map(|fingerprints| {
            let state = *fingerprints
                .entry(owner)
                .or_insert_with(|| self.session_manager.receive_input_fingerprint(owner));
            (state, inbound_receive_envelope_fingerprint(event, envelope))
        });
        if let Some((state, envelope)) = inputs {
            if let Some(cached) = self.failed_inbound_receives.get(&event.id) {
                if cached.owner == owner && cached.state == state && cached.envelope == envelope {
                    return Err(cached.failure.into_error());
                }
            }
        }
        let mut rng = OsRng;
        let mut ctx = ProtocolContext::new(NdrUnixSeconds(event.created_at.as_secs()), &mut rng);
        #[cfg(test)]
        tests::record_inbound_receive_call();
        let result = self.session_manager.receive(&mut ctx, owner, envelope);
        self.failed_inbound_receives.remove(&event.id);
        match &result {
            Ok(Some(_)) => {
                // Receive applied a ratchet. Invalidate before decoding the
                // payload, recording a journal or saving can subsequently fail.
                if let Some(fingerprints) = fingerprints {
                    fingerprints.clear();
                }
            }
            Err(error) => {
                if let (Some((state, envelope)), Some(failure)) =
                    (inputs, FailedInboundReceiveKind::from_error(error))
                {
                    if self.failed_inbound_receives.len() < FAILED_INBOUND_RECEIVE_CACHE_LIMIT {
                        self.failed_inbound_receives.insert(
                            event.id,
                            FailedInboundReceive {
                                owner,
                                state,
                                envelope,
                                failure,
                            },
                        );
                    }
                }
            }
            Ok(None) => {}
        }
        result
    }
}
