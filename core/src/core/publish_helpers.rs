use super::*;

const RELAY_PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn publish_event_with_retry(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: Event,
    label: &str,
) -> anyhow::Result<()> {
    let mut last_error = "no relays configured".to_string();

    for attempt in 0..5 {
        ensure_publish_connection(client, relay_urls).await;
        match publish_event_once(client, relay_urls, &event).await {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error.to_string(),
        }

        if attempt < 4 {
            sleep(Duration::from_millis(750 * (attempt + 1) as u64)).await;
        }
    }

    Err(anyhow::anyhow!("{label}: {last_error}"))
}

pub(super) async fn publish_event_fire_and_forget(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: &Event,
    label: &str,
) -> anyhow::Result<Vec<String>> {
    if relay_urls.is_empty() {
        return Err(anyhow::anyhow!("{label}: no relays configured"));
    }

    ensure_publish_connection(client, relay_urls).await;
    publish_event_to_any_relay(client, relay_urls, event, label).await
}

async fn ensure_publish_connection(client: &Client, relay_urls: &[RelayUrl]) {
    ensure_session_relays_configured(client, relay_urls).await;
    connect_client_with_timeout(client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS)).await;
}

pub(super) async fn publish_event_once(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: &Event,
) -> anyhow::Result<()> {
    if relay_urls.is_empty() {
        return Err(anyhow::anyhow!("no relays configured"));
    }

    publish_event_to_any_relay(client, relay_urls, event, "publish")
        .await
        .map(|_| ())
}

async fn publish_event_to_any_relay(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: &Event,
    label: &str,
) -> anyhow::Result<Vec<String>> {
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    let attempts = relay_urls.iter().cloned().map(|relay_url| {
        let client = client.clone();
        let event = event.clone();
        async move { publish_event_to_relay(client, relay_url, event).await }
    });

    for outcome in futures_util::future::join_all(attempts).await {
        match outcome {
            RelayPublishOutcome::Success(relays) => successes.extend(relays),
            RelayPublishOutcome::Failure(failure) => failures.push(failure),
        }
    }

    successes.sort();
    successes.dedup();
    if successes.is_empty() {
        return Err(anyhow::anyhow!(
            "{label}: {}",
            if failures.is_empty() {
                "no relay accepted event".to_string()
            } else {
                failures.join("; ")
            }
        ));
    }
    Ok(successes)
}

enum RelayPublishOutcome {
    Success(Vec<String>),
    Failure(String),
}

async fn publish_event_to_relay(
    client: Client,
    relay_url: RelayUrl,
    event: Event,
) -> RelayPublishOutcome {
    let relay_label = relay_url.to_string();
    let result = tokio::time::timeout(
        RELAY_PUBLISH_TIMEOUT,
        client.send_event_to(vec![relay_url], &event),
    )
    .await;

    match result {
        Ok(Ok(output)) if !output.success.is_empty() => RelayPublishOutcome::Success(
            output
                .success
                .into_iter()
                .map(|relay| relay.to_string())
                .collect(),
        ),
        Ok(Ok(output)) => {
            let reason = output
                .failed
                .values()
                .next()
                .cloned()
                .unwrap_or_else(|| "no relay accepted event".to_string());
            if relay_publish_failure_is_terminal_success(&reason) {
                RelayPublishOutcome::Success(vec![relay_label])
            } else {
                RelayPublishOutcome::Failure(format!("{relay_label}: {reason}"))
            }
        }
        Ok(Err(error)) => RelayPublishOutcome::Failure(format!("{relay_label}: {error}")),
        Err(_) => RelayPublishOutcome::Failure(format!("{relay_label}: publish timed out")),
    }
}

pub(super) fn relay_publish_failure_is_terminal_success(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("duplicate")
        || lower.contains("already have")
        || (lower.contains("replaced") && lower.contains("newer"))
}
