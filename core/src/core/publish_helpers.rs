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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    async fn delayed_ok_relay(delay: Duration) -> RelayUrl {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test relay");
        let addr = listener.local_addr().expect("test relay addr");
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let Ok(websocket) = accept_async(stream).await else {
                        return;
                    };
                    let (mut writer, mut reader) = websocket.split();
                    while let Some(Ok(message)) = reader.next().await {
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        let Some(event_id) = value
                            .as_array()
                            .and_then(|items| {
                                (items.first().and_then(|kind| kind.as_str()) == Some("EVENT"))
                                    .then_some(items)
                            })
                            .and_then(|items| items.get(1))
                            .and_then(|event| event.get("id"))
                            .and_then(|id| id.as_str())
                            .map(ToString::to_string)
                        else {
                            continue;
                        };
                        sleep(delay).await;
                        let reply = Message::Text(json!(["OK", event_id, true, ""]).to_string());
                        if writer.send(reply).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        RelayUrl::parse(&format!("ws://{addr}")).expect("parse test relay url")
    }

    fn timing_event() -> Event {
        EventBuilder::new(Kind::from(1), "publish timing")
            .sign_with_keys(&Keys::generate())
            .expect("sign timing event")
    }

    async fn run_current_publish_timing_case(
        scenario: &str,
        first_delay_ms: u64,
        second_delay_ms: u64,
    ) -> Duration {
        let first = delayed_ok_relay(Duration::from_millis(first_delay_ms)).await;
        let second = delayed_ok_relay(Duration::from_millis(second_delay_ms)).await;
        let client = Client::new(Keys::generate());
        let event = timing_event();

        let started = Instant::now();
        let accepted = publish_event_fire_and_forget(
            &client,
            &[first.clone(), second.clone()],
            &event,
            "timing current",
        )
        .await
        .expect("publish should succeed");
        let elapsed = started.elapsed();

        eprintln!(
            "CORE_TIMING_DATASET {{\"repo\":\"iris-chat-rs\",\"scenario\":\"{}\",\"strategy\":\"sequential_all_relays\",\"elapsed_ms\":{},\"accepted_relays\":{},\"first_delay_ms\":{},\"second_delay_ms\":{}}}",
            scenario,
            elapsed.as_millis(),
            accepted.len(),
            first_delay_ms,
            second_delay_ms
        );
        elapsed
    }

    #[tokio::test]
    async fn timing_current_publish_waits_for_slow_first_relay() {
        let cases = [
            ("publish_slow_first_fast_second", 600, 20),
            ("publish_fast_first_slow_second", 20, 600),
            ("publish_all_fast", 20, 30),
        ];
        for (scenario, first_delay_ms, second_delay_ms) in cases {
            let elapsed =
                run_current_publish_timing_case(scenario, first_delay_ms, second_delay_ms).await;
            let expected_serial_ms = first_delay_ms.saturating_add(second_delay_ms);
            let floor_ms = expected_serial_ms.saturating_sub(120);

            assert!(
                elapsed >= Duration::from_millis(floor_ms),
                "sequential publish should wait for both relays in {scenario}, elapsed={elapsed:?}"
            );
            assert!(
                elapsed < Duration::from_secs(3),
                "local timing relay test should not hang in {scenario}, elapsed={elapsed:?}"
            );
        }
    }
}
