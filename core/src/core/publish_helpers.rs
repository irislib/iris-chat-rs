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
    let started_ms = crate::perflog::now_ms();
    let event_id = event.id.to_string();
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<Result<Vec<String>, String>>(relay_urls.len().max(1));

    for relay_url in relay_urls.iter().cloned() {
        let client = client.clone();
        let event = event.clone();
        let event_id = event_id.clone();
        let label = label.to_string();
        let relay_label = relay_url.to_string();
        let tx = tx.clone();
        tokio::spawn(async move {
            let relay_started_ms = crate::perflog::now_ms();
            let result = tokio::time::timeout(
                RELAY_PUBLISH_TIMEOUT,
                client.send_event_to([relay_url], &event),
            )
            .await;
            let elapsed_ms = crate::perflog::now_ms().saturating_sub(relay_started_ms);
            let result = match result {
                Ok(Ok(output)) if !output.success.is_empty() => {
                    let successes = output
                        .success
                        .into_iter()
                        .map(|relay| relay.to_string())
                        .collect::<Vec<_>>();
                    crate::perflog!(
                        "publish.relay.ok label={label} event_id={event_id} relay={relay_label} elapsed_ms={elapsed_ms}"
                    );
                    Ok(successes)
                }
                Ok(Ok(output)) => {
                    let reason = output
                        .failed
                        .values()
                        .next()
                        .cloned()
                        .unwrap_or_else(|| "no relay accepted event".to_string());
                    if relay_publish_failure_is_terminal_success(&reason) {
                        crate::perflog!(
                            "publish.relay.terminal_success label={label} event_id={event_id} relay={relay_label} elapsed_ms={elapsed_ms} reason={reason}"
                        );
                        Ok(vec![relay_label])
                    } else {
                        crate::perflog!(
                            "publish.relay.failed label={label} event_id={event_id} relay={relay_label} elapsed_ms={elapsed_ms} reason={reason}"
                        );
                        Err(format!("{relay_label}: {reason}"))
                    }
                }
                Ok(Err(error)) => {
                    crate::perflog!(
                        "publish.relay.error label={label} event_id={event_id} relay={relay_label} elapsed_ms={elapsed_ms} error={error}"
                    );
                    Err(format!("{relay_label}: {error}"))
                }
                Err(_) => {
                    crate::perflog!(
                        "publish.relay.timeout label={label} event_id={event_id} relay={relay_label} elapsed_ms={elapsed_ms}"
                    );
                    Err(format!("{relay_label}: publish timed out"))
                }
            };
            let _ = tx.send(result).await;
        });
    }
    drop(tx);

    let mut failures = Vec::new();
    while let Some(result) = rx.recv().await {
        match result {
            Ok(mut successes) => {
                successes.sort();
                successes.dedup();
                crate::perflog!(
                    "publish.any.ok label={label} event_id={event_id} relays={} elapsed_ms={}",
                    successes.join(","),
                    crate::perflog::now_ms().saturating_sub(started_ms)
                );
                return Ok(successes);
            }
            Err(error) => failures.push(error),
        }
    }

    crate::perflog!(
        "publish.any.failed label={label} event_id={event_id} failures={} elapsed_ms={}",
        failures.len(),
        crate::perflog::now_ms().saturating_sub(started_ms)
    );
    Err(anyhow::anyhow!(
        "{label}: {}",
        if failures.is_empty() {
            "no relay accepted event".to_string()
        } else {
            failures.join("; ")
        }
    ))
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

    async fn delayed_relay(delay: Duration, accepted: bool, reason: &'static str) -> RelayUrl {
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
                    while let Some(Ok(incoming)) = reader.next().await {
                        let Message::Text(text) = incoming else {
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
                        let reply =
                            Message::Text(json!(["OK", event_id, accepted, reason]).to_string());
                        if writer.send(reply).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        RelayUrl::parse(&format!("ws://{addr}")).expect("parse test relay url")
    }

    async fn delayed_ok_relay(delay: Duration) -> RelayUrl {
        delayed_relay(delay, true, "").await
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
        first_accepts: bool,
        second_accepts: bool,
    ) -> anyhow::Result<(Duration, usize)> {
        let first = delayed_relay(
            Duration::from_millis(first_delay_ms),
            first_accepts,
            "test reject",
        )
        .await;
        let second = delayed_relay(
            Duration::from_millis(second_delay_ms),
            second_accepts,
            "test reject",
        )
        .await;
        let client = Client::new(Keys::generate());
        let event = timing_event();

        let started = Instant::now();
        let result = publish_event_fire_and_forget(
            &client,
            &[first.clone(), second.clone()],
            &event,
            "timing current",
        )
        .await;
        let elapsed = started.elapsed();
        let outcome = if result.is_ok() { "success" } else { "failure" };
        let accepted_relays = result.as_ref().map(|relays| relays.len()).unwrap_or(0);
        eprintln!(
            "CORE_TIMING_DATASET {{\"repo\":\"iris-chat-rs\",\"scenario\":\"{}\",\"strategy\":\"parallel_first_ack\",\"outcome\":\"{}\",\"elapsed_ms\":{},\"accepted_relays\":{},\"first_delay_ms\":{},\"second_delay_ms\":{},\"first_accepts\":{},\"second_accepts\":{}}}",
            scenario,
            outcome,
            elapsed.as_millis(),
            accepted_relays,
            first_delay_ms,
            second_delay_ms,
            first_accepts,
            second_accepts
        );
        result.map(|relays| (elapsed, relays.len()))
    }

    #[tokio::test]
    async fn timing_current_publish_returns_on_fast_first_ack() {
        let cases = [
            ("publish_slow_first_fast_second", 600, 20, true, true),
            ("publish_fast_first_slow_second", 20, 600, true, true),
            ("publish_all_fast", 20, 30, true, true),
            ("publish_fast_fail_slow_success", 20, 180, false, true),
        ];
        for (scenario, first_delay_ms, second_delay_ms, first_accepts, second_accepts) in cases {
            let (elapsed, accepted_relays) = run_current_publish_timing_case(
                scenario,
                first_delay_ms,
                second_delay_ms,
                first_accepts,
                second_accepts,
            )
            .await
            .expect("publish should succeed");
            let expected_fastest_success_ms = [
                first_accepts.then_some(first_delay_ms),
                second_accepts.then_some(second_delay_ms),
            ]
            .into_iter()
            .flatten()
            .min()
            .expect("success case has an accepting relay");

            assert!(
                elapsed < Duration::from_millis(expected_fastest_success_ms.saturating_add(300)),
                "first-ack publish should return near the fastest accepting relay in {scenario}, elapsed={elapsed:?}"
            );
            assert_eq!(accepted_relays, 1);
        }
    }

    #[tokio::test]
    async fn timing_current_publish_fails_after_all_relays_reject() {
        let started = Instant::now();
        let result =
            run_current_publish_timing_case("publish_all_reject", 20, 30, false, false).await;
        let elapsed = started.elapsed();

        assert!(result.is_err());
        assert!(
            elapsed >= Duration::from_millis(20),
            "failure should wait for at least the fastest rejection, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "failure should collect concurrent rejections without serial delay, elapsed={elapsed:?}"
        );
    }

    #[tokio::test]
    async fn timing_current_publish_accepts_terminal_duplicate_as_success() {
        let first =
            delayed_relay(Duration::from_millis(20), false, "duplicate: already have").await;
        let second = delayed_ok_relay(Duration::from_millis(600)).await;
        let client = Client::new(Keys::generate());
        let event = timing_event();
        let started = Instant::now();
        let accepted =
            publish_event_fire_and_forget(&client, &[first, second], &event, "timing duplicate")
                .await
                .expect("terminal duplicate should count as success");
        let elapsed = started.elapsed();

        eprintln!(
            "CORE_TIMING_DATASET {{\"repo\":\"iris-chat-rs\",\"scenario\":\"publish_duplicate_fast_slow_success\",\"strategy\":\"parallel_first_ack\",\"outcome\":\"success\",\"elapsed_ms\":{},\"accepted_relays\":{},\"first_delay_ms\":20,\"second_delay_ms\":600,\"first_accepts\":false,\"second_accepts\":true,\"first_terminal_success\":true}}",
            elapsed.as_millis(),
            accepted.len()
        );
        assert_eq!(accepted.len(), 1);
        assert!(
            elapsed < Duration::from_millis(320),
            "terminal duplicate should complete before the slow relay, elapsed={elapsed:?}"
        );
    }
}
