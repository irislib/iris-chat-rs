use super::remote_signer_rpc::SignerRpc;
use super::remote_signer_uri::{SignerConnection, PERMISSIONS};
use super::*;
use tokio::sync::{mpsc, oneshot};

pub(super) struct RemoteSignRequest {
    pub(super) request_id: String,
    pub(super) unsigned_event_json: String,
}

pub(super) async fn run(
    mut rpc: SignerRpc,
    relays: Vec<RelayUrl>,
    connection: Option<SignerConnection>,
    challenge: String,
    mut commands: mpsc::Receiver<RemoteSignRequest>,
    cancel: oneshot::Receiver<()>,
) {
    let result = tokio::select! {
        _ = cancel => None,
        result = tokio::time::timeout(Duration::from_secs(180), authorize(&mut rpc, &relays, connection, &challenge, &mut commands)) => {
            Some(result.unwrap_or_else(|_| Err(anyhow::anyhow!("Sign-in timed out. Try again."))))
        }
    };
    if let Some(Err(error)) = result {
        // Never surface encrypted payloads, secrets, or full connection URLs.
        let message = if error.to_string().starts_with("Signer declined") {
            "Signer declined the request."
        } else if error.to_string().starts_with("Sign-in timed out") {
            "Sign-in timed out. Try again."
        } else {
            "Could not connect to the signer. Try again."
        };
        let _ = rpc.tx.send(CoreMsg::Internal(Box::new(
            InternalEvent::RemoteSignerFailed {
                token: rpc.token.clone(),
                message: message.into(),
            },
        )));
    }
    rpc.client.shutdown().await;
}

async fn authorize(
    rpc: &mut SignerRpc,
    relays: &[RelayUrl],
    connection: Option<SignerConnection>,
    challenge: &str,
    commands: &mut mpsc::Receiver<RemoteSignRequest>,
) -> anyhow::Result<()> {
    rpc.subscribe(relays).await?;
    if let Some(connection) = connection {
        let result = rpc
            .request(
                "connect",
                vec![
                    connection.signer.to_hex(),
                    connection.secret.clone(),
                    PERMISSIONS.into(),
                    serde_json::json!({"name":"Iris Chat", "url":"https://iris.to"}).to_string(),
                ],
            )
            .await?;
        let value = result.as_str().unwrap_or_default();
        anyhow::ensure!(
            value == "ack" || (!connection.secret.is_empty() && value == connection.secret),
            "Invalid signer confirmation."
        );
    } else {
        rpc.progress(crate::RemoteSignerPhase::WaitingForSigner, None);
        loop {
            let (author, response) = rpc.next_response().await?;
            if response.get("error").is_some_and(|error| !error.is_null())
                || response.get("result").and_then(serde_json::Value::as_str) != Some(challenge)
            {
                continue;
            }
            rpc.signer = Some(author);
            break;
        }
    }
    rpc.progress(crate::RemoteSignerPhase::WaitingForApproval, None);
    rpc.switch_relays().await?;
    let result = rpc.request("get_public_key", Vec::new()).await?;
    let owner = PublicKey::from_hex(result.as_str().unwrap_or_default())?;
    let _ = rpc.tx.send(CoreMsg::Internal(Box::new(
        InternalEvent::RemoteSignerConnected {
            token: rpc.token.clone(),
            owner_pubkey_hex: owner.to_hex(),
        },
    )));
    let request = commands
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("Sign-in cancelled."))?;
    rpc.progress(crate::RemoteSignerPhase::WaitingForApproval, None);
    let result = rpc
        .request("sign_event", vec![request.unsigned_event_json])
        .await?;
    let signed_event_json = result
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid signer response."))?
        .to_string();
    let _ = rpc.tx.send(CoreMsg::Internal(Box::new(
        InternalEvent::RemoteSignerSigned {
            token: rpc.token.clone(),
            request_id: request.request_id,
            signed_event_json,
        },
    )));
    Ok(())
}
