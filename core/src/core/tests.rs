use super::protocol::build_protocol_subscription_filters;
use super::*;

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

#[derive(Clone)]
struct SwitchableFailStorage {
    inner: InMemoryStorage,
    fail_puts: Arc<std::sync::atomic::AtomicBool>,
}

impl SwitchableFailStorage {
    fn new() -> Self {
        Self {
            inner: InMemoryStorage::new(),
            fail_puts: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn set_fail_puts(&self, fail: bool) {
        self.fail_puts
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

impl StorageAdapter for SwitchableFailStorage {
    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.inner.get(key)
    }

    fn put(&self, key: &str, value: String) -> StorageResult<()> {
        if self.fail_puts.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StorageError::new("injected storage failure"));
        }
        self.inner.put(key, value)
    }

    fn del(&self, key: &str) -> StorageResult<()> {
        self.inner.del(key)
    }

    fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.list(prefix)
    }
}

fn protocol_plan_for_test(
    message_authors: Vec<PublicKey>,
    group_sender_key_authors: Vec<PublicKey>,
) -> ProtocolSubscriptionPlan {
    ProtocolSubscriptionPlan {
        runtime_subscriptions: vec!["ndr-protocol".to_string()],
        roster_authors: Vec::new(),
        invite_authors: Vec::new(),
        message_authors: message_authors
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect(),
        message_recipients: Vec::new(),
        group_sender_key_authors: group_sender_key_authors
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect(),
        group_roster_group_ids: Vec::new(),
        group_roster_authors: Vec::new(),
        invite_response_recipient: None,
    }
}

fn runtime_rumor_json(
    author: PublicKey,
    kind: u32,
    content: &str,
    created_at_secs: u64,
    tags: Vec<Vec<String>>,
) -> (String, String) {
    let tags = tags
        .into_iter()
        .map(|tag| nostr::Tag::parse(tag).expect("runtime rumor tag"))
        .collect::<Vec<_>>();
    let mut rumor = UnsignedEvent::new(
        author,
        Timestamp::from_secs(created_at_secs),
        Kind::Custom(kind as u16),
        tags,
        content.to_string(),
    );
    rumor.ensure_id();
    let id = rumor.id.as_ref().expect("runtime rumor id").to_string();
    (
        serde_json::to_string(&rumor).expect("runtime rumor json"),
        id,
    )
}

fn protocol_publish_events(effects: &[ProtocolEffect]) -> Vec<&Event> {
    effects
        .iter()
        .map(|effect| {
            let ProtocolEffect::Publish(publish) = effect;
            &publish.event
        })
        .collect()
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

include!("tests/protocol_startup_guards.rs");
include!("tests/retry_publish_ordering.rs");
include!("tests/app_keys_roster.rs");
include!("tests/app_keys_device_labels.rs");
include!("tests/app_keys_invites_requests.rs");
include!("tests/direct_messages_group_requests.rs");
include!("tests/direct_messages_readiness.rs");
include!("tests/direct_messages_typing.rs");
include!("tests/direct_messages_runtime_regressions.rs");
include!("tests/groups_sender_key.rs");
include!("tests/groups_persistence_helpers.rs");
include!("tests/groups_persistence_more.rs");
