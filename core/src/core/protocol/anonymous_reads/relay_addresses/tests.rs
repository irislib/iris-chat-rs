use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

fn server(port: u16) -> Server {
    Server {
        host: "relay.invalid".into(),
        port,
    }
}

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("DNS fixture timed out")
}

async fn expire(lookup: &Lookup) {
    lookup
        .slot
        .as_ref()
        .unwrap()
        .lock()
        .await
        .as_mut()
        .unwrap()
        .resolved_at -= TTL;
}

#[tokio::test]
async fn dns_poisoned_cache_discards_answers_and_recovers() {
    let cache = AddressCache::default();
    let key = server(443);
    let old = cache
        .resolve(&key, &|_| std::future::ready(Ok(vec![address(1)])))
        .await
        .unwrap();
    assert!(std::panic::catch_unwind(|| {
        let _guard = cache.state.lock().unwrap();
        panic!("simulate interrupted cache mutation");
    })
    .is_err());
    let fresh = cache
        .resolve(&key, &|_| std::future::ready(Ok(vec![address(2)])))
        .await
        .unwrap();
    assert_ne!(old.generation, fresh.generation);
    assert_eq!(fresh.answer.addresses[0], address(2));
    assert!(!cache.state.is_poisoned());
    cache.invalidate(&key, &old).await;
    let retained = cache
        .resolve(&key, &|_| async {
            panic!("recovered answer was discarded")
        })
        .await
        .unwrap();
    assert!(Arc::ptr_eq(
        &fresh.answer.addresses,
        &retained.answer.addresses
    ));
}

#[tokio::test]
async fn dns_coalesces_parallel_server_lookups_without_caching_errors() {
    bounded(async {
        let cache = AddressCache::default();
        let calls = AtomicUsize::new(0);
        let entered = Notify::new();
        let release = Notify::new();
        let resolver = |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            entered.notify_one();
            release.notified().await;
            Ok(vec![address(1)])
        };
        let key = server(443);
        let first = cache.resolve(&key, &resolver);
        let second = async {
            entered.notified().await;
            release.notify_one();
            cache.resolve(&key, &resolver).await
        };
        let (first, second) = tokio::join!(first, second);
        assert!(Arc::ptr_eq(
            &first.unwrap().answer.addresses,
            &second.unwrap().answer.addresses
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let failures = AtomicUsize::new(0);
        let failing = |_| {
            failures.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err(io::Error::other("DNS unavailable")))
        };
        for _ in 0..2 {
            assert!(cache.resolve(&server(444), &failing).await.is_err());
        }
        assert_eq!(failures.load(Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
async fn dns_expiry_and_old_connection_failure_preserve_new_answer() {
    let cache = AddressCache::default();
    let calls = AtomicUsize::new(0);
    let resolver = |_| {
        let port = calls.fetch_add(1, Ordering::SeqCst) as u16 + 1;
        std::future::ready(Ok(vec![address(port)]))
    };
    let key = server(443);
    let old = cache.resolve(&key, &resolver).await.unwrap();
    expire(&old).await;
    let fresh = cache.resolve(&key, &resolver).await.unwrap();
    cache.invalidate(&key, &old).await;
    let retained = cache.resolve(&key, &resolver).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(retained.answer.addresses[0], address(2));
    assert!(Arc::ptr_eq(
        &fresh.answer.addresses,
        &retained.answer.addresses
    ));
    cache.clear();
    let network = cache.resolve(&key, &resolver).await.unwrap();
    cache.invalidate(&key, &fresh).await;
    assert!(Arc::ptr_eq(
        &network.answer.addresses,
        &cache
            .resolve(&key, &resolver)
            .await
            .unwrap()
            .answer
            .addresses
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn dns_failed_connection_waits_for_contended_answer_before_retry() {
    bounded(async {
        let cache = AddressCache::default();
        let key = server(443);
        let lookups = AtomicUsize::new(0);
        let connects = AtomicUsize::new(0);
        let resolver = |_| {
            let port = lookups.fetch_add(1, Ordering::SeqCst) as u16 + 1;
            std::future::ready(Ok(vec![address(port)]))
        };
        let old = cache.resolve(&key, &resolver).await.unwrap();
        let slot = old.slot.as_ref().unwrap();
        let held = std::cell::RefCell::new(None);
        let retry = cache.connect_with(&key, resolver, |addresses| {
            let call = connects.fetch_add(1, Ordering::SeqCst);
            assert_eq!(addresses[0], address(call as u16 + 1));
            std::future::ready(if call == 0 {
                // Model a concurrent cache hit holding the slot when the
                // connection reports failure, without changing its answer.
                held.borrow_mut()
                    .replace(slot.clone().try_lock_owned().unwrap());
                Err(io::Error::other("old route failed"))
            } else {
                Ok(())
            })
        });
        tokio::pin!(retry);
        std::future::poll_fn(|cx| {
            assert!(retry.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
        drop(held.borrow_mut().take());
        retry.await.unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        assert_eq!(lookups.load(Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
async fn dns_contended_invalidation_preserves_replacement_generation() {
    bounded(async {
        let cache = AddressCache::default();
        let key = server(443);
        let old = cache
            .resolve(&key, &|_| std::future::ready(Ok(vec![address(1)])))
            .await
            .unwrap();
        let held = old.slot.as_ref().unwrap().lock().await;
        let invalidate = cache.invalidate(&key, &old);
        tokio::pin!(invalidate);
        std::future::poll_fn(|cx| {
            assert!(invalidate.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        // This must not block behind a global mutex held by invalidation.
        cache.clear();
        let replacement = cache
            .resolve(&key, &|_| std::future::ready(Ok(vec![address(2)])))
            .await
            .unwrap();
        drop(held);
        invalidate.await;
        let retained = cache
            .resolve(&key, &|_| async {
                panic!("replacement answer was invalidated")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(
            &retained.answer.addresses,
            &replacement.answer.addresses
        ));
    })
    .await;
}

#[tokio::test]
async fn dns_network_clear_discards_an_inflight_resolution() {
    bounded(async {
        let cache = AddressCache::default();
        let calls = AtomicUsize::new(0);
        let entered = Notify::new();
        let release = Notify::new();
        let resolver = |_| async {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                entered.notify_one();
                release.notified().await;
            }
            Ok(vec![address(call as u16 + 1)])
        };
        let key = server(443);
        let lookup = cache.resolve(&key, &resolver);
        let reset = async {
            entered.notified().await;
            cache.clear();
            release.notify_one();
        };
        let (lookup, ()) = tokio::join!(lookup, reset);
        let lookup = lookup.unwrap();
        assert_eq!(lookup.generation, 1);
        assert_eq!(lookup.answer.addresses[0], address(2));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(cache.state.lock().unwrap().entries.len(), 1);
        assert_eq!(
            cache
                .resolve(&key, &resolver)
                .await
                .unwrap()
                .answer
                .addresses[0],
            address(2)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
async fn dns_cancelled_lookup_releases_waiters_without_detached_work() {
    bounded(async {
        let cache = AddressCache::default();
        let entered = Notify::new();
        let key = server(443);
        let blocked = |_| async {
            entered.notify_one();
            std::future::pending::<io::Result<Vec<SocketAddr>>>().await
        };
        {
            let pending = cache.resolve(&key, &blocked);
            tokio::pin!(pending);
            tokio::select! {
                _ = &mut pending => panic!("blocked resolver completed"),
                _ = entered.notified() => {},
            }
        }
        let result = cache
            .resolve(&key, &|_| std::future::ready(Ok(vec![address(2)])))
            .await
            .unwrap();
        assert_eq!(result.answer.addresses[0], address(2));
    })
    .await;
}

#[tokio::test]
async fn dns_cache_bounds_preserve_every_address_and_server_port() {
    let cache = AddressCache::default();
    let mut retained = Vec::new();
    for port in 1..=MAX_SERVERS as u16 + 1 {
        retained.push(
            cache
                .resolve(&server(port), &|key: Server| {
                    std::future::ready(Ok(vec![address(key.port)]))
                })
                .await
                .unwrap(),
        );
    }
    assert_eq!(cache.state.lock().unwrap().entries.len(), MAX_SERVERS);
    assert!(retained.last().unwrap().slot.is_none());
    assert_eq!(retained[0].answer.addresses[0], address(1));
    assert_eq!(retained[1].answer.addresses[0], address(2));
    drop(retained);
    let addresses = (0..MAX_CACHED_ADDRESSES + 1)
        .map(|port| address(port as u16 + 1))
        .chain(["[::1]:443".parse().unwrap()])
        .collect::<Vec<_>>();
    let calls = AtomicUsize::new(0);
    let oversized = |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(addresses.clone()))
    };
    for _ in 0..2 {
        let lookup = cache.resolve(&server(999), &oversized).await.unwrap();
        assert_eq!(&*lookup.answer.addresses, addresses.as_slice());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(cache.state.lock().unwrap().entries.len(), MAX_SERVERS);
}

#[tokio::test]
async fn dns_all_address_failure_refreshes_once_with_full_fallback_order() {
    let cache = AddressCache::default();
    let lookups = AtomicUsize::new(0);
    let connects = AtomicUsize::new(0);
    let result = cache
        .connect_with(
            &server(443),
            |_| {
                let call = lookups.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(vec![
                    "[::1]:443".parse().unwrap(),
                    address(call as u16 + 1),
                ]))
            },
            |addresses| {
                let call = connects.fetch_add(1, Ordering::SeqCst);
                assert_eq!(addresses.len(), 2);
                assert!(addresses[0].is_ipv6());
                assert_eq!(addresses[1], address(call as u16 + 1));
                std::future::ready(if call == 0 {
                    Err(io::Error::other("old route failed"))
                } else {
                    Ok(())
                })
            },
        )
        .await;
    result.unwrap();
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    assert_eq!(connects.load(Ordering::SeqCst), 2);
    let failed = AtomicUsize::new(0);
    let result = cache
        .connect_with(
            &server(444),
            |_| std::future::ready(Ok(vec![address(1)])),
            |_| {
                failed.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err::<(), _>(io::Error::other("offline")))
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(failed.load(Ordering::SeqCst), 2);
}

#[test]
fn dns_server_key_excludes_path_and_handles_ip_literals() {
    let first = Server::from_relay(&RelayUrl::parse("wss://relay.invalid/one").unwrap()).unwrap();
    let second =
        Server::from_relay(&RelayUrl::parse("wss://relay.invalid/two?query=one").unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.port, 443);
    let ip = Server::from_relay(&RelayUrl::parse("ws://[::1]:81").unwrap()).unwrap();
    assert_eq!(
        ip.host.parse::<IpAddr>().unwrap(),
        IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
    );
    assert_eq!(ip.port, 81);
}

#[tokio::test]
async fn dns_wire_keeps_hostname_validation_and_anonymous_author_tls_scopes() {
    use super::super::{read_socket_with_transport, tls_tests, Collection, ScopedTlsConnector};
    use nostr::prelude::*;
    use rustls::{HandshakeKind, ProtocolVersion};

    bounded(async {
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
        let relay = tls_tests::relay(true, false, events).await;
        let address = Server::from_relay(&relay.url).unwrap();
        let address = SocketAddr::new(address.host.parse().unwrap(), address.port);
        let url = RelayUrl::parse(&format!("wss://localhost:{}", address.port())).unwrap();
        let key = Server::from_relay(&url).unwrap();
        let alice_filters = vec![Filter::new()
            .author(alice.public_key())
            .kind(Kind::Metadata)];
        let bob_filters = vec![Filter::new().author(bob.public_key()).kind(Kind::Metadata)];
        let alice_tls =
            ScopedTlsConnector::new_with_roots(&url, &alice_filters, tls_tests::roots()).unwrap();
        let bob_tls =
            ScopedTlsConnector::new_with_roots(&url, &bob_filters, tls_tests::roots()).unwrap();
        let cache = AddressCache::default();
        let lookups = AtomicUsize::new(0);
        for (filters, scope) in [
            (&alice_filters, &alice_tls),
            (&alice_filters, &alice_tls),
            (&bob_filters, &bob_tls),
        ] {
            let transport = cache.connect_with(
                &key,
                |_| {
                    lookups.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(vec![address]))
                },
                |addresses| async move { TcpStream::connect(addresses.as_ref()).await },
            );
            let mut collected = Collection::new(false);
            read_socket_with_transport(
                &url,
                filters,
                &mut collected,
                Some(scope.connector(&url, filters).unwrap()),
                transport,
            )
            .await
            .unwrap();
            assert_eq!(collected.events.len(), 1);
        }
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
        assert_eq!(
            *relay.server_names.lock().unwrap(),
            vec![Some("localhost".to_owned()); 3]
        );
        assert_eq!(
            *relay.request_hosts.lock().unwrap(),
            vec![format!("localhost:{}", address.port()); 3]
        );
        let observed = relay.handshakes.lock().unwrap().clone();
        assert_eq!(
            observed.iter().map(|row| row.0).collect::<Vec<_>>(),
            [
                HandshakeKind::Full,
                HandshakeKind::Resumed,
                HandshakeKind::Full
            ]
        );
        assert!(observed
            .iter()
            .all(|(_, version, dhe)| *version == ProtocolVersion::TLSv1_3 && *dhe));
        tls_tests::assert_wire(&relay, &[alice.public_key(), bob.public_key()], 3);
        // A cached address does not authorize the IP's certificate for a
        // different DNS name, even with the test CA explicitly trusted.
        let wrong = RelayUrl::parse(&format!("wss://wrong.invalid:{}", address.port())).unwrap();
        let wrong_tls =
            ScopedTlsConnector::new_with_roots(&wrong, &alice_filters, tls_tests::roots()).unwrap();
        let wrong_key = Server::from_relay(&wrong).unwrap();
        let transport = cache.connect_with(
            &wrong_key,
            |_| std::future::ready(Ok(vec![address])),
            |addresses| async move { TcpStream::connect(addresses.as_ref()).await },
        );
        let mut collected = Collection::new(false);
        let error = read_socket_with_transport(
            &wrong,
            &alice_filters,
            &mut collected,
            Some(wrong_tls.connector(&wrong, &alice_filters).unwrap()),
            transport,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("certificate"), "{error}");
        assert!(collected.events.is_empty());
        tls_tests::assert_wire(&relay, &[alice.public_key(), bob.public_key()], 3);
    })
    .await;
}
