use super::remote_signer_uri::{safe_auth_url, validate_signer_relays};
use super::*;
use nostr::nips::nip44;
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub(super) struct SignerRpc {
    pub(super) client: Client,
    pub(super) keys: Keys,
    pub(super) signer: Option<PublicKey>,
    pub(super) notifications: broadcast::Receiver<RelayPoolNotification>,
    pub(super) token: String,
    pub(super) tx: Sender<CoreMsg>,
    pub(super) started_at: Timestamp,
}

impl SignerRpc {
    pub(super) async fn subscribe(&self, relays: &[RelayUrl]) -> anyhow::Result<()> {
        for relay in relays {
            self.client.add_relay(relay.clone()).await?;
        }
        self.client.connect().await;
        self.client
            .wait_for_connection(Duration::from_secs(5))
            .await;
        let filter = Filter::new()
            .kind(Kind::NostrConnect)
            .pubkey(self.keys.public_key())
            .since(self.started_at);
        let output = self.client.subscribe(filter, None).await?;
        anyhow::ensure!(
            !output.success.is_empty(),
            "Could not reach the signer. Try again."
        );
        Ok(())
    }

    pub(super) async fn send(&self, method: &str, params: Vec<String>) -> anyhow::Result<String> {
        let signer = self
            .signer
            .ok_or_else(|| anyhow::anyhow!("Signer is not connected."))?;
        let id = uuid::Uuid::new_v4().to_string();
        let content = nip44::encrypt(
            self.keys.secret_key(),
            &signer,
            json!({"id": id, "method": method, "params": params}).to_string(),
            nip44::Version::V2,
        )?;
        let event = EventBuilder::new(Kind::NostrConnect, content)
            .tag(nostr::Tag::public_key(signer))
            .sign_with_keys(&self.keys)?;
        let output = self.client.send_event(&event).await?;
        anyhow::ensure!(
            !output.success.is_empty(),
            "Could not reach the signer. Try again."
        );
        Ok(id)
    }

    pub(super) async fn request(
        &mut self,
        method: &str,
        params: Vec<String>,
    ) -> anyhow::Result<Value> {
        let id = self.send(method, params).await?;
        loop {
            let (_, response) = self.next_response().await?;
            if response.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if response.get("result").and_then(Value::as_str) == Some("auth_url") {
                let auth_url = response
                    .get("error")
                    .and_then(Value::as_str)
                    .and_then(safe_auth_url);
                anyhow::ensure!(
                    auth_url.is_some(),
                    "Signer returned an invalid approval link."
                );
                self.progress(crate::RemoteSignerPhase::WaitingForApproval, auth_url);
                continue;
            }
            if response.get("error").is_some_and(|error| !error.is_null()) {
                let error = response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if method == "switch_relays"
                    && [
                        "unsupported",
                        "unknown method",
                        "not supported",
                        "not implemented",
                    ]
                    .iter()
                    .any(|phrase| error.contains(phrase))
                {
                    anyhow::bail!("Unsupported signer method.");
                }
                anyhow::bail!("Signer declined the request.");
            }
            self.progress(crate::RemoteSignerPhase::WaitingForApproval, None);
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Invalid signer response."));
        }
    }

    pub(super) async fn next_response(&mut self) -> anyhow::Result<(PublicKey, Value)> {
        loop {
            let notification = self.notifications.recv().await?;
            let RelayPoolNotification::Event { event, .. } = notification else {
                continue;
            };
            if event.kind != Kind::NostrConnect
                || event.content.len() > 100_000
                || event.created_at < self.started_at
                || event.created_at.as_secs() > Timestamp::now().as_secs() + 60
                || self.signer.is_some_and(|signer| signer != event.pubkey)
                || !event
                    .tags
                    .public_keys()
                    .any(|key| *key == self.keys.public_key())
                || event.verify().is_err()
            {
                continue;
            }
            let Ok(plaintext) =
                nip44::decrypt(self.keys.secret_key(), &event.pubkey, &event.content)
            else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&plaintext) else {
                continue;
            };
            if !value.get("id").is_some_and(Value::is_string) || value.get("method").is_some() {
                continue;
            }
            return Ok((event.pubkey, value));
        }
    }

    pub(super) async fn switch_relays(&mut self) -> anyhow::Result<()> {
        // Older deployed signers may reject or ignore this recently added method.
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            self.request("switch_relays", Vec::new()),
        )
        .await;
        let mut result = match result {
            Err(_) => return Ok(()),
            Ok(Err(error)) if error.to_string() == "Unsupported signer method." => return Ok(()),
            Ok(result) => result?,
        };
        if let Some(encoded) = result.as_str() {
            result = serde_json::from_str(encoded)?;
        }
        if result.is_null() {
            return Ok(());
        }
        let relays: Vec<String> = serde_json::from_value(result)?;
        let relays = validate_signer_relays(&relays)?;
        self.client.unsubscribe_all().await;
        self.client.remove_all_relays().await;
        self.subscribe(&relays).await
    }

    pub(super) fn progress(&self, phase: crate::RemoteSignerPhase, auth_url: Option<String>) {
        let _ = self.tx.send(CoreMsg::Internal(Box::new(
            InternalEvent::RemoteSignerProgress {
                token: self.token.clone(),
                phase,
                auth_url,
            },
        )));
    }
}
