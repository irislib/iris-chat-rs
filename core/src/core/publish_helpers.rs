use super::*;

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
    let output = client
        .send_event_to(relay_urls.to_vec(), event)
        .await
        .map_err(|error| anyhow::anyhow!("{label}: {error}"))?;
    if output.success.is_empty() {
        let reasons = output.failed.values().cloned().collect::<Vec<_>>();
        return Err(anyhow::anyhow!(
            "{label}: {}",
            if reasons.is_empty() {
                "no relay accepted event for send".to_string()
            } else {
                reasons.join("; ")
            }
        ));
    }

    let mut relays = output
        .success
        .into_iter()
        .map(|relay| relay.to_string())
        .collect::<Vec<_>>();
    relays.sort();
    Ok(relays)
}

async fn ensure_publish_connection(client: &Client, relay_urls: &[RelayUrl]) {
    client.connect().await;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let connected = client
            .relays()
            .await
            .iter()
            .filter(|(relay_url, relay)| {
                relay_urls.iter().any(|configured| configured == *relay_url)
                    && relay.status() == RelayStatus::Connected
            })
            .count();
        if connected > 0 || tokio::time::Instant::now() >= deadline {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
}

pub(super) async fn publish_event_once(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: &Event,
) -> anyhow::Result<()> {
    if relay_urls.is_empty() {
        return Err(anyhow::anyhow!("no relays configured"));
    }

    let output = client
        .send_event_to(relay_urls.to_vec(), event)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    if output.success.is_empty() {
        let reasons = output.failed.values().cloned().collect::<Vec<_>>();
        Err(anyhow::anyhow!(if reasons.is_empty() {
            "no relay accepted event".to_string()
        } else {
            reasons.join("; ")
        }))
    } else {
        Ok(())
    }
}
