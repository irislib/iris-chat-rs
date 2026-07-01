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
    if bundle
        .pointer("/protocol_engine/pending_group_fanout_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
    {
        return true;
    }

    let pending_outbound_count = bundle
        .pointer("/protocol_engine/pending_outbound_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if pending_outbound_count == 0 {
        return false;
    }

    let Some(details) = bundle
        .pointer("/protocol_engine/pending_outbound_details")
        .and_then(Value::as_array)
    else {
        return true;
    };
    let local_owner_target = bundle
        .get("local_owner_pubkey_hex")
        .and_then(Value::as_str)
        .map(|owner| format!("owner:{owner}"));

    details.iter().any(|detail| {
        let remaining_remote = detail
            .get("remaining_remote_targets")
            .and_then(Value::as_array)
            .map(|targets| !targets.is_empty())
            .unwrap_or(false);
        let remaining_local = detail
            .get("remaining_local_sibling_targets")
            .and_then(Value::as_array)
            .map(|targets| !targets.is_empty())
            .unwrap_or(false);
        if remaining_remote || remaining_local {
            return true;
        }

        let queued_targets = detail
            .get("queued_targets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if queued_targets.is_empty() {
            return false;
        }

        let is_local_sibling_probe = detail
            .get("probe_local_sibling_roster")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let only_local_owner_probe = local_owner_target.as_ref().is_some_and(|local_target| {
            queued_targets
                .iter()
                .all(|target| target.as_str() == Some(local_target.as_str()))
        });
        !(is_local_sibling_probe && only_local_owner_probe)
    })
}
