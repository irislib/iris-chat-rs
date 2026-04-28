use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
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
}

enum RelayControl {
    ReplayStored,
    Shutdown,
}

pub struct TestRelay {
    control_tx: mpsc::UnboundedSender<RelayControl>,
    join: Option<thread::JoinHandle<()>>,
}

impl TestRelay {
    pub fn start() -> Self {
        Self::start_with_bind("127.0.0.1:4848").expect("start relay")
    }

    pub fn start_with_bind(bind_addr: &str) -> Result<Self> {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let bind_addr = bind_addr.to_string();

        let join = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("relay runtime");

            runtime.block_on(async move {
                let listener = TcpListener::bind(&bind_addr)
                    .await
                    .with_context(|| format!("bind relay listener {bind_addr}"))
                    .expect("bind relay listener");
                let state = Arc::new(Mutex::new(RelayState::default()));
                let next_client_id = Arc::new(std::sync::atomic::AtomicUsize::new(1));
                ready_tx.send(()).expect("signal relay ready");

                loop {
                    tokio::select! {
                        Some(control) = control_rx.recv() => {
                            match control {
                                RelayControl::ReplayStored => replay_stored_events(&state),
                                RelayControl::Shutdown => break,
                            }
                        }
                        accept_result = listener.accept() => {
                            let (stream, _) = accept_result.expect("accept relay client");
                            let websocket = accept_async(stream).await.expect("accept websocket");
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

        ready_rx
            .recv_timeout(StdDuration::from_secs(5))
            .context("relay ready")?;

        Ok(Self {
            control_tx,
            join: Some(join),
        })
    }

    pub fn replay_stored(&self) {
        let _ = self.control_tx.send(RelayControl::ReplayStored);
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
        let state = Arc::new(Mutex::new(RelayState::default()));
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
        let mut relay = state.lock().expect("relay state lock");
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
                    let relay = state.lock().expect("relay state lock");
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
        let mut relay = state.lock().expect("relay state lock");
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
            let Some(subscription_id) = parts[1].as_str() else {
                return;
            };
            let filters: Vec<Value> = parts
                .iter()
                .skip(2)
                .filter(|value| value.is_object())
                .cloned()
                .collect();
            let (sender, events) = {
                let mut relay = state.lock().expect("relay state lock");
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
            let Some(subscription_id) = parts[1].as_str() else {
                return;
            };
            let mut relay = state.lock().expect("relay state lock");
            if let Some(subscriptions) = relay.subscriptions.get_mut(&client_id) {
                subscriptions.remove(subscription_id);
            }
        }
        "EVENT" if parts.len() >= 2 && parts[1].is_object() => {
            let event = parts[1].clone();
            let Some(event_id) = event.get("id").and_then(Value::as_str) else {
                return;
            };
            let (sender, deliveries) = {
                let mut relay = state.lock().expect("relay state lock");
                relay
                    .events_by_id
                    .insert(event_id.to_string(), event.clone());
                let sender = relay.clients.get(&client_id).cloned();
                let deliveries = matching_deliveries(&relay, &event);
                (sender, deliveries)
            };
            if let Some(sender) = sender {
                let _ = sender.send(Message::Text(json!(["OK", event_id, true, ""]).to_string()));
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
        let relay = state.lock().expect("relay state lock");
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
