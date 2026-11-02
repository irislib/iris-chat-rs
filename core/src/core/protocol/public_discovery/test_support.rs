//! Real TLS/serialized Nostr fixture. Trust is local to the test pool; production
//! certificate verification and transport options are unchanged.
use super::*;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use nostr_sdk::pool::transport::{
    error::TransportError,
    websocket::{WebSocketSink, WebSocketStream, WebSocketTransport},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio_tungstenite::{tungstenite::Message, Connector};

#[derive(Debug)]
struct TestTransport;
impl WebSocketTransport for TestTransport {
    fn support_ping(&self) -> bool {
        true
    }
    fn connect<'a>(
        &'a self,
        url: &'a nostr::Url,
        _mode: &'a nostr_sdk::pool::ConnectionMode,
        timeout: Duration,
    ) -> nostr::util::BoxedFuture<'a, Result<(WebSocketSink, WebSocketStream), TransportError>>
    {
        Box::pin(async move {
            assert_eq!(
                url.host_str(),
                Some("127.0.0.1"),
                "fixture must remain local"
            );
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(super::super::anonymous_reads::tls_tests::roots())
                .with_no_client_auth();
            let (socket, _) = tokio::time::timeout(
                timeout,
                tokio_tungstenite::connect_async_tls_with_config(
                    url.as_str(),
                    None,
                    false,
                    Some(Connector::Rustls(Arc::new(config))),
                ),
            )
            .await
            .map_err(TransportError::backend)?
            .map_err(TransportError::backend)?;
            let (send, receive) = socket.split();
            let sink: WebSocketSink = Box::new(send.sink_map_err(TransportError::backend).with(
                |message: async_wsocket::Message| {
                    futures_util::future::ready(Ok::<_, TransportError>(match message {
                        async_wsocket::Message::Text(value) => Message::Text(value),
                        async_wsocket::Message::Binary(value) => Message::Binary(value),
                        async_wsocket::Message::Ping(value) => Message::Ping(value),
                        async_wsocket::Message::Pong(value) => Message::Pong(value),
                        async_wsocket::Message::Close(_) => Message::Close(None),
                    }))
                },
            ));
            let stream: WebSocketStream = Box::pin(
                receive
                    .map_ok(|message| match message {
                        Message::Text(value) => async_wsocket::Message::Text(value),
                        Message::Binary(value) => async_wsocket::Message::Binary(value),
                        Message::Ping(value) => async_wsocket::Message::Ping(value),
                        Message::Pong(value) => async_wsocket::Message::Pong(value),
                        Message::Close(_) => async_wsocket::Message::Close(None),
                        Message::Frame(_) => panic!("raw frames cannot be received"),
                    })
                    .map_err(TransportError::backend),
            );
            Ok((sink, stream))
        })
    }
}

pub(in crate::core) fn pool() -> Arc<DiscoveryRelays> {
    Arc::new(DiscoveryRelays {
        client: Client::builder()
            .signer(Keys::generate())
            .websocket_transport(TestTransport)
            .build(),
        users: Default::default(),
        stopped: AtomicBool::new(false),
    })
}

#[derive(Clone)]
enum Command {
    Event(Event),
    Disconnect,
}

pub(in crate::core) struct Relay {
    pub url: RelayUrl,
    pub wire: Arc<Mutex<Vec<(usize, serde_json::Value)>>>,
    pub connections: Arc<AtomicUsize>,
    pub active: Arc<AtomicUsize>,
    pub events: Arc<Mutex<Vec<Event>>>,
    commands: tokio::sync::broadcast::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Relay {
    pub fn publish(&self, event: Event) {
        self.events.lock().unwrap().push(event.clone());
        let _ = self.commands.send(Command::Event(event));
    }
    pub fn disconnect(&self) {
        let _ = self.commands.send(Command::Disconnect);
    }
    pub fn requests(&self) -> usize {
        self.wire
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, row)| row[0] == "REQ")
            .count()
    }
}

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(in crate::core) async fn relay(events: Vec<Event>) -> Relay {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
    let wire = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(events));
    let (commands, _) = tokio::sync::broadcast::channel(1024);
    let task = {
        let wire = wire.clone();
        let count = connections.clone();
        let active = active.clone();
        let events = events.clone();
        let commands = commands.clone();
        tokio::spawn(async move {
            let acceptor = super::super::anonymous_reads::tls_tests::acceptor(true, false);
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        let id = count.fetch_add(1, Ordering::SeqCst);
                        active.fetch_add(1, Ordering::SeqCst);
                        let guard = Active(active.clone());
                        let acceptor = acceptor.clone(); let wire = wire.clone(); let events = events.clone();
                        let mut commands = commands.subscribe();
                        children.spawn(async move {
                            let _active = guard;
                            let Ok(stream) = acceptor.accept(stream).await else { return; };
                            let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return; };
                            if socket.send(Message::Text(serde_json::json!(["AUTH", format!("fixture-{id}")]).to_string())).await.is_err() { return; }
                            let mut authenticated = false;
                            let mut subscriptions = BTreeMap::<String, Vec<Filter>>::new();
                            loop {
                                let frames = tokio::select! {
                                    message = socket.next() => {
                                        let Some(Ok(message)) = message else { return; };
                                        let Message::Text(text) = message else {
                                            if let Message::Ping(payload) = message { if socket.send(Message::Pong(payload)).await.is_err() { return; } }
                                            continue;
                                        };
                                        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                                        wire.lock().unwrap().push((id, frame.clone()));
                                        match frame[0].as_str() {
                                            Some("AUTH") => {
                                                let event: Event = serde_json::from_value(frame[1].clone()).unwrap();
                                                event.verify().unwrap(); assert_eq!(event.kind, Kind::Authentication);
                                                authenticated = true;
                                                vec![serde_json::json!(["OK", event.id, true, ""])]
                                            }
                                            Some("REQ") if !authenticated => vec![serde_json::json!(["CLOSED", frame[1], "auth-required: fixture"])],
                                            Some("REQ") => {
                                                let filters = frame.as_array().unwrap()[2..].iter().cloned().map(serde_json::from_value).collect::<Result<Vec<Filter>, _>>().unwrap();
                                                let mut frames = events.lock().unwrap().iter().filter(|event| filters.iter().any(|filter|
                                                    filter.match_event(event, nostr::filter::MatchEventOptions::new())))
                                                    .map(|event| serde_json::json!(["EVENT", frame[1], event])).collect::<Vec<_>>();
                                                subscriptions.insert(frame[1].as_str().unwrap().into(), filters);
                                                frames.push(serde_json::json!(["EOSE", frame[1]])); frames
                                            }
                                            Some("CLOSE") => { subscriptions.remove(frame[1].as_str().unwrap()); vec![] },
                                            Some("EVENT") => panic!("discovery client published an event"),
                                            _ => vec![],
                                        }
                                    }
                                    command = commands.recv() => match command {
                                        Ok(Command::Event(event)) => subscriptions.iter().filter(|(_, filters)| filters.iter().any(|filter|
                                            filter.match_event(&event, nostr::filter::MatchEventOptions::new())))
                                            .map(|(id, _)| serde_json::json!(["EVENT", id, event])).collect(),
                                        Ok(Command::Disconnect) => return,
                                        Err(_) => return,
                                    },
                                };
                                for frame in frames { if socket.send(Message::Text(frame.to_string())).await.is_err() { return; } }
                            }
                        });
                    }
                    joined = children.join_next(), if !children.is_empty() => { joined.unwrap().unwrap(); }
                }
            }
        })
    };
    Relay {
        url,
        wire,
        connections,
        active,
        events,
        commands,
        task,
    }
}

pub(in crate::core) async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !condition() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("discovery fixture readiness deadline");
}
