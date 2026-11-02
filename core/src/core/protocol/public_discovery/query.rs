use super::*;
use futures_util::future::try_join_all;
use nostr::EventId;
use nostr_sdk::prelude::{ClientOptions, Relay, RelayLimits, RelayMessage};

const MAX_FRAME_BYTES: u32 = 128 * 1024;
const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESULT_EVENTS: usize = 16_384;

pub(super) fn client_options() -> ClientOptions {
    let mut limits = RelayLimits::default();
    limits.messages.max_size = Some(MAX_FRAME_BYTES);
    limits.events.max_size = Some(MAX_FRAME_BYTES);
    ClientOptions::default()
        .relay_limits(limits)
        .verify_subscriptions(true)
}

#[derive(Debug)]
pub(super) enum QueryFailure {
    Limit,
    Transport(String),
}

impl std::fmt::Display for QueryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit => formatter.write_str("public discovery result exceeds safety bound"),
            Self::Transport(error) => formatter.write_str(error),
        }
    }
}

impl RelayLease {
    pub(super) async fn fetch(
        &self,
        filters: Vec<Filter>,
        timeout: Duration,
    ) -> Result<Vec<Event>, QueryFailure> {
        fetch(&self.pool, &self.pool.client, &self.urls, filters, timeout).await
    }
}

// Use SDK subscriptions with explicit IDs so the byte budget and cancellation
// can close them immediately. The collecting query API applies limits too late;
// SDK stream consumers do not stop their producers when dropped. Connection,
// authentication, signature/filter validation and reconnect remain in the SDK.
// Only one query accumulates results per pool, including foreground recovery.
pub(super) async fn fetch(
    pool: &DiscoveryRelays,
    client: &Client,
    urls: &[RelayUrl],
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, QueryFailure> {
    let _permit = pool
        .queries
        .acquire()
        .await
        .map_err(|error| QueryFailure::Transport(error.to_string()))?;
    if pool.stopped.load(Ordering::Acquire) {
        return Err(QueryFailure::Transport("public discovery stopped".into()));
    }
    let filters = merge_filters(filters)
        .into_iter()
        .map(|mut filter| {
            filter.limit = Some(
                filter
                    .limit
                    .unwrap_or(MAX_RESULT_EVENTS)
                    .min(MAX_RESULT_EVENTS),
            );
            filter
        })
        .collect::<Vec<_>>();
    if filters.is_empty() || urls.is_empty() {
        return Ok(Vec::new());
    }
    let result = std::sync::Mutex::new(Collection::default());
    let queries = urls.iter().map(|url| {
        let result = &result;
        let filters = &filters;
        async move {
            let response = async {
                let relay = client
                    .relay(url.clone())
                    .await
                    .map_err(|error| QueryFailure::Transport(error.to_string()))?;
                let mut notifications = relay.notifications();
                let subscription = QuerySubscription {
                    relay,
                    id: SubscriptionId::generate(),
                };
                subscription
                    .relay
                    .subscribe_with_id(
                        subscription.id.clone(),
                        filters.clone(),
                        SubscribeOptions::default(),
                    )
                    .await
                    .map_err(|error| QueryFailure::Transport(error.to_string()))?;
                loop {
                    match notifications
                        .recv()
                        .await
                        .map_err(|error| QueryFailure::Transport(error.to_string()))?
                    {
                        RelayNotification::Message { message } => match message {
                            RelayMessage::Event {
                                subscription_id,
                                event,
                            } if subscription_id.as_ref() == &subscription.id => {
                                let event = event.into_owned();
                                if filters.iter().any(|filter| {
                                    filter.match_event(
                                        &event,
                                        nostr::filter::MatchEventOptions::new(),
                                    )
                                }) {
                                    result
                                        .lock()
                                        .map_err(|_| {
                                            QueryFailure::Transport(
                                                "discovery collection unavailable".into(),
                                            )
                                        })?
                                        .insert(event)?;
                                }
                            }
                            RelayMessage::EndOfStoredEvents(id)
                                if id.as_ref() == &subscription.id =>
                            {
                                return Ok(())
                            }
                            RelayMessage::Closed {
                                subscription_id,
                                message,
                            } if subscription_id.as_ref() == &subscription.id
                                && !message.starts_with("auth-required:") =>
                            {
                                return Err(QueryFailure::Transport(message.into_owned()));
                            }
                            _ => {}
                        },
                        RelayNotification::AuthenticationFailed | RelayNotification::Shutdown => {
                            return Err(QueryFailure::Transport(
                                "discovery relay unavailable".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            };
            match response.await {
                Ok(()) => Ok(true),
                // One offline relay must not discard another relay's verified
                // results. Resource rejection always fails the whole query.
                Err(QueryFailure::Transport(_)) => Ok(false),
                Err(QueryFailure::Limit) => Err(QueryFailure::Limit),
            }
        }
    });
    let completed = match tokio::time::timeout(timeout, try_join_all(queries)).await {
        Ok(Ok(completed)) => completed.into_iter().any(|completed| completed),
        Ok(Err(error)) => return Err(error),
        Err(_) => false,
    };
    let mut result = result
        .into_inner()
        .map_err(|_| QueryFailure::Transport("discovery collection unavailable".into()))?;
    if !completed && result.events.is_empty() {
        return Err(QueryFailure::Transport(
            "no relay completed discovery".into(),
        ));
    }
    result.events.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(result.events)
}

// Construct before the first subscription await: cancellation during setup,
// EOSE, timeout and resource rejection all remove exactly this retained ID.
struct QuerySubscription {
    relay: Relay,
    id: SubscriptionId,
}
impl Drop for QuerySubscription {
    fn drop(&mut self) {
        let relay = self.relay.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            let _ = relay.unsubscribe(&id).await;
        });
    }
}

#[derive(Default)]
struct Collection {
    events: Vec<Event>,
    ids: HashSet<EventId>,
    bytes: usize,
}

// The account receiver retains its SDK 5 MiB transport allowance for private
// receives. SDK signature verification happens before notifications; do not
// repeat that crypto here. Bound metadata acceptance before retaining it, using
// a counting writer so checking a large event does not allocate another copy.
fn metadata_size(event: &Event) -> Option<usize> {
    struct Size(usize);
    impl std::io::Write for Size {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_FRAME_BYTES as usize - self.0 {
                return Err(std::io::Error::other("metadata size limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut size = Size(0);
    serde_json::to_writer(&mut size, event).ok()?;
    Some(size.0)
}

impl Collection {
    fn insert(&mut self, event: Event) -> Result<(), QueryFailure> {
        if self.ids.contains(&event.id) {
            return Ok(());
        }
        let Some(size) = metadata_size(&event) else {
            return Ok(());
        };
        if self.bytes.saturating_add(size) > MAX_RESULT_BYTES
            || self.events.len() >= MAX_RESULT_EVENTS
        {
            return Err(QueryFailure::Limit);
        }
        self.bytes += size;
        self.ids.insert(event.id);
        self.events.push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_admission_keeps_exact_event_bound_without_allocating_a_copy() {
        let keys = Keys::generate();
        let event = |content: String| {
            EventBuilder::new(Kind::Metadata, content)
                .sign_with_keys(&keys)
                .unwrap()
        };
        let overhead = serde_json::to_vec(&event(String::new())).unwrap().len();
        let maximum = event("x".repeat(MAX_FRAME_BYTES as usize - overhead));
        assert_eq!(metadata_size(&maximum), Some(MAX_FRAME_BYTES as usize));
        let oversized = event("x".repeat(MAX_FRAME_BYTES as usize - overhead + 1));
        assert_eq!(metadata_size(&oversized), None);
        let mut collection = Collection::default();
        collection.insert(oversized).unwrap();
        assert_eq!(collection.bytes, 0);
        assert!(collection.events.is_empty() && collection.ids.is_empty());
        collection.insert(maximum).unwrap();
        assert_eq!(collection.bytes, MAX_FRAME_BYTES as usize);
        assert_eq!(collection.events.len(), 1);
    }

    #[test]
    fn metadata_collection_accepts_exact_bounds_and_rejects_overflow_before_retaining() {
        let event = EventBuilder::new(Kind::Metadata, "bounded")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let size = serde_json::to_vec(&event).unwrap().len();
        let mut collection = Collection {
            bytes: MAX_RESULT_BYTES - size,
            ..Default::default()
        };
        collection.insert(event.clone()).unwrap();
        assert_eq!(collection.bytes, MAX_RESULT_BYTES);
        collection.insert(event.clone()).unwrap();
        assert_eq!(
            collection.events.len(),
            1,
            "duplicates must not consume the budget"
        );
        let second = EventBuilder::new(Kind::Metadata, "overflow")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(matches!(
            collection.insert(second.clone()),
            Err(QueryFailure::Limit)
        ));
        assert_eq!(collection.events.len(), 1);
        assert_eq!(collection.bytes, MAX_RESULT_BYTES);

        let mut collection = Collection {
            events: vec![event; MAX_RESULT_EVENTS - 1],
            ..Default::default()
        };
        collection.insert(second).unwrap();
        assert_eq!(collection.events.len(), MAX_RESULT_EVENTS);
        let extra = EventBuilder::new(Kind::Metadata, "extra")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(matches!(collection.insert(extra), Err(QueryFailure::Limit)));
        assert_eq!(collection.events.len(), MAX_RESULT_EVENTS);
    }
}
