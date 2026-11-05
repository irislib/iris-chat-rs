use super::*;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
    Mutex,
};
use tokio_tungstenite::tungstenite::Message;

pub(super) struct Fixture {
    pub(super) url: RelayUrl,
    pub(super) wire: Arc<Mutex<Vec<serde_json::Value>>>,
    pub(super) mode: Arc<AtomicUsize>,
    pub(super) hold_eose: Arc<AtomicBool>,
    pub(super) connections: Arc<AtomicUsize>,
    pub(super) active: Arc<AtomicUsize>,
    pub(super) omitted: Arc<Mutex<HashSet<EventId>>>,
    pub(super) raw: Arc<Mutex<Vec<Message>>>,
    pub(super) reject_auth: Arc<AtomicBool>,
    pub(super) hold_handshake: Arc<AtomicBool>,
    pub(super) release_handshake: Arc<tokio::sync::Notify>,
    pub(super) disconnect_once: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, AtomicOrdering::SeqCst);
    }
}

fn repeated_fingerprint() -> String {
    // NIP-77 v1, infinity bound with zero ID prefix, Fingerprint mode, an
    // intentionally mismatching 16-byte fingerprint. It elicits another round.
    let mut bytes = vec![0x61, 0, 0, 1];
    bytes.extend([0; 16]);
    bytes.to_lower_hex_string()
}

pub(super) async fn fixture(events: Vec<Event>) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let wire = Arc::new(Mutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0));
    let hold_eose = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let omitted = Arc::new(Mutex::new(HashSet::new()));
    let raw = Arc::new(Mutex::new(Vec::new()));
    let reject_auth = Arc::new(AtomicBool::new(false));
    let hold_handshake = Arc::new(AtomicBool::new(false));
    let release_handshake = Arc::new(tokio::sync::Notify::new());
    let disconnect_once = Arc::new(AtomicBool::new(false));
    let task = {
        let (
            wire,
            mode,
            hold_eose,
            connections,
            active,
            omitted,
            raw,
            reject_auth,
            hold_handshake,
            release_handshake,
            disconnect_once,
        ) = (
            wire.clone(),
            mode.clone(),
            hold_eose.clone(),
            connections.clone(),
            active.clone(),
            omitted.clone(),
            raw.clone(),
            reject_auth.clone(),
            hold_handshake.clone(),
            release_handshake.clone(),
            disconnect_once.clone(),
        );
        tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let Ok((socket, _)) = connection else { break; };
                        connections.fetch_add(1, AtomicOrdering::SeqCst);
                        active.fetch_add(1, AtomicOrdering::SeqCst);
                        let guard = Active(active.clone());
                        let (wire, mode, hold_eose, omitted, raw, reject_auth, hold_handshake, release_handshake, disconnect_once, events) =
                            (wire.clone(), mode.clone(), hold_eose.clone(), omitted.clone(), raw.clone(), reject_auth.clone(), hold_handshake.clone(), release_handshake.clone(), disconnect_once.clone(), events.clone());
                        children.spawn(async move {
                            let _guard=guard;
                            while hold_handshake.load(Ordering::Acquire) { release_handshake.notified().await; }
                            let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else { return; };
                            let mut negotiations = HashMap::new();
                            if socket.send(Message::Text(serde_json::json!(["AUTH", "public-graph-only"]).to_string())).await.is_err() { return; }
                            while let Some(Ok(message)) = socket.next().await {
                                let text = match message {
                                    Message::Text(text) => text,
                                    Message::Ping(data) => { let _=socket.send(Message::Pong(data)).await; continue; }
                                    Message::Close(_) => break,
                                    _ => continue,
                                };
                                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                                wire.lock().unwrap().push(frame.clone());
                                if frame[0] == "NEG-OPEN" && disconnect_once.swap(false, Ordering::AcqRel) {
                                    let _=socket.close(None).await; return;
                                }
                                let response = match frame[0].as_str().unwrap() {
                                    "AUTH" => Some(serde_json::json!(["OK", frame[1]["id"], !reject_auth.load(Ordering::Acquire), "fixture auth policy"])),
                                    "NEG-OPEN" => {
                                        let id=frame[1].as_str().unwrap().to_owned();
                                        if reject_auth.load(Ordering::Acquire) { Some(serde_json::json!(["NEG-ERR", id, "blocked: authentication rejected"])) }
                                        else { match mode.load(AtomicOrdering::SeqCst) {
                                            1 => Some(serde_json::json!(["NEG-ERR", id, "unsupported"])),
                                            2 => None,
                                            3 => Some(serde_json::json!(["NEG-MSG", id, "00".repeat(8193)])),
                                            4 => Some(serde_json::json!(["NEG-MSG", id, repeated_fingerprint()])),
                                            _ => {
                                                let filter: Filter=serde_json::from_value(frame[2].clone()).unwrap();
                                                let mut storage=NegentropyStorageVector::new();
                                                for event in events.iter().filter(|event|filter.match_event(event,nostr::filter::MatchEventOptions::new())) {
                                                    storage.insert(event.created_at.as_secs(),Id::from_slice(event.id.as_bytes()).unwrap()).unwrap();
                                                }
                                                storage.seal().unwrap();
                                                let mut neg=Negentropy::owned(storage,8*1024).unwrap();
                                                let reply=neg.reconcile(&Vec::<u8>::from_hex(frame[3].as_str().unwrap()).unwrap()).unwrap();
                                                negotiations.insert(id.clone(),neg);
                                                Some(serde_json::json!(["NEG-MSG",id,reply.to_lower_hex_string()]))
                                            }
                                        }}
                                    }
                                    "NEG-MSG" => {
                                        let id=frame[1].as_str().unwrap();
                                        if mode.load(AtomicOrdering::SeqCst)==4 { Some(serde_json::json!(["NEG-MSG",id,repeated_fingerprint()])) }
                                        else {
                                            let reply=negotiations.get_mut(id).unwrap().reconcile(&Vec::<u8>::from_hex(frame[2].as_str().unwrap()).unwrap()).unwrap();
                                            Some(serde_json::json!(["NEG-MSG",id,reply.to_lower_hex_string()]))
                                        }
                                    }
                                    "NEG-CLOSE" => { negotiations.remove(frame[1].as_str().unwrap()); None }
                                    "REQ" => {
                                        if reject_auth.load(Ordering::Acquire) { Some(serde_json::json!(["CLOSED",frame[1],"blocked: authentication rejected"])) }
                                        else {
                                            let extra=std::mem::take(&mut *raw.lock().unwrap());
                                            for message in extra {
                                                let id=frame[1].as_str().unwrap();
                                                let message=match message {
                                                    Message::Text(text)=>Message::Text(text.replace("__graph_req__",id)),
                                                    Message::Binary(bytes)=>Message::Binary(String::from_utf8(bytes).unwrap().replace("__graph_req__",id).into_bytes()),
                                                    message=>message,
                                                };
                                                if socket.send(message).await.is_err(){return;}
                                            }
                                            let filters=frame.as_array().unwrap()[2..].iter().map(|filter|serde_json::from_value::<Filter>(filter.clone()).unwrap()).collect::<Vec<_>>();
                                            let omitted=omitted.lock().unwrap().clone();
                                            for event in events.iter().filter(|event|!omitted.contains(&event.id)&&filters.iter().any(|filter|filter.match_event(event,nostr::filter::MatchEventOptions::new()))) {
                                                if socket.send(Message::Text(serde_json::json!(["EVENT",frame[1],event]).to_string())).await.is_err(){return;}
                                            }
                                            (!hold_eose.load(Ordering::Acquire)).then(||serde_json::json!(["EOSE",frame[1]]))
                                        }
                                    }
                                    _ => None,
                                };
                                if let Some(response)=response { if socket.send(Message::Text(response.to_string())).await.is_err(){break;} }
                            }
                        });
                    }
                    _ = children.join_next(), if !children.is_empty() => {}
                }
            }
        })
    };
    Fixture {
        url,
        wire,
        mode,
        hold_eose,
        connections,
        active,
        omitted,
        raw,
        reject_auth,
        hold_handshake,
        release_handshake,
        disconnect_once,
        task,
    }
}

pub(super) async fn until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
pub(super) fn event(keys: &Keys, kind: Kind, content: &str) -> Event {
    EventBuilder::new(kind, content)
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(keys)
        .unwrap()
}
pub(super) fn count(fixture: &Fixture, command: &str) -> usize {
    fixture
        .wire
        .lock()
        .unwrap()
        .iter()
        .filter(|frame| frame[0] == command)
        .count()
}
pub(super) fn assert_closed(fixture: &Fixture, open: &str, close: &str) {
    let frames = fixture.wire.lock().unwrap();
    for frame in frames.iter().filter(|frame| frame[0] == open) {
        assert!(
            frames
                .iter()
                .any(|closed| closed[0] == close && closed[1] == frame[1]),
            "unclosed {frame}"
        );
    }
}
