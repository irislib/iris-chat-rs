use super::device_link_signer::DeviceLinkInfo;
use super::remote_signer_uri::SignerConnection;
use super::*;
use nostr::nips::nip44;
use serde_json::{json, Value};
use tokio::sync::oneshot;

fn progress(tx: &Sender<CoreMsg>, phase: &'static str) {
    // Diagnostics must never contain the code, account keys, or encrypted messages.
    let _ = tx.send(CoreMsg::Internal(Box::new(InternalEvent::DebugLog {
        category: "device_link.approval".into(),
        detail: phase.into(),
    })));
}

pub(super) async fn run(
    connection: SignerConnection,
    owner: PublicKey,
    roster_relays: Vec<RelayUrl>,
    token: String,
    tx: Sender<CoreMsg>,
    cancel: oneshot::Receiver<()>,
) {
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    let work = serve(
        &client,
        &keys,
        &connection,
        owner,
        &roster_relays,
        &token,
        &tx,
    );
    let success = tokio::select! {
        _ = cancel => { client.shutdown().await; return; },
        result = tokio::time::timeout(Duration::from_secs(180), work) => matches!(result, Ok(Ok(()))),
    };
    progress(
        &tx,
        if success {
            "finished"
        } else {
            "failed_or_timed_out"
        },
    );
    let _ = tx.send(CoreMsg::Internal(Box::new(
        InternalEvent::DeviceLinkSignerFinished { token, success },
    )));
    client.shutdown().await;
}

async fn response(
    client: &Client,
    keys: &Keys,
    peer: PublicKey,
    value: &Value,
) -> anyhow::Result<()> {
    let content = nip44::encrypt(
        keys.secret_key(),
        &peer,
        value.to_string(),
        nip44::Version::V2,
    )?;
    let event = EventBuilder::new(Kind::NostrConnect, content)
        .tag(nostr::Tag::public_key(peer))
        .sign_with_keys(keys)?;
    super::remote_signer_rpc::send_signer_event(client, &event).await
}

async fn serve(
    client: &Client,
    keys: &Keys,
    connection: &SignerConnection,
    owner: PublicKey,
    roster_relays: &[RelayUrl],
    token: &str,
    tx: &Sender<CoreMsg>,
) -> anyhow::Result<()> {
    let started = Timestamp::from(Timestamp::now().as_secs().saturating_sub(10));
    progress(tx, "connecting");
    let mut notifications = client.notifications();
    for relay in &connection.relays {
        client.add_relay(relay.clone()).await?;
    }
    client.connect().await;
    client.wait_for_connection(Duration::from_secs(5)).await;
    anyhow::ensure!(
        !client
            .subscribe(
                Filter::new()
                    .kind(Kind::NostrConnect)
                    .author(connection.signer)
                    .pubkey(keys.public_key())
                    .since(started),
                None
            )
            .await?
            .success
            .is_empty(),
        "Device link unavailable."
    );
    progress(tx, "checking_device_list");
    prepare_owner_roster(owner, roster_relays, token, tx).await?;
    progress(tx, "sending_confirmation");
    response(
        client,
        keys,
        connection.signer,
        &json!({"id":uuid::Uuid::new_v4().to_string(),"result":connection.secret}),
    )
    .await?;
    progress(tx, "waiting_for_new_device");
    let mut replies = BTreeMap::<String, (String, Value)>::new();
    let mut approved: Option<(String, Event, DeviceLinkInfo)> = None;
    let mut success_announced = false;
    loop {
        let RelayPoolNotification::Event { event, .. } = notifications.recv().await? else {
            continue;
        };
        if event.kind != Kind::NostrConnect
            || event.pubkey != connection.signer
            || event.content.len() > 100_000
            || event.created_at < started
            || event.created_at.as_secs() > Timestamp::now().as_secs() + 60
            || !event
                .tags
                .public_keys()
                .any(|key| *key == keys.public_key())
            || event.verify().is_err()
        {
            continue;
        }
        let Ok(plaintext) = nip44::decrypt(keys.secret_key(), &connection.signer, &event.content)
        else {
            continue;
        };
        let Ok(request) = serde_json::from_str::<Value>(&plaintext) else {
            continue;
        };
        let Some(id) = request
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
        else {
            continue;
        };
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            continue;
        };
        let Some(params) = request
            .get("params")
            .and_then(Value::as_array)
            .filter(|params| params.len() <= 4 && params.iter().all(Value::is_string))
        else {
            continue;
        };
        progress(
            tx,
            match method {
                "get_public_key" => "received_get_public_key",
                "switch_relays" => "received_switch_relays",
                "sign_event" => "received_sign_event",
                "iris_get_link_info" => "received_link_info",
                _ => "received_other_request",
            },
        );
        let fingerprint = json!([method, params]).to_string();
        if let Some((prior, reply)) = replies.get(id) {
            let changed = json!({"id":id,"error":"Request ID already used."});
            response(
                client,
                keys,
                connection.signer,
                if prior == &fingerprint {
                    reply
                } else {
                    &changed
                },
            )
            .await?;
            continue;
        }
        anyhow::ensure!(replies.len() < 64, "Too many device link requests.");
        let result: anyhow::Result<Value> = async {
            match method {
                "get_public_key" if params.is_empty() => Ok(json!(owner.to_hex())),
                "switch_relays" if params.is_empty() => Ok(Value::Null),
                "ping" if params.is_empty() => Ok(json!("pong")),
                "sign_event" if params.len() == 1 => {
                    let draft = params
                        .first()
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("Invalid device authorization."))?;
                    if let Some((prior, signed, _)) = &approved {
                        anyhow::ensure!(prior == draft, "This link already approved a device.");
                        return Ok(json!(serde_json::to_string(signed)?));
                    }
                    anyhow::ensure!(draft.len() <= 32 * 1024, "Invalid device authorization.");
                    let previous =
                        super::account_signer_relay::fetch_signer_roster(owner, roster_relays)
                            .await
                            .map_err(anyhow::Error::msg)?;
                    let (reply, receiver) = oneshot::channel();
                    tx.send(CoreMsg::Internal(Box::new(
                        InternalEvent::DeviceLinkSignerRequest {
                            token: token.to_string(),
                            unsigned_event_json: draft.to_string(),
                            previous: previous.clone(),
                            reply,
                        },
                    )))?;
                    let (signed, info) = receiver.await?.map_err(anyhow::Error::msg)?;
                    let fresh =
                        super::account_signer_relay::fetch_signer_roster(owner, roster_relays)
                            .await
                            .map_err(anyhow::Error::msg)?;
                    anyhow::ensure!(
                        fresh.as_ref().map(|event| event.id)
                            == previous.as_ref().map(|event| event.id),
                        "Device list changed. Try again."
                    );
                    let result = json!(serde_json::to_string(&signed)?);
                    approved = Some((draft.to_string(), signed, info));
                    Ok(result)
                }
                "iris_get_link_info" if params.is_empty() => {
                    let (_, _, info) = approved
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Approve a device first."))?;
                    Ok(json!(serde_json::to_string(info)?))
                }
                _ => anyhow::bail!("Unsupported method."),
            }
        }
        .await;
        let reply = match result {
            Ok(result) => json!({"id":id,"result":result}),
            Err(_) => {
                progress(tx, "request_rejected");
                json!({"id":id,"error":"Request not authorized or unsupported."})
            }
        };
        response(client, keys, connection.signer, &reply).await?;
        replies.insert(id.to_string(), (fingerprint, reply.clone()));
        if method == "sign_event" && reply.get("error").is_none() && !success_announced {
            success_announced = true;
            let _ = tx.send(CoreMsg::Internal(Box::new(
                InternalEvent::DeviceLinkSignerFinished {
                    token: token.to_string(),
                    success: true,
                },
            )));
        }
        // Keep a short retry window after success: replayed IDs receive the exact cached result.
        if success_announced && replies.len() >= 32 {
            return Ok(());
        }
    }
}

async fn prepare_owner_roster(
    owner: PublicKey,
    relays: &[RelayUrl],
    token: &str,
    tx: &Sender<CoreMsg>,
) -> anyhow::Result<()> {
    use super::account_signer_relay::{fetch_signer_roster_heads, publish_signer_roster_repair};
    let heads = fetch_signer_roster_heads(owner, relays)
        .await
        .map_err(anyhow::Error::msg)?;
    if heads.len() <= 1 {
        return Ok(());
    }
    let prepare = |heads: Vec<Event>| async {
        let (reply, receiver) = oneshot::channel();
        tx.send(CoreMsg::Internal(Box::new(
            InternalEvent::DeviceLinkSignerRepair {
                token: token.to_string(),
                heads,
                reply,
            },
        )))?;
        receiver.await?.map_err(anyhow::Error::msg)
    };
    prepare(heads.clone()).await?;
    let fresh = fetch_signer_roster_heads(owner, relays)
        .await
        .map_err(anyhow::Error::msg)?;
    let ids = |heads: &[Event]| {
        heads
            .iter()
            .map(|event| event.id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    anyhow::ensure!(
        ids(&heads) == ids(&fresh),
        "Device list changed. Try again."
    );
    // Recheck the currently authorized local membership immediately before publishing.
    let event = prepare(fresh).await?;
    publish_signer_roster_repair(owner, relays, &event)
        .await
        .map_err(anyhow::Error::msg)?;
    tx.send(CoreMsg::Internal(Box::new(InternalEvent::RelayEvent(
        event,
    ))))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approval_reply_does_not_wait_for_a_server_that_omits_acknowledgements() {
        let healthy = crate::local_relay::TestRelay::start();
        let silent = crate::local_relay::TestRelay::start();
        silent.ignore_acknowledgements(24133);
        let keys = Keys::generate();
        let client = Client::new(keys.clone());
        client.add_relay(healthy.url()).await.unwrap();
        client.add_relay(silent.url()).await.unwrap();
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(2)).await;
        let sent = tokio::time::timeout(
            Duration::from_secs(2),
            response(
                &client,
                &keys,
                Keys::generate().public_key(),
                &json!({"id":"test", "result":"approved"}),
            ),
        )
        .await;
        client.shutdown().await;
        assert!(
            matches!(sent, Ok(Ok(()))),
            "one missing acknowledgement delayed the next signing/history request: {sent:?}"
        );
    }
}
