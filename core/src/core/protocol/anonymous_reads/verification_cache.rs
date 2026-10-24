//! Positive signature results for one retained discovery configuration. Every
//! event still has its canonical ID recomputed and its current filters checked.
use super::*;

const CAPACITY: usize = 64;

#[derive(PartialEq, Eq)]
struct VerifiedSignature {
    relay: usize,
    id: EventId,
    pubkey: PublicKey,
    signature: [u8; 64],
}

/// One lazy ring bounds all relay partitions of a single author worker together.
/// Relay indices are valid only until the worker's exact configuration changes.
#[derive(Default)]
pub(in crate::core::protocol) struct VerificationCache {
    entries: Vec<VerifiedSignature>,
    next: usize,
    #[cfg(test)]
    pub(in crate::core::protocol) signature_calls: Arc<std::sync::atomic::AtomicUsize>,
}

pub(in crate::core::protocol) struct RelayVerification<'a> {
    cache: &'a mut VerificationCache,
    relay: usize,
}

impl VerificationCache {
    pub(in crate::core::protocol) fn clear(&mut self) {
        self.entries = Vec::new();
        self.next = 0;
    }

    pub(in crate::core::protocol) fn relay(&mut self, relay: usize) -> RelayVerification<'_> {
        RelayVerification { cache: self, relay }
    }
}

impl RelayVerification<'_> {
    pub(super) fn reborrow(&mut self) -> RelayVerification<'_> {
        RelayVerification {
            cache: self.cache,
            relay: self.relay,
        }
    }

    pub(super) fn verify(&mut self, event: &Event) -> bool {
        // An ID-only hit must never accept changed content, tags, time or author.
        if !event.verify_id() {
            return false;
        }
        let key = VerifiedSignature {
            relay: self.relay,
            id: event.id,
            pubkey: event.pubkey,
            signature: event.sig.serialize(),
        };
        if self.cache.entries.contains(&key) {
            return true;
        }
        #[cfg(test)]
        self.cache
            .signature_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !event.verify_signature() {
            return false;
        }
        if self.cache.entries.len() < CAPACITY {
            self.cache.entries.push(key);
        } else if let Some(entry) = self.cache.entries.get_mut(self.cache.next) {
            *entry = key;
            self.cache.next = (self.cache.next + 1) % CAPACITY;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(count: usize) -> Vec<Event> {
        let keys = Keys::generate();
        (0..count)
            .map(|index| {
                EventBuilder::new(Kind::Metadata, index.to_string())
                    .sign_with_keys(&keys)
                    .unwrap()
            })
            .collect()
    }

    fn calls(cache: &VerificationCache) -> usize {
        cache
            .signature_calls
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn positive_signature_cache_reduces_crypto_calls_without_crossing_relays() {
        let events = events(32);
        let mut cache = VerificationCache::default();
        for _ in 0..3 {
            for event in &events {
                assert!(cache.relay(0).verify(event));
            }
        }
        assert_eq!(
            calls(&cache),
            32,
            "three rounds must verify each signature once"
        );
        for event in &events {
            assert!(cache.relay(1).verify(event));
        }
        assert_eq!(
            calls(&cache),
            64,
            "a different relay has no verification hits"
        );
        for event in &events {
            assert!(cache.relay(0).verify(event));
        }
        assert_eq!(calls(&cache), 64);
        cache.clear();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.entries.capacity(), 0);
        assert!(cache.relay(0).verify(&events[0]));
        assert_eq!(
            calls(&cache),
            65,
            "replacement configuration must verify again"
        );
    }

    #[test]
    fn positive_signature_cache_rechecks_signed_fields_and_never_caches_failures() {
        let events = events(2);
        let original = &events[0];
        let mut cache = VerificationCache::default();
        assert!(cache.relay(0).verify(original));
        let mut json = serde_json::to_value(original).unwrap();
        json["ignored_extra_field"] = serde_json::json!("not part of the signed event");
        let equivalent: Event =
            serde_json::from_str(&serde_json::to_string_pretty(&json).unwrap()).unwrap();
        assert_eq!(&equivalent, original);
        assert!(cache.relay(0).verify(&equivalent));
        assert_eq!(calls(&cache), 1);
        let mut mutations = vec![original.clone(); 6];
        mutations[0].content.push('!');
        mutations[1].pubkey = Keys::generate().public_key();
        mutations[2].created_at = Timestamp::from(original.created_at.as_secs() + 1);
        mutations[3].kind = Kind::TextNote;
        mutations[4].tags = nostr::Tags::from_list(vec![nostr::Tag::hashtag("changed")]);
        mutations[5].id = events[1].id;
        for event in mutations {
            assert!(!event.verify_id());
            assert!(!cache.relay(0).verify(&event));
        }
        assert_eq!(
            calls(&cache),
            1,
            "invalid IDs must fail before Schnorr verification"
        );
        let mut forged = original.clone();
        forged.sig = events[1].sig;
        assert!(forged.verify_id());
        assert!(!forged.verify_signature());
        for _ in 0..2 {
            assert!(!cache.relay(0).verify(&forged));
        }
        assert_eq!(
            calls(&cache),
            3,
            "invalid signatures must not become cache entries"
        );
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.relay(0).verify(original));
        assert_eq!(calls(&cache), 3);
        let mut changed_body = original.clone();
        changed_body.content.push('!');
        changed_body.id = EventId::new(
            &changed_body.pubkey,
            &changed_body.created_at,
            &changed_body.kind,
            &changed_body.tags,
            &changed_body.content,
        );
        assert!(changed_body.verify_id());
        assert!(!cache.relay(0).verify(&changed_body));
        assert_eq!(
            calls(&cache),
            4,
            "a recomputed ID cannot reuse the old signature result"
        );
        assert!(cache.relay(0).verify(&events[1]));
        assert_eq!(
            calls(&cache),
            5,
            "a genuinely new event must verify before caching"
        );
    }

    #[test]
    fn positive_signature_cache_is_lazy_bounded_and_evicts_positive_entries() {
        let events = events(CAPACITY + 1);
        let mut cache = VerificationCache::default();
        assert_eq!(cache.entries.capacity(), 0);
        for event in &events {
            assert!(cache.relay(0).verify(event));
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.entries.capacity(), CAPACITY);
        assert_eq!(calls(&cache), CAPACITY + 1);
        for _ in 0..10 {
            assert!(cache.relay(0).verify(events.last().unwrap()));
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(calls(&cache), CAPACITY + 1);
        assert!(cache.relay(0).verify(&events[0]));
        assert_eq!(
            calls(&cache),
            CAPACITY + 2,
            "the evicted event must verify again"
        );
        assert!(std::mem::size_of::<VerifiedSignature>() * CAPACITY <= 9 * 1024);
    }
}
