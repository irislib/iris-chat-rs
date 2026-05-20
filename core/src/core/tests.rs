use super::protocol::build_protocol_subscription_filters;
use super::*;
use nostr_double_ratchet_runtime::{NdrRuntime, SessionManagerEvent};

const TEST_PROTOCOL_ENGINE_STATE_KEY: &str = "appcore/protocol-engine-state-v1";

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

include!("tests/protocol_runtime.rs");
include!("tests/protocol_runtime_replay.rs");
include!("tests/protocol_filters_push.rs");
include!("tests/app_keys_invites_requests.rs");
include!("tests/direct_messages_typing.rs");
include!("tests/groups_persistence_helpers.rs");
include!("tests/groups_persistence_more.rs");
