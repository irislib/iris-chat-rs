use crate::core::AppCore;
use fips_core::{FipsEndpoint, FipsEndpointPeer};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
// These counters cover authenticated links resident at the snapshots, not
// unauthenticated/discovery traffic or transient peers between observations.
const SCOPE: &str = "connected_authenticated_peers";
// Process-wide IDs also distinguish snapshots across a recovered AppCore.
static NEXT_SAMPLE: AtomicU64 = AtomicU64::new(1);
static PROCESS_NONCE: OnceLock<u128> = OnceLock::new();

pub(super) fn sample_id() -> Option<String> {
    let mut current = NEXT_SAMPLE.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1)?;
        match NEXT_SAMPLE.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => {
                return Some(format!(
                    "{:032x}-{current}",
                    PROCESS_NONCE.get_or_init(rand::random)
                ))
            }
            Err(actual) => current = actual,
        }
    }
}

#[derive(Clone, Copy, Default, Serialize)]
struct Counters {
    rx_packets: u64,
    tx_packets: u64,
    rx_bytes: u64,
    tx_bytes: u64,
}

impl Counters {
    fn valid(self) -> bool {
        [
            self.rx_packets,
            self.tx_packets,
            self.rx_bytes,
            self.tx_bytes,
        ]
        .into_iter()
        .all(|value| value <= i64::MAX as u64)
    }

    fn add(self, other: Self) -> Option<Self> {
        let sum = Self {
            rx_packets: self.rx_packets.checked_add(other.rx_packets)?,
            tx_packets: self.tx_packets.checked_add(other.tx_packets)?,
            rx_bytes: self.rx_bytes.checked_add(other.rx_bytes)?,
            tx_bytes: self.tx_bytes.checked_add(other.tx_bytes)?,
        };
        sum.valid().then_some(sum)
    }

    fn subtract(self, other: Self) -> Option<Self> {
        Some(Self {
            rx_packets: self.rx_packets.checked_sub(other.rx_packets)?,
            tx_packets: self.tx_packets.checked_sub(other.tx_packets)?,
            rx_bytes: self.rx_bytes.checked_sub(other.rx_bytes)?,
            tx_bytes: self.tx_bytes.checked_sub(other.tx_bytes)?,
        })
    }
}

#[derive(Default, Serialize)]
struct TransportTotals {
    connected_peer_count: u64,
    #[serde(flatten)]
    counters: Counters,
}

// Identities and link epochs exist only in memory for exact comparability.
// Neither this type nor the stored baseline implements Serialize.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PeerKey {
    identity: String,
    link_id: u64,
    transport: &'static str,
}

struct ObservedPeer {
    key: PeerKey,
    counters: Counters,
}

impl From<FipsEndpointPeer> for ObservedPeer {
    fn from(peer: FipsEndpointPeer) -> Self {
        let transport = match peer.transport_type.as_deref() {
            Some("udp") => "udp",
            Some("websocket") => "websocket",
            Some("webrtc") => "webrtc",
            Some("tcp") => "tcp",
            Some("ble") => "ble",
            Some("ethernet") => "ethernet",
            Some("tor") => "tor",
            _ => "other",
        };
        Self {
            key: PeerKey {
                identity: peer.npub,
                link_id: peer.link_id,
                transport,
            },
            counters: Counters {
                rx_packets: peer.packets_recv,
                tx_packets: peer.packets_sent,
                rx_bytes: peer.bytes_recv,
                tx_bytes: peer.bytes_sent,
            },
        }
    }
}

struct Baseline {
    sample_id: String,
    generation: u64,
    observed_at: Instant,
    peers: BTreeMap<PeerKey, Counters>,
}

#[derive(Default)]
pub(in crate::core) struct TrafficHistory {
    baseline: Option<Baseline>,
    services: Option<super::fips_services::History>,
}

struct Observation {
    peers: Vec<ObservedPeer>,
    services: Result<super::fips_services::Observation, &'static str>,
}

#[cfg(test)]
impl From<Vec<ObservedPeer>> for Observation {
    fn from(peers: Vec<ObservedPeer>) -> Self {
        Self {
            peers,
            services: Err("unavailable"),
        }
    }
}

fn unavailable(status: &'static str) -> Value {
    json!({"valid": false, "status": status, "scope": SCOPE})
}

impl TrafficHistory {
    fn record(
        &mut self,
        peers: Vec<ObservedPeer>,
        configured: &BTreeSet<String>,
        generation: u64,
        observed_at: Instant,
    ) -> Value {
        let result = self.record_checked(peers, configured, generation, observed_at);
        result.unwrap_or_else(|| {
            self.baseline = None;
            unavailable("invalid_counters")
        })
    }

    fn record_checked(
        &mut self,
        peers: Vec<ObservedPeer>,
        configured: &BTreeSet<String>,
        generation: u64,
        observed_at: Instant,
    ) -> Option<Value> {
        let sample_id = sample_id()?;
        let connected = peers.len();
        let configured_connected = peers
            .iter()
            .filter(|peer| configured.contains(&peer.key.identity))
            .count();
        let mut current = BTreeMap::new();
        let mut transports = BTreeMap::<&str, TransportTotals>::new();
        for peer in peers {
            if !peer.counters.valid() {
                return None;
            }
            let total = transports.entry(peer.key.transport).or_default();
            total.connected_peer_count += 1;
            total.counters = total.counters.add(peer.counters)?;
            if current.insert(peer.key, peer.counters).is_some() {
                return None;
            }
        }
        let interval = self.interval(&current, generation, observed_at);
        self.baseline = Some(Baseline {
            sample_id: sample_id.clone(),
            generation,
            observed_at,
            peers: current,
        });
        Some(json!({
            "valid": true, "status": "available", "scope": SCOPE,
            "sample_id": sample_id, "connected_peer_count": connected,
            "configured_direct_peer_count": configured.len(),
            "connected_configured_direct_peer_count": configured_connected,
            "unexpected_connected_peer_count": connected - configured_connected,
            "transports": transports, "interval": interval,
        }))
    }

    fn interval(
        &self,
        current: &BTreeMap<PeerKey, Counters>,
        generation: u64,
        now: Instant,
    ) -> Value {
        let Some(previous) = self.baseline.as_ref() else {
            return json!({"valid": false, "reason": "baseline", "since_sample_id": null,
                "elapsed_ms": null, "transport_deltas": null});
        };
        let elapsed_ms = now
            .checked_duration_since(previous.observed_at)
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .filter(|elapsed| *elapsed > 0 && *elapsed <= i64::MAX as u64);
        let interval = |reason: &str, deltas: Option<BTreeMap<&str, Counters>>| {
            json!({"valid": deltas.is_some(), "reason": reason,
                "since_sample_id": previous.sample_id, "elapsed_ms": elapsed_ms,
                "transport_deltas": deltas})
        };
        let reason = if generation != previous.generation {
            "endpoint_changed"
        } else if !current.keys().eq(previous.peers.keys()) {
            "peer_changed"
        } else if elapsed_ms.is_none() {
            "no_elapsed_time"
        } else {
            "comparable"
        };
        if reason != "comparable" {
            return interval(reason, None);
        }
        let mut deltas = BTreeMap::<&str, Counters>::new();
        for (key, counters) in current {
            let delta = previous
                .peers
                .get(key)
                .and_then(|previous| counters.subtract(*previous));
            let total = deltas.entry(key.transport).or_default();
            let Some(delta) = delta.and_then(|delta| total.add(delta)) else {
                return interval("counter_reset", None);
            };
            *total = delta;
        }
        interval("comparable", Some(deltas))
    }
}

async fn send_reply(
    bundle: Value,
    reply: flume::Sender<String>,
    query: impl Future<Output = Result<Observation, &'static str>>,
    history: Arc<Mutex<TrafficHistory>>,
    configured: BTreeSet<String>,
    generation: u64,
    timeout: Duration,
) {
    // Serialize explicit queries so out-of-order completion cannot roll the
    // baseline back. Waiting for another query shares the same total budget.
    let (diagnostic, services) = tokio::time::timeout(timeout, async {
        let mut history = history.lock().await;
        // Cancellation or query failure discards the baseline automatically.
        let previous = history.baseline.take();
        let previous_services = history.services.take();
        match query.await {
            Ok(observation) => {
                history.baseline = previous;
                let now = Instant::now();
                let services = match observation.services {
                    Ok(services) => {
                        history.services = previous_services;
                        services.record(&mut history.services, generation, now)
                    }
                    Err(status) => super::fips_services::unavailable(status),
                };
                (
                    history.record(observation.peers, &configured, generation, now),
                    services,
                )
            }
            Err(status) => (
                unavailable(status),
                super::fips_services::unavailable(status),
            ),
        }
    })
    .await
    .unwrap_or_else(|_| {
        (
            unavailable("timeout"),
            super::fips_services::unavailable("timeout"),
        )
    });
    let mut bundle = match bundle {
        Value::Object(bundle) => bundle,
        _ => serde_json::Map::new(),
    };
    bundle.insert("fips_transport".to_owned(), diagnostic);
    bundle.insert("fips_services".to_owned(), services);
    // The async task owns the sole sender, including timeout/error replies.
    let _ = reply.send(Value::Object(bundle).to_string());
}

async fn query_endpoint(
    endpoint: Option<Arc<FipsEndpoint>>,
    pubsub: Option<Arc<nostr_pubsub_fips::FipsPubsubClient>>,
) -> Result<Observation, &'static str> {
    let endpoint = endpoint.ok_or("unavailable")?;
    let peers = endpoint.peers().await.map_err(|_| "query_error")?;
    let services = super::fips_services::Observation::capture(endpoint, pubsub);
    let peers = peers
        .into_iter()
        .filter(|peer| peer.connected)
        .map(ObservedPeer::from)
        .collect();
    Ok(Observation { peers, services })
}

impl AppCore {
    pub(in crate::core) fn export_support_bundle_with_traffic(&self, reply: flume::Sender<String>) {
        let bundle =
            serde_json::to_value(self.build_support_bundle()).unwrap_or_else(|_| json!({}));
        let endpoint = self
            .device_sync
            .as_ref()
            .map(|runtime| runtime.endpoint.clone());
        let configured = self
            .device_sync
            .as_ref()
            .map(|runtime| runtime.configured_direct_peers.clone())
            .unwrap_or_default();
        let pubsub = self
            .device_sync
            .as_ref()
            .and_then(|runtime| runtime.pubsub.clone());
        self.runtime.spawn(send_reply(
            bundle,
            reply,
            query_endpoint(endpoint, pubsub),
            self.fips_traffic_history.clone(),
            configured,
            self.fips_connection_generation,
            QUERY_TIMEOUT,
        ));
    }
}

#[cfg(test)]
mod tests;
