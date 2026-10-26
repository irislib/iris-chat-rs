use super::*;

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
    let invite_response_recipients = plan
        .invite_response_recipient
        .as_deref()
        .map(pubkeys_from_comma_separated_hexes)
        .unwrap_or_default();

    let mut filters = Vec::new();
    if !roster_authors.is_empty() {
        // Profile catch-up belongs to the same isolated public-owner worker
        // as its device roster, including updates published while offline.
        filters.push(
            Filter::new()
                .kind(Kind::Metadata)
                .authors(roster_authors.clone()),
        );
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

pub(in crate::core) async fn fetch_events_for_filters(
    client: &Client,
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, String> {
    let relay_urls = client.relays().await.keys().cloned().collect::<Vec<_>>();
    super::anonymous_reads::fetch(&relay_urls, filters, timeout).await
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
