//! Fixed, local-only counters for the opt-in stack process fixture.
//! No payloads, peer identifiers, routing changes or new network requests.
use fips_core::FipsEndpoint;
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubDeliverySnapshot};
use serde_json::{json, Value};
use std::collections::BTreeMap;

// The fixture installs its collector before app startup. Other stack-feature
// test helpers retain their behavior unless that collector is explicitly installed.
pub(super) fn enable_services(endpoint: &FipsEndpoint) -> anyhow::Result<()> {
    for port in [7368, 39019] {
        endpoint.enable_service_carrier_diagnostics(port)?;
    }
    super::control::mark_services_enabled()
}

pub(super) fn snapshot(
    endpoint: &FipsEndpoint,
    client: &FipsPubsubClient,
) -> anyhow::Result<Value> {
    let mut counters = pubsub_counters(
        client.delivery_snapshot(),
        client.transport_error_count(),
        client.reputation_error_count(),
    );
    for port in [7368, 39019] {
        // The production registry retains each handle for this endpoint's life.
        // Repeated enable calls reuse counters; they neither reset nor unregister.
        let carrier = endpoint
            .enable_service_carrier_diagnostics(port)?
            .snapshot();
        counters.insert(
            format!("service.{port}.ambiguous_port_datagrams"),
            carrier.ambiguous_port_datagrams,
        );
        counters.insert(
            format!("service.{port}.discarded_outputs"),
            carrier.discarded_outputs,
        );
        for transport in carrier.transports {
            let prefix = format!("service.{port}.{}", transport.transport);
            counters.insert(
                format!("{prefix}.submitted_packets"),
                transport.submitted_packets,
            );
            counters.insert(
                format!("{prefix}.fips_payload_bytes"),
                transport.fips_payload_bytes,
            );
            counters.insert(
                format!("{prefix}.ethernet_framing_bytes"),
                transport.ethernet_framing_bytes,
            );
        }
    }
    let value = json!({"version": 2, "counters": counters, "control": super::control::snapshot()?});
    anyhow::ensure!(
        serde_json::to_vec(&value)?.len() <= 8192,
        "fixture traffic snapshot exceeds cap"
    );
    Ok(value)
}

fn pubsub_counters(
    s: FipsPubsubDeliverySnapshot,
    transport_errors: u64,
    reputation_errors: u64,
) -> BTreeMap<String, u64> {
    let mut counters = BTreeMap::new();
    macro_rules! add { ($($field:ident),+ $(,)?) => { $(counters.insert(concat!("pubsub.", stringify!($field)).to_string(), s.$field);)+ }; }
    add!(
        req_frames_received,
        close_frames_received,
        event_frames_received,
        inv_frames_received,
        want_frames_received,
        want_frames_sent,
        subscription_events_received,
        expired_wants,
        provider_cooldowns,
        tcp_receive_batches,
        tcp_datagrams_received,
        tcp_datagrams_rejected,
        tcp_poll_turns
    );
    counters.insert("pubsub.transport_errors".into(), transport_errors);
    counters.insert("pubsub.reputation_errors".into(), reputation_errors);
    counters
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_fields_keep_errors_and_message_counters_distinct() {
        let counters = pubsub_counters(
            FipsPubsubDeliverySnapshot {
                req_frames_received: 3,
                event_frames_received: 5,
                expired_wants: 7,
                ..Default::default()
            },
            11,
            13,
        );
        assert_eq!(counters.len(), 15);
        assert_eq!(counters["pubsub.req_frames_received"], 3);
        assert_eq!(counters["pubsub.event_frames_received"], 5);
        assert_eq!(counters["pubsub.expired_wants"], 7);
        assert_eq!(counters["pubsub.transport_errors"], 11);
        assert_eq!(counters["pubsub.reputation_errors"], 13);
        assert_eq!(counters["pubsub.tcp_datagrams_rejected"], 0);
        let decoded: BTreeMap<String, u64> =
            serde_json::from_slice(&serde_json::to_vec(&counters).unwrap()).unwrap();
        assert_eq!(decoded, counters);
    }
}
