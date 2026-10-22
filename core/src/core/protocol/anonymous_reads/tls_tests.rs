//! Real TLS and serialized Nostr checks. These certificates and the private
//! key are public test fixtures, never production credentials or trust roots.
use super::*;
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    server::{ServerSessionMemoryCache, StoresServerSessions},
    HandshakeKind, ProtocolVersion, RootCertStore, ServerConfig,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub(in crate::core) fn certificates() -> Vec<Vec<u8>> {
    vec![include_bytes!("test_fixtures/ca.der").to_vec()]
}

pub(in crate::core) fn roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    for certificate in certificates() {
        roots.add(CertificateDer::from(certificate)).unwrap();
    }
    roots
}

pub(in crate::core) fn acceptor(resume: bool, tls12_only: bool) -> TlsAcceptor {
    acceptor_with_sessions(resume, tls12_only, ServerSessionMemoryCache::new(256))
}

pub(in crate::core) fn acceptor_with_sessions(
    resume: bool,
    tls12_only: bool,
    sessions: Arc<dyn StoresServerSessions>,
) -> TlsAcceptor {
    let versions = if tls12_only {
        vec![&rustls::version::TLS12]
    } else {
        vec![&rustls::version::TLS13]
    };
    let mut config = ServerConfig::builder_with_protocol_versions(&versions)
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(
                include_bytes!("test_fixtures/server.der").to_vec(),
            )],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                include_bytes!("test_fixtures/server-test-only-key.der").to_vec(),
            )),
        )
        .unwrap();
    config.send_tls13_tickets = if resume { 2 } else { 0 };
    config.session_storage = sessions;
    TlsAcceptor::from(Arc::new(config))
}

pub(in crate::core) type Wire = Arc<Mutex<Vec<(usize, serde_json::Value)>>>;
pub(in crate::core) type Handshakes = Arc<Mutex<Vec<(HandshakeKind, ProtocolVersion, bool)>>>;

struct RecordHost(Arc<Mutex<Vec<String>>>);

impl tokio_tungstenite::tungstenite::handshake::server::Callback for RecordHost {
    fn on_request(
        self,
        request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        self.0
            .lock()
            .unwrap()
            .push(request.headers()["host"].to_str().unwrap().to_owned());
        Ok(response)
    }
}

pub(in crate::core) struct Relay {
    pub(in crate::core) url: RelayUrl,
    pub(in crate::core) wire: Wire,
    pub(in crate::core) handshakes: Handshakes,
    pub(in crate::core) server_names: Arc<Mutex<Vec<Option<String>>>>,
    pub(in crate::core) request_hosts: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(in crate::core) async fn relay(resume: bool, tls12_only: bool, events: Vec<Event>) -> Relay {
    relay_with_acceptor(acceptor(resume, tls12_only), events).await
}

async fn relay_with_acceptor(acceptor: TlsAcceptor, events: Vec<Event>) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
    let wire: Wire = Arc::default();
    let handshakes: Handshakes = Arc::default();
    let server_names = Arc::new(Mutex::new(Vec::new()));
    let request_hosts = Arc::new(Mutex::new(Vec::new()));
    let recorded = wire.clone();
    let observed = handshakes.clone();
    let names = server_names.clone();
    let hosts = request_hosts.clone();
    let events = Arc::new(events);
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        let mut id = 0;
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap();
                    id += 1;
                    let acceptor = acceptor.clone();
                    let wire = recorded.clone();
                    let handshakes = observed.clone();
                    let names = names.clone();
                    let hosts = hosts.clone();
                    let events = events.clone();
                    children.spawn(async move {
                        // Rejected certificates are an expected negative case.
                        let Ok(stream) = acceptor.accept(stream).await else { return; };
                        let connection = stream.get_ref().1;
                        names.lock().unwrap().push(connection.server_name().map(str::to_owned));
                        handshakes.lock().unwrap().push((
                            connection.handshake_kind().unwrap(),
                            connection.protocol_version().unwrap(),
                            connection.negotiated_key_exchange_group().is_some(),
                        ));
                        let mut socket = tokio_tungstenite::accept_hdr_async(stream, RecordHost(hosts)).await.unwrap();
                        socket.send(Message::Text(serde_json::json!(["AUTH", format!("tls-fixture-{id}")]).to_string())).await.unwrap();
                        let mut authenticated = false;
                        while let Some(Ok(Message::Text(text))) = socket.next().await {
                            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                            wire.lock().unwrap().push((id, value.clone()));
                            match value[0].as_str() {
                                Some("AUTH") => {
                                    let auth: Event = serde_json::from_value(value[1].clone()).unwrap();
                                    auth.verify().unwrap();
                                    assert_eq!(auth.kind, Kind::Authentication);
                                    authenticated = true;
                                    socket.send(Message::Text(serde_json::json!(["OK", auth.id, true, ""]).to_string())).await.unwrap();
                                }
                                Some("REQ") if authenticated => {
                                    let filters = value.as_array().unwrap()[2..].iter().cloned()
                                        .map(serde_json::from_value::<Filter>).collect::<Result<Vec<_>, _>>().unwrap();
                                    for event in events.iter().filter(|event| filters.iter().any(|filter| filter.match_event(event, nostr::filter::MatchEventOptions::new()))) {
                                        socket.send(Message::Text(serde_json::json!(["EVENT", value[1], event]).to_string())).await.unwrap();
                                    }
                                    socket.send(Message::Text(serde_json::json!(["EOSE", value[1]]).to_string())).await.unwrap();
                                }
                                Some("CLOSE") => break,
                                _ => {},
                            }
                        }
                    });
                },
                child = children.join_next(), if !children.is_empty() => {
                    child.unwrap().expect("TLS relay fixture child failed");
                }
            }
        }
    });
    Relay {
        url,
        wire,
        handshakes,
        server_names,
        request_hosts,
        task,
    }
}

fn filter(author: PublicKey) -> Filter {
    Filter::new().author(author).kind(Kind::Metadata)
}

async fn read(relay: &Relay, keys: &Keys, scope: &ScopedTlsConnector) {
    let events = poll(&relay.url, vec![filter(keys.public_key())], Some(scope))
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].pubkey, keys.public_key());
    assert!(!relay.task.is_finished(), "TLS fixture failed");
}

pub(in crate::core) fn assert_wire(relay: &Relay, authors: &[PublicKey], connections: usize) {
    let wire = relay.wire.lock().unwrap();
    let mut signers = HashSet::new();
    let mut subjects = std::collections::BTreeMap::<usize, HashSet<String>>::new();
    for (id, value) in wire.iter() {
        match value[0].as_str() {
            Some("AUTH") => {
                let event: Event = serde_json::from_value(value[1].clone()).unwrap();
                event.verify().unwrap();
                assert!(!authors.contains(&event.pubkey));
                assert!(
                    signers.insert(event.pubkey),
                    "TLS resumption must not reuse Nostr AUTH keys"
                );
            }
            Some("REQ") => {
                for filter in &value.as_array().unwrap()[2..] {
                    assert!(filter.get("#p").is_none());
                    let selected = filter["authors"].as_array().unwrap();
                    assert_eq!(selected.len(), 1);
                    let author = selected[0].as_str().unwrap().to_owned();
                    assert!(authors.iter().any(|key| key.to_hex() == author));
                    subjects.entry(*id).or_default().insert(author);
                }
            }
            Some("EVENT") => panic!("read scope published an event"),
            _ => {}
        }
    }
    assert_eq!(signers.len(), connections);
    assert_eq!(subjects.len(), connections);
    assert!(subjects.values().all(|subjects| subjects.len() == 1));
}

#[tokio::test]
async fn tls_wire_resumes_only_the_same_author_and_relay_with_fresh_anonymous_auth() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let events = [&alice, &bob]
        .iter()
        .map(|key| {
            EventBuilder::new(Kind::Metadata, "{}")
                .sign_with_keys(key)
                .unwrap()
        })
        .collect();
    let relay = relay(true, false, events).await;
    let alice_scope =
        ScopedTlsConnector::new_with_roots(&relay.url, &[filter(alice.public_key())], roots())
            .unwrap();
    read(&relay, &alice, &alice_scope).await;
    read(&relay, &alice, &alice_scope).await;
    let bob_scope =
        ScopedTlsConnector::new_with_roots(&relay.url, &[filter(bob.public_key())], roots())
            .unwrap();
    read(&relay, &bob, &bob_scope).await;
    let replacement =
        ScopedTlsConnector::new_with_roots(&relay.url, &[filter(alice.public_key())], roots())
            .unwrap();
    read(&relay, &alice, &replacement).await;
    let mut other_url = relay.url.as_str().trim_end_matches('/').to_owned();
    other_url.push_str("/other-discovery");
    let other_url = RelayUrl::parse(&other_url).unwrap();
    let other_scope =
        ScopedTlsConnector::new_with_roots(&other_url, &[filter(alice.public_key())], roots())
            .unwrap();
    assert_eq!(
        poll(
            &other_url,
            vec![filter(alice.public_key())],
            Some(&other_scope)
        )
        .await
        .unwrap()
        .len(),
        1
    );
    let kinds = relay.handshakes.lock().unwrap().clone();
    assert_eq!(
        kinds.iter().map(|row| row.0).collect::<Vec<_>>(),
        [
            HandshakeKind::Full,
            HandshakeKind::Resumed,
            HandshakeKind::Full,
            HandshakeKind::Full,
            HandshakeKind::Full
        ]
    );
    assert!(kinds
        .iter()
        .all(|(_, version, fresh_dhe)| *version == ProtocolVersion::TLSv1_3 && *fresh_dhe));
    assert_wire(&relay, &[alice.public_key(), bob.public_key()], 5);
    // Normal production roots must still reject this test CA.
    let untrusted = ScopedTlsConnector::new(&relay.url, &[filter(alice.public_key())]).unwrap();
    assert!(poll(
        &relay.url,
        vec![filter(alice.public_key())],
        Some(&untrusted)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn tls_wire_preserves_full_validation_when_resumption_is_unavailable_or_tls12() {
    for tls12_only in [false, true] {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Metadata, "{}")
            .sign_with_keys(&keys)
            .unwrap();
        let relay = relay(false, tls12_only, vec![event]).await;
        let scope =
            ScopedTlsConnector::new_with_roots(&relay.url, &[filter(keys.public_key())], roots())
                .unwrap();
        read(&relay, &keys, &scope).await;
        read(&relay, &keys, &scope).await;
        let kinds = relay.handshakes.lock().unwrap().clone();
        assert_eq!(kinds.len(), 2);
        assert!(
            kinds
                .iter()
                .all(|(kind, _, fresh_dhe)| *kind == HandshakeKind::Full && *fresh_dhe),
            "tls12_only={tls12_only}, observed handshakes: {kinds:?}"
        );
        assert_wire(&relay, &[keys.public_key()], 2);
    }
}

#[derive(Debug)]
struct CountedSessions {
    storage: Arc<dyn StoresServerSessions>,
    inserted: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl CountedSessions {
    fn new(capacity: usize) -> Self {
        Self {
            storage: ServerSessionMemoryCache::new(capacity),
            inserted: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }
}

impl StoresServerSessions for CountedSessions {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> bool {
        let stored = self.storage.put(key, value);
        if stored {
            self.inserted.fetch_add(1, Ordering::SeqCst);
        }
        stored
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.storage.get(key)
    }

    fn take(&self, key: &[u8]) -> Option<Vec<u8>> {
        let value = self.storage.take(key);
        if value.is_some() {
            self.hits.fetch_add(1, Ordering::SeqCst);
        } else {
            self.misses.fetch_add(1, Ordering::SeqCst);
        }
        value
    }

    fn can_cache(&self) -> bool {
        self.storage.can_cache()
    }
}

#[tokio::test]
async fn tls_wire_distinguishes_server_ticket_eviction_from_client_scope_reset() {
    let keys = [Keys::generate(), Keys::generate(), Keys::generate()];
    let authors = keys.iter().map(Keys::public_key).collect::<Vec<_>>();
    for (capacity, expected_second, hits, misses) in [
        (4, HandshakeKind::Full, 0, 3),
        (12, HandshakeKind::Resumed, 3, 0),
    ] {
        let sessions = Arc::new(CountedSessions::new(capacity));
        let events = keys
            .iter()
            .map(|key| {
                EventBuilder::new(Kind::Metadata, "{}")
                    .sign_with_keys(key)
                    .unwrap()
            })
            .collect();
        let relay = relay_with_acceptor(
            acceptor_with_sessions(true, false, sessions.clone()),
            events,
        )
        .await;
        // Retain the same exact author/URL scopes across both ordered sweeps.
        // Two tickets per handshake overflow four server records before the
        // first author returns; its new tickets then evict the next author's.
        let scopes = authors
            .iter()
            .map(|author| {
                ScopedTlsConnector::new_with_roots(&relay.url, &[filter(*author)], roots()).unwrap()
            })
            .collect::<Vec<_>>();
        for _ in 0..2 {
            for (key, scope) in keys.iter().zip(&scopes) {
                read(&relay, key, scope).await;
            }
        }
        let observed = relay.handshakes.lock().unwrap().clone();
        let expected = [HandshakeKind::Full; 3]
            .into_iter()
            .chain([expected_second; 3])
            .collect::<Vec<_>>();
        assert_eq!(
            observed.iter().map(|row| row.0).collect::<Vec<_>>(),
            expected
        );
        assert!(observed
            .iter()
            .all(|(_, version, fresh_dhe)| *version == ProtocolVersion::TLSv1_3 && *fresh_dhe));
        assert_eq!(sessions.inserted.load(Ordering::SeqCst), 12);
        // A lookup miss proves that the client offered a retained ticket; a
        // recreated client scope would not offer a ticket or call take at all.
        assert_eq!(sessions.hits.load(Ordering::SeqCst), hits);
        assert_eq!(sessions.misses.load(Ordering::SeqCst), misses);
        assert_wire(&relay, &authors, 6);
        println!("TLS_SERVER_SESSION_CAPACITY capacity={capacity} second_sweep={expected_second:?} hits={hits} misses={misses} fresh_auth=6 sole_author_requests=6");
    }
}
