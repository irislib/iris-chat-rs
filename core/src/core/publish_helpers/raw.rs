use super::*;

/// One event on a fresh send-only connection. No account signer, receiver
/// subscriptions, or other recipients share its authentication context.
pub(super) async fn publish_event_raw(relay_url: &RelayUrl, event: &Event) -> anyhow::Result<()> {
    let event_id = event.id.to_hex();
    let (mut socket, _) = connect_async(relay_url.as_str()).await?;
    let event_wire = serde_json::json!(["EVENT", event]).to_string();
    socket.send(Message::Text(event_wire.clone())).await?;
    let mut auth_id = None;
    let mut authenticated = false;
    let mut last_notice = "no relay response".to_string();
    while let Some(message) = socket.next().await {
        let Message::Text(text) = message? else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(parts) = value.as_array() else {
            continue;
        };
        match parts.first().and_then(|part| part.as_str()) {
            Some("AUTH") => {
                let Some(challenge) = parts.get(1).and_then(|part| part.as_str()) else {
                    continue;
                };
                anyhow::ensure!(auth_id.is_none(), "relay repeated authentication challenge");
                let auth = EventBuilder::auth(challenge, relay_url.clone())
                    .sign_with_keys(&Keys::generate())?;
                auth_id = Some(auth.id.to_hex());
                socket
                    .send(Message::Text(serde_json::json!(["AUTH", auth]).to_string()))
                    .await?;
            }
            Some("OK") => {
                let id = parts.get(1).and_then(|part| part.as_str());
                let accepted = parts
                    .get(2)
                    .and_then(|part| part.as_bool())
                    .unwrap_or(false);
                let reason = parts.get(3).and_then(|part| part.as_str()).unwrap_or("");
                if auth_id.as_deref().is_some_and(|auth| Some(auth) == id) {
                    anyhow::ensure!(
                        accepted,
                        "anonymous relay authentication rejected: {reason}"
                    );
                    if !authenticated {
                        authenticated = true;
                        socket.send(Message::Text(event_wire.clone())).await?;
                    }
                } else if id == Some(event_id.as_str()) {
                    if accepted || relay_publish_failure_is_terminal_success(reason) {
                        let _ = socket.close(None).await;
                        return Ok(());
                    }
                    // The challenge and rejection may arrive in either order.
                    // Wait for anonymous AUTH, never retry with account keys.
                    if !authenticated && reason.starts_with("auth-required:") {
                        last_notice = reason.into();
                        continue;
                    }
                    anyhow::bail!("{reason}");
                }
            }
            Some("NOTICE") => {
                last_notice = parts
                    .get(1)
                    .and_then(|part| part.as_str())
                    .unwrap_or("relay notice")
                    .to_string();
            }
            _ => {}
        }
    }
    Err(anyhow::anyhow!(last_notice))
}

#[cfg(test)]
pub(super) mod tests;
