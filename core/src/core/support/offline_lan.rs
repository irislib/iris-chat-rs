//! Explicit development-harness observations; never part of normal support exports.
use async_trait::async_trait;
use fips_core::{Config, FipsEndpoint};
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, BlobRouteContext, Hash, StoreError};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub(in crate::core) fn enabled() -> bool {
    cfg!(debug_assertions) && std::env::var("IRIS_CHAT_OFFLINE_LAN_FIXTURE").as_deref() == Ok("1")
}

#[derive(Default, Serialize)]
struct Reads {
    local_hits: u64,
    local_misses: u64,
    peer_blocks: u64,
    peer_bytes: u64,
    http_blocks: u64,
    http_bytes: u64,
}

struct Probe {
    epoch: String,
    isolation: bool,
    network: Value,
    reads: Reads,
    local_hashes: BTreeMap<String, u64>,
    peer_hashes: BTreeMap<String, u64>,
    overflowed: bool,
}

fn state() -> &'static Mutex<Option<Probe>> {
    static STATE: OnceLock<Mutex<Option<Probe>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

pub(in crate::core) fn capture_network(
    config: &Config,
    relays: &[String],
    ble: bool,
    same_host: bool,
) {
    if !enabled() {
        return;
    }
    let transports: BTreeMap<_, _> = config.transports.instance_counts().collect();
    // Fixtures archive recent peer hints before startup. Identity-only peers
    // remain valid: LAN discovery still uses the production signed roster.
    let addressed_peers = config.peers.iter().any(|peer| !peer.addresses.is_empty());
    let isolation = relays.is_empty()
        && !ble
        && !same_host
        && config.node.discovery.lan.enabled
        && !config.node.discovery.nostr.enabled
        && !addressed_peers
        && transports.len() == 1
        && transports.contains_key("udp");
    if let Ok(mut state) = state().lock() {
        *state = Some(Probe {
            epoch: format!("{:016x}", rand::random::<u64>()),
            isolation,
            network: json!({"transports": transports, "message_servers": relays.len(),
                "nostr_discovery": config.node.discovery.nostr.enabled,
                "lan_discovery": config.node.discovery.lan.enabled,
                "addressed_peers": addressed_peers, "host_bluetooth": ble, "same_host":same_host}),
            reads: Reads::default(),
            local_hashes: BTreeMap::new(),
            peer_hashes: BTreeMap::new(),
            overflowed: false,
        });
    }
}

fn record_hash(hashes: &mut BTreeMap<String, u64>, hash: &Hash) -> bool {
    let hash = hashtree_core::to_hex(hash);
    if !hashes.contains_key(&hash) && hashes.len() == 256 {
        return false;
    }
    let count = hashes.entry(hash).or_default();
    *count = count.saturating_add(1);
    true
}

pub(in crate::core) fn local_read(hash: &Hash, bytes: Option<&[u8]>) {
    if !enabled() {
        return;
    }
    if let Ok(mut state) = state().lock() {
        if let Some(probe) = state.as_mut() {
            if bytes.is_some_and(|bytes| hashtree_core::sha256(bytes) == *hash) {
                probe.reads.local_hits = probe.reads.local_hits.saturating_add(1);
                probe.overflowed |= !record_hash(&mut probe.local_hashes, hash);
            } else if bytes.is_none() {
                probe.reads.local_misses = probe.reads.local_misses.saturating_add(1);
            }
        }
    }
}

struct ObservedRoute {
    inner: Arc<dyn BlobRoute>,
    peer: bool,
}

impl ObservedRoute {
    fn observe(&self, request: BlobRequest, result: &Result<BlobReply, StoreError>) {
        let Ok(BlobReply::Data(bytes)) = result else {
            return;
        };
        if hashtree_core::sha256(bytes) != request.hash {
            return;
        }
        if let Ok(mut state) = state().lock() {
            if let Some(probe) = state.as_mut() {
                if self.peer {
                    probe.reads.peer_blocks = probe.reads.peer_blocks.saturating_add(1);
                    probe.reads.peer_bytes =
                        probe.reads.peer_bytes.saturating_add(bytes.len() as u64);
                    probe.overflowed |= !record_hash(&mut probe.peer_hashes, &request.hash);
                } else {
                    probe.reads.http_blocks = probe.reads.http_blocks.saturating_add(1);
                    probe.reads.http_bytes =
                        probe.reads.http_bytes.saturating_add(bytes.len() as u64);
                }
            }
        }
    }
}

#[async_trait]
impl BlobRoute for ObservedRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        let result = self.inner.route(request).await;
        self.observe(request, &result);
        result
    }
    async fn route_with_context(
        &self,
        request: BlobRequest,
        context: BlobRouteContext,
    ) -> Result<BlobReply, StoreError> {
        let result = self.inner.route_with_context(request, context).await;
        self.observe(request, &result);
        result
    }
}

pub(in crate::core) fn route(inner: Arc<dyn BlobRoute>, peer: bool) -> Arc<dyn BlobRoute> {
    if enabled() {
        Arc::new(ObservedRoute { inner, peer })
    } else {
        inner
    }
}

fn closed_http_list(servers: &[String]) -> bool {
    !servers.is_empty()
        && servers.iter().all(|server| {
            url::Url::parse(server).is_ok_and(|url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some_and(|port| port != 0)
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
        })
}

pub(super) async fn append(
    bundle: &mut Value,
    endpoint: Option<Arc<FipsEndpoint>>,
    relays_empty: bool,
) {
    if !enabled() {
        return;
    }
    let observation = match endpoint {
        Some(endpoint) => tokio::time::timeout(Duration::from_secs(2), endpoint.peers())
            .await
            .ok()
            .and_then(Result::ok),
        None => None,
    };
    let (read, write) = crate::core::attachment_upload::blossom_servers_from_config();
    let value = if let Ok(state) = state().lock() {
        match (state.as_ref(), observation) {
            (Some(probe), Some(peers)) => {
                let peers: Vec<_> = peers.into_iter().filter(|peer| peer.connected).take(65)
                    .map(|peer| json!({"npub":peer.npub, "transport":peer.transport_type,
                        "address":peer.transport_addr, "route":peer.last_outbound_route,
                        "link_id":peer.link_id, "bytes_sent":peer.bytes_sent, "bytes_recv":peer.bytes_recv}))
                    .collect();
                json!({"schema":1, "valid":!probe.overflowed && peers.len() <= 64,
                    "epoch":probe.epoch, "isolation_valid":probe.isolation && relays_empty
                        && closed_http_list(&read) && closed_http_list(&write),
                    "network":probe.network, "blossom_read_servers":read, "blossom_write_servers":write,
                    "peers":peers, "reads":probe.reads, "local_hash_reads":probe.local_hashes,
                    "peer_hash_reads":probe.peer_hashes})
            }
            _ => json!({"schema":1,"valid":false}),
        }
    } else {
        json!({"schema":1,"valid":false})
    };
    if let Some(bundle) = bundle.as_object_mut() {
        bundle.insert("offline_lan_attachment".into(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    struct ContextRoute;
    #[async_trait]
    impl BlobRoute for ContextRoute {
        async fn route(&self, _: BlobRequest) -> Result<BlobReply, StoreError> {
            panic!("The wrapper must preserve the caller's routing budget")
        }
        async fn route_with_context(
            &self,
            request: BlobRequest,
            context: BlobRouteContext,
        ) -> Result<BlobReply, StoreError> {
            assert_eq!(context.attempt_budget, 3);
            assert!(context.deadline > Instant::now());
            Ok(BlobReply::Data(if request.htl == 1 {
                b"valid".to_vec()
            } else {
                b"corrupt".to_vec()
            }))
        }
    }

    #[tokio::test]
    async fn fixture_route_preserves_budget_and_counts_only_hash_valid_bytes() {
        *state().lock().unwrap() = Some(Probe {
            epoch: "fixture".into(),
            isolation: true,
            network: Value::Null,
            reads: Reads::default(),
            local_hashes: BTreeMap::new(),
            peer_hashes: BTreeMap::new(),
            overflowed: false,
        });
        let route = ObservedRoute {
            inner: Arc::new(ContextRoute),
            peer: true,
        };
        let hash = hashtree_core::sha256(b"valid");
        for htl in [1, 2] {
            let result = route
                .route_with_context(
                    BlobRequest { hash, htl },
                    BlobRouteContext {
                        deadline: Instant::now() + Duration::from_secs(1),
                        attempt_budget: 3,
                    },
                )
                .await
                .unwrap();
            assert!(matches!(result, BlobReply::Data(_)));
        }
        let mut state = state().lock().unwrap();
        let probe = state.take().unwrap();
        assert_eq!(probe.reads.peer_blocks, 1);
        assert_eq!(probe.reads.peer_bytes, 5);
        assert_eq!(probe.peer_hashes[&hashtree_core::to_hex(&hash)], 1);
        assert_eq!(probe.reads.http_blocks, 0);
    }

    #[test]
    fn fixture_requires_explicit_loopback_http_configuration() {
        assert!(closed_http_list(&["http://127.0.0.1:12345".into()]));
        for values in [
            vec![],
            vec!["https://example.com".into()],
            vec!["http://localhost:12345".into()],
            vec!["http://user@127.0.0.1:12345".into()],
            vec!["http://127.0.0.1:12345#key".into()],
        ] {
            assert!(!closed_http_list(&values));
        }
    }
}
