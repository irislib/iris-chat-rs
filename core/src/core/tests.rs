use super::protocol::build_protocol_subscription_filters;
use super::*;
use nostr_double_ratchet_runtime::{NdrRuntime, SessionManagerEvent};

const TEST_PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";

#[derive(Clone)]
struct RuntimeStorageBridgeForTest {
    inner: Arc<dyn StorageAdapter>,
}

impl RuntimeStorageBridgeForTest {
    fn new(inner: Arc<dyn StorageAdapter>) -> Self {
        Self { inner }
    }
}

impl nostr_double_ratchet_runtime::StorageAdapter for RuntimeStorageBridgeForTest {
    fn get(&self, key: &str) -> nostr_double_ratchet_runtime::Result<Option<String>> {
        self.inner
            .get(key)
            .map_err(|error| nostr_double_ratchet_runtime::Error::Storage(error.to_string()))
    }

    fn put(&self, key: &str, value: String) -> nostr_double_ratchet_runtime::Result<()> {
        self.inner
            .put(key, value)
            .map_err(|error| nostr_double_ratchet_runtime::Error::Storage(error.to_string()))
    }

    fn del(&self, key: &str) -> nostr_double_ratchet_runtime::Result<()> {
        self.inner
            .del(key)
            .map_err(|error| nostr_double_ratchet_runtime::Error::Storage(error.to_string()))
    }

    fn list(&self, prefix: &str) -> nostr_double_ratchet_runtime::Result<Vec<String>> {
        self.inner
            .list(prefix)
            .map_err(|error| nostr_double_ratchet_runtime::Error::Storage(error.to_string()))
    }
}

fn runtime_storage_bridge_for_test(
    storage: Arc<dyn StorageAdapter>,
) -> Arc<dyn nostr_double_ratchet_runtime::StorageAdapter> {
    Arc::new(RuntimeStorageBridgeForTest::new(storage))
}

fn seed_protocol_storage_for_test(
    storage: &dyn StorageAdapter,
    seed_session_manager: SessionManagerSnapshot,
    seed_group_manager: GroupManagerSnapshot,
) -> anyhow::Result<()> {
    let state = serde_json::json!({
        "version": 1,
        "session_manager": seed_session_manager,
        "group_manager": seed_group_manager,
        "latest_app_keys_created_at": {},
        "pending_outbound": [],
        "pending_inbound": [],
        "pending_group_fanouts": [],
        "pending_group_pairwise_payloads": [],
        "pending_group_sender_key_messages": [],
        "pending_group_sender_key_repairs": [],
        "pending_decrypted_deliveries": [],
        "subscription_generation": 0,
        "last_backfill_attempt_secs": 0,
    });
    storage.put(TEST_PROTOCOL_ENGINE_STATE_KEY, state.to_string())?;
    Ok(())
}

fn seed_protocol_storage_if_missing_for_test(
    storage: &dyn StorageAdapter,
    seed_session_manager: SessionManagerSnapshot,
    seed_group_manager: GroupManagerSnapshot,
) -> anyhow::Result<()> {
    if storage.get(TEST_PROTOCOL_ENGINE_STATE_KEY)?.is_none() {
        seed_protocol_storage_for_test(storage, seed_session_manager, seed_group_manager)?;
    }
    Ok(())
}

fn protocol_publish_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish) => Some(&publish.event),
            _ => None,
        })
        .collect()
}

fn protocol_publish_events_with_kind(effects: &[ProtocolEffect], kind: u32) -> Vec<Event> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish) if publish.event.kind.as_u16() as u32 == kind => {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .collect()
}

fn protocol_publish_events_for_target(
    effects: &[ProtocolEffect],
    _owner_pubkey_hex: &str,
    _device_id: &str,
) -> Vec<Event> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND =>
            {
                Some(publish.event.clone())
            }
            _ => None,
        })
        .collect()
}

fn protocol_has_publish_target(
    effects: &[ProtocolEffect],
    _owner_pubkey_hex: &str,
    _device_id: &str,
) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            ProtocolEffect::Publish(publish)
                if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
        )
    })
}

fn protocol_targeted_payload_count(effects: &[ProtocolEffect], _owner_pubkey_hex: &str) -> usize {
    effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                ProtocolEffect::Publish(publish)
                    if publish.event.kind.as_u16() as u32 == MESSAGE_EVENT_KIND
            )
        })
        .count()
}

include!("tests/protocol_runtime.rs");
include!("tests/protocol_runtime_replay.rs");
include!("tests/retry_publish_ordering.rs");
include!("tests/protocol_filters_push.rs");
include!("tests/app_keys_invites_requests.rs");
include!("tests/first_contact_receiver.rs");
include!("tests/direct_messages_typing.rs");
include!("tests/direct_messages_runtime_regressions.rs");
include!("tests/direct_group_sender_key_ack.rs");
include!("tests/groups_sender_key.rs");
include!("tests/groups_sender_key_retry.rs");
include!("tests/groups_persistence_helpers.rs");
include!("tests/groups_persistence_more.rs");
