use super::*;

pub(super) fn normalize_protocol_queued_targets(targets: &mut Vec<String>) {
    targets.retain(|target| !target.is_empty());
    targets.sort();
    targets.dedup();
}

pub(super) struct ProtocolSubscriptionApplyOutput {
    pub(super) connected_before: u64,
    pub(super) connected_after: u64,
    pub(super) filter_count: u64,
    pub(super) success: bool,
    pub(super) error: Option<String>,
}

pub(in crate::core) fn build_protocol_subscription_filters(
    plan: &ProtocolSubscriptionPlan,
) -> Vec<Filter> {
    let roster_authors = pubkeys_from_hexes(&plan.roster_authors);
    let invite_authors = pubkeys_from_hexes(&plan.invite_authors);
    let message_authors = pubkeys_from_hexes(&plan.message_authors);
    let message_recipients = pubkeys_from_hexes(&plan.message_recipients);
    let group_roster_authors = pubkeys_from_hexes(&plan.group_roster_authors);
    let group_sender_key_authors = pubkeys_from_hexes(&plan.group_sender_key_authors);
    let invite_response_recipients = plan
        .invite_response_recipient
        .as_deref()
        .map(pubkeys_from_comma_separated_hexes)
        .unwrap_or_default();

    let mut filters = Vec::new();
    if !roster_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
                .authors(roster_authors),
        );
    }
    if !invite_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_EVENT_KIND as u16))
                .authors(invite_authors.clone())
                .custom_tag(SingleLetterTag::lowercase(Alphabet::L), INVITE_LIST_LABEL),
        );
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .authors(invite_authors),
        );
    }
    if !message_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .authors(message_authors),
        );
    }
    if !message_recipients.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(MESSAGE_EVENT_KIND as u16))
                .pubkeys(message_recipients),
        );
    }
    if !plan.group_roster_group_ids.is_empty() {
        filters.push(build_group_roster_fact_filter(
            plan.group_roster_group_ids.iter(),
            group_roster_authors,
        ));
    }
    if !group_sender_key_authors.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(GROUP_SENDER_KEY_MESSAGE_KIND as u16))
                .authors(group_sender_key_authors),
        );
    }
    if !invite_response_recipients.is_empty() {
        filters.push(
            Filter::new()
                .kind(Kind::from(INVITE_RESPONSE_KIND as u16))
                .pubkeys(invite_response_recipients),
        );
    }
    filters
}

pub(super) fn pubkeys_from_hexes(hexes: &[String]) -> Vec<PublicKey> {
    hexes
        .iter()
        .filter_map(|hex| PublicKey::parse(hex).ok())
        .collect()
}

pub(super) fn pubkeys_from_comma_separated_hexes(hexes: &str) -> Vec<PublicKey> {
    hexes
        .split(',')
        .filter(|hex| !hex.is_empty())
        .filter_map(|hex| PublicKey::parse(hex).ok())
        .collect()
}

pub(super) async fn current_client_relay_statuses(client: &Client) -> Vec<(String, RelayStatus)> {
    client
        .relays()
        .await
        .into_iter()
        .map(|(relay_url, relay)| {
            let relay_url = normalize_nostr_relay_url(&relay_url.to_string())
                .unwrap_or_else(|_| relay_url.to_string());
            (relay_url, relay.status())
        })
        .collect()
}

pub(super) async fn subscribe_protocol_filters_with_id(
    client: &Client,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
) -> Result<(), String> {
    let relays = client.relays().await;
    let mut attempted = 0usize;
    let mut accepted = 0usize;
    let mut last_error = None;
    for relay in relays.values() {
        if relay.status() != RelayStatus::Connected {
            continue;
        }
        attempted = attempted.saturating_add(1);
        match relay
            .subscribe_with_id(
                subscription_id.clone(),
                filters.clone(),
                SubscribeOptions::default(),
            )
            .await
        {
            Ok(()) => accepted = accepted.saturating_add(1),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if accepted > 0 {
        Ok(())
    } else if attempted == 0 {
        Err("no connected relays".to_string())
    } else {
        Err(last_error.unwrap_or_else(|| "no relay accepted subscription".to_string()))
    }
}

pub(super) async fn fetch_events_for_filters(
    client: &Client,
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, String> {
    use tokio::task::JoinSet;

    let mut tasks = JoinSet::new();
    for filter in filters {
        let client = client.clone();
        tasks.spawn(async move { client.fetch_events(filter, timeout).await });
    }

    let mut any_success = false;
    let mut last_error = None;
    let mut seen_event_ids = HashSet::new();
    let mut collected = Vec::new();

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(events)) => {
                any_success = true;
                for event in events.iter() {
                    if seen_event_ids.insert(event.id) {
                        collected.push(event.clone());
                    }
                }
            }
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
    }

    if any_success {
        Ok(collected)
    } else {
        Err(last_error.unwrap_or_else(|| "no protocol filters fetched".to_string()))
    }
}

pub(super) async fn wait_for_connected_relays(client: &Client, timeout: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let connected = client
            .relays()
            .await
            .values()
            .filter(|relay| relay.status() == RelayStatus::Connected)
            .count();
        if connected > 0 || tokio::time::Instant::now() >= deadline {
            return connected;
        }
        sleep(Duration::from_millis(100)).await;
    }
}
