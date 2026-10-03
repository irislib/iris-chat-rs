use super::*;

#[test]
fn event_validation_cache_bounds_proofs_without_caching_failed_signatures() {
    let author = Keys::generate();
    let mut cache = EventValidationCache::default();
    let events: Vec<_> = (0..=MAX_SEEN_EVENT_IDS)
        .map(|index| {
            EventBuilder::new(Kind::TextNote, index.to_string())
                .sign_with_keys(&author)
                .unwrap()
        })
        .collect();
    for event in &events[..MAX_SEEN_EVENT_IDS] {
        assert!(cache.verify(event));
    }
    assert_eq!(cache.verified.len(), MAX_SEEN_EVENT_IDS);
    assert_eq!(cache.order.len(), MAX_SEEN_EVENT_IDS);

    // A valid content ID with another event's signature must neither enter the
    // cache nor evict an authenticated proof, even after repeated attempts.
    let mut forged = events[MAX_SEEN_EVENT_IDS].clone();
    forged.sig = events[0].sig;
    for _ in 0..3 {
        assert!(!cache.verify(&forged));
    }
    assert_eq!(cache.verified.len(), MAX_SEEN_EVENT_IDS);
    assert_eq!(cache.order.len(), MAX_SEEN_EVENT_IDS);
    let checks = cache.signature_checks;
    assert!(cache.verify(&events[0]));
    assert_eq!(cache.signature_checks, checks);

    assert!(cache.verify(&events[MAX_SEEN_EVENT_IDS]));
    assert_eq!(cache.verified.len(), MAX_SEEN_EVENT_IDS);
    assert_eq!(cache.order.len(), MAX_SEEN_EVENT_IDS);
    let checks = cache.signature_checks;
    assert!(cache.verify(&events[MAX_SEEN_EVENT_IDS]));
    assert_eq!(cache.signature_checks, checks);
    assert!(cache.verify(&events[0]));
    assert_eq!(
        cache.signature_checks,
        checks + 1,
        "evicted proofs reauthenticate"
    );
}

#[test]
fn event_validation_cache_accepts_alternate_valid_signatures_for_one_id() {
    let author = Keys::generate();
    let sign = || {
        EventBuilder::new(Kind::TextNote, "identical signed body")
            .custom_created_at(Timestamp::from(123))
            .sign_with_keys(&author)
            .unwrap()
    };
    let first = sign();
    let alternate = sign();
    assert_eq!(first.id, alternate.id);
    assert_ne!(first.sig, alternate.sig);
    let mut cache = EventValidationCache::default();
    assert!(cache.verify(&first));
    assert!(cache.verify(&alternate));
    assert_eq!(cache.signature_checks, 2);
    assert!(cache.verify(&first));
    assert!(cache.verify(&alternate));
    assert_eq!(cache.signature_checks, 2);
}
