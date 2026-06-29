use super::*;

pub(super) fn sorted_hexes(values: HashSet<String>) -> Vec<String> {
    let mut sorted = values.into_iter().collect::<Vec<_>>();
    sorted.sort();
    sorted.dedup();
    sorted
}

pub(super) fn summarize_protocol_plan(plan: Option<&ProtocolSubscriptionPlan>) -> String {
    let Some(plan) = plan else {
        return "none".to_string();
    };
    format!(
        "runtime={} roster_authors={} invite_authors={} message_authors={} message_recipients={} group_roster_group_ids={} group_roster_authors={} group_sender_key_authors={} invite_response_recipient={}",
        plan.runtime_subscriptions.join(","),
        plan.roster_authors.len(),
        plan.invite_authors.len(),
        plan.message_authors.len(),
        plan.message_recipients.len(),
        plan.group_roster_group_ids.len(),
        plan.group_roster_authors.len(),
        plan.group_sender_key_authors.len(),
        plan.invite_response_recipient.as_deref().unwrap_or("")
    )
}
