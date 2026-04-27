use super::*;

pub(super) async fn publish_event_with_retry(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: Event,
    label: &str,
) -> anyhow::Result<()> {
    let mut last_error = "no relays configured".to_string();

    for attempt in 0..5 {
        connect_client_with_timeout(client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS)).await;
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

pub(super) async fn publish_event_first_ack(
    client: &Client,
    relay_urls: &[RelayUrl],
    event: &Event,
    label: &str,
) -> anyhow::Result<()> {
    if relay_urls.is_empty() {
        return Err(anyhow::anyhow!("{label}: no relays configured"));
    }

    connect_client_with_timeout(client, Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS)).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<(), String>>(relay_urls.len().max(1));

    for relay_url in relay_urls.iter().cloned() {
        let client = client.clone();
        let event = event.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = match client.send_event_to([relay_url.clone()], &event).await {
                Ok(output) if !output.success.is_empty() => Ok(()),
                Ok(output) => Err(output
                    .failed
                    .values()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "no relay accepted event".to_string())),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(result).await;
        });
    }
    drop(tx);

    let mut first_error = None;
    while let Some(result) = rx.recv().await {
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "{label}: {}",
        first_error.unwrap_or_else(|| "publish failed".to_string())
    ))
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
