use super::*;
use futures_util::future::join_all;
use nostr::RelayMessage;
use nostr_double_ratchet::VerifiedAppKeysIndex;

const SIGNER_RELAY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SIGNER_ROSTER_EVENTS: usize = 1024;

pub(super) async fn fetch_signer_roster(
    owner: PublicKey,
    relay_urls: &[RelayUrl],
) -> Result<Option<Event>, String> {
    let mut index = VerifiedAppKeysIndex::default();
    for event in fetch_signer_roster_heads(owner, relay_urls).await? {
        index
            .ingest(event, unix_now().get())
            .map_err(|_| "Invalid device list from message server. Try again.".to_string())?;
    }
    let events = index.events_for_owner(owner);
    if events.len() > 1 {
        return Err("Conflicting device lists. Try again later.".into());
    }
    Ok(events.into_iter().next())
}

pub(super) async fn fetch_signer_roster_heads(
    owner: PublicKey,
    relay_urls: &[RelayUrl],
) -> Result<Vec<Event>, String> {
    if relay_urls.is_empty() {
        return Err("No message servers available.".into());
    }
    let client = Client::default();
    let results = join_all(
        relay_urls
            .iter()
            .map(|url| fetch_signer_roster_from_relay(&client, url, owner)),
    )
    .await;
    client.shutdown().await;
    let mut index = VerifiedAppKeysIndex::default();
    let mut heads = BTreeMap::new();
    let mut newest = 0;
    for result in results {
        for event in result? {
            if event.pubkey != owner || !is_app_keys_event(&event) {
                continue;
            }
            index
                .ingest(event.clone(), unix_now().get())
                .map_err(|_| "Invalid device list from message server. Try again.".to_string())?;
            let created_at = event.created_at.as_secs();
            if created_at > newest {
                heads.clear();
                newest = created_at;
            }
            if created_at == newest {
                heads.insert(event.id, event);
            }
        }
    }
    Ok(heads.into_values().collect())
}

async fn fetch_signer_roster_from_relay(
    client: &Client,
    url: &RelayUrl,
    owner: PublicKey,
) -> Result<Vec<Event>, String> {
    let result = tokio::time::timeout(SIGNER_RELAY_TIMEOUT, async {
        client.add_relay(url.clone()).await?;
        let relay = client.relay(url.clone()).await?;
        let mut notifications = relay.notifications();
        relay.connect();
        let subscription_id = SubscriptionId::generate();
        let filter = Filter::new()
            .author(owner)
            .kind(Kind::from(APP_KEYS_EVENT_KIND as u16))
            .limit(MAX_SIGNER_ROSTER_EVENTS);
        relay
            .subscribe_with_id(subscription_id.clone(), filter, SubscribeOptions::default())
            .await?;
        let result = async {
            let mut events = Vec::new();
            loop {
                match notifications.recv().await? {
                    RelayNotification::Message {
                        message:
                            RelayMessage::Event {
                                subscription_id: incoming,
                                event,
                            },
                    } if incoming.as_ref() == &subscription_id => {
                        anyhow::ensure!(
                            events.len() < MAX_SIGNER_ROSTER_EVENTS - 1,
                            "device list lookup exceeded limit"
                        );
                        events.push(event.into_owned());
                    }
                    RelayNotification::Message {
                        message: RelayMessage::EndOfStoredEvents(incoming),
                    } if incoming.as_ref() == &subscription_id => {
                        return Ok::<_, anyhow::Error>(events)
                    }
                    RelayNotification::Message {
                        message:
                            RelayMessage::Closed {
                                subscription_id: incoming,
                                ..
                            },
                    } if incoming.as_ref() == &subscription_id => {
                        anyhow::bail!("device list lookup closed")
                    }
                    RelayNotification::AuthenticationFailed | RelayNotification::Shutdown => {
                        anyhow::bail!("message server unavailable")
                    }
                    _ => {}
                }
            }
        }
        .await;
        relay.unsubscribe(&subscription_id).await?;
        result
    })
    .await;
    result
        .map_err(|_| "Could not check all message servers. Try again.".to_string())?
        .map_err(|_| "Could not check all message servers. Try again.".to_string())
}

pub(super) async fn publish_signer_authorization(
    owner: PublicKey,
    relay_urls: Vec<RelayUrl>,
    previous: Option<&Event>,
    event: Event,
) -> Result<Event, String> {
    // A second complete lookup catches roster edits made while the signer was
    // open. A stale authorization must never overwrite a newer device list.
    let current = fetch_signer_roster(owner, &relay_urls).await?;
    if current.as_ref().map(|event| event.id) != previous.map(|event| event.id) {
        return Err("Your device list changed. Sign in again.".into());
    }
    let client = Client::default();
    let results = join_all(relay_urls.iter().map(|url| async {
        tokio::time::timeout(SIGNER_RELAY_TIMEOUT, async {
            client.add_relay(url.clone()).await?;
            let relay = client.relay(url.clone()).await?;
            relay.connect();
            relay.send_event(&event).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
    }))
    .await;
    client.shutdown().await;
    if !results.iter().any(|result| matches!(result, Ok(Ok(())))) {
        return Err("Could not save device authorization. Try again.".into());
    }
    Ok(event)
}

pub(super) async fn publish_signer_roster_repair(
    owner: PublicKey,
    relay_urls: &[RelayUrl],
    event: &Event,
) -> Result<(), String> {
    let client = Client::default();
    let results = join_all(relay_urls.iter().map(|url| async {
        tokio::time::timeout(SIGNER_RELAY_TIMEOUT, async {
            client.add_relay(url.clone()).await?;
            let relay = client.relay(url.clone()).await?;
            relay.connect();
            relay.send_event(event).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
    }))
    .await;
    client.shutdown().await;
    if !results.iter().any(|result| matches!(result, Ok(Ok(())))) {
        return Err("Could not save device list. Try again.".into());
    }
    let current = fetch_signer_roster_heads(owner, relay_urls).await?;
    if !matches!(current.as_slice(), [current] if current.id == event.id) {
        return Err("Device list changed. Try again.".into());
    }
    Ok(())
}
