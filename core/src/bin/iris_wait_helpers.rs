use iris_chat_core::AppState;
use serde_json::Value;

pub(super) fn has_pending_runtime_publishes(state: &AppState) -> bool {
    state
        .network_status
        .as_ref()
        .is_some_and(|status| status.pending_outbound_count > 0)
}

pub(super) fn has_pending_relay_transport_publishes(bundle: &Value) -> bool {
    bundle
        .pointer("/relay_transport/pending_relay_publish_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
        || bundle
            .pointer("/relay_transport/publish_drain_in_flight")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || bundle
            .pointer("/relay_transport/publish_drain_dirty")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

pub(super) fn has_delivery_blocking_protocol_work(bundle: &Value) -> bool {
    bundle
        .pointer("/protocol_engine/pending_group_fanout_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
}
