use crate::state::NetworkStatusSnapshot;

pub(super) fn content_eq(
    left: Option<&NetworkStatusSnapshot>,
    right: Option<&NetworkStatusSnapshot>,
    include_diagnostics: bool,
) -> bool {
    if include_diagnostics {
        return left == right;
    }
    match (left, right) {
        (Some(left), Some(right)) => delivery_state(left) == delivery_state(right),
        (None, None) => true,
        _ => false,
    }
}

// Be exhaustive so a new network field requires an explicit decision here.
// Diagnostics remain current in the core and join the next meaningful snapshot
// or foreground refresh, without repeatedly serializing hidden chat history.
fn delivery_state(status: &NetworkStatusSnapshot) -> impl PartialEq + '_ {
    let NetworkStatusSnapshot {
        relay_set_id,
        relay_urls,
        relay_connections,
        connected_relay_count,
        all_relays_offline_since_secs,
        syncing,
        pending_outbound_count,
        pending_group_control_count,
        recent_event_count: _,
        recent_log_count: _,
        last_debug_category: _,
        last_debug_detail: _,
    } = status;
    (
        relay_set_id,
        relay_urls,
        relay_connections,
        connected_relay_count,
        all_relays_offline_since_secs,
        syncing,
        pending_outbound_count,
        pending_group_control_count,
    )
}
