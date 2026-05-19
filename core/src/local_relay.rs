use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration as StdDuration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
struct RelayState {
    events_by_id: BTreeMap<String, Value>,
    subscriptions: HashMap<usize, HashMap<String, Vec<Value>>>,
    clients: HashMap<usize, mpsc::UnboundedSender<Message>>,
    faults: RelayFaults,
    dropped_event_ids: HashSet<String>,
}

#[derive(Clone, Default)]
struct RelayFaults {
    drop_event_ids_file: Option<PathBuf>,
    drop_matching_events_once: bool,
}

impl RelayState {
    fn from_env() -> Self {
        Self {
            faults: RelayFaults::from_env(),
            ..Self::default()
        }
    }

    fn should_drop_event(&mut self, event_id: &str) -> bool {
        let Some(path) = self.faults.drop_event_ids_file.as_ref() else {
            return false;
        };
        if self.faults.drop_matching_events_once && self.dropped_event_ids.contains(event_id) {
            return false;
        }
        if !drop_event_ids(path).contains(event_id) {
            return false;
        }
        self.dropped_event_ids.insert(event_id.to_string());
        true
    }
}

impl RelayFaults {
    fn from_env() -> Self {
        let drop_event_ids_file = std::env::var_os("IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let drop_matching_events_once = !env_flag("IRIS_LOCAL_RELAY_DROP_EVENT_IDS_ALWAYS");
        Self {
            drop_event_ids_file,
            drop_matching_events_once,
        }
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn drop_event_ids(path: &Path) -> HashSet<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    raw.lines()
        .filter_map(|line| line.split('#').next())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn lock_relay_state(state: &Arc<Mutex<RelayState>>) -> MutexGuard<'_, RelayState> {
    state.lock().unwrap_or_else(|poison| poison.into_inner())
}

enum RelayControl {
    ReplayStored,
    Snapshot(std_mpsc::Sender<Vec<Value>>),
    Shutdown,
}

pub struct TestRelay {
    control_tx: mpsc::UnboundedSender<RelayControl>,
    join: Option<thread::JoinHandle<()>>,
    url: String,
}

impl TestRelay {
    pub fn start() -> Self {
        match Self::start_with_bind("127.0.0.1:0") {
            Ok(relay) => relay,
            Err(error) => {
                eprintln!("failed to start local relay: {error}");
                let (control_tx, _) = mpsc::unbounded_channel();
                Self {
                    control_tx,
                    join: None,
                    url: String::new(),
                }
            }
        }
    }

    pub fn start_with_bind(bind_addr: &str) -> Result<Self> {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let bind_addr = bind_addr.to_string();

        let join = thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(anyhow!("relay runtime: {error}")));
                    return;
                }
            };

            runtime.block_on(async move {
                let listener = match TcpListener::bind(&bind_addr)
                    .await
                    .with_context(|| format!("bind relay listener {bind_addr}"))
                {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let local_addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow!("relay local addr: {error}")));
                        return;
                    }
                };
                let state = Arc::new(Mutex::new(RelayState::default()));
                let next_client_id = Arc::new(std::sync::atomic::AtomicUsize::new(1));
                let _ = ready_tx.send(Ok(format!("ws://{local_addr}")));

                loop {
                    tokio::select! {
                        Some(control) = control_rx.recv() => {
                            match control {
                                RelayControl::ReplayStored => replay_stored_events(&state),
                                RelayControl::Snapshot(reply_tx) => {
                                    let events = lock_relay_state(&state)
                                        .events_by_id
                                        .values()
                                        .cloned()
                                        .collect::<Vec<_>>();
                                    let _ = reply_tx.send(events);
                                }
                                RelayControl::Shutdown => break,
                            }
                        }
                        accept_result = listener.accept() => {
                            let Ok((stream, _)) = accept_result else {
                                break;
                            };
                            let websocket = match accept_async(stream).await {
                                Ok(websocket) => websocket,
                                Err(error) => {
                                    eprintln!("Ignoring failed test relay websocket handshake: {error}");
                                    continue;
                                }
                            };
                            let state = state.clone();
                            let client_id = next_client_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            tokio::spawn(async move {
                                handle_connection(client_id, websocket, state).await;
                            });
                        }
                    }
                }
            });
        });

        let url = ready_rx
            .recv_timeout(StdDuration::from_secs(5))
            .context("relay ready")??;

        Ok(Self {
            control_tx,
            join: Some(join),
            url,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn replay_stored(&self) {
        let _ = self.control_tx.send(RelayControl::ReplayStored);
    }

    pub fn events(&self) -> Vec<Value> {
        let (reply_tx, reply_rx) = std_mpsc::channel();
        let _ = self.control_tx.send(RelayControl::Snapshot(reply_tx));
        reply_rx
            .recv_timeout(StdDuration::from_secs(5))
            .unwrap_or_default()
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        let _ = self.control_tx.send(RelayControl::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn run_forever(bind_addr: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("relay runtime")?;
    let bind_addr = bind_addr.to_string();

    runtime.block_on(async move {
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("bind relay listener {bind_addr}"))?;
        let state = Arc::new(Mutex::new(RelayState::from_env()));
        let next_client_id = Arc::new(std::sync::atomic::AtomicUsize::new(1));

        println!("Local Nostr relay listening on ws://{bind_addr}");

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .with_context(|| format!("accept relay client on {bind_addr}"))?;
            let websocket = match accept_async(stream).await {
                Ok(websocket) => websocket,
                Err(error) => {
                    eprintln!("Ignoring failed websocket handshake on {bind_addr}: {error}");
                    continue;
                }
            };
            let state = state.clone();
            let client_id = next_client_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async move {
                handle_connection(client_id, websocket, state).await;
            });
        }
    })
}

async fn handle_connection(
    client_id: usize,
    websocket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    state: Arc<Mutex<RelayState>>,
) {
    let (mut sink, mut stream) = websocket.split();
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<Message>();

    {
        let mut relay = lock_relay_state(&state);
        relay.clients.insert(client_id, client_tx);
    }

    let writer = tokio::spawn(async move {
        while let Some(message) = client_rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = stream.next().await {
        let Ok(message) = message else {
            break;
        };
        match message {
            Message::Text(text) => handle_client_message(client_id, &text, &state),
            Message::Ping(payload) => {
                let sender = {
                    let relay = lock_relay_state(&state);
                    relay.clients.get(&client_id).cloned()
                };
                if let Some(sender) = sender {
                    let _ = sender.send(Message::Pong(payload));
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    {
        let mut relay = lock_relay_state(&state);
        relay.clients.remove(&client_id);
        relay.subscriptions.remove(&client_id);
    }

    writer.abort();
}

fn handle_client_message(client_id: usize, raw_message: &str, state: &Arc<Mutex<RelayState>>) {
    let Ok(message) = serde_json::from_str::<Value>(raw_message) else {
        return;
    };
    let Some(parts) = message.as_array() else {
        return;
    };
    let Some(kind) = parts.first().and_then(Value::as_str) else {
        return;
    };

    match kind {
        "REQ" if parts.len() >= 2 => {
            let Some(subscription_id) = parts.get(1).and_then(Value::as_str) else {
                return;
            };
            let filters: Vec<Value> = parts
                .iter()
                .skip(2)
                .filter(|value| value.is_object())
                .cloned()
                .collect();
            let (sender, events) = {
                let mut relay = lock_relay_state(state);
                relay
                    .subscriptions
                    .entry(client_id)
                    .or_default()
                    .insert(subscription_id.to_string(), filters.clone());
                (
                    relay.clients.get(&client_id).cloned(),
                    relay.events_by_id.values().cloned().collect::<Vec<_>>(),
                )
            };

            if let Some(sender) = sender {
                for event in events {
                    if matches_any_filter(&event, &filters) {
                        let payload =
                            Message::Text(json!(["EVENT", subscription_id, event]).to_string());
                        let _ = sender.send(payload);
                    }
                }
                let _ = sender.send(Message::Text(json!(["EOSE", subscription_id]).to_string()));
            }
        }
        "CLOSE" if parts.len() >= 2 => {
            let Some(subscription_id) = parts.get(1).and_then(Value::as_str) else {
                return;
            };
            let mut relay = lock_relay_state(state);
            if let Some(subscriptions) = relay.subscriptions.get_mut(&client_id) {
                subscriptions.remove(subscription_id);
            }
        }
        "EVENT" if parts.get(1).is_some_and(Value::is_object) => {
            let Some(event) = parts.get(1).cloned() else {
                return;
            };
            let Some(event_id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            let event_id = event_id.to_string();
            let (sender, deliveries, dropped) = {
                let mut relay = lock_relay_state(state);
                let sender = relay.clients.get(&client_id).cloned();
                if relay.should_drop_event(&event_id) {
                    (sender, Vec::new(), true)
                } else {
                    relay.events_by_id.insert(event_id.clone(), event.clone());
                    let deliveries = matching_deliveries(&relay, &event);
                    (sender, deliveries, false)
                }
            };
            if dropped {
                eprintln!("Local relay fault dropped event_id={event_id}");
            }
            if let Some(sender) = sender {
                let message = if dropped {
                    "fault: dropped by local relay"
                } else {
                    ""
                };
                let _ = sender.send(Message::Text(
                    json!(["OK", event_id, true, message]).to_string(),
                ));
            }
            if dropped {
                return;
            }

            for (target, payload) in deliveries {
                let _ = target.send(payload);
            }
        }
        _ => {}
    }
}

fn replay_stored_events(state: &Arc<Mutex<RelayState>>) {
    let deliveries = {
        let relay = lock_relay_state(state);
        relay
            .events_by_id
            .values()
            .flat_map(|event| matching_deliveries(&relay, event))
            .collect::<Vec<_>>()
    };

    for (target, payload) in deliveries {
        let _ = target.send(payload);
    }
}

fn matching_deliveries(
    relay: &RelayState,
    event: &Value,
) -> Vec<(mpsc::UnboundedSender<Message>, Message)> {
    let mut deliveries = Vec::new();
    for (client_id, subscriptions) in &relay.subscriptions {
        let Some(target) = relay.clients.get(client_id).cloned() else {
            continue;
        };
        for (subscription_id, filters) in subscriptions {
            if matches_any_filter(event, filters) {
                deliveries.push((
                    target.clone(),
                    Message::Text(json!(["EVENT", subscription_id, event]).to_string()),
                ));
            }
        }
    }
    deliveries
}

pub fn matches_any_filter(event: &Value, filters: &[Value]) -> bool {
    if filters.is_empty() {
        return true;
    }

    filters.iter().any(|filter| matches_filter(event, filter))
}

pub fn matches_filter(event: &Value, filter: &Value) -> bool {
    let Some(filter_object) = filter.as_object() else {
        return false;
    };

    if let Some(ids) = filter_object.get("ids").and_then(Value::as_array) {
        let Some(event_id) = event.get("id").and_then(Value::as_str) else {
            return false;
        };
        if !ids
            .iter()
            .filter_map(Value::as_str)
            .any(|id| id == event_id)
        {
            return false;
        }
    }

    if let Some(authors) = filter_object.get("authors").and_then(Value::as_array) {
        let Some(pubkey) = event.get("pubkey").and_then(Value::as_str) else {
            return false;
        };
        if !authors
            .iter()
            .filter_map(Value::as_str)
            .any(|author| author == pubkey)
        {
            return false;
        }
    }

    if let Some(kinds) = filter_object.get("kinds").and_then(Value::as_array) {
        let Some(kind) = event.get("kind").and_then(Value::as_u64) else {
            return false;
        };
        if !kinds
            .iter()
            .filter_map(Value::as_u64)
            .any(|value| value == kind)
        {
            return false;
        }
    }

    if let Some(since) = filter_object.get("since").and_then(Value::as_u64) {
        let Some(created_at) = event.get("created_at").and_then(Value::as_u64) else {
            return false;
        };
        if created_at < since {
            return false;
        }
    }

    if let Some(until) = filter_object.get("until").and_then(Value::as_u64) {
        let Some(created_at) = event.get("created_at").and_then(Value::as_u64) else {
            return false;
        };
        if created_at > until {
            return false;
        }
    }

    for (key, value) in filter_object {
        let Some(tag_name) = key.strip_prefix('#') else {
            continue;
        };

        let Some(expected_values) = value.as_array() else {
            return false;
        };
        if expected_values.is_empty() {
            continue;
        }

        let Some(tags) = event.get("tags").and_then(Value::as_array) else {
            return false;
        };
        let matched = tags.iter().any(|tag| {
            let Some(tag_values) = tag.as_array() else {
                return false;
            };
            if tag_values.first().and_then(Value::as_str) != Some(tag_name) {
                return false;
            }
            tag_values
                .iter()
                .skip(1)
                .filter_map(Value::as_str)
                .any(|tag_value| {
                    expected_values
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|expected| expected == tag_value)
                })
        });
        if !matched {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn drop_event_ids_file_ignores_comments_and_blank_lines() {
        let mut file = tempfile::NamedTempFile::new().expect("temp drop file");
        writeln!(file, "\n# comment\nabc\n  def  # inline comment\n").expect("write drop file");

        let ids = drop_event_ids(&file.path().to_path_buf());

        assert!(ids.contains("abc"));
        assert!(ids.contains("def"));
        assert!(!ids.contains("# comment"));
    }

    #[test]
    fn relay_fault_drops_matching_event_once_by_default() {
        let mut file = tempfile::NamedTempFile::new().expect("temp drop file");
        writeln!(file, "event-to-drop").expect("write drop file");
        let mut state = RelayState {
            faults: RelayFaults {
                drop_event_ids_file: Some(file.path().to_path_buf()),
                drop_matching_events_once: true,
            },
            ..RelayState::default()
        };

        assert!(state.should_drop_event("event-to-drop"));
        assert!(!state.should_drop_event("event-to-drop"));
        assert!(!state.should_drop_event("different-event"));
    }

    #[test]
    fn relay_fault_can_drop_matching_event_every_time() {
        let mut file = tempfile::NamedTempFile::new().expect("temp drop file");
        writeln!(file, "event-to-drop").expect("write drop file");
        let mut state = RelayState {
            faults: RelayFaults {
                drop_event_ids_file: Some(file.path().to_path_buf()),
                drop_matching_events_once: false,
            },
            ..RelayState::default()
        };

        assert!(state.should_drop_event("event-to-drop"));
        assert!(state.should_drop_event("event-to-drop"));
    }
}
