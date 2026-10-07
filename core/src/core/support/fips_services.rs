//! Opt-in attribution of local service sends, independent of peer/link churn.
//! These are carrier submissions, not delivered application bytes or CPU shares.
//! Counters persist since enable (pubsub delivery: client lifetime); elapsed_ms
//! starts at this diagnostic epoch's first observation, not at counter zero.
//! Only same-epoch snapshots with increasing time and nondecreasing counters
//! can form an interval. Shared control, transit, receive, and other services
//! are excluded; the unattributed remainder must not be called FIPS overhead.

use super::fips_traffic::sample_id;
use fips_core::endpoint::ServiceCarrierSnapshot;
use fips_core::FipsEndpoint;
use nostr_pubsub_fips::{FipsPubsubClient, FIPS_NOSTR_PUBSUB_SERVICE_PORT};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Instant;

const SCOPE: &str = "locally_originated_service_carrier_submissions";

pub(super) struct Observation {
    endpoint: Arc<FipsEndpoint>,
    pubsub: Option<Arc<FipsPubsubClient>>,
    services: BTreeMap<&'static str, ServiceCarrierSnapshot>,
    pubsub_delivery: Value,
}

pub(super) struct History {
    endpoint: Weak<FipsEndpoint>,
    pubsub: Option<Weak<FipsPubsubClient>>,
    generation: u64,
    epoch_id: String,
    first_observed_at: Instant,
}

pub(super) fn unavailable(status: &'static str) -> Value {
    json!({"valid": false, "status": status, "scope": SCOPE})
}

impl Observation {
    pub(super) fn capture(
        endpoint: Arc<FipsEndpoint>,
        pubsub: Option<Arc<FipsPubsubClient>>,
    ) -> Result<Self, &'static str> {
        // Existing FIPS counters are disabled until an explicit support export.
        // No periodic sampler, payload capture, or per-peer state is added.
        let services = [
            ("pubsub", FIPS_NOSTR_PUBSUB_SERVICE_PORT),
            ("hashtree", hashtree_fips_transport::TCP_BLOB_SERVICE_PORT),
        ]
        .into_iter()
        .map(|(name, port)| {
            endpoint
                .enable_service_carrier_diagnostics(port)
                .map(|handle| (name, handle.snapshot()))
                .map_err(|_| "query_error")
        })
        .collect::<Result<_, _>>()?;
        let pubsub_delivery = pubsub.as_ref().map_or(Value::Null, |client| {
            let snapshot = client.delivery_snapshot();
            json!({
                "req_frames_received": snapshot.req_frames_received,
                "close_frames_received": snapshot.close_frames_received,
                "event_frames_received": snapshot.event_frames_received,
                "inv_frames_received": snapshot.inv_frames_received,
                "want_frames_received": snapshot.want_frames_received,
                "want_frames_sent": snapshot.want_frames_sent,
                "subscription_events_received": snapshot.subscription_events_received,
                "expired_wants": snapshot.expired_wants,
                "provider_cooldowns": snapshot.provider_cooldowns,
                "tcp_receive_batches": snapshot.tcp_receive_batches,
                "tcp_datagrams_received": snapshot.tcp_datagrams_received,
                "tcp_datagrams_rejected": snapshot.tcp_datagrams_rejected,
                "tcp_poll_turns": snapshot.tcp_poll_turns,
                "transport_errors": client.transport_error_count(),
            })
        });
        Ok(Self {
            endpoint,
            pubsub,
            services,
            pubsub_delivery,
        })
    }

    pub(super) fn record(
        self,
        history: &mut Option<History>,
        generation: u64,
        now: Instant,
    ) -> Value {
        self.record_checked(history, generation, now)
            .unwrap_or_else(|| {
                *history = None;
                unavailable("invalid_counters")
            })
    }

    fn record_checked(
        self,
        history: &mut Option<History>,
        generation: u64,
        now: Instant,
    ) -> Option<Value> {
        let services = serde_json::to_value(self.services).ok()?;
        if !valid_counts(&services) || !valid_counts(&self.pubsub_delivery) {
            return None;
        }
        let same_epoch = history.as_ref().is_some_and(|old| {
            old.generation == generation
                && old.endpoint.ptr_eq(&Arc::downgrade(&self.endpoint))
                && match (&old.pubsub, &self.pubsub) {
                    (Some(old), Some(current)) => old.ptr_eq(&Arc::downgrade(current)),
                    (None, None) => true,
                    _ => false,
                }
        });
        if !same_epoch {
            *history = Some(History {
                endpoint: Arc::downgrade(&self.endpoint),
                pubsub: self.pubsub.as_ref().map(Arc::downgrade),
                generation,
                epoch_id: sample_id()?,
                first_observed_at: now,
            });
        }
        let baseline = history.as_ref()?;
        let elapsed_ms = u64::try_from(
            now.checked_duration_since(baseline.first_observed_at)?
                .as_millis(),
        )
        .ok()?;
        if elapsed_ms > i64::MAX as u64 {
            return None;
        }
        Some(json!({
            "valid": true, "status": "available", "scope": SCOPE,
            "sample_id": sample_id()?, "epoch_id": baseline.epoch_id,
            "elapsed_ms": elapsed_ms, "services": services,
            "pubsub_delivery": self.pubsub_delivery,
        }))
    }
}

fn valid_counts(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_u64().is_some_and(|n| n <= i64::MAX as u64),
        Value::Array(values) => values.iter().all(valid_counts),
        Value::Object(values) => values.values().all(valid_counts),
        _ => true,
    }
}

#[cfg(test)]
mod tests;
