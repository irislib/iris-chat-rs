//! Small, disposable read connections. Never construct an SDK Client here:
//! its verification caches are eagerly allocated even for an empty lookup.
use super::*;
use futures_util::{stream, SinkExt, StreamExt};
use nostr::EventId;
use std::sync::OnceLock;
use tokio::sync::Semaphore;
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    Connector,
};

mod scoped_tls;
pub(in crate::core) use scoped_tls::ScopedTlsConnector;

pub(super) const COLD_READERS: usize = 4;
pub(super) const BACKGROUND_READERS: usize = 16;
const MAX_FRAME_BYTES: usize = 128 * 1024;
const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESULT_EVENTS: usize = 16_384;
static COLD_SLOTS: OnceLock<Semaphore> = OnceLock::new();
static BACKGROUND_SLOTS: OnceLock<Semaphore> = OnceLock::new();

#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::core) mod tls_tests;

/// Cold lookups have their own admission budget so a populated account's
/// background catch-up cannot starve an invite or first-contact proof.
pub(in crate::core) async fn fetch(
    relays: &[RelayUrl],
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, String> {
    let filters = filters
        .into_iter()
        .flat_map(super::private_reads::split_public_discovery_authors)
        .collect::<Vec<_>>();
    if filters.is_empty() || relays.is_empty() {
        return Err("no discovery filters or relays".into());
    }
    // Only four futures per caller are materialized, and admission is global
    // across callers. Each current job has a finite admission wait and then
    // its network budget. Unstarted subjects do not expire while waiting for
    // an earlier portion of the same batch; dropping this future cancels both.
    let allow_history = filters.iter().any(requests_message_history);
    let relays = Arc::new(relays.to_vec());
    let jobs = filters.into_iter().flat_map(move |filter| {
        let relays = relays.clone();
        (0..relays.len()).map(move |index| {
            let relay = relays.get(index).cloned();
            let filter = filter.clone();
            async move {
                let relay =
                    relay.ok_or_else(|| ReadFailure::Unavailable("missing relay".into()))?;
                let permit = tokio::time::timeout(
                    timeout,
                    COLD_SLOTS
                        .get_or_init(|| Semaphore::new(COLD_READERS))
                        .acquire(),
                )
                .await;
                match permit {
                    Ok(Ok(_permit)) => {
                        read_one(&relay, std::slice::from_ref(&filter), timeout).await
                    }
                    Ok(Err(error)) => Err(ReadFailure::Unavailable(error.to_string())),
                    Err(_) => Err(ReadFailure::Unavailable(
                        "anonymous discovery admission timed out".into(),
                    )),
                }
            }
        })
    });
    let mut reads = stream::iter(jobs).buffer_unordered(COLD_READERS);
    let mut collected = Collection::new(allow_history);
    let mut succeeded = false;
    let mut last_error = "anonymous discovery unavailable".to_string();
    while let Some(result) = reads.next().await {
        match result {
            Ok(events) => {
                succeeded = true;
                for event in events {
                    collected.insert(event)?;
                }
            }
            Err(ReadFailure::Limit(reason)) => return Err(reason),
            Err(ReadFailure::Unavailable(reason)) => last_error = reason,
        }
    }
    // Preserve independent-scope partial success on ordinary unavailability;
    // resource-limit errors above remain fatal even after another success.
    if succeeded {
        Ok(collected.events)
    } else {
        Err(last_error)
    }
}

pub(super) async fn poll(
    relay: &RelayUrl,
    filters: Vec<Filter>,
    tls: Option<&ScopedTlsConnector>,
) -> Result<Vec<Event>, String> {
    let _permit = BACKGROUND_SLOTS
        .get_or_init(|| Semaphore::new(BACKGROUND_READERS))
        .acquire()
        .await
        .map_err(|error| error.to_string())?;
    // Bound an unresponsive subject's turn; Tokio's semaphore is FIFO, so
    // every retained author eventually runs even when all relays are offline.
    let connector = tls.map(|tls| tls.connector(relay, &filters)).transpose()?;
    read_one_with_connector(relay, &filters, Duration::from_secs(3), connector)
        .await
        .map_err(ReadFailure::message)
}

fn requests_message_history(filter: &Filter) -> bool {
    filter.kinds.as_ref().is_some_and(|kinds| {
        kinds
            .iter()
            .any(|kind| kind.as_u16() as u32 == MESSAGE_EVENT_KIND)
    })
}

fn is_message_history(event: &Event) -> bool {
    event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
}

struct Collection {
    events: Vec<Event>,
    ids: HashSet<EventId>,
    bytes: usize,
    metadata_count: usize,
    allow_history: bool,
}

impl Collection {
    fn new(allow_history: bool) -> Self {
        Self {
            events: Vec::new(),
            ids: HashSet::new(),
            bytes: 0,
            metadata_count: 0,
            allow_history,
        }
    }

    fn insert(&mut self, event: Event) -> Result<(), String> {
        if self.ids.contains(&event.id) {
            return Ok(());
        }
        // Preserve the existing unbounded valid message-history return API.
        // The startup fix bounds SDK/cache/socket allocations, not stored chat
        // history. Metadata and untrusted traffic remain separately limited.
        if !(self.allow_history && is_message_history(&event)) {
            self.bytes = self.bytes.saturating_add(
                serde_json::to_vec(&event)
                    .map_err(|error| error.to_string())?
                    .len(),
            );
            self.metadata_count += 1;
            if self.bytes > MAX_RESULT_BYTES || self.metadata_count > MAX_RESULT_EVENTS {
                return Err("anonymous discovery result exceeds safety bound".into());
            }
        }
        self.ids.insert(event.id);
        self.events.push(event);
        Ok(())
    }
}

#[derive(Debug)]
struct ReadLimit(String);
impl std::fmt::Display for ReadLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for ReadLimit {}

enum ReadFailure {
    Unavailable(String),
    Limit(String),
}
impl ReadFailure {
    fn message(self) -> String {
        match self {
            Self::Unavailable(message) | Self::Limit(message) => message,
        }
    }
}

async fn read_one(
    relay: &RelayUrl,
    filters: &[Filter],
    timeout: Duration,
) -> Result<Vec<Event>, ReadFailure> {
    read_one_with_connector(relay, filters, timeout, None).await
}

async fn read_one_with_connector(
    relay: &RelayUrl,
    filters: &[Filter],
    timeout: Duration,
    connector: Option<Connector>,
) -> Result<Vec<Event>, ReadFailure> {
    let mut collected = Collection::new(filters.iter().any(requests_message_history));
    match tokio::time::timeout(
        timeout,
        read_socket(relay, filters, &mut collected, connector),
    )
    .await
    {
        // Match the former SDK fetch deadline: already validated events remain
        // useful when a relay omits EOSE. Safety-limit failures below never
        // take this path and cannot masquerade as a successful partial fetch.
        Err(_) if !collected.events.is_empty() => Ok(collected.events),
        Err(_) => Err(ReadFailure::Unavailable(
            "anonymous discovery timed out".into(),
        )),
        Ok(Ok(())) => Ok(collected.events),
        Ok(Err(error)) => {
            if error.is::<ReadLimit>()
                || matches!(
                    error.downcast_ref::<tokio_tungstenite::tungstenite::Error>(),
                    Some(tokio_tungstenite::tungstenite::Error::Capacity(_))
                )
            {
                Err(ReadFailure::Limit(error.to_string()))
            } else {
                Err(ReadFailure::Unavailable(error.to_string()))
            }
        }
    }
}

fn check_response_budget(bytes: usize) -> anyhow::Result<()> {
    if bytes > MAX_RESULT_BYTES {
        return Err(ReadLimit("discovery response too large".into()).into());
    }
    Ok(())
}

async fn read_socket(
    relay: &RelayUrl,
    filters: &[Filter],
    collected: &mut Collection,
    connector: Option<Connector>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!filters.is_empty(), "no discovery filters");
    let authors = filters
        .iter()
        .filter_map(|filter| filter.authors.as_ref())
        .flatten()
        .collect::<HashSet<_>>();
    anyhow::ensure!(
        authors.len() <= 1,
        "discovery must isolate each public author"
    );
    let config = WebSocketConfig {
        write_buffer_size: 0,
        max_write_buffer_size: MAX_FRAME_BYTES * 2,
        max_message_size: Some(MAX_FRAME_BYTES),
        max_frame_size: Some(MAX_FRAME_BYTES),
        ..Default::default()
    };
    let (mut socket, _) =
        connect_async_tls_with_config(relay.as_str(), Some(config), false, connector).await?;
    let id = SubscriptionId::generate();
    let mut request = vec![serde_json::json!("REQ"), serde_json::json!(id)];
    for filter in filters {
        request.push(serde_json::to_value(filter)?);
    }
    let request = serde_json::to_string(&request)?;
    if request.len() > MAX_FRAME_BYTES {
        return Err(ReadLimit("discovery request too large".into()).into());
    }
    socket.send(Message::Text(request.clone())).await?;
    let mut auth_id = None;
    let mut authenticated = false;
    let allow_history = filters.iter().any(requests_message_history);
    let mut received_bytes = 0usize;
    while let Some(message) = socket.next().await {
        let Message::Text(text) = message? else {
            continue;
        };
        received_bytes = received_bytes.saturating_add(text.len());
        let value =
            serde_json::from_str::<serde_json::Value>(&text).unwrap_or(serde_json::Value::Null);
        let Some(parts) = value.as_array() else {
            check_response_budget(received_bytes)?;
            continue;
        };
        let target = parts.get(1).and_then(serde_json::Value::as_str);
        let our_subscription = target == Some(id.as_str());
        let event = if our_subscription
            && parts.first().and_then(serde_json::Value::as_str) == Some("EVENT")
        {
            parts
                .get(2)
                .cloned()
                .and_then(|value| serde_json::from_value::<Event>(value).ok())
                .filter(|event| {
                    event.verify().is_ok()
                        && filters.iter().any(|filter| {
                            filter.match_event(event, nostr::filter::MatchEventOptions::new())
                        })
                })
        } else {
            None
        };
        if allow_history && event.as_ref().is_some_and(is_message_history) {
            received_bytes = received_bytes.saturating_sub(text.len());
        }
        check_response_budget(received_bytes)?;
        match parts.first().and_then(serde_json::Value::as_str) {
            Some("AUTH") => {
                anyhow::ensure!(
                    auth_id.is_none(),
                    "repeated anonymous authentication challenge"
                );
                let challenge = target.ok_or_else(|| anyhow::anyhow!("invalid auth"))?;
                let auth = EventBuilder::auth(challenge, relay.clone())
                    .sign_with_keys(&Keys::generate())?;
                auth_id = Some(auth.id.to_hex());
                socket
                    .send(Message::Text(serde_json::json!(["AUTH", auth]).to_string()))
                    .await?;
            }
            Some("OK") if auth_id.as_deref().is_some_and(|auth| target == Some(auth)) => {
                anyhow::ensure!(
                    parts.get(2).and_then(serde_json::Value::as_bool) == Some(true),
                    "anonymous authentication rejected"
                );
                if !authenticated {
                    authenticated = true;
                    socket.send(Message::Text(request.clone())).await?;
                }
            }
            Some("EVENT") if our_subscription => {
                let Some(event) = event else {
                    continue;
                };
                collected.insert(event).map_err(ReadLimit)?;
            }
            Some("EOSE") if our_subscription => {
                // A challenged relay may answer the first REQ before AUTH.
                if auth_id.is_some() && !authenticated {
                    continue;
                }
                let _ = socket
                    .send(Message::Text(serde_json::json!(["CLOSE", id]).to_string()))
                    .await;
                let _ = socket.close(None).await;
                return Ok(());
            }
            Some("CLOSED") if our_subscription => {
                let reason = parts
                    .get(2)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                anyhow::ensure!(
                    !authenticated && reason.starts_with("auth-required:"),
                    "anonymous discovery closed"
                );
            }
            _ => {}
        }
    }
    anyhow::bail!("anonymous discovery closed before EOSE")
}
