//! Real TLS and serialized Nostr checks. These certificates and the private
//! key are public test fixtures, never production credentials or trust roots.
use super::*;
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    HandshakeKind, ProtocolVersion, RootCertStore, ServerConfig,
};
use std::sync::Mutex;
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
    TlsAcceptor::from(Arc::new(config))
}

pub(in crate::core) type Wire = Arc<Mutex<Vec<(usize, serde_json::Value)>>>;
pub(in crate::core) type Handshakes = Arc<Mutex<Vec<(HandshakeKind, ProtocolVersion, bool)>>>;

pub(in crate::core) struct Relay {
    pub(in crate::core) url: RelayUrl,
    pub(in crate::core) wire: Wire,
    pub(in crate::core) handshakes: Handshakes,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(in crate::core) async fn relay(resume: bool, tls12_only: bool, events: Vec<Event>) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = RelayUrl::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
    let acceptor = acceptor(resume, tls12_only);
    let wire: Wire = Arc::default();
    let handshakes: Handshakes = Arc::default();
    let recorded = wire.clone();
    let observed = handshakes.clone();
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
                    let events = events.clone();
                    children.spawn(async move {
                        // Rejected certificates are an expected negative case.
                        let Ok(stream) = acceptor.accept(stream).await else { return; };
                        let connection = stream.get_ref().1;
                        handshakes.lock().unwrap().push((
                            connection.handshake_kind().unwrap(),
                            connection.protocol_version().unwrap(),
                            connection.negotiated_key_exchange_group().is_some(),
                        ));
                        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
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
