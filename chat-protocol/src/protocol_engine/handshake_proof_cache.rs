const HANDSHAKE_PROOF_CACHE_ENTRIES: usize = 256;
const HANDSHAKE_PROOF_CACHE_BYTES: usize = 1024 * 1024;
type HandshakeProofKey = (nostr::EventId, nostr::EventId);

/// Retry the exact signed response. Re-encrypting its unchanged owner proof
/// randomizes the event ID, defeating both outbox and receiver deduplication.
/// Only public signed events are retained; this cache is not durable state.
#[derive(Default)]
struct HandshakeProofCache {
    events: BTreeMap<HandshakeProofKey, (Event, usize)>,
    order: std::collections::VecDeque<HandshakeProofKey>,
    bytes: usize,
}

impl HandshakeProofCache {
    fn insert(&mut self, key: HandshakeProofKey, event: Event) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&event)?.len();
        if bytes > HANDSHAKE_PROOF_CACHE_BYTES || self.events.contains_key(&key) {
            return Ok(());
        }
        while self.events.len() >= HANDSHAKE_PROOF_CACHE_ENTRIES
            || self.bytes + bytes > HANDSHAKE_PROOF_CACHE_BYTES
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, size)) = self.events.remove(&oldest) {
                self.bytes -= size;
            }
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.events.insert(key, (event, bytes));
        Ok(())
    }
}

impl ProtocolEngine {
    fn cached_invite_response_with_owner_proof(
        &self,
        response: &nostr_double_ratchet::InviteResponseEnvelope,
    ) -> anyhow::Result<Event> {
        let base = invite_response_event(response)?;
        // Check current authorization before consulting the cache. Changed
        // signed rosters use a different key; revoked devices reuse no proof.
        let Some(proof) = self.local_handshake_owner_proof() else {
            return Ok(base);
        };
        let key = (base.id, proof.id);
        if let Some((event, _)) = self.handshake_proof_cache.borrow().events.get(&key) {
            return Ok(event.clone());
        }
        let event = invite_response_with_owner_proof(response, Some(proof))?;
        self.handshake_proof_cache
            .borrow_mut()
            .insert(key, event.clone())?;
        Ok(event)
    }
}
