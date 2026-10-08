use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

use nostr::{EventBuilder, Keys, Kind, Tag, TagKind, Timestamp, ToBech32};
use nostr_pubsub::{
    EventBus, EventSource, Filter, InMemoryEventBus, NostrEventHandler, NostrEventSubscription,
    PubsubError, QueryEvent, SubscriptionDeliveryStatus, VerifiedEvent,
};
use tokio::sync::Semaphore;

const WINDOW: Duration = Duration::from_millis(35);

struct LiveProvider {
    bus: InMemoryEventBus,
    started: Semaphore,
    closed: Arc<AtomicUsize>,
    closed_signal: Arc<Semaphore>,
    status: Arc<Mutex<SubscriptionDeliveryStatus>>,
}

impl LiveProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            bus: InMemoryEventBus::new(),
            started: Semaphore::new(0),
            closed: Arc::default(),
            closed_signal: Arc::new(Semaphore::new(0)),
            status: Arc::new(Mutex::new(SubscriptionDeliveryStatus::Active)),
        })
    }

    async fn subscribed(&self) {
        self.started.acquire().await.unwrap().forget();
    }

    async fn publish(&self, event: &Event, source: EventSource) {
        self.bus
            .publish(VerifiedEvent::try_from(event.clone()).unwrap(), source)
            .await
            .unwrap();
    }
}

#[async_trait]
impl NostrEventSubscriber for LiveProvider {
    async fn subscribe(
        &self,
        filters: Vec<Filter>,
        handler: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        let inner = self.bus.subscribe(filters, handler).await?;
        self.started.add_permits(1);
        Ok(Box::new(LiveSubscription {
            inner,
            closed: self.closed.clone(),
            closed_signal: self.closed_signal.clone(),
            status: self.status.clone(),
        }))
    }
}

struct LiveSubscription {
    inner: Box<dyn NostrEventSubscription>,
    closed: Arc<AtomicUsize>,
    closed_signal: Arc<Semaphore>,
    status: Arc<Mutex<SubscriptionDeliveryStatus>>,
}

#[async_trait]
impl NostrEventSubscription for LiveSubscription {
    fn delivery_status(&self) -> Option<SubscriptionDeliveryStatus> {
        Some(*self.status.lock().unwrap())
    }

    async fn close(self: Box<Self>) -> nostr_pubsub::Result<()> {
        self.inner.close().await?;
        self.closed.fetch_add(1, Ordering::SeqCst);
        self.closed_signal.add_permits(1);
        Ok(())
    }
}

struct UnavailableProvider;

#[async_trait]
impl NostrEventSubscriber for UnavailableProvider {
    async fn subscribe(
        &self,
        _: Vec<Filter>,
        _: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        Err(PubsubError::Storage("source is offline".into()))
    }
}

struct StalledProvider(Arc<AtomicUsize>);

// Finish an interrupted source after a healthy source has already returned its
// root. This exercises the final cross-source watermark check, not just each
// underlying resolver's check during its own observation window.
struct LateObservationProvider {
    after_closed: Arc<Semaphore>,
    event: Event,
}

#[async_trait]
impl NostrEventSubscriber for LateObservationProvider {
    async fn subscribe(
        &self,
        _: Vec<Filter>,
        handler: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        Ok(Box::new(LateObservation {
            after_closed: self.after_closed.clone(),
            event: self.event.clone(),
            handler,
        }))
    }
}

struct LateObservation {
    after_closed: Arc<Semaphore>,
    event: Event,
    handler: NostrEventHandler,
}

#[async_trait]
impl NostrEventSubscription for LateObservation {
    fn delivery_status(&self) -> Option<SubscriptionDeliveryStatus> {
        Some(SubscriptionDeliveryStatus::Lagged)
    }

    async fn close(self: Box<Self>) -> nostr_pubsub::Result<()> {
        self.after_closed.acquire().await.unwrap().forget();
        (self.handler)(QueryEvent {
            event: VerifiedEvent::try_from(self.event).unwrap(),
            source: EventSource::peer("late-peer"),
            priority: 0,
        });
        Ok(())
    }
}

struct CancelledSubscribe(Arc<AtomicUsize>);

impl Drop for CancelledSubscribe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl NostrEventSubscriber for StalledProvider {
    async fn subscribe(
        &self,
        _: Vec<Filter>,
        _: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        let _cancelled = CancelledSubscribe(self.0.clone());
        std::future::pending().await
    }
}

fn fixture() -> (Keys, String) {
    let keys = Keys::generate();
    let key = format!("{}/releases/test", keys.public_key().to_bech32().unwrap());
    (keys, key)
}

fn root(keys: &Keys, timestamp: u64, byte: u8) -> Event {
    EventBuilder::new(Kind::Custom(30064), "")
        .tags([
            Tag::identifier("releases/test"),
            Tag::custom(
                TagKind::Custom("hash".into()),
                [format!("{byte:02x}").repeat(32)],
            ),
        ])
        .custom_created_at(Timestamp::from(timestamp))
        .sign_with_keys(keys)
        .unwrap()
}

#[test]
fn requires_at_least_one_provider() {
    assert!(AvailableUpdateResolver::new(Vec::new(), WINDOW).is_err());
}

#[tokio::test]
async fn quiet_peer_does_not_block_repeated_fresh_relay_checks() {
    let quiet = LiveProvider::new();
    let relay = LiveProvider::new();
    let resolver =
        AvailableUpdateResolver::new(vec![quiet.clone(), relay.clone()], WINDOW).unwrap();
    let (keys, key) = fixture();
    let event = root(&keys, 1, 1);
    for _ in 0..2 {
        let (result, ()) = tokio::join!(resolver.resolve(&key), async {
            quiet.subscribed().await;
            relay.subscribed().await;
            relay
                .publish(&event, EventSource::relay("wss://relay.example"))
                .await;
        });
        assert_eq!(result.unwrap(), Some(Cid::public([1; 32])));
    }
    assert_eq!(quiet.closed.load(Ordering::SeqCst), 2);
    assert_eq!(relay.closed.load(Ordering::SeqCst), 2);
    // A previous check's peer event remains a watermark, never fresh evidence.
    assert!(resolver.resolve(&key).await.is_err());
    assert_eq!(resolver.latest_event(&key).await.unwrap(), Some(event));
}

#[tokio::test]
async fn healthy_peer_survives_unavailable_and_stalled_sources() {
    let peer = LiveProvider::new();
    let cancelled = Arc::new(AtomicUsize::new(0));
    let resolver = AvailableUpdateResolver::new(
        vec![
            Arc::new(UnavailableProvider),
            Arc::new(StalledProvider(cancelled.clone())),
            peer.clone(),
        ],
        WINDOW,
    )
    .unwrap();
    let (keys, key) = fixture();
    let event = root(&keys, 1, 2);
    let (result, ()) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(1), resolver.resolve(&key)),
        async {
            peer.subscribed().await;
            peer.publish(&event, EventSource::fips_endpoint("fresh-peer"))
                .await;
        }
    );
    assert_eq!(result.unwrap().unwrap(), Some(Cid::public([2; 32])));
    assert_eq!(cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn local_cache_and_stale_peers_cannot_confirm_newest_root() {
    let provider = LiveProvider::new();
    let resolver = AvailableUpdateResolver::new(vec![provider.clone()], WINDOW).unwrap();
    let (keys, key) = fixture();
    let newest = root(&keys, 2, 3);
    resolver.ingest_event(newest.clone()).await.unwrap();
    for (event, source) in [
        (newest.clone(), EventSource::local_index("cache")),
        (root(&keys, 1, 2), EventSource::peer("stale-peer")),
        (
            root(&Keys::generate(), 3, 4),
            EventSource::peer("wrong-publisher"),
        ),
    ] {
        let (result, ()) = tokio::join!(resolver.resolve(&key), async {
            provider.subscribed().await;
            provider.publish(&event, source).await;
        });
        assert!(result.is_err());
        assert_eq!(
            resolver.latest_event(&key).await.unwrap(),
            Some(newest.clone())
        );
    }
}

#[tokio::test]
async fn interrupted_source_needs_independent_fresh_confirmation_of_newest_root() {
    for status in [
        SubscriptionDeliveryStatus::Lagged,
        SubscriptionDeliveryStatus::Closed,
    ] {
        for healthy_root in [None, Some(4), Some(5)] {
            let interrupted = LiveProvider::new();
            let healthy = LiveProvider::new();
            let resolver =
                AvailableUpdateResolver::new(vec![interrupted.clone(), healthy.clone()], WINDOW)
                    .unwrap();
            let (keys, key) = fixture();
            let newest = root(&keys, 2, 5);
            let (result, ()) = tokio::join!(resolver.resolve(&key), async {
                interrupted.subscribed().await;
                healthy.subscribed().await;
                interrupted
                    .publish(&newest, EventSource::peer("interrupted-peer"))
                    .await;
                *interrupted.status.lock().unwrap() = status;
                if let Some(byte) = healthy_root {
                    // A healthy source may carry an earlier announcement of
                    // exactly the same root. A failed source's newer timestamp
                    // must not invalidate that independent confirmation.
                    let event = root(&keys, 1, byte);
                    healthy
                        .publish(&event, EventSource::peer("healthy-peer"))
                        .await;
                }
            });
            if healthy_root == Some(5) {
                assert_eq!(result.unwrap(), Some(Cid::public([5; 32])));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(resolver.latest_event(&key).await.unwrap(), Some(newest));
            assert_eq!(interrupted.closed.load(Ordering::SeqCst), 1);
            assert_eq!(healthy.closed.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn final_watermark_rejects_different_root_but_preserves_identical_root_confirmation() {
    for newest_byte in [6, 7] {
        let healthy = LiveProvider::new();
        let (keys, key) = fixture();
        let confirmed = root(&keys, 1, 6);
        let newest = root(&keys, 2, newest_byte);
        let resolver = AvailableUpdateResolver::new(
            vec![
                healthy.clone(),
                Arc::new(LateObservationProvider {
                    after_closed: healthy.closed_signal.clone(),
                    event: newest.clone(),
                }),
            ],
            WINDOW,
        )
        .unwrap();
        let (result, ()) = tokio::join!(resolver.resolve(&key), async {
            healthy.subscribed().await;
            healthy
                .publish(&confirmed, EventSource::peer("healthy-peer"))
                .await;
        });
        if newest_byte == 6 {
            assert_eq!(result.unwrap(), Some(Cid::public([6; 32])));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(resolver.latest_event(&key).await.unwrap(), Some(newest));
    }
}

#[tokio::test]
async fn cancelled_check_keeps_observed_root_as_rollback_watermark() {
    let provider = LiveProvider::new();
    let resolver = AvailableUpdateResolver::new(vec![provider.clone()], WINDOW).unwrap();
    let (keys, key) = fixture();
    let newest = root(&keys, 2, 9);
    let mut check = Box::pin(resolver.resolve(&key));
    tokio::select! {
        result = &mut check => panic!("check completed before cancellation: {result:?}"),
        () = async {
            provider.subscribed().await;
            provider.publish(&newest, EventSource::peer("new-release")).await;
        } => {}
    }
    drop(check);
    assert_eq!(
        resolver.latest_event(&key).await.unwrap(),
        Some(newest.clone())
    );
    let stale = root(&keys, 1, 8);
    let (result, ()) = tokio::join!(resolver.resolve(&key), async {
        provider.subscribed().await;
        provider
            .publish(&stale, EventSource::peer("stale-peer"))
            .await;
    });
    assert!(result.is_err());
    assert_eq!(resolver.latest_event(&key).await.unwrap(), Some(newest));
}
