//! Test-only access to the application's live shared FIPS client.
//! No endpoint, transport or protocol is constructed by this adapter.

use std::sync::{Arc, OnceLock, RwLock, Weak};

use fips_core::FipsEndpoint;
use nostr::{EventBuilder, Keys, Kind};
use nostr_pubsub::{EventBus, EventSource, VerifiedEvent};
use nostr_pubsub_fips::FipsPubsubClient;
use serde_json::{json, Value};

struct RegisteredMesh {
    endpoint: Weak<FipsEndpoint>,
    client: Weak<FipsPubsubClient>,
    keys: Keys,
    lan_discovery_enabled: bool,
    relay_count: usize,
}

static MESH: OnceLock<RwLock<Option<RegisteredMesh>>> = OnceLock::new();

pub(crate) fn max_connected_peers() -> usize {
    std::env::var("IRIS_STACK_FIXTURE_PUBSUB_MAX_PEERS")
        .map_or(64, |value| value.parse().unwrap_or(0))
}

pub(crate) fn register(
    endpoint: &Arc<FipsEndpoint>,
    client: &Arc<FipsPubsubClient>,
    keys: Keys,
    lan_discovery_enabled: bool,
    relay_count: usize,
) {
    *MESH.get_or_init(RwLock::default).write().unwrap() = Some(RegisteredMesh {
        endpoint: Arc::downgrade(endpoint),
        client: Arc::downgrade(client),
        keys,
        lan_discovery_enabled,
        relay_count,
    });
}

pub struct StackMeshFixture {
    endpoint: Arc<FipsEndpoint>,
    pub client: Arc<FipsPubsubClient>,
    keys: Keys,
    lan_discovery_enabled: bool,
    relay_count: usize,
}

pub fn stack_mesh_fixture() -> Option<StackMeshFixture> {
    let mesh = MESH.get()?.read().ok()?;
    let mesh = mesh.as_ref()?;
    Some(StackMeshFixture {
        endpoint: mesh.endpoint.upgrade()?,
        client: mesh.client.upgrade()?,
        keys: mesh.keys.clone(),
        lan_discovery_enabled: mesh.lan_discovery_enabled,
        relay_count: mesh.relay_count,
    })
}

impl StackMeshFixture {
    pub async fn publish(&self, kind: u16, content: &str) -> anyhow::Result<Value> {
        let event = EventBuilder::new(Kind::from(kind), content).sign_with_keys(&self.keys)?;
        self.client
            .publish(
                VerifiedEvent::try_from(event.clone())?,
                EventSource::local_index("iris-chat"),
            )
            .await?;
        Ok(json!({"event": "published", "id": event.id.to_string(),
            "pubkey": event.pubkey.to_hex(), "kind": kind, "content": event.content}))
    }

    pub async fn status(&self) -> anyhow::Result<Value> {
        let peers = self.endpoint.peers().await?;
        Ok(json!({
            "event": "status", "npub": self.endpoint.npub(),
            "direct_peers": peers.into_iter().map(|peer| json!({
                "npub": peer.npub, "connected": peer.connected,
                "transport_addr": peer.transport_addr, "transport_type": peer.transport_type,
                "bytes_sent": peer.bytes_sent, "bytes_recv": peer.bytes_recv,
            })).collect::<Vec<_>>(),
            "pubsub_peer_count": self.client.connected_peer_count()?,
            "pubsub_max_peers": self.client.options().max_connected_peers,
            "lan_discovery_enabled": self.lan_discovery_enabled, "relay_count": self.relay_count,
        }))
    }
}
