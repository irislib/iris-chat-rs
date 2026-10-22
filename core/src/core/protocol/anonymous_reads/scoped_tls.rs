//! TLS tickets may link connections. Keep them inside one public author's
//! discovery scope and one exact relay URL, never in a shared connector cache.
use super::*;
use rustls::{
    client::{
        ClientSessionMemoryCache, ClientSessionStore, Resumption, Tls12ClientSessionValue,
        Tls12Resumption, Tls13ClientSessionValue,
    },
    pki_types::ServerName,
    ClientConfig, NamedGroup, RootCertStore,
};

/// Enforce TLS 1.3-only resumption at the storage boundary too. In the pinned
/// rustls version, disabling TLS 1.2 tickets alone still lets a memory cache
/// retain session IDs, which can resume without a fresh key exchange.
#[derive(Debug)]
struct Tls13SessionStore {
    cache: ClientSessionMemoryCache,
}

impl Tls13SessionStore {
    fn new() -> Self {
        Self {
            cache: ClientSessionMemoryCache::new(1),
        }
    }
}

impl ClientSessionStore for Tls13SessionStore {
    fn set_kx_hint(&self, server: ServerName<'static>, group: NamedGroup) {
        self.cache.set_kx_hint(server, group);
    }

    fn kx_hint(&self, server: &ServerName<'_>) -> Option<NamedGroup> {
        self.cache.kx_hint(server)
    }

    fn set_tls12_session(&self, _: ServerName<'static>, _: Tls12ClientSessionValue) {}

    fn tls12_session(&self, _: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        None
    }

    fn remove_tls12_session(&self, _: &ServerName<'static>) {}

    fn insert_tls13_ticket(&self, server: ServerName<'static>, value: Tls13ClientSessionValue) {
        self.cache.insert_tls13_ticket(server, value);
    }

    fn take_tls13_ticket(&self, server: &ServerName<'static>) -> Option<Tls13ClientSessionValue> {
        self.cache.take_tls13_ticket(server)
    }
}

pub(in crate::core) struct ScopedTlsConnector {
    relay: RelayUrl,
    filters: Vec<Filter>,
    config: Arc<ClientConfig>,
    #[cfg(test)]
    sessions: Arc<Tls13SessionStore>,
}

impl ScopedTlsConnector {
    pub(in crate::core) fn new(relay: &RelayUrl, filters: &[Filter]) -> Option<Self> {
        Self::from_config(relay, filters, default_config())
    }

    fn from_config(relay: &RelayUrl, filters: &[Filter], baseline: &ClientConfig) -> Option<Self> {
        if !relay.as_str().starts_with("wss://") || sole_public_author(filters).is_none() {
            return None;
        }
        // Rustls rounds a nonzero size up to one server with at most eight TLS
        // 1.3 tickets. The cache also owns key-exchange hints; none are shared.
        let sessions = Arc::new(Tls13SessionStore::new());
        let mut config = baseline.clone();
        config.resumption =
            Resumption::store(sessions.clone()).tls12_resumption(Tls12Resumption::Disabled);
        // TLS 1.3 resumption in rustls requires a fresh DHE exchange. Do not
        // enable replayable early data or TLS 1.2's key-reusing resumption.
        config.enable_early_data = false;
        Some(Self {
            relay: relay.clone(),
            filters: filters.to_vec(),
            config: Arc::new(config),
            #[cfg(test)]
            sessions,
        })
    }

    #[cfg(test)]
    pub(in crate::core) fn new_with_roots(
        relay: &RelayUrl,
        filters: &[Filter],
        roots: RootCertStore,
    ) -> Option<Self> {
        let mut baseline = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        baseline.resumption = Resumption::disabled();
        Self::from_config(relay, filters, &baseline)
    }

    pub(super) fn connector(
        &self,
        relay: &RelayUrl,
        filters: &[Filter],
    ) -> Result<Connector, String> {
        if &self.relay != relay || self.filters != filters || sole_public_author(filters).is_none()
        {
            return Err("anonymous TLS discovery scope changed".into());
        }
        Ok(Connector::Rustls(self.config.clone()))
    }
}

pub(super) fn sole_public_author(filters: &[Filter]) -> Option<PublicKey> {
    let mut expected = None;
    for filter in filters {
        let authors = filter.authors.as_ref()?;
        if authors.len() != 1 {
            return None;
        }
        let author = authors.first().copied()?;
        if expected.is_some_and(|expected| expected != author) {
            return None;
        }
        expected = Some(author);
    }
    expected
}

fn default_config() -> &'static ClientConfig {
    static CONFIG: OnceLock<ClientConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        // Immutable trust/provider state can be shared. This baseline must
        // never own reusable session state, including when it is cloned.
        config.resumption = Resumption::disabled();
        config
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(author: PublicKey) -> Filter {
        Filter::new().author(author).kind(Kind::Metadata)
    }

    #[test]
    fn independent_scopes_never_share_tls_session_or_key_exchange_state() {
        let relay = RelayUrl::parse("wss://relay.example").unwrap();
        let author = Keys::generate().public_key();
        let first = ScopedTlsConnector::new(&relay, &[filter(author)]).unwrap();
        let second = ScopedTlsConnector::new(&relay, &[filter(author)]).unwrap();
        assert!(!Arc::ptr_eq(&first.sessions, &second.sessions));
        let server = ServerName::try_from("relay.example").unwrap().to_owned();
        first
            .sessions
            .set_kx_hint(server.clone(), NamedGroup::X25519);
        assert_eq!(first.sessions.kx_hint(&server), Some(NamedGroup::X25519));
        assert_eq!(second.sessions.kx_hint(&server), None);
        assert!(!first.config.enable_early_data);
        assert!(!second.config.enable_early_data);
    }

    #[test]
    fn reuse_requires_exact_relay_and_unchanged_single_author_filters() {
        let relay = RelayUrl::parse("wss://relay.example/discovery").unwrap();
        let author = Keys::generate().public_key();
        let filters = vec![filter(author)];
        let scope = ScopedTlsConnector::new(&relay, &filters).unwrap();
        assert!(scope.connector(&relay, &filters).is_ok());
        for changed_relay in [
            "wss://relay.example/other",
            "wss://relay.example:444/discovery",
            "wss://other.example/discovery",
        ] {
            assert!(scope
                .connector(&RelayUrl::parse(changed_relay).unwrap(), &filters)
                .is_err());
        }
        assert!(scope
            .connector(&relay, &[filter(Keys::generate().public_key())])
            .is_err());
        assert!(scope.connector(&relay, &[filter(author).limit(5)]).is_err());
    }

    #[test]
    fn unscoped_mixed_and_plain_requests_get_no_resumption_context() {
        let relay = RelayUrl::parse("wss://relay.example").unwrap();
        let first = Keys::generate().public_key();
        let second = Keys::generate().public_key();
        for filters in [
            vec![],
            vec![Filter::new()],
            vec![filter(first), Filter::new()],
            vec![filter(first), filter(second)],
            vec![Filter::new().authors([first, second])],
        ] {
            assert!(ScopedTlsConnector::new(&relay, &filters).is_none());
        }
        assert!(ScopedTlsConnector::new(
            &RelayUrl::parse("ws://127.0.0.1:1234").unwrap(),
            &[filter(first)],
        )
        .is_none());
    }

    #[tokio::test]
    async fn tls12_session_values_cannot_enter_the_scoped_store() {
        use super::super::tls_tests;
        tokio::time::timeout(Duration::from_secs(5), async {
            let author = Keys::generate().public_key();
            let filters = vec![filter(author)];
            let relay = tls_tests::relay(false, true, vec![]).await;
            let server = ServerName::try_from("127.0.0.1").unwrap().to_owned();
            let offered = Arc::new(ClientSessionMemoryCache::new(1));
            let mut probe =
                ScopedTlsConnector::new_with_roots(&relay.url, &filters, tls_tests::roots())
                    .unwrap();
            // Obtain a genuine opaque TLS 1.2 session value; rustls exposes no
            // public constructor. This intentionally unwrapped test-only cache
            // also records the upstream behavior guarded by our storage policy.
            Arc::get_mut(&mut probe.config).unwrap().resumption =
                Resumption::store(offered.clone()).tls12_resumption(Tls12Resumption::Disabled);
            for _ in 0..2 {
                super::super::poll(&relay.url, filters.clone(), Some(&probe))
                    .await
                    .unwrap();
            }
            eprintln!(
                "TLS12 unwrapped-cache regression probe: {:?}",
                relay.handshakes.lock().unwrap()
            );
            let value = offered
                .tls12_session(&server)
                .expect("TLS 1.2 fixture offered a session ID");
            let store = Tls13SessionStore::new();
            store.set_tls12_session(server.clone(), value);
            assert!(store.tls12_session(&server).is_none());
            assert!(store.cache.tls12_session(&server).is_none());
            store.remove_tls12_session(&server);
            assert!(store.tls12_session(&server).is_none());
        })
        .await
        .expect("TLS 1.2 storage proof timed out");
    }
}
