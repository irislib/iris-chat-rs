//! Optional, download-only NIP-77 on the existing anonymous discovery pool.
use super::*;
use negentropy::{Id, Negentropy, NegentropyStorageVector};
use nostr::hashes::hex::{DisplayHex, FromHex};
use nostr::{ClientMessage, EventId};
use nostr_sdk::prelude::{Relay, RelayMessage};
use std::borrow::Cow;

const GRAPH_BYTES: usize = 2 * 1024 * 1024;
const GRAPH_DURATION: Duration = Duration::from_secs(30);
const MAX_GRAPH_RELAYS: usize = 8;
const MAX_GRAPH_AUTHORS: usize = 100;
const NEG_ROUNDS: usize = 32;

#[derive(Debug)]
pub(in crate::core) struct GraphEvents {
    pub(in crate::core) events: Vec<Event>,
    pub(in crate::core) complete: bool,
}

pub(in crate::core) struct GraphScope {
    lease: RelayLease,
    budget: GraphBudget,
}

// Counters measure complete raw WebSocket payload bytes before normalization,
// including padding, invalid JSON and duplicates. On a shared socket
// unrelated public traffic is conservatively charged too. CLOSE stops our
// producer at the threshold; already in-flight frames may still arrive.
pub(super) struct GraphBudget {
    relays: Vec<(Arc<std::sync::atomic::AtomicUsize>, usize)>,
    deadline: tokio::time::Instant,
    max_bytes: usize,
}
impl GraphBudget {
    pub(super) fn check(&self) -> Result<(), QueryFailure> {
        let received = self.relays.iter().fold(0usize, |total, (counter, start)| {
            total.saturating_add(counter.load(Ordering::Relaxed).saturating_sub(*start))
        });
        if received >= self.max_bytes {
            return Err(QueryFailure::Limit);
        }
        if tokio::time::Instant::now() >= self.deadline {
            return Err(QueryFailure::Transport(
                "graph refresh deadline exceeded".into(),
            ));
        }
        Ok(())
    }
}

impl DiscoveryRelays {
    pub(in crate::core) async fn graph_scope(
        self: &Arc<Self>,
        urls: &[RelayUrl],
    ) -> Result<GraphScope, String> {
        let urls = urls
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if urls.is_empty() || urls.len() > MAX_GRAPH_RELAYS {
            return Err("graph refresh requires between one and eight configured relays".into());
        }
        let deadline = tokio::time::Instant::now() + GRAPH_DURATION;
        let lease = tokio::time::timeout_at(deadline, self.open(&urls))
            .await
            .map_err(|_| "graph connection deadline exceeded".to_owned())??;
        let mut relays = Vec::with_capacity(urls.len());
        for url in &urls {
            let counter = self.wire.counter(url)?;
            let start = counter.load(Ordering::Relaxed);
            relays.push((counter, start));
        }
        Ok(GraphScope {
            lease,
            budget: GraphBudget {
                relays,
                deadline,
                max_bytes: GRAPH_BYTES,
            },
        })
    }
}

impl GraphScope {
    pub(in crate::core) async fn fetch(
        &self,
        filters: Vec<Filter>,
        timeout: Duration,
    ) -> Result<Vec<Event>, String> {
        let operation = async {
            let _permit = self
                .lease
                .pool
                .queries
                .acquire()
                .await
                .map_err(|error| QueryFailure::Transport(error.to_string()))?;
            query::fetch_admitted(
                &self.lease.pool,
                &self.lease.pool.client,
                &self.lease.urls,
                filters,
                timeout,
                Some(query::GraphQuery {
                    budget: &self.budget,
                    max_events: 512,
                }),
            )
            .await
            .map(|result| result.events)
        };
        tokio::time::timeout_at(self.budget.deadline, operation)
            .await
            .map_err(|_| "graph refresh deadline exceeded".to_owned())?
            .map_err(|error| error.to_string())
    }

    pub(in crate::core) async fn opinions(
        &self,
        kind: Kind,
        authors: &[PublicKey],
        known: &[Event],
        max_events: usize,
    ) -> Result<GraphEvents, String> {
        if !matches!(kind.as_u16(), 3 | 10000)
            || authors.is_empty()
            || authors.len() > MAX_GRAPH_AUTHORS
        {
            return Err("invalid graph reconciliation scope".into());
        }
        let operation = async {
            let _permit = self
                .lease
                .pool
                .queries
                .acquire()
                .await
                .map_err(|error| QueryFailure::Transport(error.to_string()))?;
            self.budget.check()?;
            if self.lease.pool.stopped.load(Ordering::Acquire) {
                return Err(QueryFailure::Transport("public discovery stopped".into()));
            }
            let filter = Filter::new().kind(kind).authors(authors.iter().copied());
            let mut events = known
                .iter()
                .filter(|event| event.kind == kind && authors.contains(&event.pubkey))
                .cloned()
                .collect::<Vec<_>>();
            let mut complete = true;
            for url in &self.lease.urls {
                self.budget.check()?;
                let relay = self
                    .lease
                    .pool
                    .client
                    .relay(url.clone())
                    .await
                    .map_err(|error| QueryFailure::Transport(error.to_string()))?;
                let result = tokio::time::timeout(
                    Duration::from_secs(1),
                    reconcile(&relay, &filter, known, &self.budget, max_events),
                )
                .await;
                let (query, required_ids) = match result {
                    Ok(Ok(ids)) if ids.is_empty() => continue,
                    Ok(Ok(ids)) => (
                        Filter::new()
                            .ids(ids.iter().copied())
                            .kind(kind)
                            .authors(authors.iter().copied()),
                        Some(ids),
                    ),
                    Ok(Err(QueryFailure::Limit)) => return Err(QueryFailure::Limit),
                    _ => (filter.clone(), None), // Unsupported/offline relays retain bounded ordinary requests.
                };
                self.budget.check()?;
                match query::fetch_admitted(
                    &self.lease.pool,
                    &self.lease.pool.client,
                    std::slice::from_ref(url),
                    vec![query.limit(max_events + 1)],
                    Duration::from_secs(3),
                    Some(query::GraphQuery {
                        budget: &self.budget,
                        max_events,
                    }),
                )
                .await
                {
                    Ok(result) => {
                        let bodies_complete = required_ids.as_ref().is_none_or(|ids| {
                            let received = result
                                .events
                                .iter()
                                .map(|event| event.id)
                                .collect::<HashSet<_>>();
                            ids.is_subset(&received)
                        });
                        complete &= result.complete && bodies_complete;
                        events.extend(result.events);
                    }
                    Err(QueryFailure::Limit) => return Err(QueryFailure::Limit),
                    Err(QueryFailure::Transport(_)) => complete = false,
                }
            }
            Ok(GraphEvents { events, complete })
        };
        tokio::time::timeout_at(self.budget.deadline, operation)
            .await
            .map_err(|_| "graph refresh deadline exceeded".to_owned())?
            .map_err(|error| error.to_string())
    }
}

struct NegSubscription {
    relay: Relay,
    id: SubscriptionId,
}
impl Drop for NegSubscription {
    fn drop(&mut self) {
        // send_msg is synchronous: even cancellation during setup sends CLOSE.
        let _ = self.relay.send_msg(ClientMessage::NegClose {
            subscription_id: Cow::Borrowed(&self.id),
        });
    }
}

async fn reconcile(
    relay: &Relay,
    filter: &Filter,
    known: &[Event],
    budget: &GraphBudget,
    max_events: usize,
) -> Result<HashSet<EventId>, QueryFailure> {
    let failure = |error: negentropy::Error| QueryFailure::Transport(error.to_string());
    let mut storage = NegentropyStorageVector::new();
    for event in known {
        if filter.match_event(event, nostr::filter::MatchEventOptions::new()) {
            storage
                .insert(
                    event.created_at.as_secs(),
                    Id::from_slice(event.id.as_bytes()).map_err(failure)?,
                )
                .map_err(failure)?;
        }
    }
    storage.seal().map_err(failure)?;
    let mut negentropy = Negentropy::borrowed(&storage, 8 * 1024).map_err(failure)?;
    relay.wait_for_connection(Duration::from_secs(1)).await;
    if !matches!(relay.status(), nostr_sdk::RelayStatus::Connected) {
        return Err(QueryFailure::Transport(
            "graph relay did not connect".into(),
        ));
    }
    budget.check()?;
    let mut notifications = relay.notifications();
    let subscription = NegSubscription {
        relay: relay.clone(),
        id: SubscriptionId::generate(),
    };
    relay
        .send_msg(ClientMessage::neg_open(
            subscription.id.clone(),
            filter.clone(),
            negentropy
                .initiate()
                .map_err(failure)?
                .to_lower_hex_string(),
        ))
        .map_err(|error| QueryFailure::Transport(error.to_string()))?;
    let mut missing = HashSet::new();
    let mut rounds = 0usize;
    let mut budget_poll = tokio::time::interval(Duration::from_millis(50));
    loop {
        budget.check()?;
        let notification = tokio::select! {
            result = notifications.recv() => result.map_err(|error| QueryFailure::Transport(error.to_string()))?,
            _ = budget_poll.tick() => { continue; }
        };
        budget.check()?;
        match notification {
            RelayNotification::Message {
                message:
                    RelayMessage::NegMsg {
                        subscription_id,
                        message,
                    },
            } if subscription_id.as_ref() == &subscription.id => {
                rounds += 1;
                if rounds > NEG_ROUNDS || message.len() > 16 * 1024 {
                    return Err(QueryFailure::Limit);
                }
                let bytes = Vec::<u8>::from_hex(&message)
                    .map_err(|error| QueryFailure::Transport(error.to_string()))?;
                let (mut have, mut need) = (Vec::new(), Vec::new());
                let next = negentropy
                    .reconcile_with_ids(&bytes, &mut have, &mut need)
                    .map_err(failure)?;
                missing.extend(
                    need.into_iter()
                        .map(|id| EventId::from_byte_array(id.to_bytes())),
                );
                if missing.len() > max_events {
                    return Err(QueryFailure::Limit);
                }
                if let Some(next) = next {
                    relay
                        .send_msg(ClientMessage::NegMsg {
                            subscription_id: Cow::Borrowed(&subscription.id),
                            message: Cow::Owned(next.to_lower_hex_string()),
                        })
                        .map_err(|error| QueryFailure::Transport(error.to_string()))?;
                } else {
                    return Ok(missing);
                }
            }
            RelayNotification::Message {
                message:
                    RelayMessage::NegErr {
                        subscription_id,
                        message,
                    },
            } if subscription_id.as_ref() == &subscription.id => {
                return Err(QueryFailure::Transport(message.into_owned()))
            }
            RelayNotification::AuthenticationFailed | RelayNotification::Shutdown => {
                return Err(QueryFailure::Transport(
                    "discovery relay unavailable".into(),
                ))
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod regression_tests;
